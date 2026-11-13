//! `Keyring` is used to sign txs after build, and before submit.
//! Specifically those fields belonging to `specified_signatories` (aka `required_signers`)
//!
//! Copied from `subbit-session`'s own `keyring` module rather than depending on
//! that crate -- claim has no other use for `subbit-session`, and pulling it in
//! just for this would drag `cardano-connector`/`cardano-session`/`cardano-wallet`
//! back into the dependency tree this crate is deliberately dropping them from.

use std::collections::BTreeMap;

use cardano_sdk::{Hash, SigningKey, Transaction, transaction::state::ReadyForSigning};

/// FIXME :: leakable signing key is unusable.
/// Encapsulation is predominantly just to have hex serde.
#[derive(
    Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub struct KeyHex(#[serde(with = "hex::serde")] [u8; 32]);

impl From<KeyHex> for SigningKey {
    fn from(value: KeyHex) -> Self {
        SigningKey::from(value.0)
    }
}

impl From<[u8; 32]> for KeyHex {
    fn from(value: [u8; 32]) -> Self {
        KeyHex(value)
    }
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Config {
    pub keys: BTreeMap<String, KeyHex>,
}

impl Config {
    pub fn build(self) -> Keyring {
        Keyring::new(self.keys.into_values().map(|k| k.into()))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no signing key for {0:?}")]
    KeyNotRecognized(Hash<28>),
}

pub struct Keyring {
    keys: BTreeMap<Hash<28>, SigningKey>,
}

impl Keyring {
    pub fn new(keys: impl IntoIterator<Item = SigningKey>) -> Self {
        let keys = keys
            .into_iter()
            .map(|k| (Hash::<28>::new(k.to_verification_key()), k))
            .collect();
        Self { keys }
    }

    pub fn vkhs(&self) -> Vec<Hash<28>> {
        self.keys.keys().cloned().collect()
    }

    fn signing_key(&self, vkh: &Hash<28>) -> Result<&SigningKey, Error> {
        self.keys.get(vkh).ok_or(Error::KeyNotRecognized(*vkh))
    }

    pub fn sign(&self, tx: &mut Transaction<ReadyForSigning>) -> Result<(), Error> {
        for vkh in tx.specified_signatories().collect::<Vec<_>>() {
            tx.sign(self.signing_key(&vkh)?);
        }
        Ok(())
    }
}
