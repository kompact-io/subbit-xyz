use subbit_core::Hash28;
use subbit_l1_dto::Constants;

use crate::Line;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// `Constants` plus the delegation (stake) credential hash: the
/// validator enforces that both are equal between an input and its
/// continuing output under a normal (non-mutual) spend.
///
/// Just the hash, not a Key/Script-tagged credential: address
/// decomposition from Floe's wire data doesn't preserve that
/// distinction either (see `floe_client::shelley_hashes`).
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ConstantsExt {
    #[n(0)]
    pub constants: Constants,
    #[n(1)]
    pub delegation: Option<Hash28>,
}

/// A channel with no lines is treated as Terminated. Re-activation
/// requires intervention.
#[derive(Debug, Clone, PartialEq, Eq, minicbor::Encode, minicbor::Decode)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Channel {
    #[n(0)]
    pub constants_ext: ConstantsExt,
    #[n(1)]
    pub lines: Vec<Line>,
}
