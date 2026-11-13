use std::cmp;
use std::collections::BTreeMap;

use cardano_sdk::{
    Address, ChangeStrategy, Hash, Input, IsTransactionBodyState, Output, PlutusData, SlotBound,
    Transaction, Value, address::kind, cbor::ToCbor, transaction::state,
    transaction::state::ReadyForSigning,
};

use subbit_core::{Hash28, Instant};
use subbit_l1_dto::{Redeemer, Step};

use crate::{Bounds, Open, accounting, to_plutus_data, validator};
use crate::{
    Channel, FEE_BUFFER, Lovelace, NetworkParameters, Utxos,
    channel::FromOutputError,
    fuel,
    step::{self, Want, Will, WillCont, WillEol},
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("no channel utxo for this input")]
    MissingInput,
    #[error("output does not carry the subbit validator's own reference script")]
    NotReferenceScript,
    #[error(transparent)]
    Channel(#[from] FromOutputError),
    #[error(transparent)]
    Step(#[from] crate::step::Error),
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum BuildError {
    #[error("reference script required when stepping channels")]
    MissingReference,
    #[error("change address required -- call set_change_address first")]
    MissingChangeAddress,
    #[error(transparent)]
    Fuel(#[from] fuel::SelectError),
    #[error("cannot balance tx: {0}")]
    Balancing(String),
    #[error("nothing eligible to pick")]
    Empty,
    #[error("a pick is already pending -- build or unpick it first")]
    AlreadyPicked,
    #[error(
        "built tx ({actual}B) still exceeds max_tx_size ({limit}B) after exhausting backoff retries"
    )]
    Oversized { actual: usize, limit: usize },
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct BuildConfig {
    /// Byte budget `pick` drains the queue against (see
    /// `accounting::Accounting`), in place of a flat per-kind headcount.
    /// Defaults to the real protocol `maxTxSize` (unchanged since
    /// Alonzo, same on preprod/mainnet) -- *not* a safety-margined
    /// value, since `pick`'s own accounting only covers the
    /// Wills'/Opens' own marginal cost (steps, redeemers, outputs,
    /// signers), not fuel/collateral/change/fee; a caller wanting
    /// headroom for those should set this lower, and
    /// `console::submit`'s own back-off still shrinks it further on
    /// an actual `MaxTxSizeUTxO` rejection.
    pub max_tx_size: usize,
    pub min_collateral_value: u64,
    pub fee_buffer: Lovelace,
    /// Shrink factor for `build`'s own retry-on-oversize loop, as a
    /// fraction of 256 (plain integer math, no float -- same trick
    /// `ChangeStrategy`-adjacent ledger code already leans on
    /// elsewhere): after a build whose *real*, measured CBOR length
    /// exceeds `max_tx_size`, the next attempt's budget is
    /// `attempt_max * backoff / 256`, re-picked from scratch (see
    /// `build`'s own doc). `0` disables retrying outright -- `build`
    /// returns `BuildError::Oversized` on the very first overflow
    /// rather than ever re-picking a smaller batch. Doesn't touch
    /// `max_tx_size` itself; the shrink is local to one `build` call.
    pub backoff: u8,
    /// How many times `build` will shrink-and-retry (via `backoff`)
    /// before giving up and returning `BuildError::Oversized` (or, on
    /// an ExUnits overrun, propagating `BuildError::Balancing`).
    /// Raised from the original `3` to `10`: an ExUnits overrun (see
    /// `build`'s own doc on `OutOfExError`) can take several shrink
    /// steps to actually drop a will, since `Accounting`'s byte
    /// estimate has no correlation to a will's real execution cost --
    /// a few percent off the byte budget can still pick the exact
    /// same wills. Each retry is a local rebuild (no chain
    /// round-trip), so more headroom here is cheap.
    pub retries: u8,
}

/// Cardano's protocol-wide `maxTxSize` -- unchanged since Alonzo,
/// identical on preprod and mainnet. The real, hard ceiling -- not a
/// usable default on its own; see `default_max_tx_size`.
pub const MAX_TX_SIZE: usize = 16_384;

/// Fixed slack on top of the one wallet witness `default_max_tx_size` already reserves,
/// covering the other costs `Accounting` deliberately has no opinion on (its own doc:
/// "no opinion on fuel, collateral, or change") -- one collateral input and one change
/// output, always present whenever any Will is in the batch, plus a few bytes of CBOR
/// integer-width rounding (e.g. the fee field crossing a byte-width boundary). Measured
/// live: a real claim batch still came in 4 bytes over with zero slack here; this is
/// deliberately generous rather than exact, same spirit as the witness margin itself.
const EXTRA_MARGIN: usize = 150;

/// `MAX_TX_SIZE` less margin for one additional wallet witness, plus `EXTRA_MARGIN` --
/// `BuildConfig::default`'s own `max_tx_size`. `Accounting` already prices a witness per
/// *required signatory* (e.g. the provider, for Sub/Settle), but the wallet paying fuel/
/// collateral/fee signs too, purely because it's spending its own inputs rather than
/// because it's named as a required signer -- so neither `Accounting` nor `Queue::build`'s
/// own pre-signing size check ever sees that witness coming (see `assemble`'s own doc).
/// Reserving room for exactly one such witness is deliberately conservative rather than
/// exact: some deployments use the same key for the wallet and the provider, in which case
/// that witness is already covered by the provider's own and this margin goes unused; the
/// common case is two distinct keys, where this is the margin that's actually needed.
pub fn default_max_tx_size() -> usize {
    MAX_TX_SIZE - step::vkey_witness_bytes() - EXTRA_MARGIN
}

impl Default for BuildConfig {
    fn default() -> Self {
        Self {
            max_tx_size: default_max_tx_size(),
            min_collateral_value: 4_000_000,
            fee_buffer: FEE_BUFFER,
            backoff: 244,
            retries: 10,
        }
    }
}

impl BuildConfig {
    /// Selects fuel inputs to cover whatever `produced` needs beyond
    /// `spent`, plus `fee_buffer`.
    fn select_fuel(
        &self,
        fuel: &Utxos,
        spent: &Value<u64>,
        produced: &Value<u64>,
    ) -> Result<Vec<Input>, BuildError> {
        let usable = usable_fuel(fuel);
        let mut required = produced.clone();
        required.with_lovelace(required.lovelace() + self.fee_buffer);
        let target = fuel::saturating_sub_value(&required, spent);
        Ok(fuel::select(&usable, &target)?)
    }

    /// One plain utxo of at least `min_collateral_value` -- `[]` when
    /// no script runs at all (a pure `Open` batch).
    fn select_collateral(&self, has_wills: bool, fuel: &Utxos) -> Vec<Input> {
        if !has_wills {
            return Vec::new();
        }
        collateral_candidates(fuel)
            .into_iter()
            .find(|(_, output)| output.value().lovelace() >= self.min_collateral_value)
            .map(|(input, _)| vec![input])
            .unwrap_or_default()
    }
}

/// The slow-changing part of a `Queue`: network, policy, reference
/// script, change address. Survives many pick/build cycles, unlike
/// `utxos`/`channels`/the staged batches, which cycle every refresh.
#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Settings {
    network_parameters: NetworkParameters,
    build_config: BuildConfig,
    reference_utxo: Option<(Input, Output)>,
    change_address: Option<Address<kind::Any>>,
}

impl Settings {
    pub fn new(network_parameters: NetworkParameters, build_config: BuildConfig) -> Self {
        Self {
            network_parameters,
            build_config,
            reference_utxo: None,
            change_address: None,
        }
    }

    pub fn network_parameters(&self) -> &NetworkParameters {
        &self.network_parameters
    }

    pub fn build_config(&self) -> &BuildConfig {
        &self.build_config
    }

    pub fn set_build_config(&mut self, build_config: BuildConfig) {
        self.build_config = build_config;
    }

    pub fn set_reference_utxo(&mut self, input: Input, output: Output) -> Result<(), Error> {
        if output.script() != Some(&crate::VALIDATOR.script) {
            return Err(Error::NotReferenceScript);
        }
        self.reference_utxo = Some((input, output));
        Ok(())
    }

    pub fn reference_utxo(&self) -> Option<&(Input, Output)> {
        self.reference_utxo.as_ref()
    }

    pub fn clear_reference_utxo(&mut self) {
        self.reference_utxo = None;
    }

    pub fn set_change_address(&mut self, change_address: Address<kind::Any>) {
        self.change_address = Some(change_address);
    }

    pub fn change_address(&self) -> Option<&Address<kind::Any>> {
        self.change_address.as_ref()
    }

    pub fn clear_change_address(&mut self) {
        self.change_address = None;
    }
}

impl Default for Settings {
    fn default() -> Self {
        Self::new(
            NetworkParameters::for_network("preprod").expect("preprod always parses"),
            BuildConfig::default(),
        )
    }
}

/// How a staged `Want` is paired to the channel it applies against.
/// `ByInput` freezes the pairing to one exact utxo, same as always --
/// if that utxo is spent by anything else before `pick`, the want is
/// gone (via `evict_invalid`/a failed resolve in `pick_at`). `ById`
/// instead re-resolves against *whichever* utxo currently backs that
/// `iou_key`+`tag` identity, fresh every time `pick_at` (or a refresh
/// via `Queue::drain_wants`/re-propose) runs -- so a want like
/// "this consumer wants an Add" survives its channel's own utxo
/// moving underneath it (e.g. a concurrent `subbit-claim` Sub/Settle
/// landing on the same channel), instead of silently going stale.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum Ref {
    ById(String),
    ByInput(Input),
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Batch {
    #[cfg_attr(feature = "serde", serde(with = "as_pairs"))]
    wants: BTreeMap<Ref, Want>,
    opens: BTreeMap<String, Open>,
}

impl Batch {
    fn is_empty(&self) -> bool {
        self.wants.is_empty() && self.opens.is_empty()
    }
}

/// What `pick` selected, plus the `Bounds` it was selected against and
/// the `Will`s derived for `batch.wants` at that moment -- all three
/// always set together, so they live together. `wills` is computed
/// once, here, not re-derived by every assembly step that needs it.
#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
struct Picked {
    bounds: Bounds,
    #[cfg_attr(feature = "serde", serde(with = "as_pairs"))]
    wills: BTreeMap<Input, Will>,
    batch: Batch,
}

impl Picked {
    fn is_empty(&self) -> bool {
        self.batch.is_empty()
    }

    /// Total value of `wills`' own existing channel utxos, looked up
    /// in `utxos` (the caller's -- `Picked` doesn't own that map).
    fn spent(&self, utxos: &BTreeMap<Input, Output>) -> Value<u64> {
        self.wills.keys().filter_map(|input| utxos.get(input)).fold(
            Value::new(0),
            |mut acc, output| {
                acc.add(output.value());
                acc
            },
        )
    }
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Queue {
    settings: Settings,
    #[cfg_attr(feature = "serde", serde(with = "as_pairs"))]
    utxos: BTreeMap<Input, Output>,
    #[cfg_attr(feature = "serde", serde(with = "as_pairs"))]
    channels: BTreeMap<Input, Channel>,
    batch: Batch,
    picked: Picked,
    #[cfg_attr(feature = "serde", serde(with = "as_pairs"))]
    in_flight: BTreeMap<Hash<32>, Batch>,
}

impl Queue {
    pub fn new(network_parameters: NetworkParameters, build_config: BuildConfig) -> Self {
        Self::from_settings(Settings::new(network_parameters, build_config))
    }

    pub fn from_settings(settings: Settings) -> Self {
        Self {
            settings,
            utxos: BTreeMap::new(),
            channels: BTreeMap::new(),
            batch: Batch::default(),
            picked: Picked::default(),
            in_flight: BTreeMap::new(),
        }
    }

    pub fn settings(&self) -> &Settings {
        &self.settings
    }

    pub fn settings_mut(&mut self) -> &mut Settings {
        &mut self.settings
    }

    pub fn add_utxos(&mut self, utxos: BTreeMap<Input, Output>) -> Result<(), Error> {
        let channels: BTreeMap<Input, Channel> = utxos
            .iter()
            .map(|(input, output)| Ok((input.clone(), Channel::try_from(output)?)))
            .collect::<Result<_, FromOutputError>>()?;
        self.channels.extend(channels);
        self.utxos.extend(utxos);
        Ok(())
    }

    /// Drops every tracked channel for which `pred` returns false,
    /// along with any staged want against it -- resolved *before* the
    /// channel's removed, so a `Ref::ById` want currently backed by a
    /// dropped channel is caught too, not just a frozen `ByInput` one.
    pub fn keep_if(&mut self, pred: impl Fn(&Channel) -> bool) {
        let drop: Vec<Input> = self
            .channels
            .iter()
            .filter(|(_, c)| !pred(c))
            .map(|(i, _)| i.clone())
            .collect();
        let drop_set: std::collections::BTreeSet<&Input> = drop.iter().collect();
        let dead_refs: Vec<Ref> = self
            .batch
            .wants
            .keys()
            .filter(|r| {
                resolve_ref(r, &self.channels).is_some_and(|(input, _)| drop_set.contains(&input))
            })
            .cloned()
            .collect();

        for input in &drop {
            self.channels.remove(input);
            self.utxos.remove(input);
        }
        for r in dead_refs {
            self.batch.wants.remove(&r);
        }
    }

    pub fn channels(&self) -> &BTreeMap<Input, Channel> {
        &self.channels
    }

    pub fn wills(&self) -> BTreeMap<Input, Will> {
        wills_for(&self.batch.wants, &self.channels)
    }

    /// Pairs `want` to this exact utxo -- see `Ref::ByInput`'s own doc.
    pub fn propose(&mut self, input: Input, want: Want) -> Result<(), Error> {
        self.propose_ref(Ref::ByInput(input), want)
    }

    /// Pairs `want` to whichever utxo currently backs the channel
    /// identified by `keytag` (`iou_key.to_string() + tag.to_string()`,
    /// same convention as `Channel::keytag`) -- re-resolved fresh at
    /// every `pick_at`, not frozen at propose time. See `Ref::ById`'s
    /// own doc.
    pub fn propose_by_id(&mut self, keytag: impl Into<String>, want: Want) -> Result<(), Error> {
        self.propose_ref(Ref::ById(keytag.into()), want)
    }

    /// Shared body of `propose`/`propose_by_id` -- also used directly
    /// by a caller (e.g. `Ctx::basket_refresh_txos`) re-staging wants
    /// it drained from a prior `Queue` against a freshly rebuilt one,
    /// preserving each want's original `Ref` kind rather than
    /// collapsing it to whatever input it last resolved to.
    pub fn propose_ref(&mut self, r: Ref, want: Want) -> Result<(), Error> {
        let (_, channel) = resolve_ref(&r, &self.channels).ok_or(Error::MissingInput)?;
        channel
            .clone()
            .try_will(want.clone())
            .map_err(|boxed| boxed.1)?;
        self.batch.wants.insert(r, want);
        Ok(())
    }

    /// Removes whichever staged want currently resolves to `input`,
    /// regardless of whether it was staged `ByInput` or `ById`.
    pub fn drop_intent(&mut self, input: &Input) -> Option<Want> {
        if let Some(want) = self.batch.wants.remove(&Ref::ByInput(input.clone())) {
            return Some(want);
        }
        let matching = self
            .batch
            .wants
            .keys()
            .find(|r| {
                matches!(r, Ref::ById(_))
                    && resolve_ref(r, &self.channels).is_some_and(|(i, _)| &i == input)
            })
            .cloned()?;
        self.batch.wants.remove(&matching)
    }

    pub fn drop_all_intents(&mut self) {
        self.batch.wants.clear();
    }

    /// Drops every staged want that no longer validates against the
    /// latest known channel state -- including a `Ref::ById` want
    /// whose identity no longer matches *any* tracked channel at all
    /// (the channel's genuinely gone), not just one whose exact input
    /// vanished.
    pub fn evict_invalid(&mut self) -> BTreeMap<Ref, Want> {
        let dead: Vec<Ref> = self
            .batch
            .wants
            .iter()
            .filter(|(r, want)| match resolve_ref(r, &self.channels) {
                None => true,
                Some((_, channel)) => channel.clone().try_will((*want).clone()).is_err(),
            })
            .map(|(r, _)| r.clone())
            .collect();
        dead.into_iter()
            .filter_map(|r| self.batch.wants.remove(&r).map(|want| (r, want)))
            .collect()
    }

    /// Drops every staged want whose own bound has permanently
    /// elapsed -- unlike `evict_invalid`, this never resolves itself.
    pub fn evict_expired(&mut self, now: Instant) -> BTreeMap<Ref, Want> {
        let dead: Vec<Ref> = resolve_wills(&self.batch.wants, &self.channels)
            .into_iter()
            .filter(|(_, _, will)| will.bounds().has_expired(now))
            .map(|(r, _, _)| r)
            .collect();
        dead.into_iter()
            .filter_map(|r| self.batch.wants.remove(&r).map(|want| (r, want)))
            .collect()
    }

    /// Everything currently staged (not yet picked), handed to the
    /// caller so it can be re-proposed (via `propose_ref`) against a
    /// freshly rebuilt `Queue` without losing each want's `Ref` kind
    /// -- `Ctx::basket_refresh_txos`'s own use case.
    pub fn drain_wants(&mut self) -> BTreeMap<Ref, Want> {
        std::mem::take(&mut self.batch.wants)
    }

    pub fn opens(&self) -> &BTreeMap<String, Open> {
        &self.batch.opens
    }

    pub fn picked_wills(&self) -> BTreeMap<Input, Will> {
        self.picked.wills.clone()
    }

    pub fn picked_opens(&self) -> &BTreeMap<String, Open> {
        &self.picked.batch.opens
    }

    pub fn add_open(&mut self, open: Open) {
        let keytag = open.channel().constants().iou_key().to_string()
            + &open.channel().constants().tag().to_string();
        self.batch.opens.insert(keytag, open);
    }

    pub fn drop_open(&mut self, keytag: &str) -> Option<Open> {
        self.batch.opens.remove(keytag)
    }

    pub fn drop_all_opens(&mut self) {
        self.batch.opens.clear();
    }

    /// Drains the queue into `self.picked`, in strict priority order
    /// (`will_priority`, with every `Open` ranked below every `Will`
    /// kind -- new business is never as urgent as rescuing an
    /// existing channel), against `build_config`'s byte budget
    /// (`max_tx_size`, see `accounting::Accounting`) instead of a flat
    /// per-kind headcount: the first candidate that would push the
    /// running `Accounting::size` past that budget stops the drain
    /// right there (the batch is "saturated"), rather than skipping
    /// ahead to look for a smaller one that might still fit. Errors
    /// if a pick is already pending, or if nothing ends up eligible.
    /// A will whose own bounds can't coexist with those already picked
    /// (a `Close`'s upper against an `Elapse`'s later lower) is skipped,
    /// not batched -- it stays staged for a later pick.
    pub fn pick(&mut self, bounds: Bounds) -> Result<(), BuildError> {
        let max_size = self.settings.build_config().max_tx_size;
        self.pick_at(bounds, max_size)
    }

    /// `pick`'s own body, against an explicit `max_size` rather than
    /// always reading `build_config().max_tx_size` -- the seam
    /// `build`'s own backoff loop re-picks through at a shrunk
    /// budget, without needing to mutate `settings` to do it.
    fn pick_at(&mut self, bounds: Bounds, max_size: usize) -> Result<(), BuildError> {
        if !self.picked.is_empty() {
            return Err(BuildError::AlreadyPicked);
        }

        enum Candidate {
            // The `Ref` as originally staged, plus the `Input` it
            // resolves to *right now* -- resolved once here, at the
            // top of `pick_at`, and reused rather than re-resolved
            // (`self.channels` doesn't change within one `pick_at`
            // call).
            Will(Ref, Input, Will),
            Open(String),
        }

        // One rank past `will_priority`'s lowest (`End` = 5): every
        // Open is less urgent than any existing channel's own step.
        const OPEN_PRIORITY: u8 = 6;

        let mut candidates: Vec<(u8, Candidate)> = self
            .batch
            .wants
            .iter()
            .filter_map(|(r, want)| {
                let (input, channel) = resolve_ref(r, &self.channels)?;
                let will = channel.clone().try_will(want.clone()).ok()?;
                will.bounds().intersect(&bounds).is_feasible().then(|| {
                    (
                        will_priority(&will),
                        Candidate::Will(r.clone(), input, will),
                    )
                })
            })
            .chain(
                self.batch
                    .opens
                    .keys()
                    .map(|keytag| (OPEN_PRIORITY, Candidate::Open(keytag.clone()))),
            )
            .collect();
        candidates.sort_by(|a, b| {
            a.0.cmp(&b.0).then_with(|| match (&a.1, &b.1) {
                (Candidate::Will(_, xi, wa), Candidate::Will(_, yi, wb)) => {
                    match (sub_gain(wa), sub_gain(wb)) {
                        // Within the `Sub` priority tier, the batch's own byte budget
                        // caps how many fit -- so pick the most valuable ones first,
                        // not an arbitrary (if deterministic) Input order.
                        (Some(ga), Some(gb)) => gb.cmp(&ga).then_with(|| xi.cmp(yi)),
                        _ => xi.cmp(yi),
                    }
                }
                (Candidate::Open(x), Candidate::Open(y)) => x.cmp(y),
                (Candidate::Will(..), Candidate::Open(_)) => cmp::Ordering::Less,
                (Candidate::Open(_), Candidate::Will(..)) => cmp::Ordering::Greater,
            })
        });

        let mut accounting = accounting::Accounting::default();
        let mut picked_refs = Vec::new();
        let mut picked_keytags = Vec::new();
        // Narrowed to the actual intersection of every picked Will's own bounds (e.g. a
        // `Close`'s finite upper, a validator-enforced requirement -- see `close.ak`'s own
        // `expect Finite(upper_bound)`) -- starts at the caller's `bounds` and only ever
        // tightens, since `intersect` never widens. Using the caller's `bounds` verbatim
        // (as this used to) left the tx's own validity interval unbounded whenever the
        // caller passed `Bounds::default()` (every current caller does), regardless of
        // what was actually picked -- a tx the validator then rejects outright.
        let mut picked_bounds = bounds;

        for (_, candidate) in candidates {
            let fits = match &candidate {
                Candidate::Will(_, input, will) => {
                    let Some(channel) = self.channels.get(input) else {
                        continue;
                    };
                    if !picked_bounds.intersect(&will.bounds()).is_feasible() {
                        continue;
                    }
                    let signer = will.signer(channel.constants());
                    accounting.try_add_will(will, signer, max_size)
                }
                Candidate::Open(keytag) => {
                    let Some(open) = self.batch.opens.get(keytag) else {
                        continue;
                    };
                    accounting.try_add_open(open, max_size)
                }
            };
            if !fits {
                break;
            }
            match candidate {
                Candidate::Will(r, _, will) => {
                    picked_bounds = picked_bounds.intersect(&will.bounds());
                    picked_refs.push(r);
                }
                Candidate::Open(keytag) => picked_keytags.push(keytag),
            }
        }

        // Evicted from `batch` straight into `picked` -- no window
        // where a want sits duplicated in both.
        let picked_wants: BTreeMap<Ref, Want> = picked_refs
            .into_iter()
            .filter_map(|r| self.batch.wants.remove(&r).map(|want| (r, want)))
            .collect();
        let picked_opens: BTreeMap<String, Open> = picked_keytags
            .into_iter()
            .filter_map(|keytag| self.batch.opens.remove(&keytag).map(|open| (keytag, open)))
            .collect();

        if picked_wants.is_empty() && picked_opens.is_empty() {
            return Err(BuildError::Empty);
        }

        let wills = wills_for(&picked_wants, &self.channels);
        self.picked = Picked {
            bounds: picked_bounds,
            wills,
            batch: Batch {
                wants: picked_wants,
                opens: picked_opens,
            },
        };
        Ok(())
    }

    /// Undoes a pending `pick`, merging it back into the staged
    /// batch. Idempotent.
    pub fn unpick(&mut self) {
        let picked = std::mem::take(&mut self.picked);
        self.batch.wants.extend(picked.batch.wants);
        self.batch.opens.extend(picked.batch.opens);
    }

    /// Assembles `self.picked` into one transaction, using
    /// `picked_bounds` as the tx's own validity interval verbatim,
    /// retrying against a shrunk budget (`BuildConfig::backoff`/
    /// `retries`) if the real, measured result is oversized -- see
    /// `build`'s own doc for why this exists as a loop around this.
    fn assemble(&self, fuel: &Utxos) -> Result<Transaction<ReadyForSigning>, BuildError> {
        if self.picked.is_empty() {
            return Err(BuildError::Empty);
        }
        let change_address = self
            .settings
            .change_address()
            .cloned()
            .ok_or(BuildError::MissingChangeAddress)?;

        let reference_utxo = self.settings.reference_utxo().cloned();
        let reference_inputs: Vec<Input> = reference_utxo.iter().map(|x| x.0.clone()).collect();
        if !self.picked.wills.is_empty() && reference_inputs.is_empty() {
            return Err(BuildError::MissingReference);
        }

        let outputs = self.tx_outputs();
        let spent = self.picked.spent(&self.utxos);
        let produced = outputs.iter().fold(Value::new(0), |mut acc, output| {
            acc.add(output.value());
            acc
        });
        // `usable_fuel` deliberately allows a script-bearing utxo as fuel -- but the
        // reference-script utxo itself is already going in as a reference input above, and
        // amaru rejects a tx that spends the very utxo it also references. Only relevant
        // when the wallet paying fuel is the same one hosting the reference script.
        let fuel: Utxos = match &reference_utxo {
            Some((ref_input, _)) => fuel
                .iter()
                .filter(|(input, _)| *input != ref_input)
                .map(|(i, o)| (i.clone(), o.clone()))
                .collect(),
            None => fuel.clone(),
        };
        let fuel = &fuel;
        let build_config = self.settings.build_config();
        let fuel_inputs = build_config.select_fuel(fuel, &spent, &produced)?;
        let collaterals = build_config.select_collateral(!self.picked.wills.is_empty(), fuel);

        let inputs: Vec<_> = self
            .tx_inputs()
            .into_iter()
            .map(|(input, data)| (input, Some(data)))
            .chain(fuel_inputs.iter().map(|i| (i.clone(), None)))
            .collect();
        let specified_signatories = self.tx_signers();
        let (lower_bound, upper_bound) = self.validity_interval();

        // Only inputs actually used in this tx get resolved -- never
        // a full scan of `self.utxos`/`fuel`.
        let mut resolved_inputs: BTreeMap<Input, Output> = BTreeMap::new();
        for input in self.picked.wills.keys() {
            if let Some(output) = self.utxos.get(input) {
                resolved_inputs.insert(input.clone(), output.clone());
            }
        }
        for input in fuel_inputs.iter().chain(collaterals.iter()) {
            if let Some(output) = fuel.get(input) {
                resolved_inputs
                    .entry(input.clone())
                    .or_insert_with(|| output.clone());
            }
        }
        if let Some((input, output)) = &reference_utxo {
            resolved_inputs.insert(input.clone(), output.clone());
        }

        let build_result = Transaction::build(
            &self.settings.network_parameters().protocol_parameters,
            &resolved_inputs,
            |transaction| {
                transaction
                    .with_inputs(inputs.clone())
                    .with_collaterals(collaterals.clone())
                    .with_reference_inputs(reference_inputs.clone())
                    .with_outputs(outputs.clone())
                    .with_specified_signatories(specified_signatories.clone())
                    .with_validity_interval(lower_bound, upper_bound)
                    .with_change_strategy(ChangeStrategy::as_last_output(change_address.clone()))
                    .ok()
            },
        );

        let tx = match build_result {
            Ok(tx) => tx,
            Err(err) => {
                // `Transaction::build` evaluates every script itself (to measure real
                // execution units for fee estimation) -- a Plutus failure happens *inside*
                // that call, before it ever hands back a `Transaction` to dump. Replaying
                // the same `with_*` calls directly (skipping `build`'s own fee/change-
                // finalization loop) gets a stand-in tx that still carries the exact same
                // inputs/outputs/redeemers the real attempt did, which is all `aiken tx
                // simulate` needs to reproduce the failure.
                let mut attempt = Transaction::<state::InConstruction>::default();
                attempt
                    .with_inputs(inputs.clone())
                    .with_collaterals(collaterals.clone())
                    .with_reference_inputs(reference_inputs.clone())
                    .with_outputs(outputs.clone())
                    .with_specified_signatories(specified_signatories.clone())
                    .with_validity_interval(lower_bound, upper_bound);
                dump_for_simulate(&attempt, &resolved_inputs, "-failed");
                return Err(BuildError::Balancing(format!("{err:#}")));
            }
        };

        dump_for_simulate(&tx, &resolved_inputs, "");
        Ok(tx)
    }

    /// `assemble`'s own result is measured against `max_tx_size`
    /// directly -- the *real* CBOR length, not `pick`'s own
    /// `accounting::Accounting` estimate (which deliberately never
    /// accounted for fuel/collateral/change/fee in the first place;
    /// see `accounting.rs`'s module doc) -- so this is the first
    /// point an overrun can actually be caught, offline, before ever
    /// reaching a chain's own `MaxTxSizeUTxO` rejection.
    ///
    /// On an overrun, with `backoff` nonzero and retries remaining:
    /// `unpick`s everything back to `batch`, shrinks the budget to
    /// `attempt_max * backoff / 256`, and re-`pick_at`s + reassembles
    /// from scratch against that smaller budget -- a clean re-
    /// derivation each time, not an incremental trim of the existing
    /// selection, so the same priority order (`pick`'s own doc)
    /// always governs which wills/opens survive a shrink. Exhausting
    /// `retries` (or `backoff == 0`) returns `BuildError::Oversized`
    /// instead of ever submitting something the chain would reject.
    ///
    /// Leaves `self.picked` untouched on failure (final shrink
    /// included -- `assemble`'s own doc); on success, moves it
    /// (already evicted from `batch` by `pick`) into `in_flight`
    /// keyed by the built tx's id.
    pub fn build(&mut self, fuel: &Utxos) -> Result<Transaction<ReadyForSigning>, BuildError> {
        let build_config = self.settings.build_config().clone();
        let bounds = self.picked.bounds.clone();
        let mut attempt_max = build_config.max_tx_size;
        let mut retries_left = build_config.retries;

        loop {
            let tx = match self.assemble(fuel) {
                Ok(tx) => tx,
                // A real tx whose script execution cost exceeds the protocol's per-tx
                // ExUnits budget surfaces here as a `Balancing` error (from `uplc`'s own
                // `OutOfExError`, evaluated inside `Transaction::build` itself) rather
                // than as a post-build size overrun below -- `Accounting` has no opinion
                // on ExUnits at all (its own module doc), so a byte-budget-feasible batch
                // can still overrun on CPU/mem alone. Treated the same as an oversized tx:
                // shrink and re-pick, since both mean "picked too much."
                Err(BuildError::Balancing(msg))
                    if msg.contains("OutOfExError")
                        && build_config.backoff > 0
                        && retries_left > 0 =>
                {
                    retries_left -= 1;
                    let next_max = attempt_max * build_config.backoff as usize / 256;
                    tracing::warn!(
                        attempt_max,
                        next_max,
                        retries_left,
                        "build: real tx exceeds its ExUnits budget, shrinking and re-picking"
                    );
                    attempt_max = next_max;
                    self.unpick();
                    self.pick_at(bounds.clone(), attempt_max)?;
                    continue;
                }
                Err(e) => return Err(e),
            };
            let actual = tx.to_cbor().len();
            if actual > build_config.max_tx_size {
                if build_config.backoff > 0 && retries_left > 0 {
                    retries_left -= 1;
                    let next_max = attempt_max * build_config.backoff as usize / 256;
                    tracing::warn!(
                        actual,
                        limit = build_config.max_tx_size,
                        attempt_max,
                        next_max,
                        retries_left,
                        "build: real tx exceeds max_tx_size, shrinking and re-picking"
                    );
                    attempt_max = next_max;
                    self.unpick();
                    self.pick_at(bounds.clone(), attempt_max)?;
                    continue;
                }
                return Err(BuildError::Oversized {
                    actual,
                    limit: build_config.max_tx_size,
                });
            }

            let picked = std::mem::take(&mut self.picked);
            self.in_flight.insert(tx.id(), picked.batch);
            return Ok(tx);
        }
    }

    pub fn pick_and_build(
        &mut self,
        bounds: Bounds,
        fuel: &Utxos,
    ) -> Result<Transaction<ReadyForSigning>, BuildError> {
        self.pick(bounds)?;
        self.build(fuel)
    }

    pub fn in_flight(&self) -> bool {
        !self.in_flight.is_empty()
    }

    pub fn in_flight_ids(&self) -> impl Iterator<Item = Hash<32>> + '_ {
        self.in_flight.keys().copied()
    }

    pub fn finalize(&mut self, id: Hash<32>) -> bool {
        self.in_flight.remove(&id).is_some()
    }

    pub fn rollback(&mut self, id: Hash<32>) -> bool {
        let Some(batch) = self.in_flight.remove(&id) else {
            return false;
        };
        self.batch.wants.extend(batch.wants);
        self.batch.opens.extend(batch.opens);
        true
    }

    /// Continuing outputs (from `self.picked.wills`) followed by
    /// new-channel outputs (from `self.picked.batch.opens`).
    fn tx_outputs(&self) -> Vec<Output> {
        let cont = self
            .picked
            .wills
            .iter()
            .filter_map(|(input, will)| match will {
                Will::Cont {
                    output: channel, ..
                } => {
                    let pre_output = self.utxos.get(input)?;
                    Some(
                        Output::new(pre_output.address().clone(), channel.buffered_value())
                            .with_datum(to_plutus_data(channel.datum())),
                    )
                }
                Will::Eol { .. } => None,
            });
        let network_id = self.settings.network_parameters().network_id;
        let opens = self.picked.batch.opens.values().map(|open| {
            Output::new(
                validator::address(network_id, open.delegation()).into(),
                open.channel().buffered_value(),
            )
            .with_datum(to_plutus_data(open.channel().datum()))
        });
        cont.chain(opens).collect()
    }

    fn tx_inputs(&self) -> Vec<(Input, PlutusData<'static>)> {
        let mut steps = Some(
            self.picked
                .wills
                .values()
                .map(Will::to_step)
                .collect::<Vec<Step>>(),
        );
        self.picked
            .wills
            .keys()
            .enumerate()
            .map(|(i, input)| {
                let redeemer = if i == 0 {
                    Redeemer::Main(steps.take().expect("only index 0 consumes steps"))
                } else {
                    Redeemer::Defer
                };
                (input.clone(), to_plutus_data(redeemer))
            })
            .collect()
    }

    fn tx_signers(&self) -> Vec<Hash<28>> {
        let mut signers: Vec<Hash28> = self
            .picked
            .wills
            .iter()
            .filter_map(|(input, will)| {
                let constants = self.channels.get(input)?.constants();
                Some(will.signer(constants))
            })
            .collect();
        signers.sort();
        signers.dedup();
        signers
            .into_iter()
            .map(|x| Hash::<28>::from(<[u8; 28]>::from(x)))
            .collect()
    }

    fn validity_interval(&self) -> (SlotBound, SlotBound) {
        let protocol_parameters = &self.settings.network_parameters().protocol_parameters;
        let to_slot = |d: Instant| protocol_parameters.posix_to_slot(*d);
        let lower = self
            .picked
            .bounds
            .lower
            .map_or(SlotBound::None, |d| SlotBound::Inclusive(to_slot(d)));
        let upper = self
            .picked
            .bounds
            .upper
            .map_or(SlotBound::None, |d| SlotBound::Exclusive(to_slot(d)));
        (lower, upper)
    }
}

/// Dumps `tx` to `tx-dump-<id><tag>-{tx,inputs,outputs}.txt` in the current directory, in
/// the exact three-file hex-CBOR shape `aiken tx simulate` takes (see
/// `scripts/aiken-simulate.sh`) -- lets a tx the chain later rejects (or one that failed to
/// build locally, via `assemble`'s own stand-in reconstruction -- `tag` is `"-failed"` then)
/// be replayed locally against a trace-enabled validator override, without having to
/// reconstruct it by hand. Opt-in via the `tx_dump` tracing target (`RUST_LOG=tx_dump=info`),
/// not always-on I/O.
fn dump_for_simulate<State: IsTransactionBodyState>(
    tx: &Transaction<State>,
    resolved_inputs: &BTreeMap<Input, Output>,
    tag: &str,
) {
    if !tracing::enabled!(target: "tx_dump", tracing::Level::INFO) {
        return;
    }
    let id = format!("{}{tag}", tx.id());
    let (inputs, outputs): (Vec<Input>, Vec<Output>) = resolved_inputs
        .iter()
        .map(|(input, output)| (input.clone(), output.clone()))
        .unzip();
    for (suffix, bytes) in [
        ("tx", tx.to_cbor()),
        ("inputs", inputs.to_cbor()),
        ("outputs", outputs.to_cbor()),
    ] {
        let path = format!("tx-dump-{id}-{suffix}.txt");
        if let Err(error) = std::fs::write(&path, hex::encode(bytes)) {
            tracing::warn!(target: "tx_dump", %path, %error, "failed to write tx dump file");
        }
    }
    tracing::info!(target: "tx_dump", %id, "dumped tx for `aiken tx simulate`");
}

/// `Some(gain)` for a `Sub`, `None` for every other `Will` kind -- used to break ties
/// within `pick`'s `Sub` priority tier by most-valuable-first instead of Input order.
fn sub_gain(will: &Will) -> Option<u64> {
    match will {
        Will::Cont {
            step: WillCont::Sub { gain, .. },
            ..
        } => Some(*gain),
        _ => None,
    }
}

fn will_priority(will: &Will) -> u8 {
    match will {
        Will::Cont {
            step: WillCont::Settle { .. },
            ..
        } => 0,
        Will::Cont {
            step: WillCont::Close { .. },
            ..
        } => 1,
        Will::Eol {
            step: WillEol::Elapse { .. },
        } => 2,
        Will::Cont {
            step: WillCont::Sub { .. },
            ..
        } => 3,
        Will::Cont {
            step: WillCont::Add { .. },
            ..
        } => 4,
        Will::Eol { step: WillEol::End } => 5,
    }
}

fn wills_for(
    wants: &BTreeMap<Ref, Want>,
    channels: &BTreeMap<Input, Channel>,
) -> BTreeMap<Input, Will> {
    wants
        .iter()
        .filter_map(|(r, want)| {
            let (input, channel) = resolve_ref(r, channels)?;
            let will = channel.clone().try_will(want.clone()).ok()?;
            Some((input, will))
        })
        .collect()
}

/// Resolves one staged `Ref` against the latest known channel state:
/// `ByInput` is a direct lookup, `ById` searches for whichever channel's
/// own `iou_key`+`tag` identity currently matches -- first match wins,
/// same tolerance `propose_by_id`'s predecessor had (a `keytag` is
/// expected unique per provider in practice).
fn resolve_ref<'a>(
    r: &Ref,
    channels: &'a BTreeMap<Input, Channel>,
) -> Option<(Input, &'a Channel)> {
    match r {
        Ref::ByInput(input) => channels.get(input).map(|c| (input.clone(), c)),
        Ref::ById(id) => channels
            .iter()
            .find(|(_, c)| &c.keytag() == id)
            .map(|(input, c)| (input.clone(), c)),
    }
}

