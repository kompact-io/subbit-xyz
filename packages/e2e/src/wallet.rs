//! A test keypair standing in for a real consumer wallet, plus the
//! same signing operations a real client would do: sign an `Iou`'s
//! cumulative amount, or prove liveness with a `Pop`. No real chain
//! key is involved; the matching "deposit" is faked via
//! [`crate::indexer`] instead of a real UTxO.

use subbit_core::{Duration, Signature, Tag, VerifyingKey};
use subbit_l2_dto::{ChannelId, Commit, KeyTagTtl, PopMacTtl, PopTtl, Request};

pub struct Wallet {
    sk: [u8; 64],
    pub key: VerifyingKey,
    pub tag: Tag,
}

impl Wallet {
    /// Deterministic from `seed`, so scenarios can rebuild the same
    /// wallet (e.g. to resume a channel) without storing key material.
    pub fn new(seed: [u8; 32], tag: impl Into<Vec<u8>>) -> Self {
        let (sk, pk) = cryptoxide::ed25519::keypair(&seed);
        Self {
            sk,
            key: VerifyingKey::from(pk),
            tag: Tag::from(tag.into()),
        }
    }

    pub fn channel_id(&self) -> ChannelId {
        ChannelId {
            iou_key: self.key,
            tag: self.tag.clone(),
        }
    }

    /// `key || tag`, the same byte layout `subbit-server`'s own
    /// `Keytag` and `subbit-index`'s wire `Keytag` both use — see
    /// `subbit_index::naive::keytag_of`.
    pub fn keytag_bytes(&self) -> Vec<u8> {
        let key_bytes: [u8; 32] = self.key.into();
        let mut buf = key_bytes.to_vec();
        buf.extend_from_slice(self.tag.as_ref());
        buf
    }

    /// Sign a cumulative claim of `amount` over this wallet's tag. Bare
    /// `subbit_core::Iou` (no channel id): wrap in a `Request::ById`
    /// yourself, or use `iou_payload` for that already done.
    pub fn sign_iou(&self, amount: u64) -> subbit_core::Iou {
        let tbs = subbit_core::TagTbs::new(self.tag.clone(), amount).to_vec();
        let signature = Signature::from(cryptoxide::ed25519::signature(&tbs, &self.sk));
        subbit_core::Iou::new(amount, signature)
    }

    /// Prove liveness: a fresh `Pop` good for `ttl` from now.
    pub fn sign_pop(&self, ttl: Duration) -> PopMacTtl {
        let body = KeyTagTtl {
            key: self.key,
            tag: self.tag.clone(),
            ttl: now() + ttl,
        };
        let sig = Signature::from(cryptoxide::ed25519::signature(&body.tbs(), &self.sk));
        PopMacTtl::Pop(PopTtl { body, sig })
    }

    /// A payload naming this wallet's channel directly, claiming a
    /// cumulative amount (`Request::ById`).
    pub fn iou_payload(&self, amount: u64) -> Request {
        Request::ById {
            channel_id: self.channel_id(),
            commit: Commit::Iou(self.sign_iou(amount)),
        }
    }

    /// A liveness-only payload: `auth` present, nothing new claimed
    /// (`Request::ByPopMac`).
    pub fn pop_payload(&self, ttl: Duration) -> Request {
        Request::ByPopMac {
            auth: self.sign_pop(ttl),
            commit: None,
        }
    }
}

pub fn now() -> Duration {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis() as u64;
    Duration::from_millis(ms)
}
