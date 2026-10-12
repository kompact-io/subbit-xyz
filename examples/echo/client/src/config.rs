//! Config for the echo client.
use serde::{Deserialize, Serialize};
use subbit_issuer::local;

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Dialect {
    #[default]
    Vanilla,
    X402,
}

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub base_url: String,
    /// Which wire dialect to meter requests under. Only read when
    /// `subbit` is present.
    pub dialect: Dialect,
    /// Present => requests are metered through the issuer middleware.
    /// Absent => plain requests.
    pub subbit: Option<local::Config>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:3246".into(),
            dialect: Dialect::default(),
            // Included (rather than omitted) so `init` shows the shape of
            // it. Delete the section, or set signing_key/tag, to go plain.
            subbit: Some(local::Config::default()),
        }
    }
}

impl Config {
    /// Write a starter config to `path`.
    pub fn write_default(path: &std::path::Path) -> anyhow::Result<()> {
        let toml = toml::to_string_pretty(&Self::default())?;
        std::fs::write(path, toml)?;
        Ok(())
    }
}
