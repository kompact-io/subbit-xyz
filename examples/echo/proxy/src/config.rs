use std::net::SocketAddr;

use serde::{Deserialize, Serialize};

#[cfg(feature = "subbit")]
use crate::subbit;

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub listen: SocketAddr,
    pub upstream: SocketAddr,
    #[cfg(feature = "subbit")]
    pub subbit: Option<crate::subbit::Config>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen: "0.0.0.0:6432".parse().unwrap(),
            upstream: "0.0.0.0:3246".parse().unwrap(),
            #[cfg(feature = "subbit")]
            subbit: Some(subbit::Config::default()),
        }
    }
}

impl Config {
    pub fn write_default(path: &std::path::Path) -> anyhow::Result<()> {
        let toml = toml::to_string_pretty(&Self::default())?;
        std::fs::write(path, toml)?;
        Ok(())
    }
}
