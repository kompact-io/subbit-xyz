use ed25519_dalek::{Signer as _, SigningKey};
use serde::{Deserialize, Serialize};
use subbit_core::{Instant, Iou, Signature, Tag, TagTbs, VerifyingKey};
use subbit_l2_dto::{KeyTagTtl as Body, PopTtl};

/// On-disk/wire shape for an [`Id`]: permissive raw types (any 32
/// bytes, any `Tag`, no validation at this layer), (de)serializable
/// via `serde` (TOML config) and `minicbor` (cache/wire). Converts
/// into the real, in-memory `Id` via `Id::from(config)`.
#[derive(Debug, Clone, Serialize, Deserialize, minicbor::Encode, minicbor::Decode)]
pub struct Config {
    /// hex-encoded ed25519 signing key (32 bytes)
    #[n(0)]
    #[serde(with = "hex::serde")]
    #[cbor(with = "minicbor::bytes")]
    pub signing_key: [u8; 32],
    #[n(1)]
    pub tag: Tag,
}

#[derive(Debug, Clone)]
pub struct Id {
    key: SigningKey,
    tag: Tag,
}

impl Id {
    pub fn new(key: impl Into<SigningKey>, tag: impl Into<Tag>) -> Self {
        Self {
            key: key.into(),
            tag: tag.into(),
        }
    }

    fn sign(&self, msg: &[u8]) -> Signature {
        self.key.sign(msg).to_bytes().into()
    }

    fn verifying_key(&self) -> VerifyingKey {
        subbit_core::VerifyingKey::from(self.key.verifying_key().to_bytes())
    }

    pub fn pop(&self, expires_at: Instant) -> PopTtl {
        let body = Body {
            key: self.verifying_key(),
            tag: self.tag.clone(),
            expires_at,
        };
        let sig = self.sign(&body.tbs());
        PopTtl { body, sig }
    }

    fn tag_tbs(&self, amount: u64) -> TagTbs {
        TagTbs::new(self.tag.clone(), amount)
    }

    pub fn iou(&self, amount: u64) -> Iou {
        let sig = self.sign(&self.tag_tbs(amount).to_vec());
        Iou::new(amount, sig)
    }

    /// This id's channel identifier: the same `(key, tag)` pair
    /// `pop` signs over, needed separately to embed in a `Request::Iou`.
    pub fn channel_id(&self) -> subbit_l2_dto::ChannelId {
        subbit_l2_dto::ChannelId {
            iou_key: self.verifying_key(),
            tag: self.tag.clone(),
        }
    }

    pub fn verify_iou(&self, iou: &Iou) -> bool {
        self.key
            .verify(
                &self.tag_tbs(iou.amount()).to_vec(),
                &<[u8; 64]>::from(*iou.signature()).into(),
            )
            .is_ok()
    }
}

impl From<Config> for Id {
    fn from(config: Config) -> Self {
        Id::new(SigningKey::from_bytes(&config.signing_key), config.tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn id(seed: u8) -> Id {
        Id::new(
            SigningKey::from_bytes(&[seed; 32]),
            Tag::from(b"t".to_vec()),
        )
    }

    #[test]
    fn an_iou_verifies_against_the_id_that_signed_it() {
        let id = id(1);
        let iou = id.iou(500);
        assert!(id.verify_iou(&iou));
    }

    #[test]
    fn an_iou_does_not_verify_against_a_different_id() {
        let mine = id(1);
        let other = id(2);
        let iou = mine.iou(500);
        assert!(!other.verify_iou(&iou));
    }

    #[test]
    fn tampering_with_the_amount_breaks_verification() {
        let id = id(1);
        let iou = id.iou(500);
        let tampered = Iou::new(600, *iou.signature());
        assert!(!id.verify_iou(&tampered));
    }

    #[test]
    fn channel_id_carries_the_verifying_key_and_tag() {
        let id = id(1);
        let channel_id = id.channel_id();
        assert_eq!(channel_id.iou_key, id.verifying_key());
        assert_eq!(channel_id.tag, Tag::from(b"t".to_vec()));
    }

    #[test]
    fn pop_signs_over_the_key_tag_and_ttl() {
        let id = id(1);
        let pop = id.pop(Instant::from_secs(60));
        assert_eq!(pop.body.key, id.verifying_key());
        assert_eq!(pop.body.tag, Tag::from(b"t".to_vec()));
    }

    #[test]
    fn config_roundtrips_into_an_equivalent_id() {
        let config = Config {
            signing_key: [7u8; 32],
            tag: Tag::from(b"t".to_vec()),
        };
        let id = Id::from(config);
        let iou = id.iou(42);
        assert!(id.verify_iou(&iou));
    }
}
