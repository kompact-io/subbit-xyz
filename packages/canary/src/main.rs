mod chain;
mod consumer;
mod identity;
mod orchestrator;
mod process;
mod provider;
mod status;
mod time;

use std::sync::Arc;

use anyhow::Context;
use clap::Parser;

use consumer::{Consumer, Dialect};
use orchestrator::Orchestrator;

#[derive(Parser)]
struct Cli {
    /// One roster line, `<name>[:<modifier>]=<count>` -- e.g. `alice=5`, `richy:x402=3`.
    #[arg(long = "persona")]
    personas: Vec<String>,

    /// The catalog is a single route of fixed cost.
    #[arg(long, default_value_t = 100)]
    flat_cost: u64,

    /// Resume an interrupted run from this round instead of round 0.
    #[arg(long, default_value_t = 0)]
    resume: usize,

    /// Abandon the current scenario and tear down every channel.
    #[arg(long)]
    tidy: bool,

    /// Print current status (channels, stages, _etc_)
    #[arg(long)]
    status: bool,

    /// Upper bound on consumer *index* to scan for `--status`/`--tidy`.
    #[arg(long, default_value_t = 256)]
    scan_consumers: usize,

    /// Floe gRPC endpoint.
    #[arg(long, default_value = "https://grpc.floe.kompact.io")]
    floe_endpoint: String,

    /// Block depth the spawned server requires before treating a lineage item as backing.
    #[arg(long, default_value_t = 1)]
    finality_depth: u64,

    /// Skip the `Elapse` demo; every consumer settles normally via `subbit-claim` instead.
    #[arg(long)]
    no_elapse: bool,

    /// Fixed lovelace target each `:burst` persona commits to, overriding the
    /// default full-deposit stop condition -- e.g. `1000000` for exactly 1 ADA.
    #[arg(long)]
    burst_target_lovelace: Option<u64>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "warn".into()),
        )
        .init();

    // Debug-mode signing is ~50-60x slower -- throughput numbers from a debug run are meaningless.
    if cfg!(debug_assertions) {
        eprintln!(
            "canary: WARNING -- debug build, throughput numbers are meaningless. Use --release."
        );
    }

    let cli = Cli::parse();

    let l2_seed_hex = std::env::var("L2_SEED_HEX")
        .context("L2_SEED_HEX (hex-encoded 32-byte root seed) must be set")?;
    let l2_seed: [u8; 32] = hex::decode(l2_seed_hex.trim())
        .context("L2_SEED_HEX is not valid hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("L2_SEED_HEX must decode to 32 bytes"))?;

    let signing_key_hex = std::env::var("WALLET_SKEY").context(
        "WALLET_SKEY (hex-encoded 32-byte signing key of an already-funded preprod wallet) \
         must be set",
    )?;
    let signing_key: [u8; 32] = hex::decode(signing_key_hex.trim())
        .context("WALLET_SKEY is not valid hex")?
        .try_into()
        .map_err(|_| anyhow::anyhow!("WALLET_SKEY must decode to 32 bytes"))?;

    println!("canary: building preprod chain session (real Floe, real wallet)");
    let floe_endpoint = cli.floe_endpoint.clone();
    let chain = chain::Chain::build_preprod(floe_endpoint.clone(), signing_key).await?;
    dispatch(cli, l2_seed, chain, floe_endpoint).await
}

/// Picks `tidy`/`status`/the default `orchestrator::run`.
async fn dispatch(
    cli: Cli,
    l2_seed: [u8; 32],
    mut chain: chain::Chain,
    floe_endpoint: String,
) -> anyhow::Result<()> {
    if cli.status {
        return status::run(l2_seed, cli.scan_consumers, chain).await;
    }

    let provider_identity = identity::Identity::from_seed(identity::provider_seed(&l2_seed));
    chain.register("provider", provider_identity.seed())?;

    if cli.tidy {
        let mut consumers = Vec::with_capacity(cli.scan_consumers);
        for i in 0..cli.scan_consumers {
            let (identity, seed, tag) = consumer::identity_for(&l2_seed, i);
            chain.register(format!("consumer-{i}"), identity.seed())?;
            consumers.push(Arc::new(Consumer::new(
                seed,
                tag,
                None,
                Dialect::Vanilla,
                "",
            )?));
        }
        let mut orchestrator = Orchestrator::new_teardown(chain, provider_identity, consumers);
        return orchestrator.tidy().await;
    }

    let provider = provider::Provider::spawn_demo(
        cli.flat_cost,
        provider_identity,
        orchestrator::close_period(),
        cli.finality_depth,
    )
    .await?;
    let roster = consumer::plan_roster(&l2_seed, &cli.personas, cli.burst_target_lovelace)?;
    let mut consumers = Vec::with_capacity(roster.len());
    for (i, seat) in roster.into_iter().enumerate() {
        chain.register(format!("consumer-{i}"), seat.identity.seed())?;
        let consumer = consumer::build_consumer(
            seat.seed,
            seat.tag,
            provider.base_url(),
            seat.persona_name,
            seat.behavior,
        )
        .await?;
        consumers.push(Arc::new(consumer));
    }

    let claim_fee_identity = identity::Identity::from_seed(identity::claim_fee_seed(&l2_seed));
    chain.register("claim-fee", claim_fee_identity.seed())?;
    let mut orchestrator = Orchestrator::new(
        chain,
        provider,
        provider_identity,
        consumers,
        cli.resume,
        floe_endpoint,
        claim_fee_identity,
        !cli.no_elapse,
    );
    if let Err(e) = orchestrator.check() {
        println!("canary: {e:#} -- tidying this run's own consumers before continuing");
        orchestrator.tidy().await?;
    }
    orchestrator.run().await
}
