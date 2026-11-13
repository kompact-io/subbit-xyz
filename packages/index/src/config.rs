use serde::{Deserialize, Serialize};
use url::Url;

use crate::cardano;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Floe endpoint
    pub cardano: cardano::Config,
    /// Gate admin index endpoint
    pub gate: Url,
    pub poll_interval_secs: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            cardano: Default::default(),
            gate: Url::parse("http://127.0.0.1:7822/v1/a/index").expect("static URL always parses"),
            poll_interval_secs: 30,
        }
    }
}
