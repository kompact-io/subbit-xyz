//! Config for the echo client.
use serde::{Deserialize, Serialize};
use subbit_issuer::native_url_lookup;

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub base_url: String,
    /// Present => requests are metered through the issuer middleware.
    /// Absent => plain requests.
    pub subbit: Option<native_url_lookup::Config>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            base_url: "http://127.0.0.1:3246".into(),
            // Included (rather than omitted) so `init` shows the shape of
            // it. Delete the section, or set signing_key/tag, to go plain.
            subbit: Some(native_url_lookup::Config::default()),
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
