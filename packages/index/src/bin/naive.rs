use std::iter;
use std::path::Path;

use cardano_connector::CardanoConnector;

use clap::{Parser, Subcommand};
use futures::future::try_join_all;
use subbit_tx::VALIDATOR;
use tokio::time::{Duration as TokioDuration, interval};
use tracing::{info, warn};

use subbit_index::client::Client;
use subbit_index::naive::{Config, rows_from_channels};

/// Naive client: polls chain state and posts every matching channel's
/// current (keytag -> backing) every tick. Closed channels post as
/// `Backing: None`.
#[derive(Parser)]
#[command(
    name = "naive-index",
    about = "Submits whatever it sees at tip as backing (after filter)",
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
        let cardano = config.cardano.clone().build();
        let client = Client::new(config.endpoint.clone());
        let delegations = iter::once(None)
            .chain(config.delegations.clone().into_iter().map(Some))
            .collect::<Vec<_>>();
        let subbit_cred = VALIDATOR.to_credential();
        let mut ticker = interval(TokioDuration::from_secs(config.poll_interval_secs));
        loop {
            ticker.tick().await;

            let utxos = try_join_all(
                delegations
                    .iter()
                    .map(|d| cardano.utxos_at(&subbit_cred, d.as_ref())),
            )
            .await?
            .into_iter()
            .flatten()
            .collect();
            let rows = rows_from_channels(&utxos, &config);
            info!(count = rows.len(), "posting naive rows");
            match client.send(rows).await {
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
        .init();

    Cli::parse().run().await
}
