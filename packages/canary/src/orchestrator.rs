//! Orchestrates scenarios.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration as StdDuration, Instant as StdInstant};

use anyhow::Context;
use subbit_core::{Duration, Instant as SubbitInstant, Tag};
use subbit_l1_dto::Constants;
use subbit_tx::step::Can;

use crate::chain::Chain;
use crate::consumer;
use crate::consumer::{Consumer, Dialect};
use crate::identity::Identity;
use crate::provider::Provider;

pub(crate) fn close_period() -> Duration {
    Duration::from_secs(60)
}

// Must stay well above close_upper's 600s margin + close_period (~660s before Elapse is even
// reachable), plus real landing time under load (observed 790-845s for a 40-consumer batch).
const MAX_LOOP_TIME: StdDuration = StdDuration::from_secs(2400);

const ONE_ADA_LOVELACE: u64 = 1_000_000;

pub struct Orchestrator {
    chain: Chain,
    provider: Option<Provider>,
    provider_identity: Identity,
    consumers: Vec<Arc<Consumer>>,
    resume_round: usize,
    /// `None` only for a teardown-only build (`tidy()` never calls `run()`).
    floe_endpoint: Option<String>,
    /// A separately-funded wallet for the spawned `subbit-claim` process's own fee/collateral.
    claim_fee_identity: Option<Identity>,
}

impl Orchestrator {
    /// `demo_elapse` marks the last consumer to `tidy` once closed, demonstrating `Elapse`
    /// instead of a normal `subbit-claim` settle.
    pub fn new(
        chain: Chain,
        provider: Provider,
        provider_identity: Identity,
        consumers: Vec<Arc<Consumer>>,
        resume_round: usize,
        floe_endpoint: String,
        claim_fee_identity: Identity,
        demo_elapse: bool,
    ) -> Self {
        if demo_elapse && consumers.len() >= 2 {
            consumers.last().unwrap().tidy_once_closed();
        }
        Self {
            chain,
            provider: Some(provider),
            provider_identity,
            consumers,
            resume_round,
            floe_endpoint: Some(floe_endpoint),
            claim_fee_identity: Some(claim_fee_identity),
        }
    }

    /// Teardown-only build: `tidy()` never needs a `Provider`, Floe endpoint, or claim-fee identity.
    pub fn new_teardown(
        chain: Chain,
        provider_identity: Identity,
        consumers: Vec<Arc<Consumer>>,
    ) -> Self {
        Self {
            chain,
            provider: None,
            provider_identity,
            consumers,
            resume_round: 0,
            floe_endpoint: None,
            claim_fee_identity: None,
        }
    }

