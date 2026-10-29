//! Orchestrates scenarios

use std::collections::BTreeSet;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration as StdDuration, Instant};

use anyhow::Context;
use cardano_connector::CardanoConnector;
use subbit_core::{Duration, Hash28, Tag};
use subbit_l1_dto::Constants;
use subbit_tx::step::Can;

use crate::consumer::{Consumer, Dialect};
use crate::identity::Identity;
use crate::l1::L1;
use crate::provider::Provider;

pub(crate) fn close_period() -> Duration {
    Duration::from_secs(60)
}

/// Abandon scenario if waiting longer than this
const MAX_LOOP_TIME: StdDuration = StdDuration::from_secs(900);

/// `--mimic`'s own parameters, built once in `main.rs` (needs `l2_seed`,
/// which `Orchestrator` itself otherwise has no reason to carry) and
/// handed to `Orchestrator::new`. The scenario this drives is meant to
/// look like a *normal* run, just with a decoy channel thrown in
/// alongside every real consumer -- not a separate, bespoke scenario of
/// its own. See `Orchestrator::open_and_close_mimics`.
pub struct MimicScenario {
    /// A funding wallet derived just for the spawned `subbit-claim`
    /// process's own fee/collateral -- never the main funding wallet
    /// `L1` itself drives Open/Close through.
    pub claim_fee_identity: Identity,
    /// Needed for the spawned `subbit-claim`'s own `keyring` (its
    /// required-signatory key) -- `Orchestrator` otherwise only ever
    /// keeps this identity's `hash28` (`provider_hash28`), not its raw
    /// key material.
    pub provider_identity: Identity,
}

pub struct Orchestrator<C: CardanoConnector> {
    l1: L1<C>,
    provider: Option<Provider>,
    provider_hash28: Hash28,
    consumers: Vec<Arc<Consumer>>,
    resume_round: usize,
    /// `Some(endpoint)` runs `index()` against a live Floe instance
    /// (the real `subbit_index` pipeline, see `l1::index_via_floe`);
    /// `None` keeps the in-process shortcut (`--mode mock`, which has
    /// no live Floe server fronting its chain).
    floe_endpoint: Option<String>,
    mimic: Option<MimicScenario>,
    /// `false` once `run_mimic_scenario` has handed claiming off to a
    /// really-spawned `subbit-claim` process -- `run_until_done`'s own
    /// per-round loop then skips `claim_tick` entirely, since that
    /// process claims asynchronously on its own poll interval instead.
    claim_in_process: bool,
}

impl<C: CardanoConnector> Orchestrator<C> {
    /// Marks the last of at least 2 consumers to `tidy` once closed
    /// (demonstrates `Elapse` instead of every consumer settling via
    /// `subbit-claim`).
    pub fn new(
        l1: L1<C>,
        provider: Option<Provider>,
        provider_hash28: Hash28,
        consumers: Vec<Arc<Consumer>>,
        resume_round: usize,
        floe_endpoint: Option<String>,
        mimic: Option<MimicScenario>,
    ) -> Self {
        if consumers.len() >= 2 {
            consumers.last().unwrap().tidy_once_closed();
        }
        Self {
            l1,
            provider,
            provider_hash28,
            consumers,
            resume_round,
            floe_endpoint,
            mimic,
            claim_in_process: true,
        }
    }

    /// Verifies the chain is clear of duplicates (more than one channel
    /// sharing a consumer's tag) and that any channel that already
    /// exists is still `Opened` (resumable). Doesn't decide `Open` vs
    /// resume -- `run`'s own first step does that, now that this has
    /// confirmed it's safe to. `Err` means `tidy` should run first.
    ///
    /// Skips the Opened-stage check entirely when `--resume` is set
    /// (`resume_round > 0`): a consumer already past `Opened` (`Closed`,
    /// or gone from the validator address entirely once `Settled`) is
    /// exactly what a legitimately-resumed run looks like for any
    /// consumer whose earlier attempt already finished its own
    /// lifecycle, not a stale leftover from an unrelated run -- there's
    /// no way to tell those two apart from the channel's stage alone, so
    /// `--resume` is the caller's explicit signal to trust it. The
    /// duplicate-tag check (inside `can_for_consumer`, scoped to this
    /// exact consumer's own identity -- not just its tag, so a
    /// `--mimic` decoy sharing the tag under a *different* identity
    /// never trips this) still runs regardless: a same-identity
    /// collision is a genuine ambiguity no amount of resuming excuses.
    pub fn check(&self) -> anyhow::Result<()> {
        for (i, consumer) in self.consumers.iter().enumerate() {
            if let Some((_, can)) =
                self.l1
                    .can_for_consumer(&consumer.iou_key, &consumer.tag, &consumer.l1_key)?
            {
                anyhow::ensure!(
                    self.resume_round > 0 || is_resumable(&can),
                    "consumer {i}'s existing channel (tag {:?}) is past Opened already",
                    consumer.tag
                );
            }
        }
        Ok(())
    }