/// Like `wills_for`, but keeps each resolved `Will`'s originating
/// `Ref` alongside it -- needed wherever a caller has to remove the
/// *staged* want (keyed by `Ref`) that produced a given resolved
/// `Will`, not just read it (`evict_expired`'s own use case).
fn resolve_wills(
    wants: &BTreeMap<Ref, Want>,
    channels: &BTreeMap<Input, Channel>,
) -> Vec<(Ref, Input, Will)> {
    wants
        .iter()
        .filter_map(|(r, want)| {
            let (input, channel) = resolve_ref(r, channels)?;
            let will = channel.clone().try_will(want.clone()).ok()?;
            Some((r.clone(), input, will))
        })
        .collect()
}

/// A datum-*hash* fuel input isn't accommodated -- spending it would
/// need the matching datum bytes as witness data, which this builder
/// never tracks. An inline datum is already in the output, so it
/// spends fine as plain fuel. Everything else -- including a
/// reference-script-bearing utxo -- is fair game; fuel quality is the
/// caller's call, not this builder's.
fn usable_fuel(fuel: &Utxos) -> Utxos {
    fuel.iter()
        .filter(|(_, output)| !matches!(output.datum(), Some(cardano_sdk::Datum::Hash(_))))
        .map(|(i, o)| (i.clone(), o.clone()))
        .collect()
}

