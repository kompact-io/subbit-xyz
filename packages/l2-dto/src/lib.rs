//! Subbit's own L2 wire types: shared shapes both dialects use, plus
//! the dialects themselves, each behind its own feature flag.
//!
//! `ChannelStatus` wraps `subbit_core::Iou`, not this crate's own
//! `Request`/`Commit::Iou`. `Response`'s `status` is the part x402
//! has no field of its own for, so it rides in `extensions` there
//! (`dialect::x402::settlement`). `Catalog`'s compiled resolver
//! (`subbit_cost::Catalog`) is deliberately not re-exported here --
//! not a wire shape, depend on `subbit-cost` directly for it.

pub use subbit_cost::dto::{
    CLAIM_BUDGET_HEADER, CLAIM_COST_HEADER, Catalog, Cost, CostByEgress, ResourceInfo,
};

mod channel_id;
pub use channel_id::ChannelId;

mod channel_status;
pub use channel_status::ChannelStatus;

mod mutual;
pub use mutual::Mutual;

mod output_ref;
pub use output_ref::OutputRef;

mod pop_mac_ttl;
pub use pop_mac_ttl::{KeyTagTtl, MacTtl, PopMacTtl, PopTtl};

mod request;
pub use request::{Commit, Request};

mod response;
pub use response::{Response, Status};

mod error;
pub use error::Error;

mod network;
pub use network::Network;

pub use subbit_core::Asset;

mod terms;
pub use terms::{Terms, WELL_KNOWN_PATH};

pub mod dialect;
