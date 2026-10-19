use std::collections::BTreeMap;
use std::path::Path;

use anyhow::Context;
use cardano_connector_direct::Blockfrost;
use cardano_wallet::Embedded;
use clap::{Parser, Subcommand};
use subbit_session::Session;
use subbit_tx::Channel;
use tokio::time::{Duration as TokioDuration, interval};
use tracing::{info, warn};

use subbit_claim::client::Client;
use subbit_claim::config::Config;
use subbit_claim::decide::{self, DecisionConfig};
use subbit_claim::keytag::is_ours;

#[derive(Parser)]
#[command(
    name = "subbit-claim",
    about = "Provider tx service: Sub/Settle against this provider's own channels",
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
    /// Run the claiming poller
    Run,
}

fn now() -> subbit_core::Duration {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis() as u64;
    subbit_core::Duration::from_millis(ms)
}

impl Cli {
    async fn run(self) -> anyhow::Result<()> {
        let sources = self
            .config
            .into_sources(Path::new("./subbit-claim-config.toml"));

        if let Command::Init = self.command {
            Config::write_default(sources.base)?;
            println!("wrote demo config to {}", sources.base.display());
            return Ok(());
        }

        let config: Config = sources.load()?;
        let mut session: Session<Blockfrost, Embedded<Blockfrost>> = config
            .session
            .clone()
            .build()
            .await
            .context("building chain session")?;
        session
            .refresh_all()
            .await
            .context("fetching initial chain state")?;
        session
            .ref_script()
            .context("no subbit validator reference script tracked yet -- upload one first (e.g. via subbit-cli)")?;

        let keyring = config.keyring.clone().build();
        anyhow::ensure!(
            !keyring.vkhs().is_empty(),
            "no signing keys configured -- subbit-claim needs at least the provider's own key"
        );

        let client = Client::new(config.server_endpoint.clone());
        let decision_cfg = DecisionConfig {
            min_total: config.min_total,
            min_single: config.min_single,
            panic_settle: subbit_core::Duration::from_millis(config.panic_settle_ms),
        };

        let mut ticker = interval(TokioDuration::from_secs(config.poll_interval_secs));
        loop {
            ticker.tick().await;

            session
                .refresh_channels()
                .await
                .context("refreshing tracked channels")?;

            let channels: BTreeMap<_, _> = session
                .channels()
                .into_iter()
                .filter_map(|(input, output)| {
                    let channel = Channel::try_from(&output).ok()?;
                    is_ours(channel.constants(), &keyring).then_some((input, channel))
                })
                .collect();

            let summaries = match client.channel_summaries().await {
                Ok(summaries) => summaries,
                Err(e) => {
                    warn!(error = %e, "fetching channel summaries failed, will retry next tick");
                    continue;
                }
            };

            let wants = decide::decide(&channels, &summaries, &decision_cfg, now());
            if wants.is_empty() {
                info!("nothing to claim this tick");
                continue;
            }
            info!(count = wants.len(), "submitting claims this tick");

            for chunk in wants.chunks(config.step_chunk) {
                let mut tx = session.stage_tx().context("staging tx")?;
                for (input, want) in chunk {
                    tx.propose(input.clone(), want.clone())
                        .context("proposing step")?;
                }
                let mut built = session.build_tx(&mut tx).context("building tx")?;
                keyring
                    .sign(&mut built)
                    .context("signing required signatories")?;
                let id = session
                    .sign_and_submit(built)
                    .await
                    .context("signing and submitting tx")?;
                session
                    .wait_til(&id)
                    .await
                    .context("waiting for tx confirmation")?;
                info!(%id, "claim batch confirmed");
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