/// Collateral candidates: no datum, no script, no native assets.
fn collateral_candidates(fuel: &Utxos) -> Utxos {
    fuel.iter()
        .filter(|(_, output)| {
            output.datum().is_none()
                && output.script().is_none()
                && output.value().assets().is_empty()
        })
        .map(|(i, o)| (i.clone(), o.clone()))
        .collect()
}

/// serde_json cannot handle non-string map keys.
#[cfg(feature = "serde")]
mod as_pairs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<K, V, S>(map: &BTreeMap<K, V>, s: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize + Ord,
        V: Serialize,
        S: Serializer,
    {
        map.iter().collect::<Vec<_>>().serialize(s)
    }

    pub fn deserialize<'de, K, V, D>(d: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Ok(Vec::<(K, V)>::deserialize(d)?.into_iter().collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cardano_sdk::{Credential, Hash, cbor::ToCbor};
    use subbit_core::{Duration, Tag, VerifyingKey};
    use subbit_l1_dto::{Asset, Constants, Stage};

    use crate::Variables;

    /// Pinned so a future change to `step::vkey_witness_bytes`'s own
    /// measured value is caught here too, not just where it's defined.
    #[test]
    fn default_max_tx_size_reserves_one_wallet_witness() {
        assert_eq!(
            default_max_tx_size(),
            MAX_TX_SIZE - step::vkey_witness_bytes() - EXTRA_MARGIN
        );
    }

    fn network_parameters() -> NetworkParameters {
        NetworkParameters::for_network("preprod").unwrap()
    }

    fn change_address() -> Address<kind::Any> {
        cardano_sdk::Address::new(
            network_parameters().network_id,
            Credential::from_key(Hash::<28>::from([9u8; 28])),
        )
        .into()
    }

    fn fuel() -> Utxos {
        let fee_input = Input::new(Hash::<32>::from([4u8; 32]), 0);
        let fee_output = Output::new(change_address(), Value::new(50_000_000));
        BTreeMap::from([(fee_input, fee_output)])
    }

    fn constants(tag_byte: u8) -> Constants {
        Constants::new(
            Tag::from(vec![tag_byte]),
            Asset::Lovelace,
            VerifyingKey::from([1u8; 32]),
            Hash28::from([2u8; 28]),
            Hash28::from([3u8; 28]),
            Duration::from_secs(900),
        )
    }

    #[test]
    fn pick_and_build_caps_opens_and_leaves_the_rest_pending() {
        // `build`'s own real-size check (not just `pick`'s marginal
        // `Accounting` estimate) now has to fit too, so the budget
        // below is measured off a real 2-Open build (fuel/change/fee
        // included), not `Open::size_estimate() * 2` -- that alone
        // was always an underestimate of the real total by design
        // (see `accounting.rs`'s module doc), and would just trigger
        // `backoff` here instead of capping cleanly at 2.
        let two_opens_size = {
            let mut probe = Queue::new(
                network_parameters(),
                BuildConfig {
                    max_tx_size: usize::MAX,
                    ..BuildConfig::default()
                },
            );
            probe.settings_mut().set_change_address(change_address());
            for i in 0..2u8 {
                probe.add_open(Open::new(
                    Channel::new(
                        constants(i),
                        Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
                    ),
                    None,
                ));
            }
            probe
                .pick_and_build(Bounds::default(), &fuel())
                .expect("2 Opens should build with no cap")
                .to_cbor()
                .len()
        };

        // Room for exactly 2 Opens, not a 3rd.
        let config = BuildConfig {
            max_tx_size: two_opens_size,
            ..BuildConfig::default()
        };
        let mut queue = Queue::new(network_parameters(), config);
        queue.settings_mut().set_change_address(change_address());
        for i in 0..5u8 {
            queue.add_open(Open::new(
                Channel::new(
                    constants(i),
                    Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
                ),
                None,
            ));
        }
        assert_eq!(queue.opens().len(), 5);

        let built = queue
            .pick_and_build(Bounds::default(), &fuel())
            .expect("a 2-open batch out of 5 staged should build");
        assert!(!built.to_cbor().is_empty());
        assert_eq!(queue.opens().len(), 3);

        queue.finalize(built.id());

        let built2 = queue
            .pick_and_build(Bounds::default(), &fuel())
            .expect("a second batch should pick up the remaining Opens");
        assert!(!built2.to_cbor().is_empty());
        assert_eq!(queue.opens().len(), 1);
    }

    /// A budget that `pick`'s own marginal `Accounting` estimate says
    /// 2 Opens fit under, but the *real* built tx (fuel/change/fee
    /// included) doesn't -- with `backoff` disabled, `build` must
    /// fail outright rather than silently submit something oversized.
    #[test]
    fn build_errors_oversized_when_backoff_is_disabled() {
        let one_open = Open::new(
            Channel::new(
                constants(0),
                Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
            ),
            None,
        );
        // Comfortably above the marginal estimate for 2 Opens, but
        // (per the previous test's own measurement) well under their
        // real built size once fuel/change/fee are included.
        let max_tx_size = one_open.size_estimate() * 2 + 50;
        let config = BuildConfig {
            max_tx_size,
            backoff: 0,
            ..BuildConfig::default()
        };
        let mut queue = Queue::new(network_parameters(), config);
        queue.settings_mut().set_change_address(change_address());
        for i in 0..2u8 {
            queue.add_open(Open::new(
                Channel::new(
                    constants(i),
                    Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
                ),
                None,
            ));
        }

        queue
            .pick(Bounds::default())
            .expect("2 Opens fit the marginal estimate");
        let result = queue.build(&fuel());
        assert!(
            matches!(result, Err(BuildError::Oversized { limit, .. }) if limit == max_tx_size),
            "{result:?}"
        );
    }

    /// Same setup as `build_errors_oversized_when_backoff_is_disabled`,
    /// but with retries available: `build` must shrink, re-pick, and
    /// succeed with fewer Opens -- restaging whatever it had to drop
    /// back to `batch`, not losing it.
    #[test]
    fn build_shrinks_and_retries_when_backoff_is_enabled() {
        let one_open = Open::new(
            Channel::new(
                constants(0),
                Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
            ),
            None,
        );
        let max_tx_size = one_open.size_estimate() * 2 + 50;
        let config = BuildConfig {
            max_tx_size,
            backoff: 240,
            retries: 3,
            ..BuildConfig::default()
        };
        let mut queue = Queue::new(network_parameters(), config);
        queue.settings_mut().set_change_address(change_address());
        for i in 0..2u8 {
            queue.add_open(Open::new(
                Channel::new(
                    constants(i),
                    Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
                ),
                None,
            ));
        }

        queue
            .pick(Bounds::default())
            .expect("2 Opens fit the marginal estimate");
        let built = queue
            .build(&fuel())
            .expect("build should shrink-and-retry down to a size that actually fits");
        assert!(built.to_cbor().len() <= max_tx_size);
        // The Open bumped by the shrink is back in `batch`, not lost.
        assert_eq!(queue.opens().len(), 1);
    }

    #[test]
    fn rollback_restages_a_built_batchs_own_opens() {
        let mut queue = Queue::new(network_parameters(), BuildConfig::default());
        queue.settings_mut().set_change_address(change_address());
        queue.add_open(Open::new(
            Channel::new(
                constants(1),
                Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
            ),
            None,
        ));

        let built = queue
            .pick_and_build(Bounds::default(), &fuel())
            .expect("a single Open should build");
        assert_eq!(queue.opens().len(), 0);

        assert!(queue.rollback(built.id()));
        assert_eq!(queue.opens().len(), 1);
        assert!(!queue.in_flight());
    }

    #[test]
    fn datum_hash_fuel_is_never_selected() {
        let mut queue = Queue::new(network_parameters(), BuildConfig::default());
        queue.settings_mut().set_change_address(change_address());
        queue.add_open(Open::new(
            Channel::new(
                constants(1),
                Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
            ),
            None,
        ));

        let datum_input = Input::new(Hash::<32>::from([7u8; 32]), 0);
        let datum_output = Output::new(change_address(), Value::new(90_000_000))
            .with_datum_hash(Hash::<32>::from([1u8; 32]));
        let fuel = BTreeMap::from([(datum_input, datum_output)]);

        let result = queue.pick_and_build(Bounds::default(), &fuel);
        assert!(matches!(result, Err(BuildError::Fuel(_))));
    }

    #[test]
    fn inline_datum_fuel_is_still_selected() {
        let mut queue = Queue::new(network_parameters(), BuildConfig::default());
        queue.settings_mut().set_change_address(change_address());
        queue.add_open(Open::new(
            Channel::new(
                constants(1),
                Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
            ),
            None,
        ));

        let inline_input = Input::new(Hash::<32>::from([7u8; 32]), 0);
        let inline_output =
            Output::new(change_address(), Value::new(90_000_000)).with_datum(to_plutus_data(42u64));
        let fuel = BTreeMap::from([(inline_input, inline_output)]);

        let result = queue.pick_and_build(Bounds::default(), &fuel);
        assert!(
            result.is_ok(),
            "an inline-datum fuel input is spendable as-is"
        );
    }

    #[test]
    fn pick_skips_a_will_whose_bounds_conflict_with_those_already_picked() {
        let mut queue = Queue::new(network_parameters(), BuildConfig::default());

        let close_channel = Channel::new(
            constants(1),
            Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
        );
        let elapse_channel = Channel::new(
            constants(2),
            Variables::new(
                5_000_000,
                Stage::Closed {
                    subbed: 0,
                    elapse_at: Instant::from_secs(0),
                },
            ),
        );
        let validator_address: Address<kind::Any> =
            validator::address(network_parameters().network_id, None).into();
        let close_input = Input::new(Hash::<32>::from([1u8; 32]), 0);
        let elapse_input = Input::new(Hash::<32>::from([2u8; 32]), 0);
        queue
            .add_utxos(BTreeMap::from([
                (
                    close_input.clone(),
                    Output::new(validator_address.clone(), close_channel.buffered_value())
                        .with_datum(to_plutus_data(close_channel.datum())),
                ),
                (
                    elapse_input.clone(),
                    Output::new(validator_address.clone(), elapse_channel.buffered_value())
                        .with_datum(to_plutus_data(elapse_channel.datum())),
                ),
            ]))
            .unwrap();
        queue
            .propose(
                close_input.clone(),
                Want::Close {
                    upper: Instant::from_secs(100),
                },
            )
            .unwrap();
        queue
            .propose(
                elapse_input.clone(),
                Want::Elapse {
                    lower: Instant::from_secs(200),
                },
            )
            .unwrap();

        queue.pick(Bounds::default()).unwrap();
        assert_eq!(queue.picked.batch.wants.len(), 1);
        assert!(queue.picked.bounds.is_feasible());
        assert_eq!(
            queue.batch.wants.len(),
            1,
            "the conflicting will stays staged"
        );
    }

    #[test]
    fn pick_errors_if_already_picked() {
        let mut queue = Queue::new(network_parameters(), BuildConfig::default());
        queue.add_open(Open::new(
            Channel::new(
                constants(1),
                Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
            ),
            None,
        ));

        queue.pick(Bounds::default()).unwrap();
        assert!(matches!(
            queue.pick(Bounds::default()),
            Err(BuildError::AlreadyPicked)
        ));

        queue.unpick();
        assert!(queue.pick(Bounds::default()).is_ok());
    }

    #[test]
    fn pick_evicts_from_batch_not_just_copies_into_picked() {
        let mut queue = Queue::new(network_parameters(), BuildConfig::default());
        queue.add_open(Open::new(
            Channel::new(
                constants(1),
                Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
            ),
            None,
        ));

        queue.pick(Bounds::default()).unwrap();
        assert_eq!(
            queue.opens().len(),
            0,
            "a picked Open must be gone from batch, not duplicated in both"
        );
        assert_eq!(queue.picked_opens().len(), 1);

        queue.unpick();
        assert_eq!(queue.opens().len(), 1, "unpick must restore it to batch");
        assert_eq!(queue.picked_opens().len(), 0);
    }

    #[test]
    fn unpick_with_nothing_picked_is_a_noop() {
        let mut queue = Queue::new(network_parameters(), BuildConfig::default());
        queue.add_open(Open::new(
            Channel::new(
                constants(1),
                Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
            ),
            None,
        ));
        queue.unpick();
        assert_eq!(queue.opens().len(), 1);
    }

    #[test]
    fn build_with_nothing_picked_is_empty() {
        let mut queue = Queue::new(network_parameters(), BuildConfig::default());
        let result = queue.build(&fuel());
        assert!(matches!(result, Err(BuildError::Empty)));
    }

    fn opened_channel(tag_byte: u8) -> Channel {
        Channel::new(
            constants(tag_byte),
            Variables::new(5_000_000, Stage::Opened { subbed: 0 }),
        )
    }

    /// Validates `Open::size_estimate` against `cardano-sdk`'s own
    /// builder directly: build real transactions with one and two
    /// Opens, and assert the measured CBOR-length delta is *exactly*
    /// the second Open's own `size_estimate` -- not approximately,
    /// to the byte.
    #[test]
    fn open_cbor_delta_matches_size_estimate_exactly() {
        let mut base = Queue::new(network_parameters(), BuildConfig::default());
        base.settings_mut().set_change_address(change_address());

        let mut one = base.clone();
        one.add_open(Open::new(opened_channel(1), None));
        one.pick(Bounds::default()).unwrap();
        let size_one = one.build(&fuel()).unwrap().to_cbor().len();

        let second_open = Open::new(opened_channel(2), None);
        let mut two = base.clone();
        two.add_open(Open::new(opened_channel(1), None));
        two.add_open(second_open.clone());
        two.pick(Bounds::default()).unwrap();
        let size_two = two.build(&fuel()).unwrap().to_cbor().len();

        assert_eq!(size_two - size_one, second_open.size_estimate());
    }

    /// Same idea, for `Will`s: both channels use the same consumer
    /// key (`constants`'s fixed consumer/provider hashes), so the
    /// second `Add` adds no new `required_signers` entry -- the
    /// measured delta should be exactly `Will::size_estimate` plus
    /// one `redeemer_witness_overhead`.
    #[test]
    fn will_cbor_delta_matches_size_estimate_plus_redeemer_overhead() {
        let mut base = Queue::new(network_parameters(), BuildConfig::default());
        base.settings_mut().set_change_address(change_address());
        base.settings_mut()
            .set_reference_utxo(
                Input::new(Hash::<32>::from([99u8; 32]), 0),
                Output::new(change_address(), Value::new(5_000_000))
                    .with_plutus_script(crate::VALIDATOR.script.clone()),
            )
            .unwrap();

        let validator_address: Address<kind::Any> =
            validator::address(network_parameters().network_id, None).into();
        let channel_output = |tag_byte: u8| {
            let channel = opened_channel(tag_byte);
            Output::new(validator_address.clone(), channel.buffered_value())
                .with_datum(to_plutus_data(channel.datum()))
        };

        let input_1 = Input::new(Hash::<32>::from([1u8; 32]), 0);
        let input_2 = Input::new(Hash::<32>::from([2u8; 32]), 0);
        let add = Want::Add { amount: 1_000_000 };

        let mut one = base.clone();
        one.add_utxos(BTreeMap::from([(input_1.clone(), channel_output(1))]))
            .unwrap();
        one.propose(input_1.clone(), add.clone()).unwrap();
        one.pick(Bounds::default()).unwrap();
        let size_one = one.build(&fuel()).unwrap().to_cbor().len();

        let mut two = base.clone();
        two.add_utxos(BTreeMap::from([
            (input_1.clone(), channel_output(1)),
            (input_2.clone(), channel_output(2)),
        ]))
        .unwrap();
        two.propose(input_1.clone(), add.clone()).unwrap();
        two.propose(input_2.clone(), add.clone()).unwrap();
        two.pick(Bounds::default()).unwrap();
        let size_two = two.build(&fuel()).unwrap().to_cbor().len();

        // Not an exact match: `redeemer_witness_overhead` prices the
        // redeemer-pointer/ex-units entry at the worst case
        // (max index, protocol-max ex-units) on purpose, while this
        // unsigned, not-yet-evaluated build still carries a cheaper
        // placeholder ex-units value (evaluation hasn't run). The
        // real, measured delta must never exceed the upper bound --
        // that's the actual property worth asserting here.
        let second_will = opened_channel(2).try_will(add).unwrap();
        let upper_bound = second_will.size_estimate() + crate::step::redeemer_witness_overhead();
        let measured_delta = size_two - size_one;

        assert!(
            measured_delta <= upper_bound,
            "measured={measured_delta} must not exceed upper_bound={upper_bound}"
        );
    }

    fn channel_output(validator_address: &Address<kind::Any>, channel: &Channel) -> Output {
        Output::new(validator_address.clone(), channel.buffered_value())
            .with_datum(to_plutus_data(channel.datum()))
    }

    /// The headline property `Ref::ById` exists for: a want staged
    /// against a channel's *identity* survives that channel's utxo
    /// moving underneath it (e.g. a concurrent `subbit-claim` Sub/
    /// Settle landing on the same channel between propose and pick),
    /// while a want staged `ByInput` -- pinned to the old utxo -- does
    /// not. Mirrors exactly what `Ctx::basket_refresh_txos` does on a
    /// real utxo-set refresh: drain the old queue's wants and
    /// re-propose each (via `propose_ref`) against a freshly rebuilt
    /// one.
    #[test]
    fn by_id_want_survives_its_channel_moving_to_a_new_utxo_but_by_input_does_not() {
        let validator_address: Address<kind::Any> =
            validator::address(network_parameters().network_id, None).into();
        let channel = opened_channel(1);
        let keytag = channel.keytag();
        let old_input = Input::new(Hash::<32>::from([1u8; 32]), 0);
        let new_input = Input::new(Hash::<32>::from([2u8; 32]), 0);

        let mut queue = Queue::new(network_parameters(), BuildConfig::default());
        queue
            .add_utxos(BTreeMap::from([(
                old_input.clone(),
                channel_output(&validator_address, &channel),
            )]))
            .unwrap();

        let add = Want::Add { amount: 1_000_000 };
        queue.propose_by_id(keytag.clone(), add.clone()).unwrap();
        queue.propose(old_input.clone(), add.clone()).unwrap();
        // Both staged (two distinct `Ref`s); `wills()` itself would
        // only show one, since both currently resolve to the same
        // `Input` and collapse onto one key in that Input-keyed map --
        // not a real conflict, since nothing stages both an `ById` and
        // a `ByInput` want against the same channel in practice.
        assert_eq!(queue.batch.wants.len(), 2, "both wants are staged");

        // Simulate the channel's utxo moving -- same shape as
        // `Ctx::basket_refresh_txos`: drain, rebuild against the new
        // utxo set, re-propose each drained want by its original `Ref`.
        let wants = queue.drain_wants();
        let mut refreshed = Queue::new(network_parameters(), BuildConfig::default());
        refreshed
            .add_utxos(BTreeMap::from([(
                new_input.clone(),
                channel_output(&validator_address, &channel),
            )]))
            .unwrap();
        for (r, want) in wants {
            let _ = refreshed.propose_ref(r, want);
        }

        let survivors = refreshed.wills();
        assert_eq!(
            survivors.len(),
            1,
            "only the by-id want should survive the utxo move"
        );
        assert!(survivors.contains_key(&new_input));
    }

    /// `drop_intent` (the stale-input recovery path `console::submit`
    /// uses after a `MissingInput`-shaped chain rejection) must find a
    /// `ById` want by whatever input it *currently* resolves to, not
    /// just a `ByInput` one.
    #[test]
    fn drop_intent_finds_a_by_id_want_by_its_current_resolution() {
        let validator_address: Address<kind::Any> =
            validator::address(network_parameters().network_id, None).into();
        let channel = opened_channel(1);
        let keytag = channel.keytag();
        let input = Input::new(Hash::<32>::from([1u8; 32]), 0);

        let mut queue = Queue::new(network_parameters(), BuildConfig::default());
        queue
            .add_utxos(BTreeMap::from([(
                input.clone(),
                channel_output(&validator_address, &channel),
            )]))
            .unwrap();
        queue
            .propose_by_id(keytag, Want::Add { amount: 1_000_000 })
            .unwrap();

        assert!(queue.drop_intent(&input).is_some());
        assert!(queue.wills().is_empty());
    }
}