    /// tidys every one of this orchestrator's own consumers (see
    /// `Consumer::tidy`), then runs the same loop `run` does until
    /// nothing's left -- tidy is a teardown scenario, not a different
    /// thing.
    pub async fn tidy(&mut self) -> anyhow::Result<()> {
        for consumer in &self.consumers {
            consumer.tidy();
        }
        self.run_until_done().await?;
        println!("canary: tidy complete -- no recognized channels left");
        Ok(())
    }

    /// Drives the full lifecycle: open whatever still needs opening
    /// (real roster and, if `--mimic`, every decoy already mixed into
    /// `self.consumers` as a `.tidy()`-ing `Consumer` -- see `main.rs`'s
    /// own construction, right next to `MimicScenario`), index, then
    /// loop every consumer's own decision until done. A decoy needs no
    /// special-casing anywhere in here: `Consumer::want` already picks
    /// `Close` while `Opened`, `Elapse` once `Closed` (if the real
    /// `subbit-claim` process hasn't `Settle`d it first), and `End`
    /// the moment anything (real or decoy) reaches `Can::End` -- the
    /// exact three steps a decoy needs, for free. `_services` (when
    /// `--mimic`) stays alive for this whole function, since dropping
    /// it (`kill_on_drop`) tears down the spawned processes it holds.
    pub async fn run(&mut self) -> anyhow::Result<()> {
        self.open_pending().await?;

        let mimic = self.mimic.take();
        let _services = match &mimic {
            Some(mimic) => {
                let services = self.spawn_real_services(mimic).await?;
                // The real `subbit-claim` process now claims
                // asynchronously on its own poll interval -- stop
                // `run_until_done` from also doing it in-process.
                self.claim_in_process = false;
                Some(services)
            }
            None => {
                self.index().await?;
                None
            }
        };

        self.run_until_done().await?;
        println!("canary: full lifecycle complete");
        self.print_l2_summary();
        Ok(())
    }

    /// Spawns the real `lineage-index`/`subbit-claim` binaries in place
    /// of canary's own in-process `index_via_floe`/`claim_tick`
    /// shortcuts -- the whole point of `--mimic` is exercising the real
    /// pipeline against the decoys, not canary's shortcuts for them.
    async fn spawn_real_services(
        &mut self,
        mimic: &MimicScenario,
    ) -> anyhow::Result<(crate::services::LineageIndex, crate::services::ClaimProcess)> {
        let floe_endpoint = self
            .floe_endpoint
            .clone()
            .expect("--mimic implies floe_endpoint.is_some() -- see main.rs");
        let server_base_url = self.provider().base_url();

        println!("canary: --mimic: spawning the real lineage-index binary");
        let lineage_index = crate::services::LineageIndex::spawn(&floe_endpoint, &server_base_url)
            .await
            .context("spawning lineage-index")?;

        let network_id = self.l1.network_id();
        let fee_lovelace = 5 * cardano_wallet::txs::ONE_ADA_LOVELACE;
        self.l1
            .fund_address(
                mimic.claim_fee_identity.address(network_id).into(),
                fee_lovelace,
            )
            .await
            .context("funding the spawned subbit-claim's own fee wallet")?;

        println!("canary: --mimic: spawning the real subbit-claim binary");
        let claim_process = crate::services::ClaimProcess::spawn(
            &floe_endpoint,
            &server_base_url,
            cardano_sdk::Network::Preprod,
            mimic.claim_fee_identity.seed(),
            self.l1.change_address(),
            mimic.provider_identity.seed(),
        )
        .await
        .context("spawning subbit-claim")?;
        println!(
            "canary: --mimic: real subbit-claim/lineage-index now running as child processes -- \
             claiming/indexing is fully asynchronous from here"
        );
        Ok((lineage_index, claim_process))
    }

    /// Decides what still needs a fresh batched `Open` (anything
    /// `check` didn't find an existing channel for) and opens it.
    async fn open_pending(&mut self) -> anyhow::Result<()> {
        let mut to_open = Vec::new();
        for (i, consumer) in self.consumers.iter().enumerate() {
            let existing =
                self.l1
                    .can_for_consumer(&consumer.iou_key, &consumer.tag, &consumer.l1_key)?;
            if existing.is_some() {
                println!("canary: [{i}] resuming already-open channel (skipping Open)");
                continue;
            }
            println!("canary: [{i}] queuing for batched Open");
            let constants = Constants::new(
                Tag::from(consumer.tag.clone()),
                subbit_l1_dto::Asset::Lovelace,
                consumer.iou_key,
                consumer.l1_key,
                self.provider_hash28,
                close_period(),
            );
            to_open.push((constants, consumer.deposit));
        }
        if !to_open.is_empty() {
            println!(
                "canary: opening {} channel(s), saturating as many per transaction as the chain accepts",
                to_open.len()
            );
            self.l1.open(to_open).await.context("batched Open")?;
        }
        Ok(())
    }

