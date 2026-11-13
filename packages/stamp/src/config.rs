use serde::{Deserialize, Serialize};
use subbit_core::Duration;

use crate::id;

/// On-disk/wire shape for an stamp's identity: the permissive
/// `id::Config` plus the TTL used for proof-of-possession envelopes.
/// `Stamp` doesn't hold this directly, see `Stamp::from_parts`, which
/// resolves `id` into a real `id` once and stores id/ttl
/// flattened from then on. This type exists purely for (de)serialization.
#[derive(Debug, Clone, Serialize, Deserialize, minicbor::Encode, minicbor::Decode)]
pub struct Config {
    #[n(0)]
    pub id: id::Config,
    #[n(1)]
    pub ttl_relative: Duration,
}

impl Config {
    pub fn new(id: id::Config, ttl_relative: Duration) -> Self {
        Self { id, ttl_relative }
    }
}
