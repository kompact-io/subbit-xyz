//! `Config::build` resolves config into a live `Issuer`/cost pair — shared
//! by `NativeUrlLookup` below and by other consumers (e.g. the reqwest
//! middleware) that wire the pieces up differently.
//!
//! This is a mix of components and opinions. You're opinions may vary!

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use subbit_core::Auth;
use subbit_core::envelope::Body;

use crate::cost::url_lookup::{self, UrlLookup};
use crate::{Cache, Issuer};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read/write cache file")]
    Io(#[from] std::io::Error),
    #[error("failed to encode cache")]
    EncodeCache(#[from] minicbor::encode::Error<std::convert::Infallible>),
    #[error("bad cost table")]
    Cost(#[from] url_lookup::Error),
    #[error(transparent)]
    Response(#[from] crate::issuer::ResponseError),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub cache: PathBuf,
    pub issuer: crate::Config,
    pub cost: url_lookup::Config,
    pub auto_write: bool,
}

impl Default for Config {
    /// Placeholder identity — edit before real use.
    fn default() -> Self {
        let account = crate::account::Config {
            signing_key: [0; 32],
            tag: subbit_core::Tag::from(hex::decode("deadbeef").expect("valid hex")),
        };
        Self {
            cache: PathBuf::from("/tmp/subbit-state.cbor"),
            issuer: crate::Config::new(account, subbit_core::Duration::from_secs(3600)),
            cost: Default::default(),
            auto_write: true,
        }
    }
}

impl Config {
    pub fn build(&self) -> Result<(Issuer, UrlLookup), Error> {
        let cache = read_cache(&self.cache).unwrap_or_default();
        let issuer = Issuer::from_parts(self.issuer.clone(), cache);
        let cost = UrlLookup::new(self.cost.clone())?;
        Ok((issuer, cost))
    }
}

/// Missing or unreadable cache is treated as "start fresh".
pub fn read_cache(path: &Path) -> Option<Cache> {
    std::fs::read(path)
        .ok()
        .and_then(|b| minicbor::decode(&b).ok())
}

pub fn write_cache(path: &Path, cache: &Cache) -> Result<(), Error> {
    std::fs::write(path, minicbor::to_vec(cache)?)?;
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct Status {
    pub spent: u64,
    pub committed: Option<u64>,
    pub auth: Auth<Body, Body>,
}

/// No-network driver: builds/applies envelopes directly, auto-persisting
/// the issuer's cache after each — see `auto_write`.
pub struct NativeUrlLookup {
    cache_path: PathBuf,
    issuer: Issuer,
    cost: UrlLookup,
    pub auto_write: bool,
}

impl NativeUrlLookup {
    pub fn build(config: &Config) -> Result<Self, Error> {
        let (issuer, cost) = config.build()?;
        Ok(Self {
            issuer,
            cost,
            cache_path: config.cache.clone(),
            auto_write: config.auto_write,
        })
    }

    pub fn request(&mut self, path: &str) -> Result<String, Error> {
        let envelope = self.issuer.request(self.cost.lookup(path));
        self.maybe_write_cache()?;
        Ok(envelope)
    }

    pub fn respond(&mut self, envelope: &str) -> Result<(), Error> {
        self.issuer.response(envelope)?;
        self.maybe_write_cache()?;
        Ok(())
    }

    pub fn issuer(&self) -> &Issuer {
        &self.issuer
    }

    pub fn status(&self) -> Status {
        Status {
            spent: self.issuer.spent(),
            committed: self.issuer.committed(),
            auth: self.issuer.auth(),
        }
    }

    /// Persist the cache now, regardless of `auto_write`.
    pub fn write_cache(&self) -> Result<(), Error> {
        write_cache(&self.cache_path, self.issuer.cache())
    }

    fn maybe_write_cache(&self) -> Result<(), Error> {
        if self.auto_write {
            self.write_cache()?;
        }
        Ok(())
    }
}
