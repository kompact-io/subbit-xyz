use ed25519_dalek::{Signer as _, SigningKey};
use serde::{Deserialize, Serialize};
use subbit_core::{Duration, Iou, Pop, Signature, Tag, TagTbs, VerifyingKey, envelope::Body};

/// On-disk/wire shape for an [`Account`]: permissive raw types (any 32
/// bytes, any `Tag` — no validation at this layer), (de)serializable both
/// via `serde` (TOML config) and `minicbor` (cache/wire), since `Account`
/// itself no longer is either. Converts into the real, in-memory `Account`
/// via `Account::from(config)`.
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

    pub fn pop(&self, ttl: Duration) -> Pop<Body> {
        let body = Body {
            key: self.verifying_key(),
            tag: self.tag.clone(),
            ttl,
        };
        let sig = self.sign(&body.tbs());
        Pop { body, sig }
    }

    fn tag_tbs(&self, amount: u64) -> TagTbs {
        TagTbs::new(self.tag.clone(), amount)
    }

    pub fn iou(&self, amount: u64) -> Iou {
        let sig = self.sign(&self.tag_tbs(amount).to_vec());
        Iou::new(amount, sig)
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
