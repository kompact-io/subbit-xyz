use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
use std::time::Duration as StdDuration;

use anyhow::Context;
use cardano_sdk::{Address, Credential, Hash, Input, SigningKey, address::kind};
use clap::{Parser, Subcommand};
use subbit_tx::{
    Bounds,
    queue::{BuildError, Queue},
};
use tokio::time::{Duration as TokioDuration, interval};
use tracing::{info, warn};

use subbit_claim::client::Client;
use subbit_claim::config::Config;
use subbit_claim::decide::{self, DecisionConfig};
use subbit_claim::floe;
use subbit_claim::keytag::is_ours;
use subbit_claim::pending::{Pending, Resolution};

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

/// amaru's own wording for "I already have this exact tx" (confirmed live against
/// preprod: `rejected by amaru: Transaction is a duplicate`) -- not a failure, just a
/// sign this rebroadcast was redundant and should back off rather than repeat.
fn is_duplicate_rejection(message: &str) -> bool {
    message.to_lowercase().contains("duplicate")
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

        let wallet_signing_key: SigningKey = config.wallet_key.clone().into();
        let wallet_vkh = Hash::<28>::new(wallet_signing_key.to_verification_key());
        let wallet_pay_hash = <[u8; 28]>::from(wallet_vkh);
        let change_address: Address<kind::Any> =
            Address::new(network_id, Credential::from_key(wallet_vkh)).into();

        let keyring = config.keyring.clone().build();
        anyhow::ensure!(
            !keyring.vkhs().is_empty(),
            "no signing keys configured -- subbit-claim needs at least the provider's own key"
        );

        let mut floe = config.floe.connect().await.context("connecting to Floe")?;

        let host_pay_hash = match &config.ref_script_host {
            Some(host) => <[u8; 28]>::from(host.payment().select(|h| h, |h| h)),
            None => wallet_pay_hash,
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
        let rebroadcast_interval = StdDuration::from_secs(config.wait_interval_secs);
        let mut pending = Pending::default();

        let mut ticker = interval(TokioDuration::from_secs(config.poll_interval_secs));
        loop {
            ticker.tick().await;

            let utxos = match floe::search_all(&mut floe).await {
                Ok(utxos) => utxos,
                Err(e) => {
                    warn!(error = ?e, "searching Floe failed, will retry next tick");
                    continue;
                }
            };
            let all_channels: BTreeMap<_, _> = utxos
                .iter()
                .filter_map(|utxo| {
                    let channel = floe::channel_from_utxo(utxo).ok()?;
                    is_ours(channel.constants(), &keyring)
                        .then(|| (floe::to_input(&utxo.txo_ref), channel))
                })
                .collect();

            // Reconcile submissions from earlier ticks before deciding what
            // is spendable now. A positive `ReadTx` means landed; a channel
            // input missing from the live set means the tx can never land,
            // because something else already spent that input.
            let mut landed = BTreeSet::new();
            for id in pending.ids().collect::<Vec<_>>() {
                match floe::read_tx(&mut floe, id).await {
                    Ok(true) => {
                        landed.insert(id);
                    }
                    Ok(false) => {}
                    Err(e) => warn!(
                        %id,
                        error = ?e,
                        "checking an in-flight claim batch failed, leaving it pending"
                    ),
                }
            }
            let live_inputs: BTreeSet<Input> = all_channels.keys().cloned().collect();
            for (id, resolution) in pending.advance(&landed, &live_inputs) {
                match resolution {
                    Resolution::Landed => info!(%id, "in-flight claim batch confirmed"),
                    Resolution::Dead => warn!(
                        %id,
                        "in-flight claim batch can no longer land -- its channel input was spent \
                         by another tx; freeing its inputs for a fresh attempt"
                    ),
                }
            }

            // Rebroadcast whatever is still pending, byte-for-byte. A tx Floe
            // accepted but never observed is most likely to land by being
            // offered again; a duplicate submission is harmless.
            let observed_at = std::time::Instant::now();
            let due = pending.due_for_rebroadcast(observed_at, rebroadcast_interval);
            let mut rebroadcast = Vec::new();
            let mut duplicate_rejected = Vec::new();
            for (id, cbor) in &due {
                match floe::submit_cbor(&mut floe, cbor).await {
                    Ok(()) => {
                        info!(%id, "rebroadcast in-flight claim batch");
                        rebroadcast.push(*id);
                    }
                    // amaru already has this exact tx -- it's not failed, just not yet
                    // reflected back through our own confirmation view. Back off this
                    // one specifically rather than hammering it every tick for however
                    // long that confirmation lag lasts (observed live: several minutes).
                    Err(e) if is_duplicate_rejection(&e.to_string()) => {
                        warn!(
                            %id,
                            error = ?e,
                            "rebroadcast rejected as a duplicate -- already landed or still \
                             pending under amaru's own view, backing off"
                        );
                        duplicate_rejected.push(*id);
                    }
                    Err(e) => warn!(
                        %id,
                        error = ?e,
                        "rebroadcasting an in-flight claim batch failed, will retry"
                    ),
                }
            }
            pending.note_rebroadcast(&rebroadcast, observed_at);
            pending.note_duplicate_rejected(&duplicate_rejected, observed_at);

            // Anything an in-flight tx already spends must not be re-proposed
            // or re-selected as fuel until that tx either lands or is dead.
            let blocked = pending.inputs();
            let channels: BTreeMap<_, _> = all_channels
                .into_iter()
                .filter(|(input, _)| !blocked.contains(input))
                .collect();

            let summaries = match client.channel_summaries().await {
                Ok(summaries) => summaries,
                Err(e) => {
                    warn!(error = ?e, "fetching channel summaries failed, will retry next tick");
                    continue;
                }
            };

            let wants = decide::decide(&channels, &summaries, &decision_cfg, now());
            if wants.is_empty() {
                info!(pending = pending.len(), "nothing to claim this tick");
                continue;
            }
            info!(
                count = wants.len(),
                pending = pending.len(),
                "submitting claims this tick"
            );

            let channel_outputs: BTreeMap<_, _> = utxos
                .iter()
                .filter_map(|utxo| floe::to_input_output(utxo).ok())
                .filter(|(input, _)| channels.contains_key(input))
                .collect();

            let mut queue = Queue::new(network_parameters.clone(), config.build_config.clone());
            queue
                .settings_mut()
                .set_change_address(change_address.clone());
            if let Err(e) = queue
                .settings_mut()
                .set_reference_utxo(ref_script.0.clone(), ref_script.1.clone())
            {
                warn!(error = ?e, "reference utxo rejected, will retry next tick");
                continue;
            }
            if let Err(e) = queue.add_utxos(channel_outputs) {
                warn!(error = ?e, "staging channel utxos failed, will retry next tick");
                continue;
            }
            for (input, want) in wants {
                if let Err(e) = queue.propose(input.clone(), want) {
                    warn!(%input, error = ?e, "a claim candidate no longer validates, skipping it");
                }
            }

            // One submitted tx per `Queue::pick` -- drained by its own
            // measured byte budget (`config.build_config.max_tx_size`),
            // not a fixed-count chunk -- until nothing more is staged
            // or something fails. A successful submission is tracked in
            // `pending` rather than waited on here, so the next tick can
            // keep rebroadcasting it while also making progress on other
            // channels. Any failure just warns and retries next tick,
            // rather than propagating out of `run` and exiting.
            loop {
                if let Err(e) = queue.pick(Bounds::default()) {
                    if !matches!(e, BuildError::Empty) {
                        warn!(error = ?e, "picking a claim batch failed, will retry next tick");
                    }
                    break;
                }

                let blocked = pending.inputs();
                let fuel: BTreeMap<_, _> = match floe::search_by_pay_hash(
                    &mut floe,
                    wallet_pay_hash,
                    None,
                )
                .await
                {
                    Ok(utxos) => utxos
                        .iter()
                        .filter_map(|u| floe::to_input_output(u).ok())
                        .filter(|(input, _)| !blocked.contains(input))
                        .collect(),
                    Err(e) => {
                        warn!(error = ?e, "refetching fee-wallet utxos failed, will retry next tick");
                        queue.unpick();
                        break;
                    }
                };

                let mut built = match queue.build(&fuel) {
                    Ok(tx) => tx,
                    Err(e) => {
                        warn!(error = ?e, "building claim batch failed, will retry next tick");
                        queue.unpick();
                        break;
                    }
                };
                let id = built.id();

                if let Err(e) = keyring.sign(&mut built) {
                    warn!(error = ?e, "signing claim batch failed, will retry next tick");
                    queue.rollback(id);
                    break;
                }
                built.sign(&wallet_signing_key);

                match floe::submit(&mut floe, &built).await {
                    Ok(submitted) => {
                        let channel_inputs: BTreeSet<Input> = built
                            .inputs()
                            .filter(|input| channels.contains_key(input))
                            .collect();
                        info!(%submitted, "claim batch submitted; tracking until it lands");
                        pending.insert(&built, channel_inputs);
                        queue.finalize(submitted);
                    }
                    Err(e) => {
                        warn!(error = ?e, "submitting claim batch failed, will retry next tick");
                        queue.rollback(id);
                        break;
                    }
                }
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
