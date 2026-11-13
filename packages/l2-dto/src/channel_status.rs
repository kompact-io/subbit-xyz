use minicbor::{Decode, Encode};
#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Encode, Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ChannelStatus {
    /// Latest IOU, channel id stripped.
    /// Client not at latest state, verifies signature, and syncs from here.
    #[n(0)]
    pub iou: subbit_core::Iou,
    /// `None` if no backing is found: no matching UTXO, a bad datum,
    /// insufficient block depth, or the provider treats the channel
    /// as inactive for any other reason.
    #[n(1)]
    pub spendable: Option<u64>,

    /// Backing not yet committed. `None`: no backing. `0`: fully
    /// committed.
    #[n(2)]
    pub uncommitted: Option<u64>,
}
