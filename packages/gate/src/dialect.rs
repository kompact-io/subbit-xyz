//! One module per wire dialect. Each knows how to recognize its own
//! request shape, decode it into the crate's shared `Request`, and
//! encode `Ctx`'s results back into its own wire shape (a `wire::
//! Wire`, see that module's own doc comment for what that shape is
//! and why). `Ctx` holds the configured dialects and is the one that
//! walks them (see `Ctx::handle_spend`/`Ctx::handle_refund`); nothing
//! here knows about `Ctx` at all.

use http::HeaderMap;
use subbit_l2_dto::{ChannelStatus, CoreError, MacTtl, Request, Terms};

use crate::{Keytag, catalog::ResourceInfo, wire::Wire};

#[cfg(feature = "vanilla")]
pub mod vanilla;
#[cfg(feature = "x402")]
pub mod x402;

/// What `Ctx`'s catalog/terms resolve for one request, bundled once
/// (`Ctx::quote`) rather than passed as several loose parameters: a
/// dialect building a "payment required" or error response (or
/// validating a decoded payload against current terms, see
/// `Dialect::decode`) reads whichever of these it needs and ignores
/// the rest (vanilla ignores all of them). `resource_url` is this
/// request's resource identifier the way x402's own `resource` field
/// wants it (a fully qualified URL when the provider has a
/// `public_base_url` configured, see `Ctx::resource_url`); it is not
/// the bare local path `Ctx::handle_spend`/`Ctx::handle_refund` use
/// for catalog lookups, which a `Dialect` never sees at all. `Clone`
/// so `Ctx` can hand the same one to `decode` and, afterward, to
/// whichever response-building method actually runs.
#[derive(Clone)]
pub struct Quote<'a> {
    pub resource_url: String,
    pub cost: u64,
    pub terms: &'a Terms,
    pub resource: ResourceInfo,
}

/// One wire dialect's full request/response shape. `Ctx` holds a
/// priority-ordered list of these (see `Ctx::dialects`) and walks it
/// per request: the first dialect that `recognizes` the headers wins,
/// and when none do, the first *configured* dialect speaks for the
/// fallback "payment required" response.
pub trait Dialect: Send + Sync {
    /// Does `headers` carry this dialect's own marker at all? Decides
    /// which dialect gets to `decode` a given request, not whether
    /// that decode will actually succeed.
    fn recognizes(&self, headers: &HeaderMap) -> bool;

    /// Parse this dialect's own wire payload out of `headers` into the
    /// crate's shared, dialect-agnostic `Request`, or this dialect's
    /// own wire-shaped error response when it can't. `quote` is what
    /// this provider is currently offering for the request's own url:
    /// a dialect whose envelope echoes back which terms the client is
    /// paying under (x402's `accepted`) checks it against `quote`
    /// here, rejecting a stale/mismatched echo as a decode failure
    /// rather than trusting it; a dialect with no such echo (vanilla)
    /// ignores `quote` entirely.
    fn decode(&self, headers: &HeaderMap, quote: Quote) -> Result<Request, Wire>;

    /// This dialect's "payment required": nothing recognizable was
    /// sent at all, so there's no channel to report on yet.
    fn challenge(&self, quote: Quote) -> Wire;

    /// A successful spend, in this dialect's own wire shape. `keytag`
    /// names the channel that absorbed it: a dialect with its own
    /// notion of a settlement/commitment identifier (x402's
    /// `batch-settlement` scheme requires a non-empty one on success,
    /// see `specs/schemes/batch-settlement`) uses it for that; vanilla
    /// has no such concept and ignores it.
    fn spend_ok(
        &self,
        quote: Quote,
        keytag: &Keytag,
        status: ChannelStatus,
        mac: Option<MacTtl>,
    ) -> Wire;

    /// A successful refund, in this dialect's own wire shape. Same
    /// `keytag` reasoning as `spend_ok`.
    fn refund_ok(&self, quote: Quote, keytag: &Keytag, status: ChannelStatus) -> Wire;

    /// Any other outcome but success. `current` is the channel's
    /// standing right now, when `Ctx` could find one, only ever
    /// present alongside `CoreError::Unspendable`, so a dialect that
    /// can rebuild a `challenge`-shaped response around it (x402's
    /// corrective 402) does; a dialect with no such shape (vanilla)
    /// ignores `quote` and `current` both, and just reports the bare
    /// error.
    fn error(&self, err: CoreError, quote: Quote, current: Option<ChannelStatus>) -> Wire;
}
