//! An opinionated config loader supporting secrets.
//!
//! Layered config loading: TOML base, optional TOML overlay, optional env
//! overrides. Precedence (later wins): `base` < `overlay` < env.
//! Content-unaware — no `_FILE` convention, no key inspection; overlay is
//! just "another TOML file merged after base" (e.g. a secrets file kept
//! outside version control).

use std::path::Path;

use figment::Figment;
use figment::providers::{Env, Format, Toml};
use serde::de::DeserializeOwned;

#[derive(Debug, thiserror::Error)]
#[error(transparent)]
pub struct Error(#[from] figment::Error);

#[derive(Debug, Clone, PartialEq)]
pub struct Sources<'a> {
    /// Lowest precedence. Typically checked into version control.
    pub base: &'a Path,
    /// Merged after `base`; overrides matching keys. Typically a secrets
    /// file, kept out of version control.
    pub overlay: Option<&'a Path>,
    /// Merged last; overrides both files. Nested keys use `__` as the
    /// separator, e.g. `PREFIX_LIMITER__MAX_REQUESTS` -> `limiter.max_requests`.
    pub env_prefix: Option<&'a str>,
}

impl<'a> Sources<'a> {
    pub fn new(base: &'a Path) -> Self {
        Self {
            base,
            overlay: None,
            env_prefix: None,
        }
    }
    /// Assemble (without extracting) the layered `Figment` for `sources`.
    /// Exposed separately from `load` so callers can keep merging additional
    /// providers before extracting.
    pub fn figment(&self) -> Figment {
        let mut f = Figment::new();
        f = f.merge(Toml::file(self.base));
        if let Some(overlay) = self.overlay {
            f = f.merge(Toml::file(overlay));
        }
        if let Some(prefix) = self.env_prefix {
            f = f.merge(Env::prefixed(prefix).split("__"));
        }
        f
    }

    /// Load and extract `T` from `sources`. See [`Sources`] for precedence.
    pub fn load<T: DeserializeOwned>(&self) -> Result<T, Error> {
        Ok(self.figment().extract()?)
    }
}

#[cfg(feature = "clap")]
mod with_clap {
    use std::path::PathBuf;

    use super::Sources;

    /// No baked-in defaults. Consumer decides what "no flag given" resolves to via `into_sources`.
    #[derive(Debug, clap::Args)]
    pub struct ConfigArgs {
        #[arg(long)]
        pub config: Option<PathBuf>,
        #[arg(long)]
        pub secrets: Option<PathBuf>,
        #[arg(long)]
        pub env_prefix: Option<String>,
    }

    impl ConfigArgs {
        /// `default_base` is used only if `--config` wasn't passed.
        pub fn into_sources<'a>(&'a self, default_base: &'a std::path::Path) -> Sources<'a> {
            Sources {
                base: self.config.as_deref().unwrap_or(default_base),
                overlay: self.secrets.as_deref(),
                env_prefix: self.env_prefix.as_deref(),
            }
        }
    }
}

#[cfg(feature = "clap")]
pub use with_clap::ConfigArgs as Args;
