use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::limiter::action;

/// Cost per action against a shared pool.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Encode, Decode)]
pub struct Config {
    #[n(0)]
    pub capacity: u64,
    #[n(1)]
    pub actions: action::Config,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            capacity: 30_000,
            actions: Default::default(),
        }
    }
}
