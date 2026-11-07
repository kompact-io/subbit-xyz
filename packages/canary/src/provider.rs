//! `Provider`: the three real sibling processes a provider actually is -- `gate`, `index`,
//! `claim` -- orchestrated together as one unit with one combined lifetime. `index`/`claim`
//! are spawned once `gate` is up and `claim`'s fee wallet is funded, so they start as `None`.

mod claim;
mod gate;
mod index;

use claim::ClaimProcess;
use gate::GateProcess;
use index::IndexProcess;

use anyhow::Context;

use crate::identity::Identity;

pub struct Provider {
    gate: GateProcess,
    _index: Option<IndexProcess>,
    _claim: Option<ClaimProcess>,
}

impl Provider {
    pub async fn spawn(config: subbit_gate::Config) -> anyhow::Result<Self> {
        Ok(Self {
            gate: GateProcess::spawn(config).await?,
            _index: None,
            _claim: None,
        })
    }

    pub fn base_url(&self) -> String {
        self.gate.base_url()
    }

    /// Spawns a real `subbit-gate` configured for this run's own demo `Terms`.
    pub async fn spawn_demo(
        flat_cost: u64,
        provider_identity: Identity,
        close_period: subbit_core::Duration,
        finality_depth: u64,
    ) -> anyhow::Result<Self> {
        let mut config = subbit_gate::Config::default();
        config.terms.provider = provider_identity.hash28;
        config.terms.asset = subbit_l2_dto::Asset::Lovelace;
        config.terms.close_period = close_period;
        config.terms.script =
            subbit_core::Hash28::from(<[u8; 28]>::from(subbit_tx::VALIDATOR.hash));
        config.terms.catalog = demo_catalog(flat_cost);
        config.finality_depth = finality_depth;
        // Latency investigation (results.md): the defaults' cumulative
        // thresholds are tiny relative to real ADA amounts, so they --
        // not the 1s/30s timers -- are what's actually triggering
        // flush/checkpoint under canary's load. Bumped to real ADA
        // scale to let the timers dominate instead and see if that
        // changes the measured L2 latency.
        config.db.buffer.max_pending = 10_000_000; // 10 ADA
        config.db.buffer.max_buffer_interval = std::time::Duration::from_secs(1);
        config.db.exposure.single_write_threshold = 1_000_000; // 1 ADA
        config.db.exposure.max_cumulative_exposure = 10_000_000; // 10 ADA
        config.db.exposure.max_checkpoint_interval = std::time::Duration::from_secs(1);
        println!("canary: spawning provider");
        let provider = Self::spawn(config).await?;
        println!("canary: provider up at {}", provider.base_url());
        Ok(provider)
    }

    /// Spawns the real `lineage-index`/`subbit-claim` binaries alongside this already-running
    /// gate, completing the three-process `Provider`. `claim_fee_seed`'s own wallet must already
    /// be funded on-chain before this is called.
    pub async fn spawn_support_processes(
        &mut self,
        floe_endpoint: &str,
        network: cardano_sdk::Network,
        claim_fee_seed: [u8; 32],
        ref_script_host: cardano_sdk::Address<cardano_sdk::address::kind::Shelley>,
        provider_signing_key: [u8; 32],
    ) -> anyhow::Result<()> {
        let base_url = self.base_url();

        println!("canary: spawning the real lineage-index binary");
        self._index = Some(
            IndexProcess::spawn(floe_endpoint, &base_url)
                .await
                .context("spawning lineage-index")?,
        );

        println!("canary: spawning the real subbit-claim binary");
        self._claim = Some(
            ClaimProcess::spawn(
                floe_endpoint,
                &base_url,
                network,
                claim_fee_seed,
                ref_script_host,
                provider_signing_key,
            )
            .await
            .context("spawning subbit-claim")?,
        );

        println!(
            "canary: real subbit-claim/lineage-index now running as child processes -- \
             claiming/indexing is fully asynchronous from here"
        );
        Ok(())
    }
}

/// This run's own demo cost catalog: a flat per-request cost for `/v1/spend/echo`.
fn demo_catalog(flat_cost: u64) -> subbit_l2_dto::Catalog {
    subbit_l2_dto::Catalog {
        resources: vec![(
            "/{*rest}".to_string(),
            subbit_l2_dto::ResourceInfo {
                cost: subbit_l2_dto::Cost::ByRequest(flat_cost),
                description: "Canary smoke-test resource".to_string(),
                mime_type: "text/plain".to_string(),
                max_timeout_seconds: 60,
                service_name: None,
                tags: None,
                icon_url: None,
            },
        )],
    }
}
