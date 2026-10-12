use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Mutual tx.
///
/// The consumer builds, signs,
/// and sends the transaction; the provider validates, co-signs, and
/// broadcasts it.
///
/// Before co-signing, the provider requires:
///
/// - first output paying its unclaimed amount to the address derived
///   from its key (no stake, no datum, no script).
/// - the channel id is an input.
/// - no inputs belong to the provider. Most robust check: find UTxOs
///   at tip, and verify the payment credential does not match.
///
/// The consumer covers the fee and any min ada required. The provider
/// broadcasts, and must treat the channel as settled/inactive.
///
/// Channel id lives on `Request::ById`/`ByPopMac`, not here.
#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Mutual {
    #[n(0)]
    #[cbor(with = "minicbor::bytes")]
    #[cfg_attr(feature = "serde", serde(with = "subbit_core::hex_bytes"))]
    pub transaction: Vec<u8>,
}
