//! `Config::build` resolves config into a live `Stamp`, its catalog
//! already loaded (`Stamp::set_catalog`) -- shared by `Local` below
//! and by other native callers (e.g. `examples/echo/client`) that
//! wire it into a reqwest middleware instead.
//!
//! `Local` itself is now a thin wrapper: catalog lookup and miss
//! handling both live on `Stamp` (see `catalog`), so this module's
//! own job is just native cache-file I/O plus the vanilla wire
//! framing (`dialect::vanilla`).

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use subbit_l2_dto::Catalog as CatalogDto;

use subbit_l2_dto::PopMacTtl;

use crate::catalog::{CatalogError, Charge, MissPolicy};
use crate::dialect::vanilla;
use crate::{Cache, Stamp};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("failed to read/write cache file")]
    Io(#[from] std::io::Error),
    #[error("failed to encode cache")]
    EncodeCache(#[from] minicbor::encode::Error<std::convert::Infallible>),
    #[error("bad catalog")]
    Catalog(#[from] CatalogError),
    /// The configured `policy` rejected a path matching nothing in
    /// the catalog -- see `catalog::MissPolicy`/`Stamp::resolve_charge`.
    #[error("no cost known for {0}, and the miss policy rejected it")]
    Rejected(String),
    #[error(transparent)]
    Response(#[from] vanilla::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub cache: PathBuf,
    pub stamp: crate::Config,
    pub catalog: CatalogDto,
    pub auto_write: bool,
}

impl Default for Config {
    /// Placeholder identity, edit before real use.
    fn default() -> Self {
        let account = crate::id::Config {
            signing_key: [0; 32],
            tag: subbit_core::Tag::from(hex::decode("deadbeef").expect("valid hex")),
        };
        Self {
            cache: PathBuf::from("/tmp/subbit-state.cbor"),
            stamp: crate::Config::new(account, subbit_core::Duration::from_secs(3600)),
            catalog: CatalogDto {
                resources: vec![(
                    "/{*rest}".to_string(),
                    subbit_l2_dto::ResourceInfo {
                        cost: subbit_l2_dto::Cost::ByRequest(0),
                        description: String::new(),
                        mime_type: String::new(),
                        max_timeout_seconds: 60,
                        service_name: None,
                        tags: None,
                        icon_url: None,
                    },
                )],
            },
            auto_write: true,
        }
    }
}

impl Config {
    /// A single `Stamp`, already carrying this config's own catalog
    /// (see `Stamp::set_catalog`) -- the one place that used to hand
    /// back a separate `(Stamp, Catalog)` pair for a caller to wire
    /// up itself.
    pub fn build(&self) -> Result<Stamp, Error> {
        let cache = read_cache(&self.cache).unwrap_or_default();
        let mut stamp = Stamp::from_parts(self.stamp.clone(), cache);
        stamp.set_catalog(self.catalog.clone())?;
        Ok(stamp)
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
    pub auth: PopMacTtl,
}

/// No-network driver: builds/applies envelopes directly, auto-persisting
/// the stamp's cache after each, see `auto_write`. A thin wrapper over
/// `Stamp` itself now that catalog/miss handling both live there --
/// this type's only remaining job is native cache-file I/O and the
/// vanilla wire framing.
pub struct Local {
    cache_path: PathBuf,
    stamp: Stamp,
    pub auto_write: bool,
}

impl Local {
    pub fn build(config: &Config) -> Result<Self, Error> {
        let stamp = config.build()?;
        Ok(Self {
            stamp,
            cache_path: config.cache.clone(),
            auto_write: config.auto_write,
        })
    }

    /// Builds a request envelope for `path`, resolving its cost
    /// against the stamp's own catalog and falling back to `policy`
    /// on a miss (see `Stamp::resolve_charge`). `Charge::Unpriced`
    /// has no vanilla-wire equivalent to send unmetered through this
    /// driver (there's no transport here to send a bare request
    /// over), so it's treated the same as `Reject`.
    pub fn request(&mut self, path: &str, policy: &dyn MissPolicy) -> Result<String, Error> {
        let cost = match self.stamp.resolve_charge(path, policy)? {
            Charge::Priced(cost) => cost,
            Charge::Reject | Charge::Unpriced => return Err(Error::Rejected(path.to_string())),
        };
        let envelope = vanilla::request(&mut self.stamp, cost);
        self.maybe_write_cache()?;
        Ok(envelope)
    }

    pub fn respond(&mut self, envelope: &str) -> Result<(), Error> {
        let result = vanilla::response(&mut self.stamp, envelope);
        self.maybe_write_cache()?;
        Ok(result?)
    }

    pub fn respond_bytes(&mut self, bytes: &[u8]) -> Result<(), Error> {
        let result = vanilla::response_bytes(&mut self.stamp, bytes);
        self.maybe_write_cache()?;
        Ok(result?)
    }

    pub fn stamp(&self) -> &Stamp {
        &self.stamp
    }

    pub fn status(&self) -> Status {
        Status {
            spent: self.stamp.spent(),
            committed: self.stamp.committed(),
            auth: self.stamp.auth(),
        }
    }

    /// Persist the cache now, regardless of `auto_write`.
    pub fn write_cache(&self) -> Result<(), Error> {
        write_cache(&self.cache_path, self.stamp.cache())
    }

    fn maybe_write_cache(&self) -> Result<(), Error> {
        if self.auto_write {
            self.write_cache()?;
        }
        Ok(())
    }
}
