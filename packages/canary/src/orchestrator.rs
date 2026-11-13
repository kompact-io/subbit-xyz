//! Orchestrates scenarios.

use std::time::{Duration as StdDuration, Instant as StdInstant};

use anyhow::Context;
use subbit_core::{Duration, Instant as SubbitInstant};
use subbit_l1_dto::Constants;
use subbit_tx::step::{Can, Want};

use crate::chain::Chain;
use crate::consumer::{Consumer, Dialect};
use crate::identity::Identity;
use crate::provider::Provider;
use crate::time::Round;

// Must stay well above close_upper's 180s margin + close_period (~240s before Elapse is even
// reachable), plus landing time under load (observed 790-845s for a 40-consumer batch
// when the margin was still 600s). Left at 2400s rather than shrunk alongside the margin: the
// 790-845s landing-time figure predates this change and hasn't been re-measured at 180s.
const MAX_LOOP_TIME: StdDuration = StdDuration::from_secs(2400);

/// Bound on a single chain step (a batched `Open`, or one round's `flush`), independent
/// of `MAX_LOOP_TIME`'s whole-run budget. Without this, a hung `submit`/`wait_for_landing`
/// blocks the whole loop silently until `MAX_LOOP_TIME` finally trips, 2400s later, rather
/// than surfacing the stall promptly. Must stay well above genuine landing latency, though,
/// or it fires on a tx that was never actually stuck -- just slow to confirm (see
/// `MAX_LOOP_TIME`'s own doc: 790-845s observed for a 40-consumer batch). Confirmed live at
/// 120s: a severity-40 `Open` timed out here, but `canary status` afterwards showed all 40
/// channels landed `Opened` anyway -- a false-positive failure, not a real stall. Set well
/// above that figure rather than just above it: the 790-845s number predates this exact
/// path and amaru's own confirmation-view lag (the plain `rejected by amaru: Transaction is
/// a duplicate` wording, correlated with these slow landings) isn't itself bounded.
const CHAIN_STEP_TIMEOUT: StdDuration = StdDuration::from_secs(900);

pub struct Orchestrator {
    chain: Chain,
    /// Held only so the provider's child processes live as long as the run does.
    _provider: Option<Provider>,
    provider_identity: Identity,
    consumers: Vec<Consumer>,
    resume_round: u8,
    /// Hard cap on active rounds before every consumer's `Agent` is killed and the
    /// whole run switches over to `Round::Tidying` regardless of any seat's own
    /// schedule. Irrelevant to a `new_tidy` build, which starts in `Round::Tidying`
    /// right away.
    max_rounds: u8,
    /// How long the slow-poll phase waits before forcing the next round --
    /// irrelevant to a `new_tidy` build (see `max_rounds`'s own doc).
    round_poll_upper_bound_secs: u64,
    /// `None` only for a tidy-only build (`tidy()` never calls `run()`).
    floe_endpoint: Option<String>,
    /// A separately-funded wallet for the spawned `subbit-claim` process's own fee/collateral.
    claim_wallet: Option<Identity>,
    /// `true` only for a tidy-only build: `run_until_done` then forces a `Settle` itself
    /// (via `chain::Chain::stage_settle`, the provider-admin action) for any `Closed` channel,
    /// rather than waiting out `Elapse`'s own close-period margin. A normal `run()` never does
    /// this -- the spawned `subbit-claim` owns every claim there (see `simplify.md`'s own
    /// note on why `tidy` alone can't just ask it to).
    admin_settle: bool,
    /// Test-only invariant: the live claim service must settle every closed channel
    /// before the consumer can stage its own unilateral `Elapse`.
    require_claim_win: bool,
    /// The provider's close period, mirrored into every opened channel's `Constants`.
    close_period: Duration,
}

