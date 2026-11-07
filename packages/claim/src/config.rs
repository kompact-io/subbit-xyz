use std::path::Path;

use anyhow::{Context, Result, bail};
use cardano_sdk::{Network, NetworkId, ProtocolParameters, address::kind};
use serde::{Deserialize, Serialize};
use url::Url;

use crate::keyring::KeyHex;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Floe gRPC backend -- this service's only chain backend (both for
    /// deciding what to claim and for building/submitting/confirming the
    /// actual transaction).
    pub floe: crate::floe::Config,
    /// Which network's protocol parameters to use -- see
    /// `Self::network_parameters`. Floe doesn't serve protocol
    /// parameters (nor a chain tip authoritative enough to derive them
    /// from), so this is a hardcoded per-network snapshot; if a hard
    /// fork changes them, this needs a manual bump (and a redeploy).
    pub network: Network,
    /// The fee-paying wallet's own signing key: pays tx fees, supplies
    /// collateral, receives change, and provides the tx-level witness.
    /// Distinct from `keyring`, which only signs `specified_signatories`
    /// (the provider identity authorizing Sub/Settle) -- in principle
    /// the same key could serve both roles, but the two concerns are
    /// kept separate here the same way they always have been.
    pub fee_key: KeyHex,
    /// Where the subbit validator's reference script lives, if not at
    /// the fee wallet's own address (the default assumption).
    pub ref_script_host: Option<cardano_sdk::Address<kind::Shelley>>,
    pub keyring: crate::keyring::Config,
    /// `subbit-gate`'s `GET /v1/a/channels` endpoint.
    pub server_endpoint: Url,
    pub poll_interval_secs: u64,
    /// See `decide::DecisionConfig`.
    pub min_total: u64,
    pub min_single: u64,
    pub panic_settle_ms: u64,
    /// How many `Want`s to batch per submitted transaction. A flat
    /// conservative cap, not a measured ceiling -- matches
    /// `Basket::MAX_OPENS_PER_TX`'s own reasoning. TODO: make this
    /// dynamic (shrink/retry on `MaxTxSizeUTxO`, the way `console::
    /// submit`'s own Opens batching already does) instead of a fixed
    /// number.
    pub step_chunk: usize,
    /// How many times to poll Floe's `ReadTx` for a submitted tx before
    /// giving up, and how long to wait between attempts.
    pub wait_attempts: u32,
    pub wait_interval_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            floe: Default::default(),
            network: Network::Preprod,
            fee_key: [0; 32].into(),
            ref_script_host: None,
            keyring: Default::default(),
            server_endpoint: Url::parse("http://127.0.0.1:7822/v1/a/channels")
                .expect("static URL always parses"),
            poll_interval_secs: 30,
            min_total: 1_000_000,
            min_single: 100_000,
            panic_settle_ms: 5 * 60 * 1000,
            step_chunk: 40,
            wait_attempts: 30,
            wait_interval_secs: 10,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            bail!(
                "no config at {}; run `init --config {}` to scaffold one",
                path.display(),
                path.display()
            );
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn write_default(path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(&Self::default())?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }

    /// Protocol parameters + network id, straight from `self.network`'s
    /// hardcoded snapshot -- see `Self::network`'s own doc.
    pub fn network_parameters(&self) -> subbit_tx::NetworkParameters {
        subbit_tx::NetworkParameters {
            network_id: NetworkId::from(self.network),
            protocol_parameters: ProtocolParameters::from(self.network),
        }
    }
}
