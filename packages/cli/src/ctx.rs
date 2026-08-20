use std::path::PathBuf;

use anyhow::Result;

use crate::config::Config;

/// Shared context every command runs against: the loaded config, plus the
/// path it came from so a command that mutates it (`keyring add`,
/// `session add-delegation`, ...) can write it back.
pub struct Ctx {
    pub config_path: PathBuf,
    pub config: Config,
}

impl Ctx {
    pub fn load(config_path: PathBuf) -> Result<Self> {
        let config = Config::load(&config_path)?;
        Ok(Self {
            config_path,
            config,
        })
    }

    pub fn save(&self) -> Result<()> {
        self.config.save(&self.config_path)
    }
}