    /// `run`'s own steps that need a real `Provider` -- a teardown-only
    /// build (`build_teardown`) never calls any of them, only `tidy`.
    fn provider(&self) -> &Provider {
        self.provider.as_ref().expect(
            "run()'s own steps need a Provider; a teardown-only build should only call tidy()",
        )
    }

    async fn index(&self) -> anyhow::Result<()> {
        match &self.floe_endpoint {
            Some(floe_endpoint) => {
                println!(
                    "canary: registering {} deposits as backings (real lineage pipeline, against live Floe at {floe_endpoint})",
                    self.consumers.len()
                );
                crate::l1::index_via_floe(floe_endpoint, &self.provider().base_url()).await
            }
            None => {
                println!(
                    "canary: registering {} deposits as backings (real naive-index logic, driven directly against the active connector)",
                    self.consumers.len()
                );
                self.l1
                    .index(
                        &self.provider().base_url(),
                        self.provider_hash28,
                        close_period(),
                    )
                    .await
            }
        }
    }

    /// Each pass: ask every consumer with a live channel for its own
    /// next `Want` (its persona schedule, or `tidy`'s exit path),
    /// submit whatever comes back, then let a real provider's own
    /// `subbit-claim` logic pick up `Settle` for anyone `Closed` and not
    /// tidying. Loops until no consumer has a live channel left.
    /// Bounded by `MAX_LOOP_TIME` against a stuck chain, on top of
    /// whatever `L1` itself already errors out on.
    async fn run_until_done(&mut self) -> anyhow::Result<()> {
        let start = Instant::now();
        let mut round = self.resume_round;
        if round > 0 {
            println!("canary: resuming main loop at round {round}");
        }

        loop {
            anyhow::ensure!(
                start.elapsed() < MAX_LOOP_TIME,
                "lifecycle loop exceeded {MAX_LOOP_TIME:?} without every channel completing"
            );

            // Needed unconditionally, not just after this loop's own
            // `flush()` (which refreshes as a side effect of waiting
            // for its own submission): once claiming is handed off to
            // a real, externally-spawned `subbit-claim` process
            // (`--mimic`), nothing *this* loop submits ever changes a
            // channel's final `Settle` -- without refreshing here too,
            // this loop would never observe that external process's
            // own txs landing, and would just spin until
            // `MAX_LOOP_TIME`.
            self.l1
                .refresh()
                .await
                .context("refreshing tracked channels")?;

            // Tolerant of more than one tracked channel per consumer
            // identity (never errors the way `can_for_consumer` does) --
            // `tidy()` drives this same loop, and its whole point is
            // reclaiming every leftover channel it finds, duplicates
            // included; a normal (non-tidy) run never actually produces
            // more than one per identity in the first place (`check`
            // already refused to start if it had), so this is a no-op
            // widening for that path.
            let mut live = Vec::new();
            for consumer in &self.consumers {
                for (input, can) in self.l1.channels_for_consumer(
                    &consumer.iou_key,
                    &consumer.tag,
                    &consumer.l1_key,
                ) {
                    live.push((input, can, Arc::clone(consumer)));
                }
            }
            if live.is_empty() {
                break;
            }

            let close_upper = close_upper();
            let wants: Vec<_> = live
                .iter()
                .filter_map(|(input, can, consumer)| {
                    consumer
                        .want(round, can, close_upper)
                        .map(|w| (input.clone(), w))
                })
                .collect();

            // Firehose runs on its own cloned consumer list for the
            // whole duration of this round's chain step below, not
            // just before it: see `plan-canary-sustained-firehose.md`.
            let stop = Arc::new(AtomicBool::new(false));
            let firehose = tokio::spawn({
                let stop = Arc::clone(&stop);
                let consumers = self.consumers.clone();
                async move {
                    let mut total = 0usize;
                    while !stop.load(Ordering::Relaxed) {
                        let mut set = tokio::task::JoinSet::new();
                        for consumer in &consumers {
                            let consumer = Arc::clone(consumer);
                            set.spawn(async move { consumer.firehose_l2(round).await });
                        }
                        let mut pass = 0usize;
                        while let Some(result) = set.join_next().await {
                            pass += result.context("firehose task panicked")?;
                        }
                        total += pass;
                        // Every consumer already done this round (e.g.
                        // past a burst's own commit_max): don't hot-spin.
                        if pass == 0 {
                            tokio::time::sleep(StdDuration::from_millis(50)).await;
                        }
                    }
                    anyhow::Ok(total)
                }
            });

            let chain_start = Instant::now();
            let mut submitted = 0usize;
            for (input, want) in wants {
                if self.l1.want(input, want) {
                    submitted += 1;
                }
            }
            if submitted > 0 {
                self.l1.flush().await.context("batched lifecycle step")?;
            }

            if self.claim_in_process
                && let Some(provider) = &self.provider
            {
                let exclude: BTreeSet<Vec<u8>> = self
                    .consumers
                    .iter()
                    .filter(|c| c.is_tidying())
                    .map(|c| c.tag.clone())
                    .collect();
                let claimed = self
                    .l1
                    .claim_tick(&provider.base_url(), close_period(), &exclude)
                    .await?;
                if claimed > 0 {
                    println!("canary: subbit-claim submitted {claimed} want(s)");
                }
            }
            let chain_elapsed = chain_start.elapsed();

            stop.store(true, Ordering::Relaxed);
            let spent = firehose.await.context("firehose task panicked")??;

            report_round(round, spent, submitted, chain_elapsed);
            round += 1;

            // Nothing happened: avoid busy-looping while waiting (e.g.
            // for an `Elapse` window, see `L1::want`).
            if spent == 0 && submitted == 0 {
                tokio::time::sleep(StdDuration::from_secs(1)).await;
            }
        }
        Ok(())
    }

