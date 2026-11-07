//! `CoreError` is Subbit's own, dialect-agnostic error vocabulary
//! (shared with `vanilla`, whose wire format serializes it as-is via
//! its own `Display`). x402 defines a closed-looking but actually
//! open `errorReason`/`invalidReason` string field (x402-specification-v2.md
//! §5.3/§5.4: typed `string`, not an enum) with a documented standard
//! vocabulary (§9) that generic clients special-case, plus room for a
//! scheme to mint its own codes where none of the standard ones fit —
//! the EVM `batch-settlement` binding already does this
//! (`invalid_batch_settlement_evm_cumulative_amount_mismatch`).
//!
//! This is that translation for the Cardano `batch-settlement`
//! binding: a closed `X402Error` enum (so a new `CoreError` variant
//! fails to compile here until someone decides where it maps, rather
//! than silently falling through) and the mapping from `CoreError`.
//!
//! `RateLimited` has no `X402Error` variant: rate limiting is an
//! HTTP-layer concern (plain 429), not a payment-validity one, and
//! callers SHOULD respond before constructing a `CoreError`-bearing
//! `Response` at all. Nothing enforces that, though, so `from_core`
//! stays total and falls back to `CoreError`'s own `Display` string
//! for this one case, rather than mapping it to a code that would
//! mislead a generic client or panicking on a reachable state.

use std::fmt;

use crate::CoreError;

/// x402 `errorReason`/`invalidReason` codes this binding emits: the
/// standard §9 vocabulary where it genuinely fits, and scheme-specific
/// `invalid_batch_settlement_cardano_*` codes where nothing standard
/// does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum X402Error {
    /// `CoreError::Unspendable`.
    InsufficientFunds,
    /// `CoreError::NoChannel`, `NoIou`, `OldAuth`, `InvalidIou`. The
    /// distinctions between these (no backing UTxO vs. no IOU yet vs.
    /// a bad IOU signature) aren't ones any current client acts on
    /// differently.
    InvalidPayload,
    /// `CoreError::InvalidAuth`: a bad liveness proof (`Pop`/`Mac`),
    /// kept distinct from `InvalidPayload` because `subbit-stamp`'s
    /// own x402 client matches on exactly this code to decide whether
    /// to drop its cached `Mac` and fall back to a fresh `Pop`.
    AuthInvalid,
    /// `CoreError::Other`.
    UnexpectedVerifyError,
    /// `CoreError::OldIou`: the presented cumulative amount doesn't
    /// exceed what's already committed. Mirrors the EVM binding's own
    /// `invalid_batch_settlement_evm_cumulative_amount_mismatch`.
    CumulativeAmountMismatch,
    /// `CoreError::Inactive`: the channel is closed, settled, or
    /// otherwise no longer usable.
    ChannelInactive,
    /// `CoreError::MutualNotSupported`.
    MutualUnsupported,
}

impl X402Error {
    pub fn as_str(&self) -> &'static str {
        match self {
            X402Error::InsufficientFunds => "insufficient_funds",
            X402Error::InvalidPayload => "invalid_payload",
            X402Error::AuthInvalid => "invalid_batch_settlement_cardano_auth_invalid",
            X402Error::UnexpectedVerifyError => "unexpected_verify_error",
            X402Error::CumulativeAmountMismatch => {
                "invalid_batch_settlement_cardano_cumulative_amount_mismatch"
            }
            X402Error::ChannelInactive => "invalid_batch_settlement_cardano_channel_inactive",
            X402Error::MutualUnsupported => "invalid_batch_settlement_cardano_mutual_unsupported",
        }
    }
}

impl fmt::Display for X402Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<&CoreError> for X402Error {
    /// `Err` only for `CoreError::RateLimited`, which has no code in
    /// this binding's x402 vocabulary (see module docs).
    type Error = ();

    fn try_from(err: &CoreError) -> Result<Self, Self::Error> {
        match err {
            CoreError::Unspendable => Ok(X402Error::InsufficientFunds),
            CoreError::NoChannel
            | CoreError::NoIou
            | CoreError::OldAuth
            | CoreError::InvalidIou => Ok(X402Error::InvalidPayload),
            CoreError::InvalidAuth => Ok(X402Error::AuthInvalid),
            CoreError::Other => Ok(X402Error::UnexpectedVerifyError),
            CoreError::OldIou => Ok(X402Error::CumulativeAmountMismatch),
            CoreError::Inactive => Ok(X402Error::ChannelInactive),
            CoreError::MutualNotSupported => Ok(X402Error::MutualUnsupported),
            CoreError::RateLimited => Err(()),
        }
    }
}

/// Total over every `CoreError`: this binding's x402 `errorReason`
/// code where one exists, else `CoreError`'s own `Display` string
/// (today, only for `RateLimited`) — never a panic on a reachable
/// state.
pub fn error_reason(err: &CoreError) -> String {
    X402Error::try_from(err)
        .map(|e| e.as_str().to_string())
        .unwrap_or_else(|()| err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unspendable_maps_to_insufficient_funds() {
        assert_eq!(error_reason(&CoreError::Unspendable), "insufficient_funds");
    }

    #[test]
    fn old_iou_maps_to_cumulative_amount_mismatch() {
        assert_eq!(
            error_reason(&CoreError::OldIou),
            "invalid_batch_settlement_cardano_cumulative_amount_mismatch"
        );
    }

    #[test]
    fn inactive_maps_to_channel_inactive() {
        assert_eq!(
            error_reason(&CoreError::Inactive),
            "invalid_batch_settlement_cardano_channel_inactive"
        );
    }

    #[test]
    fn mutual_not_supported_maps_to_mutual_unsupported() {
        assert_eq!(
            error_reason(&CoreError::MutualNotSupported),
            "invalid_batch_settlement_cardano_mutual_unsupported"
        );
    }

    #[test]
    fn payload_shaped_errors_collapse_to_invalid_payload() {
        for err in [
            CoreError::NoChannel,
            CoreError::NoIou,
            CoreError::OldAuth,
            CoreError::InvalidIou,
        ] {
            assert_eq!(error_reason(&err), "invalid_payload");
        }
    }

    #[test]
    fn invalid_auth_keeps_its_own_code() {
        assert_eq!(
            error_reason(&CoreError::InvalidAuth),
            "invalid_batch_settlement_cardano_auth_invalid"
        );
    }

    #[test]
    fn other_maps_to_unexpected_verify_error() {
        assert_eq!(error_reason(&CoreError::Other), "unexpected_verify_error");
    }

    #[test]
    fn rate_limited_has_no_x402_code_and_falls_back_to_core_error_display() {
        assert!(X402Error::try_from(&CoreError::RateLimited).is_err());
        assert_eq!(error_reason(&CoreError::RateLimited), "rate limited");
    }
}
