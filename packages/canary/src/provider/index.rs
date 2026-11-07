//! Spawns the real `lineage-index` binary.

use tokio::process::Child;

use crate::process::spawn_with_config;

#[derive(serde::Serialize)]
struct IndexConfig {
    cardano: subbit_index::cardano::Config,
    endpoint: String,
    poll_interval_secs: u64,
}

pub(crate) struct IndexProcess {
    _child: Child,
    _dir: tempfile::TempDir,
}

impl IndexProcess {
    pub(crate) async fn spawn(
        floe_endpoint: &str,
        provider_base_url: &str,
    ) -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        let config = IndexConfig {
            cardano: subbit_index::cardano::Config {
                endpoint: floe_endpoint.to_string(),
            },
            endpoint: format!("{provider_base_url}/v1/a/index"),
            // Short poll so the scenario doesn't sit idle waiting on it.
            poll_interval_secs: 5,
        };
        let child = spawn_with_config(&dir, "lineage-index", &config, &["run"])?;
        Ok(Self {
            _child: child,
            _dir: dir,
        })
    }
}
