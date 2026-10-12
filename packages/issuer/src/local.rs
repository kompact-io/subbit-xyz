//! `Config::build` resolves config into a live `Issuer`/`CatalogIndex`
//! pair, shared by `Local` below and by other consumers (e.g. the
//! reqwest middleware) that wire the pieces up differently.
//!
//! `Local` drives the vanilla dialect directly, via `dialect::vanilla`,
//! quoting from a locally configured `Catalog` rather than a server.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use subbit_l2_dto::{Catalog, CatalogError, CatalogIndex};

use crate::dialect::vanilla;
use crate::{Auth, Cache, Issuer};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read/write cache file")]
    Io(#[from] std::io::Error),
    #[error("failed to encode cache")]
    EncodeCache(#[from] minicbor::encode::Error<std::convert::Infallible>),
    #[error("bad catalog")]
    Catalog(#[from] CatalogError),
    #[error(transparent)]
    Response(#[from] vanilla::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub cache: PathBuf,
    pub issuer: crate::Config,
    pub catalog: Catalog,
    pub auto_write: bool,
}

impl Default for Config {
    /// Placeholder identity, edit before real use.
    fn default() -> Self {
        let account = crate::account::Config {
            signing_key: [0; 32],
            tag: subbit_core::Tag::from(hex::decode("deadbeef").expect("valid hex")),
        };
        Self {
            cache: PathBuf::from("/tmp/subbit-state.cbor"),
            issuer: crate::Config::new(account, subbit_core::Duration::from_secs(3600)),
            catalog: Catalog {
                default: subbit_l2_dto::ResourceInfo {
                    cost: subbit_l2_dto::Cost::ByRequest(0),
                    description: String::new(),
                    mime_type: String::new(),
                    max_timeout_seconds: 60,
                    service_name: None,
                    tags: None,
                    icon_url: None,
                },
                resources: Vec::new(),
            },
            auto_write: true,
        }
    }
}

impl Config {
    pub fn build(&self) -> Result<(Issuer, CatalogIndex), Error> {
        let cache = read_cache(&self.cache).unwrap_or_default();
        let issuer = Issuer::from_parts(self.issuer.clone(), cache);
        let catalog = CatalogIndex::new(self.catalog.clone())?;
        Ok((issuer, catalog))
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
    pub auth: Auth,
}

/// No-network driver: builds/applies envelopes directly, auto-persisting
/// the issuer's cache after each, see `auto_write`.
pub struct Local {
    cache_path: PathBuf,
    issuer: Issuer,
    catalog: CatalogIndex,
    pub auto_write: bool,
}

impl Local {
    pub fn build(config: &Config) -> Result<Self, Error> {
        let (issuer, catalog) = config.build()?;
        Ok(Self {
            issuer,
            catalog,
            cache_path: config.cache.clone(),
            auto_write: config.auto_write,
        })
    }

    pub fn request(&mut self, path: &str) -> Result<String, Error> {
        let envelope = vanilla::request(&mut self.issuer, self.catalog.cost(path));
        self.maybe_write_cache()?;
        Ok(envelope)
    }

    pub fn respond(&mut self, envelope: &str) -> Result<(), Error> {
        let result = vanilla::response(&mut self.issuer, envelope);
        self.maybe_write_cache()?;
        Ok(result?)
    }

    pub fn respond_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let result = vanilla::response_bytes(&mut self.issuer, bytes);
        self.maybe_write_cache()?;
        Ok(result?)
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
