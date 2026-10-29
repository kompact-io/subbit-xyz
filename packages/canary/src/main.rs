mod consumer;
mod floe_connector;
mod identity;
mod l1;
mod mock_chain;
mod orchestrator;
mod provider;
mod services;
mod status;
mod time;

use std::sync::Arc;

use anyhow::Context;
use clap::Parser;

use consumer::{Consumer, ConsumerBuilder, Dialect};
use orchestrator::Orchestrator;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
enum Mode {
    /// In-memory chain, real UPLC validator evaluation, no real network
    /// or funded wallet needed.
    Mock,
    /// Real Cardano preprod via Floe. Needs WALLET_SKEY (an
    /// already-funded wallet's raw 32-byte ed25519 signing key,
    /// hex-encoded) in the environment.
    Preprod,
}

#[derive(Parser)]
struct Cli {
    #[arg(long, value_enum, default_value_t = Mode::Mock)]
    mode: Mode,

    /// One roster line, `<name>[:<dialect>]=<count>` -- e.g. `alice=5`,
    /// `richy:x402=3` (3 richy, x402-only). Repeatable. Known persona
    /// names: alice, richy. Falls back to `alice=3 richy=1` when omitted
    /// entirely. See `consumer::persona::parse_roster`.
    #[arg(long = "persona")]
    personas: Vec<String>,

    /// The catalog is a single route of fixed cost.
    #[arg(long, default_value_t = 100)]
    flat_cost: u64,

    /// If set resume for some round.
    #[arg(long, default_value_t = 0)]
    resume: usize,

    /// Abandon any scenario, and tear down every channel.
    /// Will still to know personas.
    #[arg(long)]
    tidy: bool,

    /// Print current status (channels, stages, _etc_)
    #[arg(long)]
    status: bool,

    /// FIXME :: what is this doing? Why arent we handling  larger cases.
    #[arg(long, default_value_t = 256)]
    scan_consumers: usize,

    /// Floe gRPC endpoint, in `--mode preprod` -- both `L1`'s own chain
    /// reads/writes and channel indexing go through it. Ignored in
    /// `--mode mock`, which has no live Floe server fronting its
    /// in-memory chain.
    #[arg(long, default_value = "https://grpc.floe.kompact.io")]
    floe_endpoint: String,

    /// Block depth the spawned server requires before it'll treat a
    /// lineage item as a backing candidate (`subbit_gate::Config`'s
    /// own `finality_depth`). Production wants this generous (guards
    /// against a rollback); canary wants it small, so a scenario run
    /// doesn't sit idle for however many blocks the production
    /// default demands after every `Open` before anything's spendable.
    #[arg(long, default_value_t = 1)]
    finality_depth: u64,

    /// Opens a decoy "mimic" channel sharing roster consumer 0's
    /// `(iou_key, tag)` under a different `consumer` identity, closes it
    /// early (while consumer 0's "real" channel is still `Opened`), then
    /// drives the rest of the scenario
    #[arg(long)]
    mimic: bool,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    // Debug-mode signing measured ~50-60x slower than release (see
    // plan-fix-canary.md) -- throughput numbers from a debug run are
    // meaningless. Unconditional, not log-gated, on purpose.
    if cfg!(debug_assertions) {
        eprintln!(
            "canary: WARNING -- debug build, throughput numbers are meaningless. Use --release."
        );
    }

    let cli = Cli::parse();

    // Root seed
    let l2_seed: [u8; 32] = match std::env::var("L2_SEED_HEX") {
        Ok(hex_str) => hex::decode(hex_str.trim())
            .context("L2_SEED_HEX is not valid hex")?
            .try_into()
            .map_err(|_| anyhow::anyhow!("L2_SEED_HEX must decode to 32 bytes"))?,
        Err(_) if cli.mode == Mode::Preprod => {
            anyhow::bail!(
                "L2_SEED_HEX must be set for --mode preprod, so identities stay reproducible \
                 across runs against the same real chain"
            )
        }
        Err(_) => [0u8; 32],
    };

    match cli.mode {
        Mode::Mock => {
            if cli.mimic {
                eprintln!(
                    "canary: WARNING -- --mimic needs --mode preprod (real Floe/real spawned \
                     binaries); ignoring it under --mode mock"
                );
            }
            println!("canary: building mock chain session");
            let (_mock, l1) = l1::L1::build_mock().await?;
            dispatch(cli, l2_seed, l1, None).await
        }
        Mode::Preprod => {
            let signing_key_hex = std::env::var("WALLET_SKEY").context(
                "WALLET_SKEY (hex-encoded 32-byte signing key of an already-funded preprod \
                 wallet) must be set for --mode preprod",
            )?;
            let signing_key: [u8; 32] = hex::decode(signing_key_hex.trim())
                .context("WALLET_SKEY is not valid hex")?
                .try_into()
                .map_err(|_| anyhow::anyhow!("WALLET_SKEY must decode to 32 bytes"))?;

            println!("canary: building preprod chain session (real Floe, real wallet)");
            let floe_endpoint = cli.floe_endpoint.clone();
            let l1 = l1::L1::build_preprod(floe_endpoint.clone(), signing_key).await?;
            dispatch(cli, l2_seed, l1, Some(floe_endpoint)).await
        }
    }
}

