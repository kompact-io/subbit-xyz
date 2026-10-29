use crate::{Channel, Point};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// Snapshot of the index service's full view at a given chain tip.
///
/// Snapshot mode: a channel (identified by its `constants_ext`) missing
/// from `channels` is implied Terminated.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Output {
    #[n(0)]
    pub tip: Point,
    #[n(1)]
    pub channels: Vec<Channel>,
}
