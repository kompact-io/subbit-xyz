//! Claim transactions that have been submitted but not yet observed landing.
//!
//! `subbit-claim` rebuilds its `Queue` from Floe's confirmed-chain view on
//! every poll tick. That view keeps showing an input as live until a tx
//! spending it actually lands, so without a record of our own unconfirmed
//! submissions two consecutive ticks can build mutually-exclusive txs against
//! the same channel input. This tracks every submission across ticks so:
//!
//! - a still-pending tx keeps a real chance to land before its inputs are
//!   touched again (it is rebroadcast at a configurable cadence);
//! - a tx positively known to be gone -- Floe has seen it, or one of its
//!   channel inputs was spent by something else -- frees those inputs for a
//!   genuinely new attempt;
//! - nothing double-submits against an input already covered by an unconfirmed
//!   in-flight tx of our own making.
//!
//! Deliberately stores the submitted CBOR, not the transaction, so a
//! rebroadcast is byte-for-byte idempotent (a second submission of the same
//! tx can never be a conflicting replacement).

use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};

use cardano_sdk::{Hash, Input, Transaction, cbor::ToCbor, transaction::state::ReadyForSigning};

#[derive(Debug, Clone)]
struct PendingTx {
    cbor: Vec<u8>,
    /// Every input the tx spends, including fuel/collateral.
    inputs: BTreeSet<Input>,
    /// The subset of `inputs` that are channel utxos. A missing channel input
    /// means another tx spent it -- this tx can now never land.
    channel_inputs: BTreeSet<Input>,
    last_submitted: Instant,
    /// Consecutive ticks on which one of `channel_inputs` was absent from
    /// Floe's view. Requiring more than one avoids mistaking a transiently
    /// incomplete search result for a genuinely spent input.
    missing_observations: u8,
    /// Multiplies the base rebroadcast interval -- grown each time amaru rejects a
    /// rebroadcast of this exact tx as a duplicate (it already has this tx; our own
    /// confirmation view via `read_tx` just hasn't caught up yet), reset to 1 on a
    /// rebroadcast that doesn't hit that rejection. Without this, a tx that's really
    /// just slow to confirm gets hammered with a resubmission every tick for as long
    /// as the confirmation lag lasts, observed live to run several minutes.
    duplicate_backoff: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Resolution {
    /// Floe observed the tx -- it landed.
    Landed,
    /// One of the tx's own channel inputs was spent by another tx, so this
    /// transaction is permanently unlandable.
    Dead,
}

#[derive(Debug, Clone, Default)]
pub struct Pending {
    txs: BTreeMap<Hash<32>, PendingTx>,
}

impl Pending {
    pub fn is_empty(&self) -> bool {
        self.txs.is_empty()
    }

    pub fn len(&self) -> usize {
        self.txs.len()
    }

    pub fn ids(&self) -> impl Iterator<Item = Hash<32>> + '_ {
        self.txs.keys().copied()
    }

    /// Tracks an already-submitted, signed transaction. `channel_inputs` must
    /// be the subset of its inputs that reference tracked channel utxos --
    /// only those can be invalidated by a competing chain step.
    pub fn insert(&mut self, tx: &Transaction<ReadyForSigning>, channel_inputs: BTreeSet<Input>) {
        // Collateral is spent too, even though it is not part of `inputs()` --
        // reusing it as another tx's fuel or collateral would make the two
        // txs mutually exclusive, so it belongs in the blocked set.
        let inputs = tx.inputs().chain(tx.collaterals()).collect();
        self.insert_submitted(
            tx.id(),
            tx.to_cbor(),
            inputs,
            channel_inputs,
            Instant::now(),
        );
    }

    /// Shared by `insert` and tests, so the tracking/resolution logic can be
    /// exercised without building and signing a real transaction.
    fn insert_submitted(
        &mut self,
        id: Hash<32>,
        cbor: Vec<u8>,
        inputs: BTreeSet<Input>,
        channel_inputs: BTreeSet<Input>,
        now: Instant,
    ) {
        self.txs.insert(
            id,
            PendingTx {
                cbor,
                inputs,
                channel_inputs,
                last_submitted: now,
                missing_observations: 0,
                duplicate_backoff: 1,
            },
        );
    }

    /// Every input currently covered by an unconfirmed submission. Callers
    /// must exclude these from both channel staging and fuel selection.
    pub fn inputs(&self) -> BTreeSet<Input> {
        self.txs
            .values()
            .flat_map(|tx| tx.inputs.iter().cloned())
            .collect()
    }