impl Orchestrator {
    /// Each `Consumer` already carries and has spawned its own `Agent` (`Consumer::init`)
    /// -- this layer never touches that wiring. Every consumer heads to `Elapse` on its
    /// own once its `Close` lands (`L1::want`'s standard behaviour), racing
    /// `subbit-claim`'s `Settle` rather than waiting for it exclusively.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        chain: Chain,
        provider: Provider,
        provider_identity: Identity,
        consumers: Vec<Consumer>,
        resume_round: u8,
        max_rounds: u8,
        round_poll_upper_bound_secs: u64,
        floe_endpoint: String,
        claim_wallet: Identity,
        require_claim_win: bool,
        close_period: Duration,
    ) -> Self {
        Self {
            chain,
            _provider: Some(provider),
            provider_identity,
            consumers,
            resume_round,
            max_rounds,
            round_poll_upper_bound_secs,
            floe_endpoint: Some(floe_endpoint),
            claim_wallet: Some(claim_wallet),
            admin_settle: false,
            require_claim_win,
            close_period,
        }
    }

    /// Tidy-only build: `tidy()` never needs a `Provider`, and its placeholder
    /// consumers (`Consumer::init` with no `AgentConfig`) never have an `Agent` to
    /// go with them. `max_rounds`/`round_poll_upper_bound_secs` are irrelevant here --
    /// `tidy()` drives straight into `Round::Tidying` from the first pass.
    pub fn new_tidy(
        chain: Chain,
        provider_identity: Identity,
        consumers: Vec<Consumer>,
        floe_endpoint: String,
        claim_wallet: Identity,
        close_period: Duration,
    ) -> Self {
        Self {
            chain,
            _provider: None,
            provider_identity,
            consumers,
            resume_round: 0,
            max_rounds: 0,
            round_poll_upper_bound_secs: 0,
            floe_endpoint: Some(floe_endpoint),
            claim_wallet: Some(claim_wallet),
            admin_settle: true,
            require_claim_win: false,
            close_period,
        }
    }

    /// Verifies the chain has no duplicate-identity channels, and any existing channel is still
    /// `Opened` (resumable) unless `--resume` is set. `Err` means `tidy` should run first.
    pub fn check(&self) -> anyhow::Result<()> {
        for (i, consumer) in self.consumers.iter().enumerate() {
            let id = &consumer.settings().identity;
            if let Some((_, can)) =
                self.chain
                    .can_for_consumer(&id.iou_key, id.tag.as_ref(), &id.vkh)?
            {
                anyhow::ensure!(
                    self.resume_round > 0 || is_resumable(&can),
                    "consumer {i}'s existing channel (tag {:?}) is past Opened already",
                    id.tag
                );
            }
        }
        Ok(())
    }

    /// Drives every consumer straight into `Round::Tidying` from the first pass (no
    /// slow-poll phase -- there's only one thing left to ask), then runs the same loop
    /// `run` does until nothing's left, then sweeps whatever's left in the claim wallet
    /// back to the main wallet.
    pub async fn tidy(&mut self) -> anyhow::Result<()> {
        self.run_until_done(Round::Tidying).await?;
        println!("canary: tidy complete -- no recognized channels left");
        self.reclaim_claim_wallet().await?;
        Ok(())
    }

    /// No-op unless this build actually carries a claim wallet and Floe endpoint (both are
    /// only ever `None` for a `new_tidy` build predating this reclaim step).
    async fn reclaim_claim_wallet(&mut self) -> anyhow::Result<()> {
        let (Some(floe_endpoint), Some(claim_wallet)) =
            (self.floe_endpoint.clone(), self.claim_wallet)
        else {
            return Ok(());
        };
        let batches = self
            .chain
            .reclaim_wallet(&floe_endpoint, claim_wallet.seed())
            .await
            .context("reclaiming the claim wallet's leftover balance")?;
        match batches {
            0 => println!("canary: claim wallet had nothing worth reclaiming"),
            n => println!("canary: reclaimed the claim wallet's leftover balance ({n} tx(s))"),
        }
        Ok(())
    }

    /// Drives the full lifecycle: open, loop until done.
    pub async fn run(&mut self) -> anyhow::Result<()> {
        self.open_pending().await?;
        // FIXME: each Agent is always built fresh (Consumer::init), unaware of
        // resume_round -- only L1's own schedule resumes correctly. Not an issue while
        // --resume is unused.
        self.run_until_done(Round::Active(self.resume_round))
            .await?;
        println!("canary: full lifecycle complete");
        self.print_l2_summary();
        Ok(())
    }

    /// Opens a fresh batched `Open` for every consumer `check` didn't find an existing channel for.
    /// FIXME :: Move to consumer
    async fn open_pending(&mut self) -> anyhow::Result<()> {
        let mut to_open = Vec::new();
        for (i, consumer) in self.consumers.iter().enumerate() {
            let s = consumer.settings();
            let id = &s.identity;
            let existing = self
                .chain
                .can_for_consumer(&id.iou_key, id.tag.as_ref(), &id.vkh)?;
            if existing.is_some() {
                println!("canary: [{i}] resuming already-open channel (skipping Open)");
                continue;
            }
            println!("canary: [{i}] queuing for batched Open");
            let constants = Constants::new(
                id.tag.clone(),
                subbit_l1_dto::Asset::Lovelace,
                id.iou_key,
                id.vkh,
                self.provider_identity.hash28,
                self.close_period,
            );
            to_open.push((constants, s.deposit));
        }
        if !to_open.is_empty() {
            println!(
                "canary: opening {} channel(s), saturating as many per transaction as the chain accepts",
                to_open.len()
            );
            tokio::time::timeout(CHAIN_STEP_TIMEOUT, self.chain.open(to_open))
                .await
                .map_err(|_| {
                    anyhow::anyhow!(
                        "batched Open timed out after {CHAIN_STEP_TIMEOUT:?} (chain step hung)"
                    )
                })?
                .context("batched Open")?;
        }
        Ok(())
    }

    /// Four-phase loop, driven entirely by `round`: slow-poll (skipped once
    /// `Round::Tidying`), do one round (collect every consumer's next `Want`),
    /// drain it (submit, handling failures by leaving them for the next round's own
    /// fresh `can` to re-decide), then `next()` every consumer unconditionally -- the
    /// coupling that keeps `next()` from ever firing on an independent cadence.
    /// `max_rounds` bounds the active phase; once exceeded, every `Agent` is killed and
    /// `round` becomes `Round::Tidying` for good. Bounded overall by `MAX_LOOP_TIME`
    /// against a stuck chain.
    async fn run_until_done(&mut self, start_round: Round) -> anyhow::Result<()> {
        let start = StdInstant::now();
        let mut round = start_round;
        let mut elapse_staged = 0u64;
        if let Round::Active(r) = round
            && r > 0
        {
            println!("canary: resuming main loop at round {r}");
        }

        loop {
            anyhow::ensure!(
                start.elapsed() < MAX_LOOP_TIME,
                "lifecycle loop exceeded {MAX_LOOP_TIME:?} without every channel completing"
            );

            // Phase 1 (slow poll): no smarter round-due signal yet (see
            // `plan-canary-orchestrator-triggers.md`'s own open question 1) -- simplest
            // version is just the upper bound itself, every active pass. Moot once
            // `Tidying`: there's only one thing left to ask every consumer.
            if let Round::Active(_) = round {
                tokio::time::sleep(StdDuration::from_secs(self.round_poll_upper_bound_secs)).await;
            }

            self.chain
                .refresh()
                .await
                .context("refreshing tracked channels")?;

            let mut live = Vec::new();
            for consumer in &self.consumers {
                let id = &consumer.settings().identity;
                for (input, can) in
                    self.chain
                        .channels_for_consumer(&id.iou_key, id.tag.as_ref(), &id.vkh)
                {
                    live.push((input, can, consumer));
                }
            }
            if live.is_empty() {
                break;
            }

            // Admin-only (`tidy` builds): force a `Settle` for any `Closed` channel right away
            // -- `Stage::Settled => vec![Can::End]` (`packages/tx/src/channel.rs`) means this
            // turns an otherwise-long `Elapse` wait (the full close-period margin) into an
            // immediate `End` next round, rather than the consumer's own `exit_want` having to
            // wait it out. Staged here, submitted below alongside this round's other wants.
            // Needs an `Agent` to re-sign the committed IOU from -- a tidy build's
            // placeholder consumers never have one (see `new_tidy`'s own doc), so this is
            // a no-op there in practice until a run's own seats carry it forward.
            let mut staged_settles = 0usize;
            if self.admin_settle {
                for (input, can, consumer) in &live {
                    if !can.iter().any(|c| matches!(c, Can::Settle { .. })) {
                        continue;
                    }
                    let iou = match consumer.status().and_then(|s| s.committed_iou) {
                        Some(iou) => iou,
                        // No connected `Agent` to ask (a `tidy` placeholder consumer
                        // never has one) -- sign a zero-gain IOU for the channel's own
                        // already-on-chain `subbed` instead, entirely offline, so this
                        // Closed channel still gets a `Settle` rather than
                        // silently falling through to the consumer's own `Elapse`
                        // (see `Identity::sign_iou`'s own doc).
                        None => {
                            let Some(channel) = self.chain.channels().get(input) else {
                                continue;
                            };
                            let subbed = match channel.variables().stage() {
                                subbit_l1_dto::Stage::Closed { subbed, .. } => *subbed,
                                _ => continue,
                            };
                            consumer.settings().identity.sign_iou(subbed)
                        }
                    };
                    let keytag = consumer.settings().identity.keytag();
                    match self.chain.stage_settle(&keytag, iou) {
                        Ok(()) => staged_settles += 1,
                        Err(e) => println!(
                            "canary: tidy: admin settle failed for a consumer (will retry next round): {e:#}"
                        ),
                    }
                }
            }

            let close_upper = close_upper();
            let wants: Vec<_> = live
                .iter()
                .filter_map(|(input, can, consumer)| {
                    consumer
                        .run(round, can, close_upper)
                        .map(|w| (input.clone(), consumer.settings().identity.keytag(), w))
                })
                .collect();

            let chain_start = StdInstant::now();
            let mut submitted = 0usize;
            let mut targeted_inputs = Vec::new();
            for (input, keytag, want) in wants {
                if matches!(&want, Want::Elapse { .. }) {
                    elapse_staged += 1;
                }
                // An `Add` is staged by the channel's own identity, not a
                // frozen input -- robust to that channel's own utxo moving
                // underneath it between this round's refresh and this
                // flush/submit (e.g. a concurrent `subbit-claim` Sub/Settle
                // landing on the same channel). Everything else keeps the
                // exact-input pairing it always had.
                let accepted = if matches!(want, Want::Add { .. }) {
                    self.chain.want_by_id(keytag, want)
                } else {
                    self.chain.want(input.clone(), want)
                };
                if accepted {
                    submitted += 1;
                    targeted_inputs.push(input);
                }
            }
            if submitted > 0 || staged_settles > 0 {
                let flush_result =
                    match tokio::time::timeout(CHAIN_STEP_TIMEOUT, self.chain.flush()).await {
                        Ok(result) => result,
                        Err(_) => {
                            anyhow::bail!(
                                "round {round:?}: flush timed out after {CHAIN_STEP_TIMEOUT:?} \
                             (chain step hung)"
                            );
                        }
                    };
                if let Err(e) = flush_result {
                    // `flush`'s own `wait_for_landing` only ever watches the one hash it
                    // itself submitted -- it has no way to notice a *different* tx
                    // (an earlier retry of this exact step that the client-side error
                    // handling wrongly treated as rejected, or a genuinely competing
                    // step) consuming the same channel input and landing the intended
                    // effect anyway. Re-check directly: if every input this round
                    // targeted is already gone from the chain's live view, the step's
                    // effect already happened on-chain under some other tx -- resolved,
                    // not a failure. Mirrors `subbit-claim`'s own `Pending::advance`
                    // Dead-on-missing-input rule (`packages/claim/src/pending.rs`),
                    // generalized to this round loop, which had no equivalent check.
                    self.chain.refresh().await.context(
                        "refreshing to check for contention after a lifecycle-step error",
                    )?;
                    let live_channels = self.chain.channels();
                    let contended = targeted_inputs
                        .iter()
                        .filter(|i| !live_channels.contains_key(i))
                        .count();
                    if contended == 0 {
                        return Err(e).context("batched lifecycle step");
                    }
                    println!(
                        "canary: round {round:?}: batched lifecycle step didn't confirm landing \
                         under its own tx, but {contended}/{} targeted input(s) were already \
                         spent by something else -- treating this round's effect as resolved \
                         ({e:#})",
                        targeted_inputs.len(),
                    );
                }
            }
            let chain_elapsed = chain_start.elapsed();

            // Each seat's L2 agent runs entirely on its own, independent of this round's
            // chain step -- queried fresh here purely to report it, never to decide
            // anything (the sleep below gates on chain activity alone, not this).
            let total_ious: u64 = self
                .consumers
                .iter()
                .filter_map(Consumer::status)
                .map(|s| s.ious_total)
                .sum();

            report_round(round, total_ious, submitted, chain_elapsed);

            // Phase 4 (next): unconditionally, for every consumer -- cheap/safe on one
            // that wasn't actually parked (`Agent::init`'s `restarts: u32::MAX` means an
            // extra `next()` just arms an unused restart budget). Only ever fires right
            // after this round's own drain, never on an independent cadence.
            for consumer in &self.consumers {
                consumer.next();
            }

            round = match round {
                Round::Active(r) => match r.checked_add(1) {
                    Some(next_round) if next_round <= self.max_rounds => Round::Active(next_round),
                    _ => {
                        println!(
                            "canary: round budget ({}) exhausted -- killing every agent and tidying",
                            self.max_rounds
                        );
                        for consumer in &self.consumers {
                            consumer.kill_agent();
                        }
                        Round::Tidying
                    }
                },
                Round::Tidying => Round::Tidying,
            };
        }
        if self.require_claim_win && elapse_staged > 0 {
            anyhow::bail!(
                "claim-win invariant failed: {elapse_staged} consumer Elapse want(s) were \
                 reached before subbit-claim settled"
            );
        }
        Ok(())
    }

    /// Off-chain L2 totals, by dialect.
    fn print_l2_summary(&self) {
        let mut by_dialect = [
            (Dialect::Vanilla, 0usize, 0u64, 0u64),
            (Dialect::X402, 0, 0, 0),
        ];
        let agents: Vec<_> = self
            .consumers
            .iter()
            .filter_map(|c| Some((c.settings().dialect?, c.status()?)))
            .collect();
        for (dialect, status) in &agents {
            let slot = by_dialect
                .iter_mut()
                .find(|(d, ..)| d == dialect)
                .expect("both dialects pre-seeded above");
            slot.1 += 1;
            slot.2 += status.spent;
            slot.3 += status.committed.unwrap_or(0);
        }
        let total_spent: u64 = by_dialect.iter().map(|(.., spent, _)| spent).sum();
        let total_committed: u64 = by_dialect.iter().map(|(.., committed)| committed).sum();
        println!(
            "canary: L2 summary -- {} consumer(s), total spent={total_spent} total committed={total_committed}",
            agents.len()
        );
        for (dialect, count, spent, committed) in by_dialect {
            println!(
                "canary: L2 summary -- {dialect:?}: {count} consumer(s), spent={spent} committed={committed}"
            );
        }
    }
}

