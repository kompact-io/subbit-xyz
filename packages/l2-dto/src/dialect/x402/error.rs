//! x402's `errorReason` is an open string (x402-specification-v2.md
//! §5.3/§5.4), with a standard vocabulary (§9) plus room for a scheme
//! to mint its own codes, as the EVM `batch-settlement` binding
//! already does (`invalid_batch_settlement_evm_cumulative_amount_mismatch`).
//! [`X402Error`] is that encoding for Cardano: a newtype over `Error`
//! rather than a parallel enum, with `Display`/`FromStr` a faithful
//! bijection -- one code per `Error` variant, in declaration order.

use std::fmt;
use std::str::FromStr;

use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct X402Error(pub Error);

impl fmt::Display for X402Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self.0 {
            Error::Other => "unexpected_verify_error",
            Error::NoChannel => "invalid_payload",
            Error::Inactive => "invalid_batch_settlement_cardano_channel_inactive",
            Error::OldAuth => "invalid_batch_settlement_cardano_auth_old",
            Error::InvalidAuth => "invalid_batch_settlement_cardano_auth_invalid",
            Error::NoIou => "invalid_batch_settlement_cardano_no_iou",
            // Mirrors the EVM binding's own
            // `invalid_batch_settlement_evm_cumulative_amount_mismatch`.
            Error::OldIou => "invalid_batch_settlement_cardano_cumulative_amount_mismatch",
            Error::InvalidIou => "invalid_batch_settlement_cardano_invalid_iou",
            Error::InsufficientFunds => "insufficient_funds",
            Error::RateLimited => "invalid_batch_settlement_cardano_rate_limited",
            Error::MutualNotSupported => "invalid_batch_settlement_cardano_mutual_unsupported",
        };
        f.write_str(s)
    }
}

/// The exact inverse of `Display`.
impl FromStr for X402Error {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let err = match s {
            "unexpected_verify_error" => Error::Other,
            "invalid_payload" => Error::NoChannel,
            "invalid_batch_settlement_cardano_channel_inactive" => Error::Inactive,
            "invalid_batch_settlement_cardano_auth_old" => Error::OldAuth,
            "invalid_batch_settlement_cardano_auth_invalid" => Error::InvalidAuth,
            "invalid_batch_settlement_cardano_no_iou" => Error::NoIou,
            "invalid_batch_settlement_cardano_cumulative_amount_mismatch" => Error::OldIou,
            "invalid_batch_settlement_cardano_invalid_iou" => Error::InvalidIou,
            "insufficient_funds" => Error::InsufficientFunds,
            "invalid_batch_settlement_cardano_rate_limited" => Error::RateLimited,
            "invalid_batch_settlement_cardano_mutual_unsupported" => Error::MutualNotSupported,
            _ => return Err(()),
        };
        Ok(X402Error(err))
    }
}

impl Serialize for X402Error {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for X402Error {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let s = String::deserialize(deserializer)?;
        s.parse()
            .map_err(|()| serde::de::Error::custom(format!("unrecognized x402 error code: {s:?}")))
    }
}

pub fn error_reason(err: &Error) -> String {
    X402Error(*err).to_string()
}

/// The inverse of [`error_reason`]; an unrecognized code falls back
/// to `Error::Other` rather than `None`.
pub fn core_error(reason: &str) -> Error {
    reason
        .parse::<X402Error>()
        .map(|x| x.0)
        .unwrap_or(Error::Other)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_error_round_trips_exactly_through_the_wire_string() {
        for err in [
            Error::Other,
            Error::NoChannel,
            Error::Inactive,
            Error::OldAuth,
            Error::InvalidAuth,
            Error::NoIou,
            Error::OldIou,
            Error::InvalidIou,
            Error::InsufficientFunds,
            Error::RateLimited,
            Error::MutualNotSupported,
        ] {
            assert_eq!(core_error(&error_reason(&err)), err);
        }
    }
}
