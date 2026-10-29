//! Wire-compatible with `subbit_gate::Keytag` (same raw-bytes-as-CBOR-
//! bytestring shape: `iou_key` bytes ++ `tag` bytes) -- this crate keeps
//! its own copy rather than depending on `subbit-gate` itself, the
//! same way `subbit_index::naive::keytag_of` already does.

use subbit_l1_dto::Constants;

use crate::keyring::Keyring;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, minicbor::Decode)]
#[cbor(transparent)]
pub struct Keytag(#[cbor(with = "minicbor::bytes")] Vec<u8>);

impl Keytag {
    pub fn of(constants: &Constants) -> Self {
        let mut bytes = AsRef::<[u8]>::as_ref(constants.iou_key()).to_vec();
        bytes.extend_from_slice(constants.tag().as_ref());
        Self(bytes)
    }
}

/// `constants.provider()` matches one of `keyring`'s own held keys. A
/// shared validator address can carry channels for other providers too,
/// so this is the filter every caller (this crate's own `main`, and
/// anyone else driving the same decision logic) must apply before
/// handing `channels` to `decide::decide` -- never assume every channel
/// at the address belongs to you.
pub fn is_ours(constants: &Constants, keyring: &Keyring) -> bool {
    let provider: [u8; 28] = (*constants.provider()).into();
    keyring
        .vkhs()
        .iter()
        .any(|vkh| <[u8; 28]>::from(*vkh) == provider)
}
