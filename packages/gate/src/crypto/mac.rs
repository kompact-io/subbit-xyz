/// Length in bytes of the MAC key//! MAC using BLAKE3's keyed hash, truncated to 20 bytes.
use std::ops::Deref;

use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, Encode, Decode, Default,
)]
pub struct Key(
    #[serde(with = "crate::hex_bytes")]
    #[n(0)]
    [u8; blake3::KEY_LEN],
);

impl Deref for Key {
    type Target = [u8; blake3::KEY_LEN];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<[u8; blake3::KEY_LEN]> for Key {
    fn from(bytes: [u8; blake3::KEY_LEN]) -> Self {
        Self(bytes)
    }
}

impl From<Key> for [u8; blake3::KEY_LEN] {
    fn from(key: Key) -> Self {
        key.0
    }
}

#[derive(Debug, Clone, Copy, Eq)]
pub struct Signature([u8; 20]);

impl Deref for Signature {
    type Target = [u8; 20];
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl From<[u8; 20]> for Signature {
    fn from(bytes: [u8; 20]) -> Self {
        Self(bytes)
    }
}

impl From<Signature> for [u8; 20] {
    fn from(sig: Signature) -> Self {
        sig.0
    }
}

// Constant-time: avoids leaking which byte differs via early exit.
impl PartialEq for Signature {
    fn eq(&self, other: &Self) -> bool {
        let mut diff = 0u8;
        for (a, b) in self.0.iter().zip(other.0.iter()) {
            diff |= a ^ b;
        }
        diff == 0
    }
}

/// Pre-keyed MAC service; reuse one instance to sign and verify.
///
/// The keyed hasher is built once from `key` and cloned per call, avoiding
/// re-deriving the key schedule on every `sign`/`verify`.
#[derive(Debug, Clone)]
pub struct Mac(blake3::Hasher);

impl Mac {
    pub fn new(key: Key) -> Self {
        Self(blake3::Hasher::new_keyed(&key))
    }

    pub fn sign(&self, msg: &[u8]) -> Signature {
        let mut hasher = self.0.clone();
        hasher.update(msg);
        let full = hasher.finalize();
        let mut out = [0u8; 20];
        out.copy_from_slice(&full.as_bytes()[..20]);
        out.into()
    }

    pub fn verify(&self, msg: &[u8], sig: &Signature) -> bool {
        self.sign(msg) == *sig
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_is_20_bytes() {
        let mac = Mac::new(Key::from([9u8; blake3::KEY_LEN]));
        assert_eq!(mac.sign(b"hello subbit").len(), 20);
    }

    #[test]
    fn verifies_a_signature_it_produced() {
        let mac = Mac::new(Key::from([9u8; blake3::KEY_LEN]));
        let msg = b"hello subbit";

        let sig = mac.sign(msg);

        assert!(mac.verify(msg, &sig));
    }

    #[test]
    fn rejects_a_tampered_message() {
        let mac = Mac::new(Key::from([9u8; blake3::KEY_LEN]));
        let sig = mac.sign(b"hello subbit");

        assert!(!mac.verify(b"hello SUBBIT", &sig));
    }

    #[test]
    fn rejects_the_wrong_key() {
        let mac = Mac::new(Key::from([9u8; blake3::KEY_LEN]));
        let wrong_mac = Mac::new(Key::from([1u8; blake3::KEY_LEN]));
        let msg = b"hello subbit";
        let sig = mac.sign(msg);

        assert!(!wrong_mac.verify(msg, &sig));
    }
}
