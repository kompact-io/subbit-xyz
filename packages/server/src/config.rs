use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

use crate::{costings, crypto::mac, db, limiter};

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, Default)]
pub struct Config {
    /// HMAC
    #[n(0)]
    pub mac_key: mac::Key,

    /// Limiter config
    #[n(1)]
    pub limiter: limiter::Config,

    /// Costings config
    #[n(2)]
    pub costings: costings::Config,

    #[n(3)]
    pub db: db::Config,
    // TODO :: Add bucket
}