/// The one stage `check`/`open_pending` know how to resume.
fn is_resumable(can: &[Can]) -> bool {
    can.iter()
        .any(|c| matches!(c, Can::Add | Can::Sub { .. } | Can::Close))
}

/// One line per pass, skipped when nothing happened. `total_ious` is every consumer
/// agent's current reported `ious_total`, queried fresh this round -- observational
/// only, since each agent runs on its own schedule, unrelated to this round's chain
/// step (`chain_elapsed`).
fn report_round(round: Round, total_ious: u64, submitted: usize, chain_elapsed: StdDuration) {
    if total_ious == 0 && submitted == 0 {
        return;
    }
    let round = match round {
        Round::Active(r) => r.to_string(),
        Round::Tidying => "tidying".to_string(),
    };
    println!(
        "canary: round {round}: {total_ious} total spend(s) observed, {submitted} chain step(s) submitted in {chain_elapsed:?}"
    );
}

// 180s margin (was 600s, briefly 120s before that -- raised from 120s after
// live-reproduced silent Close failures under heavy load: accepted by submit, never
// landed, no rejection surfaced, at a 40-consumer batch). Still a guess; the fix
// is deriving it from observed chain inclusion lag. Side effect: a bigger margin
// delays Elapse becoming available for a straggler channel by the same amount,
// unrecoverably -- watch for that same silent-failure pattern resurfacing at scale
// before trusting 180s there too.
pub(crate) fn close_upper() -> SubbitInstant {
    crate::time::now() + Duration::from_secs(180)
}
