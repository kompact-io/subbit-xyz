use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Encode, Decode, thiserror::Error)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum CoreError {
    #[n(0)]
    #[error("other")]
    Other,
    /// A UTxO may exist at tip but be too shallow, carry a bad datum,
    /// or the wrong currency.
    #[n(1)]
    #[error("no such channel")]
    NoChannel,
    /// Set for any reason the provider decides; always set once it
    /// has observed the channel close, even if that transaction later
    /// rolls back.
    #[n(2)]
    #[error("channel not active")]
    Inactive,
    #[n(3)]
    #[error("stale proof of possession")]
    OldAuth,
    #[n(4)]
    #[error("invalid auth")]
    InvalidAuth,
    #[n(5)]
    #[error("channel has no iou")]
    NoIou,
    /// An Iou's amount must be strictly greater than what's already
    /// committed.
    #[n(6)]
    #[error("iou is old")]
    OldIou,
    /// Doesn't parse, or its signature doesn't verify.
    #[n(7)]
    #[error("iou invalid")]
    InvalidIou,
    #[n(8)]
    #[error("cost exceeds what is spendable (deactivated, insufficient funds, _etc_)")]
    Unspendable,
    #[n(9)]
    #[error("rate limited")]
    RateLimited,
    /// No `MutualService` exists yet to validate, co-sign, or
    /// broadcast a cooperative close.
    #[n(10)]
    #[error("mutual close not supported")]
    MutualNotSupported,
}