    /// Verifies the chain has no duplicate-identity channels, and any existing channel is still
    /// `Opened` (resumable) unless `--resume` is set. `Err` means `tidy` should run first.
    pub fn check(&self) -> anyhow::Result<()> {
        for (i, consumer) in self.consumers.iter().enumerate() {
            if let Some((_, can)) =
                self.chain
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

    /// Tidies every consumer, then runs the same loop `run` does until nothing's left.
    pub async fn tidy(&mut self) -> anyhow::Result<()> {
        for consumer in &self.consumers {
            consumer.tidy();
        }
        self.run_until_done().await?;
        println!("canary: tidy complete -- no recognized channels left");
        Ok(())
    }

    /// Drives the full lifecycle: open, spawn real `lineage-index`/`subbit-claim`, loop until done.
    pub async fn run(&mut self) -> anyhow::Result<()> {
        self.open_pending().await?;
        self.spawn_real_services().await?;
        self.run_until_done().await?;
        println!("canary: full lifecycle complete");
        self.print_l2_summary();
        Ok(())
    }

    /// Funds the claim-fee wallet, then spawns `lineage-index`/`subbit-claim` into `provider`.
    async fn spawn_real_services(&mut self) -> anyhow::Result<()> {
        let floe_endpoint = self
            .floe_endpoint
            .clone()
            .expect("run() always has a live Floe endpoint -- see main.rs");
        let claim_fee_identity = self
            .claim_fee_identity
            .expect("run() always derives a claim-fee identity -- see main.rs");

        let network_id = self.chain.network_id();
        let fee_lovelace = 5 * ONE_ADA_LOVELACE;
        self.chain
            .fund_address(claim_fee_identity.address(network_id), fee_lovelace)
            .await
            .context("funding the spawned subbit-claim's own fee wallet")?;

        let ref_script_host = self.chain.change_address_shelley();
        let provider_seed = self.provider_identity.seed();
        self.provider_mut()
            .spawn_support_processes(
                &floe_endpoint,
                cardano_sdk::Network::Preprod,
                claim_fee_identity.seed(),
                ref_script_host,
                provider_seed,
            )
            .await
    }

    /// Opens a fresh batched `Open` for every consumer `check` didn't find an existing channel for.
    async fn open_pending(&mut self) -> anyhow::Result<()> {
        let mut to_open = Vec::new();
        for (i, consumer) in self.consumers.iter().enumerate() {
            let existing =
                self.chain
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
                self.provider_identity.hash28,
                close_period(),
            );
            to_open.push((constants, consumer.deposit));
        }
        if !to_open.is_empty() {
            println!(
                "canary: opening {} channel(s), saturating as many per transaction as the chain accepts",
                to_open.len()
            );
            self.chain.open(to_open).await.context("batched Open")?;
        }
        Ok(())
    }

    /// `run`'s own steps that need a real `Provider` -- a teardown-only build never calls these.
    fn provider_mut(&mut self) -> &mut Provider {
        self.provider.as_mut().expect(
            "run()'s own steps need a Provider; a teardown-only build should only call tidy()",
        )
    }

    /// Each pass: collect every consumer's next `Want`, submit, loop until no live channel remains.
    /// Bounded by `MAX_LOOP_TIME` against a stuck chain.
    async fn run_until_done(&mut self) -> anyhow::Result<()> {
        let start = StdInstant::now();
        let mut round = self.resume_round;
        if round > 0 {
            println!("canary: resuming main loop at round {round}");
        }

        loop {
            anyhow::ensure!(
                start.elapsed() < MAX_LOOP_TIME,
                "lifecycle loop exceeded {MAX_LOOP_TIME:?} without every channel completing"
            );

            self.chain
                .refresh()
                .await
                .context("refreshing tracked channels")?;

            let mut live = Vec::new();
            for consumer in &self.consumers {
                for (input, can) in self.chain.channels_for_consumer(
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

            // Firehose runs on its own cloned consumer list for this whole round's chain step.
            // Timed independently of `chain_elapsed` below: for a continuous (non-burst)
            // persona the two spans do coincide (the firehose only stops once the chain step
            // does), but a `:burst` persona's `firehose_l2` never checks `stop` at all -- it
            // runs to its own `commit_max`/`deadline`, on a timeline unrelated to the chain
            // step's. Using `chain_elapsed` as this round's spend-rate denominator silently
            // assumed the former held always; it doesn't for burst.
            let firehose_start = StdInstant::now();
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
                        if pass == 0 {
                            tokio::time::sleep(StdDuration::from_millis(50)).await;
                        }
                    }
                    anyhow::Ok(total)
                }
            });

            let chain_start = StdInstant::now();
            let mut submitted = 0usize;
            for (input, want) in wants {
                if self.chain.want(input, want) {
                    submitted += 1;
                }
            }
            if submitted > 0 {
                self.chain.flush().await.context("batched lifecycle step")?;
            }
            let chain_elapsed = chain_start.elapsed();

            stop.store(true, Ordering::Relaxed);
            let spent = firehose.await.context("firehose task panicked")??;
            let spend_elapsed = firehose_start.elapsed();

            report_round(round, spent, submitted, spend_elapsed, chain_elapsed);
            round += 1;

            if spent == 0 && submitted == 0 {
                tokio::time::sleep(StdDuration::from_secs(1)).await;
            }
        }
        Ok(())
    }

    /// Off-chain L2 totals, by dialect.
    fn print_l2_summary(&self) {
        let mut by_dialect = [
            (Dialect::Vanilla, 0usize, 0u64, 0u64),
            (Dialect::X402, 0, 0, 0),
        ];
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
        if let Some((p50, p90, p99, n)) = consumer::latency_percentiles() {
            println!(
                "canary: L2 latency -- {n} spend_burst round-trip(s), p50={p50:?} p90={p90:?} p99={p99:?}"
            );
        }
    }
}

/// The one stage `check`/`open_pending` know how to resume.
fn is_resumable(can: &[Can]) -> bool {
    can.iter()
        .any(|c| matches!(c, Can::Add | Can::Sub { .. } | Can::Close))
}

/// One line per pass, skipped when nothing happened. `spend_elapsed` is the firehose task's
/// own wall-clock span (what the req/s rate is computed from); `chain_elapsed` is the L1
/// chain step's span, reported separately since the two can differ substantially for a
/// `:burst` persona (see this function's call site).
fn report_round(
    round: usize,
    spent: usize,
    submitted: usize,
    spend_elapsed: StdDuration,
    chain_elapsed: StdDuration,
) {
    if spent == 0 && submitted == 0 {
        return;
    }
    let req_per_sec = spent as f64 / spend_elapsed.as_secs_f64().max(f64::EPSILON);
    println!(
        "canary: round {round}: {spent} spend request(s) in {spend_elapsed:?} ({req_per_sec:.1} req/s), {submitted} chain step(s) submitted in {chain_elapsed:?}"
    );
}

// 600s margin (was 120s, raised after live-reproduced silent Close failures under heavy load --
// accepted by submit, never landed, no rejection surfaced). Still a guess; the real fix is
// deriving it from observed chain inclusion lag. Side effect: a bigger margin delays Elapse
// becoming available for a straggler channel by the same amount, unrecoverably.
pub(crate) fn close_upper() -> SubbitInstant {
    crate::time::now() + Duration::from_secs(600)
}
