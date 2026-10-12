use crate::{Iou, Signature, Tag, VerifyingKey};

/// Raw Ed25519 verification: `sig` over `msg`, checked against `key`.
pub fn verify(key: &VerifyingKey, msg: &[u8], sig: &Signature) -> bool {
    cryptoxide::ed25519::verify(msg, key.as_ref(), sig.as_ref())
}

/// Verifies an iou's signature was made by `iou_key` over `(tag, iou.amount())`.
pub fn verify_iou(iou_key: &VerifyingKey, tag: &Tag, iou: &Iou) -> bool {
    verify(iou_key, &iou.tbs(tag.clone()).to_vec(), iou.signature())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verifies_a_real_signature() {
        let (sk, pk) = cryptoxide::ed25519::keypair(&[7u8; 32]);
        let msg = b"subbit";
        let sig = cryptoxide::ed25519::signature(msg, &sk);

        let key = VerifyingKey::from(pk);
        let sig = Signature::from(sig);

        assert!(verify(&key, msg, &sig));
        assert!(!verify(&key, b"not subbit", &sig));
    }

    #[test]
    fn verify_iou_matches_signed_tag_and_amount() {
        let (sk, pk) = cryptoxide::ed25519::keypair(&[9u8; 32]);
        let key = VerifyingKey::from(pk);
        let tag = Tag::from(b"my-tag".to_vec());

        let amount = 42u64;
        let tbs = crate::TagTbs::new(tag.clone(), amount).to_vec();
        let sig = Signature::from(cryptoxide::ed25519::signature(&tbs, &sk));
        let iou = Iou::new(amount, sig);

        assert!(verify_iou(&key, &tag, &iou));
        assert!(!verify_iou(&key, &Tag::from(b"other-tag".to_vec()), &iou));
    }
}
