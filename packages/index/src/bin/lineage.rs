use std::path::Path;

use clap::{Parser, Subcommand};
use tokio::time::{Duration as TokioDuration, interval};
use tracing::{info, warn};

use subbit_index::lineage::build_channels;
use subbit_index::{Config, Gate};

pub fn write_config(config: &Config, path: &Path) -> anyhow::Result<()> {
    std::fs::write(path, toml::to_string_pretty(config)?)?;
    Ok(())
}

#[derive(Parser)]
#[command(
    name = "lineage-index",
    about = "Reconstructs full channel lineage from Floe and posts it to /v1/a/index (mimic-aware)",
    version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")"),
)]
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
    /// Run the lineage poller
    Run,
}

impl Cli {
    async fn run(self) -> anyhow::Result<()> {
        let sources = self
            .config
            .into_sources(Path::new("./subbit-lineage-index-config.toml"));

        if let Command::Init = self.command {
            write_config(&Config::default(), sources.base)?;
            println!("wrote demo config to {}", sources.base.display());
            return Ok(());
        }

        let config: Config = sources.load()?;
        let mut floe = config.cardano.connect().await?;
        let gate = Gate::new(config.gate.clone());
        let mut ticker = interval(TokioDuration::from_secs(config.poll_interval_secs));
        loop {
            ticker.tick().await;

            let output = match build_channels(&mut floe).await {
                Ok(output) => output,
                Err(e) => {
                    warn!(error = %e, "building lineage output failed, will retry next tick");
                    continue;
                }
            };
            info!(count = output.channels.len(), "posting lineage output");
            match gate.send_output(&output).await {
                Ok(result) => info!(status = result.status, "posted"),
                Err(e) => warn!(error = %e, "send failed, will retry next tick"),
            }
        }
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .with_writer(std::io::stderr)
        .init();

    Cli::parse().run().await
}