    /// Drops every tx that is positively resolved. `landed` is the set whose
    /// id Floe reported via `ReadTx`; `live_channels` is every channel input
    /// currently present in Floe's confirmed view. Anything still pending is
    /// left untouched.
    pub fn advance(
        &mut self,
        landed: &BTreeSet<Hash<32>>,
        live_channels: &BTreeSet<Input>,
    ) -> Vec<(Hash<32>, Resolution)> {
        // A single absence can be an incomplete search snapshot, so require
        // the input to stay missing across two ticks before calling the tx
        // dead. Freeing inputs on a false positive would allow a genuinely
        // conflicting re-submission -- the exact bug this module exists to
        // prevent.
        const MISSING_TICKS_BEFORE_DEAD: u8 = 2;

        let mut resolved = Vec::new();
        for id in self.txs.keys().copied().collect::<Vec<_>>() {
            let Some(tx) = self.txs.get_mut(&id) else {
                continue;
            };
            if landed.contains(&id) {
                self.txs.remove(&id);
                resolved.push((id, Resolution::Landed));
                continue;
            }
            let missing = tx
                .channel_inputs
                .iter()
                .any(|input| !live_channels.contains(input));
            if !missing {
                tx.missing_observations = 0;
                continue;
            }
            tx.missing_observations = tx.missing_observations.saturating_add(1);
            if tx.missing_observations >= MISSING_TICKS_BEFORE_DEAD {
                self.txs.remove(&id);
                resolved.push((id, Resolution::Dead));
            }
        }
        resolved
    }

    /// Caps `duplicate_backoff`'s growth at 32x the base interval (e.g. 32x a 5s
    /// interval is ~2.5min between attempts) -- high enough to stop hammering a
    /// merely-slow-to-confirm tx, not so high that a genuinely dead-but-misreported
    /// one goes unnoticed for long.
    const MAX_DUPLICATE_BACKOFF: u32 = 32;

    /// The still-pending txs whose last submission was at least `interval` (scaled by
    /// that tx's own `duplicate_backoff`, if it's been repeatedly rejected as a
    /// duplicate) ago, as `(id, cbor)` pairs ready to hand straight back to Floe.
    pub fn due_for_rebroadcast(
        &self,
        now: Instant,
        interval: Duration,
    ) -> Vec<(Hash<32>, Vec<u8>)> {
        self.txs
            .iter()
            .filter(|(_, tx)| {
                now.duration_since(tx.last_submitted) >= interval * tx.duplicate_backoff
            })
            .map(|(id, tx)| (*id, tx.cbor.clone()))
            .collect()
    }

    /// A rebroadcast actually went out and wasn't rejected as a duplicate -- reset
    /// any backoff this tx had accrued.
    pub fn note_rebroadcast(&mut self, ids: &[Hash<32>], now: Instant) {
        for id in ids {
            if let Some(tx) = self.txs.get_mut(id) {
                tx.last_submitted = now;
                tx.duplicate_backoff = 1;
            }
        }
    }

