use anyhow::{Context, Result};
use cardano_connector::CardanoConnector;
use cardano_connector_direct::Blockfrost;
use cardano_sdk::{Credential, Hash, Input};
use cardano_wallet::{Embedded, Wallet};
use clap::Subcommand;
use serde::Serialize;
use std::collections::BTreeMap;

use subbit_session::Session;
use subbit_tx::Channel;

use crate::{ctx::Ctx, json_label};

/// TODO:: upstream this ?? Rebuilds a live `Session` from config on every call.
pub async fn build(
    config: &subbit_session::session::Config,
) -> anyhow::Result<Session<Blockfrost, Embedded<Blockfrost>>> {
    let mut session = config.clone().build().await.context("building session")?;
    session
        .init()
        .await
        .context("initializing session (fetching chain state)")?;
    Ok(session)
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Rebuild the session against current chain state and print a summary.
    Status,
    /// Upload the subbit validator's reference script to the wallet.
    Upload,
    /// Reclaim a tracked reference script back into the wallet.
    Teardown { hash: Hash<28> },
    /// Start tracking channels at an additional delegation.
    AddDelegation { credential: Credential },
    /// Stop tracking channels at a delegation.
    RemoveDelegation { credential: Credential },
}

impl Cmd {
    pub async fn run(self, mut ctx: Ctx) -> Result<()> {
        let mut session = build(&ctx.config.session).await?;
        match self {
            Cmd::Status => {
                println!(
                    "{}",
                    StatusReport::from_session(&session).pretty(&ctx.label_lookup())?
                );
                Ok(())
            }
            Cmd::Upload => {
                session.upload().await?;
                println!("validator ref script uploaded and confirmed");
                Ok(())
            }
            Cmd::Teardown { hash } => {
                let id = session.teardown(&hash).await?;
                println!("teardown submitted: {id}");
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
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct StatusReport {
    pub network: String,
    pub change_address: String,
    pub script_host: String,
    pub fuel: FuelStatus,
    pub ref_scripts: Vec<RefScriptEntry>,
    pub delegations: Vec<String>,
    pub channels: Vec<(Input, Channel)>,
}

#[derive(Debug, Clone, Serialize)]
pub struct FuelStatus {
    pub utxo_count: usize,
    pub total_lovelace: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct RefScriptEntry {
    pub hash: String,
}

impl StatusReport {
    pub fn from_session<C: CardanoConnector, W: Wallet>(session: &Session<C, W>) -> Self {
        let cardano = session.cardano();

        let fuel = cardano.fuel();
        let fuel_lovelace: u64 = fuel.values().map(|o| o.value().lovelace()).sum();

        let ref_scripts = Vec::new();
        // cardano .ref_scripts() .map(|(hash, _, _)| RefScriptEntry { hash: hash.to_string(), }) .collect();

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
            ref_scripts,
            delegations,
            channels,
        }
    }

    pub fn pretty(&self, lookup: &BTreeMap<String, String>) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(&json_label::map_labels(serde_json::to_value(self)?, lookup))
    }
}
