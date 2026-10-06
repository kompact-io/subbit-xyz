//! `Keyring` is used to sign txs after build, and before submit.
//! Specifically those fields belonging to `specified_signatories` (aka `required_signers`)

use std::collections::BTreeMap;

use cardano_sdk::{
    Hash, SigningKey, Transaction, VerificationKey, transaction::state::ReadyForSigning,
};

/// FIXME :: leakable signing key is unusable.
/// Encapsulation is predominantly just to have hex serde.
#[derive(Debug, Clone, Default, PartialEq, Eq, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize,))]
pub struct KeyHex(#[cfg_attr(feature = "serde", serde(with = "hex::serde"))] [u8; 32]);

impl From<KeyHex> for SigningKey {
    fn from(value: KeyHex) -> Self {
        SigningKey::from(value.0)
    }
}

impl From<&KeyHex> for [u8; 32] {
    fn from(value: &KeyHex) -> Self {
        value.0
    }
}

impl From<[u8; 32]> for KeyHex {
    fn from(value: [u8; 32]) -> Self {
        KeyHex(value)
    }
}

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize,))]
pub struct Config {
    pub keys: BTreeMap<String, KeyHex>,
}

impl Config {
    pub fn build(self) -> Keyring {
        Keyring::new(self.keys.into_values().map(|k| k.into()))
    }

    pub fn insert(&mut self, label: String, key: [u8; 32]) -> bool {
        self.keys.insert(label, key.into()).is_none()
    }

    pub fn remove_by_key(&mut self, _key: [u8; 32]) -> bool {
        todo!();
        //self.keys.remove(&key.into()).is_some()
    }

    pub fn flip(&self) -> BTreeMap<KeyHex, String> {
        self.keys.clone().into_iter().map(|(k, v)| (v, k)).collect()
    }

    pub fn remove(&mut self, label: String) -> bool {
        self.keys.remove(&label).is_some()
    }

    pub fn label(&self, key: &KeyHex) -> String {
        self.flip().get(key).cloned().unwrap_or("NONE".to_string())
    }

    /// labels for verifying key and vkhs
    pub fn labels(&self) -> BTreeMap<String, String> {
        self.keys
            .iter()
            .map(|(l, k)| (SigningKey::from(k.clone()).to_verification_key(), l))
            .flat_map(|(k, l)| {
                [
                    (l.clone(), hex::encode(k)),
                    (
                        format!("{}_vkh", l.clone()),
                        hex::encode(Hash::<28>::new(k)),
                    ),
                ]
            })
            .collect()
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

    pub fn vks(&self) -> BTreeMap<Hash<28>, VerificationKey> {
        self.keys
            .iter()
            .map(|(vkh, sk)| (*vkh, sk.to_verification_key()))
            .collect()
    }

    pub fn contains(&self, vkh: &Hash<28>) -> bool {
        self.keys.contains_key(vkh)
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
