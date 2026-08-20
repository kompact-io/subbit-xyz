use std::path::Path;

use clap::{Parser, Subcommand};
use tokio::time::{Duration as TokioDuration, interval};
use tracing::{info, warn};

use subbit_index::client::Client;
use subbit_index::naive::{Config, rows_from_channels};

/// Naive client: polls chain state and posts every matching channel's
/// current (keytag -> backing) every tick. Closed channels post as
/// `Backing: None`.
#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    config: subbit_config::Args,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a demo config (TOML) to start from
    Init,
    /// Run the naive poller
    Run,
}

impl Cli {
    async fn run(self) -> anyhow::Result<()> {
        let sources = self
            .config
            .into_sources(Path::new("./subbit-naive-index-config.toml"));

        if let Command::Init = self.command {
            Config::write_default(sources.base)?;
            println!("wrote demo config to {}", sources.base.display());
            return Ok(());
        }

        let config: Config = sources.load()?;

        match self.command {
            Command::Init => unreachable!(),
            Command::Run => run(config).await,
        }
    }
}

async fn run(cfg: Config) -> anyhow::Result<()> {
    let mut session = cfg.session.clone().build().await?;
    session.init().await?;
    let client = Client::new(cfg.endpoint.clone());

    let mut ticker = interval(TokioDuration::from_secs(cfg.poll_interval_secs));
    loop {
        ticker.tick().await;

        if let Err(e) = session.reload_channels().await {
            warn!(error = %e, "reload_channels failed, will retry next tick");
            continue;
        }

        let rows = rows_from_channels(&session.channels(), &cfg);
        info!(count = rows.len(), "posting naive rows");
        match client.send(rows).await {
            Ok(result) => info!(status = result.status, "posted"),
            Err(e) => warn!(error = %e, "send failed, will retry next tick"),
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    Cli::parse().run().await
}
