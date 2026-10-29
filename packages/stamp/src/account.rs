use ed25519_dalek::{Signer as _, SigningKey};
use serde::{Deserialize, Serialize};
use subbit_core::{Duration, Iou, Signature, Tag, TagTbs, VerifyingKey};
use subbit_l2_dto::{KeyTagTtl as Body, PopTtl};

/// On-disk/wire shape for an [`Account`]: permissive raw types (any 32
/// bytes, any `Tag`, no validation at this layer), (de)serializable
/// via `serde` (TOML config) and `minicbor` (cache/wire). Converts
/// into the real, in-memory `Account` via `Account::from(config)`.
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
pub struct Account {
    key: SigningKey,
    tag: Tag,
}

impl Account {
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

    pub fn pop(&self, ttl: Duration) -> PopTtl {
        let body = Body {
            key: self.verifying_key(),
            tag: self.tag.clone(),
            ttl,
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

    /// This account's channel identifier: the same `(key, tag)` pair
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

impl From<Config> for Account {
    fn from(config: Config) -> Self {
        Account::new(SigningKey::from_bytes(&config.signing_key), config.tag)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(seed: u8) -> Account {
        Account::new(
            SigningKey::from_bytes(&[seed; 32]),
            Tag::from(b"t".to_vec()),
        )
    }

    #[test]
    fn an_iou_verifies_against_the_account_that_signed_it() {
        let account = account(1);
        let iou = account.iou(500);
        assert!(account.verify_iou(&iou));
    }

    #[test]
    fn an_iou_does_not_verify_against_a_different_account() {
        let mine = account(1);
        let other = account(2);
        let iou = mine.iou(500);
        assert!(!other.verify_iou(&iou));
    }

    #[test]
    fn tampering_with_the_amount_breaks_verification() {
        let account = account(1);
        let iou = account.iou(500);
        let tampered = Iou::new(600, *iou.signature());
        assert!(!account.verify_iou(&tampered));
    }

    #[test]
    fn channel_id_carries_the_verifying_key_and_tag() {
        let account = account(1);
        let channel_id = account.channel_id();
        assert_eq!(channel_id.iou_key, account.verifying_key());
        assert_eq!(channel_id.tag, Tag::from(b"t".to_vec()));
    }

    #[test]
    fn pop_signs_over_the_key_tag_and_ttl() {
        let account = account(1);
        let pop = account.pop(Duration::from_secs(60));
        assert_eq!(pop.body.key, account.verifying_key());
        assert_eq!(pop.body.tag, Tag::from(b"t".to_vec()));
    }

    #[test]
    fn config_roundtrips_into_an_equivalent_account() {
        let config = Config {
            signing_key: [7u8; 32],
            tag: Tag::from(b"t".to_vec()),
        };
        let account = Account::from(config);
        let iou = account.iou(42);
        assert!(account.verify_iou(&iou));
    }
}
