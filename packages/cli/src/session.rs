use anyhow::Context;
use cardano_connector_direct::Blockfrost;
use cardano_sdk::Input;
use serde::Serialize;
use subbit_session::{self, Session};
use subbit_tx::Channel;

/// Rebuilds a live `Session` from config on every call
pub async fn build(
    config: &subbit_session::session::Config,
) -> anyhow::Result<Session<Blockfrost>> {
    let mut session = config.clone().build().await.context("building session")?;
    session
        .init()
        .await
        .context("initializing session (fetching chain state)")?;
    Ok(session)
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
    pub fn from_session(session: &Session<Blockfrost>) -> Self {
        let cardano = session.cardano();

        let fuel = cardano.fuel();
        let fuel_lovelace: u64 = fuel.values().map(|o| o.value().lovelace()).sum();

        let ref_scripts = cardano
            .ref_scripts()
            .map(|(hash, _, _)| RefScriptEntry {
                hash: format!("{hash:?}"),
            })
            .collect();

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

        StatusReport {
            network: format!("{:?}", cardano.network_id()),
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
}

pub fn print_status(session: &Session<Blockfrost>) {
    let status = StatusReport::from_session(session);
    match serde_json::to_string_pretty(&status) {
        Ok(json) => println!("{json}"),
        Err(e) => eprintln!("failed to serialize status report: {e}"),
    }
}
