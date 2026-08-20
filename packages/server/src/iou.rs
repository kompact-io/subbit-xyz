use subbit_core::{Iou, Tag, VerifyingKey};

use crate::crypto::ed25519;

pub fn is_valid(key: &VerifyingKey, tag: &Tag, iou: &Iou) -> bool {
    ed25519::verify(key, iou.tbs(tag.clone()).to_vec().as_ref(), iou.signature())
}
