use std::path::{Path, PathBuf};

use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize)]
pub struct Config {
    pub session: subbit_session::session::Config,

    pub keyring: subbit_session::keyring::Config,

    /// Where the staged (in-progress) `Tx` gets cached between CLI
    /// invocations. Ephemeral/derived -- safe to delete, `tx new`
    /// recreates it.
    pub cache_path: PathBuf,
}

impl Default for Config {
    fn default() -> Self {
        let cache_path = PathBuf::from("/tmp/subbit-cli/tx.json");
        Self {
            session: Default::default(),
            keyring: Default::default(),
            cache_path,
        }
    }
}

impl Config {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            bail!(
                "no config at {}; run `subbit init --config {}` to scaffold one",
                path.display(),
                path.display()
            );
        }
        let text =
            std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))
    }

    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }
}
