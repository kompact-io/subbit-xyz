use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use subbit_core::{Tag, VerifyingKey};

/// Identifies a channel at the L2, a synonym for `Keytag` elsewhere.
///
/// WARNING :: Uniqueness is not guaranteed at the L1 (docs/adrs/tag.md);
/// No tooling should assume otherwiser
#[derive(Debug, Clone, PartialEq, Eq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ChannelId {
    #[n(0)]
    pub iou_key: VerifyingKey,
    #[n(1)]
    pub tag: Tag,
}
