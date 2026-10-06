use std::{collections::BTreeMap, path::PathBuf};

use anyhow::{Result, anyhow};
use cardano_connector::CardanoConnector;
use cardano_wallet::Wallet;
use subbit_session::Session;
use subbit_tx::tx::Tx;

use crate::{cache, config::Config};

/// Shared context every command runs against: the loaded config, plus the
/// path it came from so a command that mutates it (`keyring add`,
/// `session add-delegation`, ...) can write it back.
pub struct Ctx {
    pub config_path: PathBuf,
    pub config: Config,
}

impl Ctx {
    pub fn label_lookup(&self) -> BTreeMap<String, String> {
        self.config
            .labels
            .iter()
            .cloned()
            .chain(self.config.keyring.labels())
            .map(|(k, v)| (v, k))
            .collect()
    }

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

    pub fn load_tx(&self) -> Result<Tx> {
        cache::try_load(&self.config.cache_path)?.ok_or_else(|| {
            anyhow!(
                "no staged tx at {}; run `tx new` first",
                self.config.cache_path.display()
            )
        })
    }

    pub fn save_tx(&self, tx: &Tx) -> Result<()> {
        cache::save(&self.config.cache_path, tx)
    }

    pub fn sync_session<C: CardanoConnector, W: Wallet>(
        &mut self,
        session: &Session<C, W>,
    ) -> Result<()> {
        self.config.session.delegations = session.delegations().clone();
        self.config.session.script_host = session.script_host().cloned();
        self.save()
    }
}
