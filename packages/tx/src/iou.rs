use cardano_sdk::{Signature, VerificationKey};
use subbit_core::{Iou, Tag};

/// IOU failed verification against the channel's `iou_key`.
#[derive(Debug, Clone, thiserror::Error)]
#[error("IOU failed verification")]
pub struct Error;

pub(crate) fn verify(
    iou_key: &subbit_core::VerifyingKey,
    tag: &Tag,
    iou: &Iou,
) -> Result<(), Error> {
    let message = iou.tbs(tag.clone()).to_vec();
    let vk_bytes: &[u8; 32] = iou_key.as_ref();
    let sig_bytes: &[u8; 64] = iou.signature().as_ref();
    if VerificationKey::from(*vk_bytes).verify(&message, &Signature::from(*sig_bytes)) {
        Ok(())
    } else {
        Err(Error)
    }
}
