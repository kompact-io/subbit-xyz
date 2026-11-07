//! Spawns the real `subbit-claim` binary.

use cardano_sdk::address::kind;
use tokio::process::Child;

use crate::process::spawn_with_config;

pub(crate) struct ClaimProcess {
    _child: Child,
    _dir: tempfile::TempDir,
}

impl ClaimProcess {
    pub(crate) async fn spawn(
        floe_endpoint: &str,
        provider_base_url: &str,
        network: cardano_sdk::Network,
        fee_key: [u8; 32],
        ref_script_host: cardano_sdk::Address<kind::Shelley>,
        provider_signing_key: [u8; 32],
    ) -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        let mut keys = std::collections::BTreeMap::new();
        keys.insert(
            "provider".to_string(),
            subbit_claim::keyring::KeyHex::from(provider_signing_key),
        );
        let config = subbit_claim::config::Config {
            floe: subbit_claim::floe::Config {
                endpoint: floe_endpoint.to_string(),
            },
            network,
            fee_key: subbit_claim::keyring::KeyHex::from(fee_key),
            ref_script_host: Some(ref_script_host),
            keyring: subbit_claim::keyring::Config { keys },
            server_endpoint: format!("{provider_base_url}/v1/a/channels").parse()?,
            // Short poll + generous waits -- the scenario, not a
            // deployment economizing on request volume.
            poll_interval_secs: 5,
            min_total: 1,
            min_single: 1,
            panic_settle_ms: 3_600_000,
            step_chunk: 40,
            wait_attempts: 30,
            wait_interval_secs: 10,
        };
        let child = spawn_with_config(&dir, "subbit-claim", &config, &["run"])?;
        Ok(Self {
            _child: child,
            _dir: dir,
        })
    }
}
