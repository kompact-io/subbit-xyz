use subbit_core::Hash32;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A position on chain: block slot, hash, and height.
///
/// `height` (not just `slot`) matters downstream: the server's
/// settled-depth check (README's "Downstream" section) counts block
/// depth, not slot distance -- slots per block vary, so the two aren't
/// interchangeable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Point {
    #[n(0)]
    pub slot: u64,
    #[n(1)]
    pub hash: Hash32,
    #[n(2)]
    pub height: u64,
}

/// Reference to a TxO: the hash of the transaction that produced it, and
/// its output index within that transaction.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, minicbor::Encode, minicbor::Decode,
)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct OutputReference {
    #[n(0)]
    pub tx_hash: Hash32,
    #[n(1)]
    pub index: u64,
}
