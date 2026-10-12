//! Subbit's own L2 wire types: shared shapes both dialects use, plus
//! the dialects themselves, each behind its own feature flag.
//!
//! - `ChannelId`/`Mutual`/`Request`/`Commit`: the consumer-to-provider
//!   envelope both dialects use. See each type's own doc comment.
//! - `KeyTagTtl`/`PopTtl`/`MacTtl`/`PopMacTtl`: the liveness-proof
//!   envelope, built on `VerifyingKey`/`Tag`/`Duration`/`Signature`/
//!   `MacSig`.
//! - `ChannelStatus`: what a provider reports back over the wire.
//!   Wraps `subbit_core::Iou` (the bare `{amount, signature}` the
//!   server holds), not this crate's own `Request`/`Commit::Iou`.
//! - `CoreError`: the error vocabulary both dialects' wire errors are
//!   built from.
//! - `Terms`: what a provider requires before serving a request.
//!   `dialect::vanilla` re-exports it as-is; `dialect::x402::SubbitExtra`
//!   wraps it in x402's own shape.
//! - `Response`: what a provider reports back after a spend/refund.
//!   `success`/`error` are hoisted onto x402's own envelope fields;
//!   `status` (its own type, `Status`) is the part x402 has no field
//!   of its own for, so it rides in `extensions`. `dialect::vanilla`
//!   re-exports `Response` as-is, see `dialect::x402::settlement`.
//! - `Cost`/`OutputRef`: `ResourceInfo.cost` and
//!   `Terms.reference_script`, respectively.
//! - `Catalog`/`ResourceInfo`/`ResourceInfoOverride`: `Terms.catalog`,
//!   what a provider charges and presents, resolved per path.
//! - `dialect::vanilla`, `dialect::x402`: each dialect's own wire
//!   shapes, see their own module doc comments.

mod catalog;
pub use catalog::{Catalog, ResourceInfo, ResourceInfoOverride};

mod channel_id;
pub use channel_id::ChannelId;

mod channel_status;
pub use channel_status::ChannelStatus;

mod cost;
pub use cost::{CLAIM_BUDGET_HEADER, CLAIM_COST_HEADER, Cost, CostByEgress};

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

mod core_error;
pub use core_error::CoreError;

mod network;
pub use network::Network;

mod asset;
pub use asset::Asset;

mod terms;
pub use terms::Terms;

pub mod dialect;
