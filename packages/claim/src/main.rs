use std::collections::BTreeMap;
use std::path::Path;
use std::time::Duration as StdDuration;

use anyhow::Context;
use cardano_sdk::{Address, Credential, Hash, SigningKey, address::kind};
use clap::{Parser, Subcommand};
use subbit_tx::tx::Tx;
use tokio::time::{Duration as TokioDuration, interval};
use tracing::{info, warn};

use subbit_claim::client::Client;
use subbit_claim::config::Config;
use subbit_claim::decide::{self, DecisionConfig};
use subbit_claim::floe;
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

fn now() -> subbit_core::Instant {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis() as u64;
    subbit_core::Instant::from_millis(ms)
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
        let network_parameters = config.network_parameters();
        let network_id = network_parameters.network_id;

        let fee_signing_key: SigningKey = config.fee_key.clone().into();
        let fee_vkh = Hash::<28>::new(fee_signing_key.to_verification_key());
        let fee_pay_hash = <[u8; 28]>::from(fee_vkh);
        let change_address: Address<kind::Any> =
            Address::new(network_id, Credential::from_key(fee_vkh)).into();

        let keyring = config.keyring.clone().build();
        anyhow::ensure!(
            !keyring.vkhs().is_empty(),
            "no signing keys configured -- subbit-claim needs at least the provider's own key"
        );

        let mut floe = config.floe.connect().await.context("connecting to Floe")?;

        let host_pay_hash = match &config.ref_script_host {
            Some(host) => <[u8; 28]>::from(host.payment().select(|h| h, |h| h)),
            None => fee_pay_hash,
        };
        let ref_script = floe::find_ref_script(&mut floe, host_pay_hash)
            .await
            .context("looking up subbit validator reference script")?
            .context(
                "no subbit validator reference script tracked yet -- upload one first (e.g. via subbit-cli)",
            )?;

        let client = Client::new(config.server_endpoint.clone());
        let decision_cfg = DecisionConfig {
            min_total: config.min_total,
            min_single: config.min_single,
            panic_settle: subbit_core::Duration::from_millis(config.panic_settle_ms),
        };
        let wait_interval = StdDuration::from_secs(config.wait_interval_secs);

        let mut ticker = interval(TokioDuration::from_secs(config.poll_interval_secs));
        loop {
            ticker.tick().await;

            let utxos = match floe::search_all(&mut floe).await {
                Ok(utxos) => utxos,
                Err(e) => {
                    warn!(error = %e, "searching Floe failed, will retry next tick");
                    continue;
                }
            };
            let channels: BTreeMap<_, _> = utxos
                .iter()
                .filter_map(|utxo| {
                    let channel = floe::channel_from_utxo(utxo).ok()?;
                    is_ours(channel.constants(), &keyring)
                        .then(|| (floe::to_input(&utxo.txo_ref), channel))
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
                // One `?`-free unit per chunk, matching the
                // search_all/channel_summaries fetches above: any
                // failure anywhere in here (including a slow/stuck
                // confirmation wait -- preprod confirmation lag is
                // real-world variance, not a reason to take the whole
                // daemon down) just warns and retries fresh next tick,
                // rather than propagating out of `run` and exiting.
                match submit_claim_batch(
                    &mut floe,
                    chunk,
                    &network_parameters,
                    &ref_script,
                    &change_address,
                    &keyring,
                    &fee_signing_key,
                    fee_pay_hash,
                    config.wait_attempts,
                    wait_interval,
                )
                .await
                {
                    Ok(id) => info!(%id, "claim batch confirmed"),
                    Err(e) => {
                        warn!(error = %e, "submitting claim batch failed, will retry next tick");
                        break;
                    }
                }
            }
        }
    }
}

/// Builds, signs, submits and waits-for-confirmation of one claim batch.
/// Refetches `channel_utxos`/`fuel` fresh rather than reusing anything
/// cached across chunks -- a prior chunk in this same tick may already
/// have spent some of these, and Floe reads are cheap enough that
/// there's no need to track staleness instead.
#[allow(clippy::too_many_arguments)]
async fn submit_claim_batch(
    floe: &mut floe_client::grpc::Client,
    chunk: &[(cardano_sdk::Input, subbit_tx::step::Want)],
    network_parameters: &subbit_tx::NetworkParameters,
    ref_script: &(cardano_sdk::Input, cardano_sdk::Output),
    change_address: &cardano_sdk::Address<kind::Any>,
    keyring: &subbit_claim::keyring::Keyring,
    fee_signing_key: &SigningKey,
    fee_pay_hash: [u8; 28],
    wait_attempts: u32,
    wait_interval: StdDuration,
) -> anyhow::Result<Hash<32>> {
    let channel_utxos = floe::search_all(floe)
        .await
        .context("refetching channel utxos")?
        .iter()
        .filter_map(|u| floe::to_input_output(u).ok())
        .collect::<BTreeMap<_, _>>();
    let fuel = floe::search_by_pay_hash(floe, fee_pay_hash, None)
        .await
        .context("refetching fee-wallet utxos")?
        .iter()
        .filter_map(|u| floe::to_input_output(u).ok())
        .collect::<BTreeMap<_, _>>();

    let mut tx = Tx::new(channel_utxos).context("staging tx")?;
    for (input, want) in chunk {
        tx.propose(input.clone(), want.clone())
            .context("proposing step")?;
    }
    let mut built = tx
        .build(
            network_parameters,
            Some(ref_script),
            change_address.clone(),
            &fuel,
        )
        .context("building tx")?;
    keyring
        .sign(&mut built)
        .context("signing required signatories")?;
    built.sign(fee_signing_key);

    let id = floe::submit(floe, &built).await.context("submitting tx")?;
    floe::wait_for_tx(floe, id, wait_attempts, wait_interval)
        .await
        .context("waiting for tx confirmation")?;
    Ok(id)
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
