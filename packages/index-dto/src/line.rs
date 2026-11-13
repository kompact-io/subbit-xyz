use crate::point::{OutputReference, Point};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A variant of channel stage relevant to choosing a Backing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Stagish {
    /// Potential Channel backing.
    #[n(0)]
    Opened {
        #[n(0)]
        amount: u64,
        #[n(1)]
        subbed: u64,
    },
    /// Channel (line) closed, not yet settled.
    #[n(1)]
    Closed,
    /// Channel (line) settled on-chain -- the legitimate end of the
    /// line, nothing left to do.
    #[n(3)]
    Settled,
    /// Channel (line) terminated other than by close (a mutual spend, or
    /// an item dropped for violating well-formedness).
    #[n(2)]
    Terminated,
}

/// One TxO on a lineage.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Item {
    #[n(0)]
    pub created_at: Point,
    #[n(1)]
    pub txo_ref: OutputReference,
    #[n(2)]
    pub stage: Stagish,
}

/// A sequence of TxOs related by lineage (parent -> child via continuing
/// output). An empty Line is treated as Terminated.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cbor(transparent)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(transparent))]
pub struct Line(#[n(0)] pub Vec<Item>);
