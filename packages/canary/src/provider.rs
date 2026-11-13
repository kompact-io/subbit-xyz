//! `Provider`: the three sibling processes a provider actually is -- `gate`, `index`,
//! `claim` -- orchestrated together as one unit with one combined lifetime.

mod claim;
mod gate;
mod index;

use anyhow::Context;
use cardano_sdk::{Address, address::kind};
use subbit_core::Duration;

use crate::identity::Identity;

/// Everything needed to build the provider's three child-process configs.
pub struct Config {
    pub(crate) provider_identity: Identity,
    pub(crate) claim_wallet: Identity,
    pub(crate) ref_script_host: Address<kind::Shelley>,
    pub(crate) unit_cost: u64,
    pub(crate) close_period: Duration,
    pub(crate) finality_depth: u64,
    pub(crate) floe_endpoint: String,
}

impl Config {
    /// The `subbit-gate` config for this run's demo `Terms`.
    pub fn gate(&self) -> subbit_gate::Config {
        let mut config = subbit_gate::Config::default();
        config.terms.provider = self.provider_identity.hash28;
        config.terms.asset = subbit_l2_dto::Asset::Lovelace;
        config.terms.close_period = self.close_period;
        config.terms.script =
            subbit_core::Hash28::from(<[u8; 28]>::from(subbit_tx::VALIDATOR.hash));
        config.terms.catalog = crate::catalog::dto(self.unit_cost);
        config.finality_depth = self.finality_depth;
        // Latency investigation (results-ii.md): the defaults' cumulative
        // thresholds are tiny relative to ADA amounts, so they --
        // not the 1s/30s timers -- are what's actually triggering
        // flush/checkpoint under canary's load. Bumped to ADA
        // scale to let the timers dominate instead and see if that
        // changes the measured L2 latency.
        config.db.buffer.max_pending = 10_000_000; // 10 ADA
        config.db.buffer.max_buffer_interval = std::time::Duration::from_secs(1);
        config.db.exposure.single_write_threshold = 1_000_000; // 1 ADA
        config.db.exposure.max_cumulative_exposure = 10_000_000; // 10 ADA
        config.db.exposure.max_checkpoint_interval = std::time::Duration::from_secs(1);
        config
    }

    /// The `lineage-index` config for a running gate at `base_url`.
    pub fn index(&self, base_url: &str) -> anyhow::Result<subbit_index::Config> {
        Ok(subbit_index::Config {
            cardano: subbit_index::cardano::Config {
                endpoint: self.floe_endpoint.clone(),
            },
            gate: format!("{base_url}/v1/a/index").parse()?,
            // Short poll so the scenario doesn't sit idle waiting on it.
            poll_interval_secs: 5,
        })
    }

    /// The `subbit-claim` config for a running gate at `base_url`.
    pub fn claim(&self, base_url: &str) -> anyhow::Result<subbit_claim::config::Config> {
        let mut keys = std::collections::BTreeMap::new();
        keys.insert(
            "provider".to_string(),
            subbit_claim::keyring::KeyHex::from(self.provider_identity.seed()),
        );
        Ok(subbit_claim::config::Config {
            floe: subbit_claim::floe::Config {
                endpoint: self.floe_endpoint.clone(),
            },
            network: cardano_sdk::Network::Preprod,
            wallet_key: subbit_claim::keyring::KeyHex::from(self.claim_wallet.seed()),
            ref_script_host: Some(self.ref_script_host.clone()),
            keyring: subbit_claim::keyring::Config { keys },
            server_endpoint: format!("{base_url}/v1/a/channels").parse()?,
            // Short poll + generous waits -- the scenario, not a
            // deployment economizing on request volume.
            poll_interval_secs: 5,
            min_total: 5_000_000,
            min_single: 500_000,
            panic_settle_ms: 3_600_000,
            build_config: Default::default(),
            wait_interval_secs: 10,
        })
    }
}

pub struct Provider {
    gate: gate::Process,
    _index: index::Process,
    _claim: claim::Process,
}

impl Provider {
    /// Spawns `subbit-gate`, then `lineage-index`/`subbit-claim` against it.
    /// `config.claim_wallet` must already be funded on-chain.
    pub async fn spawn(config: Config) -> anyhow::Result<Self> {
        println!("canary: spawning the subbit-gate binary");
        let gate = gate::Process::spawn(config.gate())
            .await
            .context("spawning subbit-gate")?;
        let base_url = gate.base_url();

        println!("canary: spawning the lineage-index binary");
        let index = index::Process::spawn(config.index(&base_url)?)
            .await
            .context("spawning lineage-index")?;

        println!("canary: spawning the subbit-claim binary");
        let claim = claim::Process::spawn(config.claim(&base_url)?)
            .await
            .context("spawning subbit-claim")?;

        Ok(Self {
            gate,
            _index: index,
            _claim: claim,
        })
    }

    pub fn base_url(&self) -> String {
        self.gate.base_url()
    }
}
