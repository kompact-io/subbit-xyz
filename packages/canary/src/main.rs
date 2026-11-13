mod catalog;
mod chain;
mod config;
mod consumer;
mod identity;
mod orchestrator;
mod process;
mod provider;
mod status;
mod time;

use std::path::Path;

use anyhow::Context;
use clap::{Parser, Subcommand};

use consumer::{AgentConfig, Consumer};
use orchestrator::Orchestrator;

const CLAIM_FEE_LOVELACE: u64 = 5_000_000;

#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    sources: subbit_config::Args,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a starter config to --config and exit.
    Init,
    /// Run the scenario.
    Run,
    /// Print current status (channels, stages, _etc_).
    Status,
    /// Abandon the current scenario and tear down every channel.
    Tidy,
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

    let mut sources = cli
        .sources
        .into_sources(Path::new("subbit-canary-config.toml"));
    // The existing `L2_SEED_HEX`/`WALLET_SKEY` env vars are unprefixed.
    sources.env_prefix = Some(sources.env_prefix.unwrap_or(""));

    if matches!(cli.command, Command::Init) {
        config::Config::write_default(sources.base)?;
        println!("wrote starter config to {}", sources.base.display());
        return Ok(());
    }

    let config = config::Config::load(&sources)?;
    let command = cli.command;

    let l2_seed = config.l2_seed()?;
    let signing_key = config.wallet_skey()?;

    println!("canary: building preprod chain session (Floe, wallet)");
    let chain = chain::Chain::build_preprod(config.floe_endpoint.clone(), signing_key).await?;
    dispatch(config, command, l2_seed, chain).await
}

/// Picks `tidy`/`status`/the default `orchestrator::run`.
async fn dispatch(
    config: config::Config,
    command: Command,
    l2_seed: [u8; 32],
    mut chain: chain::Chain,
) -> anyhow::Result<()> {
    if matches!(command, Command::Status) {
        return status::run(l2_seed, config.scan_consumers, chain).await;
    }

    let elapse_margin = subbit_core::Duration::from_secs(config.elapse_margin_secs);
    let close_period = subbit_core::Duration::from_secs(config.close_period_secs);

    let provider_identity = identity::Identity::from_seed(identity::provider_seed(&l2_seed));
    chain.register("provider", provider_identity.seed(), chain::Role::Provider)?;
    let claim_wallet = identity::Identity::from_seed(identity::claim_wallet_seed(&l2_seed));
    chain.register("claim-wallet", claim_wallet.seed(), chain::Role::Consumer)?;

    if matches!(command, Command::Tidy) {
        let mut consumers = Vec::with_capacity(config.scan_consumers);
        for i in 0..config.scan_consumers {
            let (identity, seed, tag) = consumer::identity_for(&l2_seed, i);
            chain.register(
                format!("consumer-{i}"),
                identity.seed(),
                chain::Role::Consumer,
            )?;
            consumers.push(Consumer::init(seed, tag, None, elapse_margin).await?);
        }
        let mut orchestrator = Orchestrator::new_tidy(
            chain,
            provider_identity,
            consumers,
            config.floe_endpoint,
            claim_wallet,
            close_period,
        );
        return orchestrator.tidy().await;
    }

    // subbit-claim pays its own fees/collateral, so its wallet is funded before it's spawned.
    chain
        .fund_address(claim_wallet.address(chain.network_id()), CLAIM_FEE_LOVELACE)
        .await
        .context("funding the spawned subbit-claim's own wallet")?;

    let provider_config = config.provider_config(
        provider_identity,
        claim_wallet,
        chain.change_address_shelley(),
    );
    let provider = provider::Provider::spawn(provider_config).await?;
    let roster = consumer::plan_roster(&l2_seed, &config.personas, config.burst_target_lovelace)?;
    let mut consumers = Vec::with_capacity(roster.len());
    for (i, consumer) in roster.into_iter().enumerate() {
        chain.register(
            format!("consumer-{i}"),
            consumer.identity.seed(),
            chain::Role::Consumer,
        )?;
        let consumer = Consumer::init(
            consumer.seed,
            consumer.tag,
            Some(AgentConfig {
                provider_base_url: provider.base_url(),
                behavior: consumer.behavior,
            }),
            elapse_margin,
        )
        .await?;
        consumers.push(consumer);
    }

    let mut orchestrator = Orchestrator::new(
        chain,
        provider,
        provider_identity,
        consumers,
        config.resume,
        config.max_rounds,
        config.round_poll_upper_bound_secs,
        config.floe_endpoint,
        claim_wallet,
        config.require_claim_win,
        close_period,
    );
    if let Err(e) = orchestrator.check() {
        println!("canary: {e:#} -- tidying this run's own consumers before continuing");
        orchestrator.tidy().await?;
    }
    orchestrator.run().await
}
