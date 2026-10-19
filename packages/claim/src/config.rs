use std::path::Path;

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use url::Url;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub session: subbit_session::session::Config,
    pub keyring: subbit_session::keyring::Config,
    /// `subbit-server`'s `GET /v1/a/channels` endpoint.
    pub server_endpoint: Url,
    pub poll_interval_secs: u64,
    /// See `decide::DecisionConfig`.
    pub min_total: u64,
    pub min_single: u64,
    pub panic_settle_ms: u64,
    /// How many `Want`s to batch per submitted transaction.
    pub step_chunk: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            session: Default::default(),
            keyring: Default::default(),
            server_endpoint: Url::parse("http://127.0.0.1:7822/v1/a/channels")
                .expect("static URL always parses"),
            poll_interval_secs: 30,
            min_total: 1_000_000,
            min_single: 100_000,
            panic_settle_ms: 5 * 60 * 1000,
            step_chunk: 20,
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
}