    /// amaru rejected the rebroadcast as a duplicate -- it already has this tx, so
    /// resubmitting again right away would just hit the same rejection. Push the next
    /// attempt out further each time this keeps happening, rather than retrying on
    /// the same fixed cadence as a genuinely failed send.
    pub fn note_duplicate_rejected(&mut self, ids: &[Hash<32>], now: Instant) {
        for id in ids {
            if let Some(tx) = self.txs.get_mut(id) {
                tx.last_submitted = now;
                tx.duplicate_backoff =
                    (tx.duplicate_backoff.saturating_mul(2)).min(Self::MAX_DUPLICATE_BACKOFF);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input(seed: u8, index: u64) -> Input {
        Input::new(Hash::<32>::from([seed; 32]), index)
    }

    fn id(seed: u8) -> Hash<32> {
        Hash::<32>::from([seed; 32])
    }

    fn pending(inputs: Vec<Input>, channel_inputs: Vec<Input>, now: Instant) -> Pending {
        let mut pending = Pending::default();
        pending.insert_submitted(
            id(1),
            vec![0xc0],
            inputs.into_iter().collect(),
            channel_inputs.into_iter().collect(),
            now,
        );
        pending
    }

    #[test]
    fn blocks_every_input_while_pending() {
        let now = Instant::now();
        let channel = input(2, 0);
        let fuel = input(3, 0);
        let pending = pending(vec![channel.clone(), fuel.clone()], vec![channel], now);
        assert_eq!(pending.inputs(), BTreeSet::from([input(2, 0), input(3, 0)]));
    }

    #[test]
    fn stays_pending_while_channel_input_is_live() {
        let now = Instant::now();
        let channel = input(2, 0);
        let mut pending = pending(vec![channel.clone()], vec![channel.clone()], now);
        let resolved = pending.advance(&BTreeSet::new(), &BTreeSet::from([channel]));
        assert!(resolved.is_empty());
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn landed_tx_is_removed_and_frees_inputs() {
        let now = Instant::now();
        let channel = input(2, 0);
        let mut pending = pending(vec![channel.clone()], vec![channel.clone()], now);
        let resolved = pending.advance(&BTreeSet::from([id(1)]), &BTreeSet::from([channel]));
        assert_eq!(resolved, vec![(id(1), Resolution::Landed)]);
        assert!(pending.is_empty());
        assert!(pending.inputs().is_empty());
    }

    #[test]
    fn one_missing_observation_is_not_enough_to_call_a_tx_dead() {
        let now = Instant::now();
        let channel = input(2, 0);
        let mut pending = pending(vec![channel.clone()], vec![channel.clone()], now);
        // A transient gap in one search result must not free the input.
        assert!(
            pending
                .advance(&BTreeSet::new(), &BTreeSet::new())
                .is_empty()
        );
        assert_eq!(pending.len(), 1);
        // Seeing it live again resets the suspicion.
        assert!(
            pending
                .advance(&BTreeSet::new(), &BTreeSet::from([channel]))
                .is_empty()
        );
        assert_eq!(pending.len(), 1);
    }

    #[test]
    fn a_spent_channel_input_makes_the_tx_dead_after_two_ticks() {
        let now = Instant::now();
        let channel = input(2, 0);
        let mut pending = pending(vec![channel], vec![input(2, 0)], now);
        assert!(
            pending
                .advance(&BTreeSet::new(), &BTreeSet::new())
                .is_empty()
        );
        let resolved = pending.advance(&BTreeSet::new(), &BTreeSet::new());
        assert_eq!(resolved, vec![(id(1), Resolution::Dead)]);
        assert!(pending.is_empty());
    }

    #[test]
    fn multi_channel_tx_dies_if_any_channel_input_was_spent() {
        let now = Instant::now();
        let live = input(2, 0);
        let spent = input(3, 0);
        let mut pending = pending(
            vec![live.clone(), spent.clone()],
            vec![live.clone(), spent],
            now,
        );
        assert!(
            pending
                .advance(&BTreeSet::new(), &BTreeSet::from([live.clone()]))
                .is_empty()
        );
        let resolved = pending.advance(&BTreeSet::new(), &BTreeSet::from([live]));
        assert_eq!(resolved, vec![(id(1), Resolution::Dead)]);
    }

    #[test]
    fn only_pending_txs_past_the_interval_are_due_for_rebroadcast() {
        let now = Instant::now();
        let channel = input(2, 0);
        let mut pending = pending(vec![channel.clone()], vec![channel], now);
        assert!(
            pending
                .due_for_rebroadcast(now, Duration::from_secs(10))
                .is_empty()
        );
        assert_eq!(
            pending
                .due_for_rebroadcast(now + Duration::from_secs(10), Duration::from_secs(10))
                .len(),
            1
        );

        pending.note_rebroadcast(&[id(1)], now + Duration::from_secs(10));
        assert!(
            pending
                .due_for_rebroadcast(now + Duration::from_secs(15), Duration::from_secs(10))
                .is_empty()
        );
    }

    #[test]
    fn a_duplicate_rejection_doubles_the_wait_each_time_until_capped() {
        let now = Instant::now();
        let channel = input(2, 0);
        let mut pending = pending(vec![channel.clone()], vec![channel], now);
        let interval = Duration::from_secs(10);

        // First duplicate rejection: next attempt waits 2x the base interval, not 1x.
        pending.note_duplicate_rejected(&[id(1)], now);
        assert!(
            pending
                .due_for_rebroadcast(now + interval, interval)
                .is_empty()
        );
        assert_eq!(
            pending
                .due_for_rebroadcast(now + interval * 2, interval)
                .len(),
            1
        );

        // A second consecutive duplicate rejection doubles it again, to 4x.
        pending.note_duplicate_rejected(&[id(1)], now + interval * 2);
        assert!(
            pending
                .due_for_rebroadcast(now + interval * 2 + interval * 3, interval)
                .is_empty()
        );
        assert_eq!(
            pending
                .due_for_rebroadcast(now + interval * 2 + interval * 4, interval)
                .len(),
            1
        );
    }

    #[test]
    fn a_clean_rebroadcast_resets_any_accrued_duplicate_backoff() {
        let now = Instant::now();
        let channel = input(2, 0);
        let mut pending = pending(vec![channel.clone()], vec![channel], now);
        let interval = Duration::from_secs(10);

        pending.note_duplicate_rejected(&[id(1)], now);
        pending.note_rebroadcast(&[id(1)], now + interval * 2);

        // Back to the base interval (1x), not still waiting out the earlier 2x backoff:
        // due again after one more base interval, not two.
        assert_eq!(
            pending
                .due_for_rebroadcast(now + interval * 2 + interval, interval)
                .len(),
            1
        );
    }
}
