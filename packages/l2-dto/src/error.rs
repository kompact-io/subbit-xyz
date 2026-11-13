use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Subbit's own, dialect-agnostic error vocabulary. `vanilla`
/// serializes it as-is (CBOR `#[n(_)]` tags); `x402` wraps it in
/// [`crate::dialect::x402::X402Error`] for its own wire codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Error {
    #[n(0)]
    #[error("other")]
    Other,
    /// UTxO at tip too shallow, bad datum, or wrong currency.
    #[n(1)]
    #[error("no channel")]
    NoChannel,
    /// Sticky once the provider has observed the close, even if that
    /// transaction later rolls back.
    #[n(2)]
    #[error("inactive channel")]
    Inactive,
    #[n(3)]
    #[error("old auth")]
    OldAuth,
    #[n(4)]
    #[error("invalid auth")]
    InvalidAuth,
    /// An initial IOU may be required even for a free request.
    #[n(5)]
    #[error("channel has no iou")]
    NoIou,
    /// Amount must strictly exceed what's already committed.
    #[n(6)]
    #[error("iou is old")]
    OldIou,
    #[n(7)]
    #[error("iou invalid")]
    InvalidIou,
    #[n(8)]
    #[error("cost exceeds what is spendable (deactivated, insufficient funds, _etc_)")]
    InsufficientFunds,
    #[n(9)]
    #[error("rate limited")]
    RateLimited,
    #[n(10)]
    #[error("mutual close not supported")]
    MutualNotSupported,
}