/// FIXME :: This implies that status should be subcommands.
/// Tidy may also be a subcommand or, it maybe treated as vaiant of `round`
/// that skips to the "tidy" phase.
///
/// Picks `tidy`/`status`/the default `orchestrator::run`. `l1` arrives
/// already settled and ready to read (see `L1::build_preprod`).
/// `floe_endpoint` is `Some` only for `--mode preprod` -- see
/// `Orchestrator::floe_endpoint`'s own doc.
async fn dispatch<C: cardano_connector::CardanoConnector>(
    cli: Cli,
    l2_seed: [u8; 32],
    mut l1: l1::L1<C>,
    floe_endpoint: Option<String>,
) -> anyhow::Result<()> {
    if cli.status {
        return status::run(l2_seed, cli.scan_consumers, l1).await;
    }

    let provider_identity = identity::Identity::from_seed(identity::provider_seed(&l2_seed));
    l1.register(provider_identity.signing_key());

    if cli.tidy {
        let mut consumers = Vec::with_capacity(cli.scan_consumers);
        for i in 0..cli.scan_consumers {
            let (identity, seed, tag) = consumer::identity_for(&l2_seed, i);
            l1.register(identity.signing_key());
            consumers.push(Arc::new(Consumer::new(
                seed,
                tag,
                None,
                Dialect::Vanilla,
                "",
            )?));
        }
        // Also reclaims any `--mimic` decoy channels, unconditionally --
        // `--tidy`'s whole point is tearing down leftover state
        // regardless of what scenario produced it, so this shouldn't
        // need the caller to remember `--mimic` too. A decoy opens
        // against real consumer `i`'s own `(iou_key, tag)`, one per
        // consumer (see `MimicScenario`'s own doc); if none was ever
        // opened for a given `i`, that one is just a harmless no-op
        // lookup. `Consumer::new` ties `l1_key`/`iou_key` to the one seed
        // it's given, so there's no constructor for "mimic `i`'s l1_key,
        // consumer `i`'s iou_key" directly -- build from the mimic's own
        // seed (which correctly derives its `l1_key`), then overwrite
        // just `iou_key`/`tag` to match what the decoy actually shares
        // with consumer `i`. `want`'s own Close/Elapse decision never
        // reads `iou_key` at all, only `L1::can_for_consumer`'s lookup
        // does -- this is the one place that lookup needs to land on.
        let real_consumers = consumers.clone();
        for (i, consumer) in real_consumers.iter().enumerate() {
            let mimic_identity = identity::Identity::from_seed(identity::mimic_seed(&l2_seed, i));
            l1.register(mimic_identity.signing_key());
            let mut mimic_consumer = Consumer::new(
                mimic_identity.seed(),
                consumer.tag.clone(),
                None,
                Dialect::Vanilla,
                "",
            )?;
            mimic_consumer.iou_key = consumer.iou_key;
            consumers.push(Arc::new(mimic_consumer));
        }
        // `tidy()` never calls `index()`, so the Floe endpoint is moot here.
        let mut orchestrator =
            Orchestrator::new(l1, None, provider_identity.hash28, consumers, 0, None, None);
        return orchestrator.tidy().await;
    }

    let provider = provider::Provider::spawn_demo(
        cli.flat_cost,
        provider_identity,
        orchestrator::close_period(),
        cli.finality_depth,
    )
    .await?;
    let roster = consumer::plan_roster(&l2_seed, &cli.personas)?;
    let mut consumers = Vec::with_capacity(roster.len());
    for seat in roster {
        l1.register(seat.identity.signing_key());
        let consumer = ConsumerBuilder::new(seat.seed, seat.tag, provider.base_url())
            .behavior(seat.persona_name, seat.behavior)
            .build()
            .await?;
        consumers.push(Arc::new(consumer));
    }

    // Already warned (mock mode) or `floe_endpoint` is genuinely absent
    // -- either way, silently skip rather than erroring out of a run
    // that otherwise asked for nothing unusual.
    let mimic_scenario = if cli.mimic && floe_endpoint.is_some() {
        anyhow::ensure!(
            !consumers.is_empty(),
            "--mimic needs at least one consumer in the roster to pose as"
        );
        // One decoy per real consumer, built the same way `--tidy`'s own
        // mimic cleanup already does: `Consumer::new` correctly derives
        // `l1_key` from the mimic's own seed, then `iou_key`/`tag`/
        // `deposit` are overwritten to match the target it's posing
        // alongside. `.tidy()` immediately -- unlike a real consumer, a
        // decoy has no persona schedule to run first; it should head
        // for `Close` on round 0, same as a straggler already would.
        // Prepended, not appended: `Orchestrator::new`'s own
        // `consumers.last()` marks the *real* roster's own straggler to
        // demonstrate `Elapse` -- appending decoys after it would steal
        // that off a mimic instead.
        let mut mimics = Vec::with_capacity(consumers.len());
        for (i, consumer) in consumers.iter().enumerate() {
            let mimic_identity = identity::Identity::from_seed(identity::mimic_seed(&l2_seed, i));
            l1.register(mimic_identity.signing_key());
            let mut mimic_consumer = Consumer::new(
                mimic_identity.seed(),
                consumer.tag.clone(),
                None,
                Dialect::Vanilla,
                "",
            )?;
            mimic_consumer.iou_key = consumer.iou_key;
            mimic_consumer.deposit = 5_000_000;
            mimic_consumer.tidy();
            mimics.push(Arc::new(mimic_consumer));
        }
        mimics.extend(consumers);
        consumers = mimics;

        let claim_fee_identity = identity::Identity::from_seed(identity::claim_fee_seed(&l2_seed));
        Some(orchestrator::MimicScenario {
            claim_fee_identity,
            provider_identity,
        })
    } else {
        None
    };

    let mut orchestrator = Orchestrator::new(
        l1,
        Some(provider),
        provider_identity.hash28,
        consumers,
        cli.resume,
        floe_endpoint,
        mimic_scenario,
    );
    if let Err(e) = orchestrator.check() {
        println!("canary: {e:#} -- tidying this run's own consumers before continuing");
        orchestrator.tidy().await?;
    }
    orchestrator.run().await
}
