mod consumer;
mod identity;
mod l1;
mod mock_chain;
mod orchestrator;
mod provider;
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
    /// Real Cardano preprod via Blockfrost. Needs BLOCKFROST_PROJECT_ID
    /// and CANARY_SIGNING_KEY_HEX (an already-funded wallet's raw
    /// 32-byte ed25519 signing key, hex-encoded) in the environment.
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
            println!("canary: building mock chain session");
            let (_mock, l1) = l1::L1::build_mock().await?;
            dispatch(cli, l2_seed, l1).await
        }
        Mode::Preprod => {
            let project_id = std::env::var("BLOCKFROST_PROJECT_ID")
                .context("BLOCKFROST_PROJECT_ID must be set for --mode preprod")?;
            let signing_key_hex = std::env::var("CANARY_SIGNING_KEY_HEX").context(
                "CANARY_SIGNING_KEY_HEX (hex-encoded 32-byte signing key of an already-funded \
                 preprod wallet) must be set for --mode preprod",
            )?;
            let signing_key: [u8; 32] = hex::decode(signing_key_hex.trim())
                .context("CANARY_SIGNING_KEY_HEX is not valid hex")?
                .try_into()
                .map_err(|_| anyhow::anyhow!("CANARY_SIGNING_KEY_HEX must decode to 32 bytes"))?;

            println!("canary: building preprod chain session (real Blockfrost, real wallet)");
            let l1 = l1::L1::build_preprod(project_id, signing_key).await?;
            dispatch(cli, l2_seed, l1).await
        }
    }
}

/// Picks `tidy`/`status`/the default `orchestrator::run`. `l1` arrives
/// already settled and ready to read (see `L1::build_preprod`).
async fn dispatch<C: cardano_connector::CardanoConnector>(
    cli: Cli,
    l2_seed: [u8; 32],
    mut l1: l1::L1<C>,
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
        let mut orchestrator = Orchestrator::new(l1, None, provider_identity.hash28, consumers, 0);
        return orchestrator.tidy().await;
    }

    let provider = provider::Provider::spawn_demo(
        cli.flat_cost,
        provider_identity,
        orchestrator::close_period(),
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

    let mut orchestrator = Orchestrator::new(
        l1,
        Some(provider),
        provider_identity.hash28,
        consumers,
        cli.resume,
    );
    if let Err(e) = orchestrator.check() {
        println!("canary: {e:#} -- tidying this run's own consumers before continuing");
        orchestrator.tidy().await?;
    }
    orchestrator.run().await
}
