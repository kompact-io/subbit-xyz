use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};
use subbit_l2_dto::{Asset, Catalog, Cost, Network, ResourceInfo, Terms};

use crate::{crypto::mac, db, limiter};

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct Config {
    /// HMAC
    #[n(0)]
    pub mac_key: mac::Key,

    /// Limiter config
    #[n(1)]
    pub limiter: limiter::Config,

    #[n(2)]
    pub db: db::Config,

    /// Which dialects' routes to mount.
    #[n(3)]
    pub dialects: Dialects,

    /// What this provider requires before serving a request: which
    /// validator script it accepts, the channel-open terms, and its
    /// `Catalog` (cost, and per-resource info for dialects that need
    /// it, x402's `description`/`mimeType`/`maxTimeoutSeconds`/etc.).
    /// Dialect-agnostic (`subbit_l2_dto::Terms`'s own doc comment):
    /// vanilla would serve this from `.well-known/subbit` (not yet
    /// routed); x402 embeds it in every 402.
    #[n(4)]
    pub terms: Terms,

    /// This provider's own public URL (scheme and host, no trailing
    /// slash), if it has one. x402's `resource` field wants a fully
    /// qualified URL; without this, it falls back to the bare local
    /// path instead.
    #[n(5)]
    pub public_base_url: Option<String>,
    // TODO :: Add bucket
}

/// Which wire dialects this server answers. Both on by default; either
/// or both may be turned off. With both off the server still runs,
/// serving only the admin routes.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, Encode, Decode, PartialEq, Eq)]
pub struct Dialects {
    #[n(0)]
    pub vanilla: bool,
    #[n(1)]
    pub x402: bool,
}

impl Default for Dialects {
    fn default() -> Self {
        Self {
            vanilla: true,
            x402: true,
        }
    }
}

impl Default for Config {
    fn default() -> Self {
        Self {
            mac_key: Default::default(),
            limiter: Default::default(),
            db: Default::default(),
            dialects: Default::default(),
            terms: Terms {
                network: Network::Preprod,
                script: subbit_core::Hash28::from([0u8; 28]),
                asset: Asset::Lovelace,
                provider: subbit_core::Hash28::from([0u8; 28]),
                close_period: subbit_core::Duration::from_secs(900),
                confirmation_depth: 3,
                min_iou_amount: 0,
                // A starter value only, written out to `config.toml`
                // by `init` for a provider to edit: `Catalog` has no
                // hardcoded fallback of its own, an incomplete config
                // fails to load rather than silently using this.
                catalog: Catalog {
                    default: ResourceInfo {
                        cost: Cost::ByRequest(0),
                        description: "Access to a metered resource".to_string(),
                        mime_type: "application/octet-stream".to_string(),
                        max_timeout_seconds: 60,
                        service_name: None,
                        tags: None,
                        icon_url: None,
                    },
                    resources: Vec::new(),
                },
                reference_script: None,
            },
            public_base_url: None,
        }
    }
}
