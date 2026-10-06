// session.rs

use std::collections::BTreeMap;
use std::path::Path;

use anyhow::{Context, Result};
use cardano_connector::CardanoConnector;
use cardano_connector_direct::Blockfrost;
use cardano_sdk::{Credential, Hash, Input};
use cardano_session::Session as CardanoSession;
use cardano_wallet::{Embedded, Wallet};
use clap::Subcommand;
use serde::Serialize;

use cardano_session::tip::TipVec;
use subbit_session::Session;
use subbit_tx::{Channel, VALIDATOR};

use crate::{cache, ctx::Ctx, json_label};

#[derive(Subcommand)]
pub enum Cmd {
    /// Force a full refresh: bypass the tip/addressbook cache and refetch
    /// from chain, then re-persist the cache.
    Refresh,
    /// Rebuild the session against current chain state and print a summary.
    Status,
    /// Upload the subbit validator's reference script to the wallet.
    Upload,
    /// Reclaim a tracked reference script back into the wallet. Default to subbit
    Teardown { hash: Option<Hash<28>> },
    /// Start tracking channels at an additional delegation. TODO:: UNTESTED
    AddDelegation { credential: Credential },
    /// Stop tracking channels at a delegation. TODO:: UNTESTED
    RemoveDelegation { credential: Credential },
}

impl Cmd {
    pub async fn run(self, mut ctx: Ctx) -> Result<()> {
        let force = false; // matches!(self, Cmd::Refresh);
        let mut session = build(&ctx.config.session, force).await?;

        let result = match self {
            Cmd::Refresh => {
                session.refresh_all().await?;
                session.refresh_channels().await?;
                println!("session refreshed");
                Ok(())
            }
            Cmd::Status => {
                println!(
                    "{}",
                    StatusReport::from_session(&session).pretty(&ctx.label_lookup())?
                );
                Ok(())
            }
            Cmd::Upload => {
                let id = session.upload().await?;
                println!("{id}");
                Ok(())
            }
            Cmd::Teardown { hash } => {
                let hash = hash.unwrap_or(VALIDATOR.hash);
                let id = session.teardown(&hash).await?;
                println!("{id}");
                Ok(())
            }
            Cmd::AddDelegation { credential } => {
                let added = session.add_delegation(credential).await?;
                ctx.sync_session(&session)?;
                println!(
                    "{}",
                    if added {
                        "delegation added"
                    } else {
                        "delegation already present"
                    }
                );
                Ok(())
            }
            Cmd::RemoveDelegation { credential } => {
                let removed = session.remove_delegation(&credential)?;
                ctx.sync_session(&session)?;
                println!(
                    "{}",
                    if removed {
                        "delegation removed"
                    } else {
                        "delegation was not tracked"
                    }
                );
                Ok(())
            }
        };

        persist(
            session.cardano(),
            &ctx.config.session.tip_cache_path,
            &ctx.config.session.addressbook_path,
        )?;
        result
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub network: String,
    pub change_address: String,
    pub script_host: String,
    pub fuel: FuelStatus,
    pub ref_script: Option<Input>,
    pub delegations: Vec<String>,
    pub channels: Vec<(Input, Channel)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FuelStatus {
    pub utxo_count: usize,
    pub total_lovelace: u64,
}

impl StatusReport {
    pub fn from_session<C: CardanoConnector, W: Wallet>(session: &Session<C, W>) -> Self {
        let cardano = session.cardano();

        let fuel = cardano.fuel();
        let fuel_lovelace: u64 = fuel.values().map(|o| o.value().lovelace()).sum();
        let ref_script = session.ref_script().ok().map(|(i, _o)| i);

        let delegations = session
            .delegations()
            .iter()
            .map(|d| d.to_string())
            .collect();

        let channels = session
            .channels()
            .into_iter()
            .filter_map(|tup| Channel::try_from(&tup.1).ok().map(|c| (tup.0, c)))
            .collect();

        Self {
            network: cardano.network_id().to_string(),
            change_address: cardano.change_address().to_string(),
            script_host: session
                .script_host()
                .map(|a| a.to_string())
                .unwrap_or_else(|| "wallet (default)".into()),
            fuel: FuelStatus {
                utxo_count: fuel.len(),
                total_lovelace: fuel_lovelace,
            },
            ref_script,
            delegations,
            channels,
        }
    }

    pub fn pretty(&self, lookup: &BTreeMap<String, String>) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&json_label::map_labels(serde_json::to_value(self)?, lookup))
    }
}

/// Rebuilds a live `Session` from config. `force` skips the tip/addressbook
/// cache and does a full chain fetch instead of hydrating from disk.
pub async fn build(
    config: &subbit_session::session::Config,
    force: bool,
) -> Result<Session<Blockfrost, Embedded<Blockfrost>>> {
    let mut cardano = CardanoSession::init(config.cardano.clone())
        .await
        .context("initializing cardano session")?;
    if !force {
        hydrate(
            &mut cardano,
            &config.tip_cache_path,
            &config.addressbook_path,
        )
        .context("hydrating session cache")?;
    }
    Ok(Session::new(
        cardano,
        config.script_host.clone(),
        config.delegations.clone(),
    ))
}

fn hydrate<C: CardanoConnector, W: Wallet>(
    cardano: &mut CardanoSession<C, W>,
    tip_cache_path: &Path,
    addressbook_path: &Path,
) -> Result<()> {
    if let Some(tip) = cache::try_load::<TipVec>(tip_cache_path)? {
        cardano.load_tip(tip.into());
    }
    if let Some(addressbook) = cache::try_load(addressbook_path)? {
        cardano.load_addressbook(addressbook)?;
    }
    Ok(())
}

fn persist<C: CardanoConnector, W: Wallet>(
    cardano: &CardanoSession<C, W>,
    tip_cache_path: &Path,
    addressbook_path: &Path,
) -> Result<()> {
    let tip_vec: TipVec = cardano.tip().clone().into();
    cache::save(tip_cache_path, &tip_vec)?;
    cache::save(addressbook_path, cardano.addressbook())
}
