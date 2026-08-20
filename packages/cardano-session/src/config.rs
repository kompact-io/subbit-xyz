use serde::{Deserialize, Serialize};

use crate::{connector, waiter, wallet};

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub cardano: connector::Config,
    #[serde(default)]
    pub wallet: wallet::Config,
    #[serde(default)]
    pub wait: waiter::Config,
}