    /// Off-chain L2 totals, by dialect -- `spent`/`committed` come from
    /// each `Consumer`'s own `subbit_stamp::Cache`.
    fn print_l2_summary(&self) {
        let mut by_dialect = [
            (Dialect::Vanilla, 0usize, 0u64, 0u64),
            (Dialect::X402, 0, 0, 0),
        ];
        // A `--mimic` decoy is a plain `Consumer::new` with no persona
        // (never spends, never connects) -- excluded here so it doesn't
        // inflate the count/dialect breakdown of actual L2 activity.
        let real_consumers = self.consumers.iter().filter(|c| !c.persona.is_empty());
        for consumer in real_consumers.clone() {
            let slot = by_dialect
                .iter_mut()
                .find(|(d, ..)| *d == consumer.dialect)
                .expect("both dialects pre-seeded above");
            slot.1 += 1;
            slot.2 += consumer.spent();
            slot.3 += consumer.committed().unwrap_or(0);
        }
        let total_spent: u64 = by_dialect.iter().map(|(.., spent, _)| spent).sum();
        let total_committed: u64 = by_dialect.iter().map(|(.., committed)| committed).sum();
        println!(
            "canary: L2 summary -- {} consumer(s), total spent={total_spent} total committed={total_committed}",
            real_consumers.count()
        );
        for (dialect, count, spent, committed) in by_dialect {
            println!(
                "canary: L2 summary -- {dialect:?}: {count} consumer(s), spent={spent} committed={committed}"
            );
        }
    }
}

/// `can` matches an `Opened`-stage channel (`Add`/`Sub`/`Close` are the
/// only options `Channel::can` ever produces for that stage) -- the one
/// stage `check`/`open_pending` know how to resume.
fn is_resumable(can: &[Can]) -> bool {
    can.iter()
        .any(|c| matches!(c, Can::Add | Can::Sub { .. } | Can::Close))
}

/// One line per pass, skipped when nothing happened. `req/s` covers
/// `spend_elapsed`, the chain step's own duration -- firehose runs
/// for that whole window, so this is sustained throughput through it.
fn report_round(round: usize, spent: usize, submitted: usize, spend_elapsed: StdDuration) {
    if spent == 0 && submitted == 0 {
        return;
    }
    let req_per_sec = spent as f64 / spend_elapsed.as_secs_f64().max(f64::EPSILON);
    println!(
        "canary: round {round}: {spent} spend request(s) in {spend_elapsed:?} ({req_per_sec:.1} req/s), {submitted} chain step(s) submitted"
    );
}

/// `Want::Close{upper}` becomes the submitted tx's own ledger-level
/// validity-interval upper bound (`Bounds{upper: Some(upper), ..}`,
/// `packages/tx/src/step.rs:121`), so it needs real slack above `now()`:
/// a block producer including the tx after `upper` passes makes the
/// ledger drop it silently rather than erroring back to the submitter.
/// 120s margin: real preprod confirmation lag tops out around 70-80s
/// (7-8 polls at the 10s `Waiter` interval).
pub(crate) fn close_upper() -> Duration {
    crate::time::now() + Duration::from_secs(120)
}
