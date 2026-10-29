//! Console's own keyring, parallel to -- not shared with, not upstreamed
//! from -- `subbit-claim`'s `src/keyring.rs`/`subbit-cli`'s
//! `src/cmd/keyring.rs`. See plan-subbit-console.md section E.
//!
//! Each labelled entry carries two independent role flags, `is_consumer`
//! and `is_iou`: a single ed25519 keypair can legitimately serve as
//! `Constants.consumer` (hashed to a vkh, Cardano-style) and/or
//! `Constants.iou_key` (used as-is) for different channels, since both
//! are the same underlying cryptographic primitive, just used two
//! different ways (see `docs/adrs/x402.md`'s keytag note, and plan
//! section E's resolution on why a wallet's own vkh is `consumer`-only,
//! never `iou_key`). `Keyring::consumers`/`Keyring::iou_keys` are the
//! "populated by the keyring" half of `crate::Consumers`/`crate::
//! IouKeys`'s own `effective` views -- see those modules' docs.

use std::collections::BTreeMap;
use std::str::FromStr;

use cardano_sdk::Hash;
use ed25519_dalek::SigningKey;

use subbit_core::{Hash28, VerifyingKey};

use crate::{Consumers, Error, IouKeys};

/// On-disk/wire shape for a held signing key: hex-encoded 32-byte
/// ed25519 seed, the exact shape `subbit_stamp::account::Config::
/// signing_key` expects -- a Keyring entry is directly exportable as a
/// Stamp `Config` once paired with a tag (see plan section E's "Export
/// to Stamp is insisted on, not optional").
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SigningKeyHex(#[serde(with = "hex::serde")] [u8; 32]);

impl SigningKeyHex {
    pub fn to_bytes(&self) -> [u8; 32] {
        self.0
    }

    pub fn verifying_key(&self) -> VerifyingKey {
        let sk = SigningKey::from_bytes(&self.0);
        VerifyingKey::from(sk.verifying_key().to_bytes())
    }

    /// This key's `Constants.consumer`-shaped identity: a Cardano-style
    /// payment-key hash (blake2b-224) of the verifying key, *not* the
    /// raw verifying key itself -- `cardano_sdk::Hash::<28>::new` hashes
    /// whatever bytes it's given, the same way `claim`/`session`'s own
    /// keyrings derive a vkh from a signing key.
    pub fn consumer_hash(&self) -> Hash28 {
        Hash28::from(<[u8; 28]>::from(Hash::<28>::new(self.verifying_key())))
    }
}

impl From<[u8; 32]> for SigningKeyHex {
    fn from(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }
}

impl FromStr for SigningKeyHex {
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Error> {
        let bytes = hex::decode(s)?;
        let bytes: [u8; 32] = bytes
            .try_into()
            .map_err(|_| Error::Key("signing key must be 32 bytes".to_string()))?;
        Ok(Self(bytes))
    }
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct Entry {
    key: SigningKeyHex,
    is_consumer: bool,
    is_iou: bool,
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Keyring {
    entries: BTreeMap<String, Entry>,
}

impl Keyring {
    fn label_holding(&self, key: &SigningKeyHex) -> Option<&String> {
        self.entries
            .iter()
            .find(|(_, e)| &e.key == key)
            .map(|(label, _)| label)
    }

    /// Checked against by `Consumers::insert`/`IouKeys::insert` so a
    /// label can't exist as both a keyring-derived and an independent
    /// entry in the same namespace at once.
    pub fn is_consumer_label(&self, label: &str) -> bool {
        self.entries.get(label).is_some_and(|e| e.is_consumer)
    }

    pub fn is_iou_label(&self, label: &str) -> bool {
        self.entries.get(label).is_some_and(|e| e.is_iou)
    }

    /// Every `is_consumer`-flagged entry's label + derived vkh -- the
    /// "populated by the keyring" half of `Consumers::effective`.
    pub fn consumers(&self) -> impl Iterator<Item = (&String, Hash28)> {
        self.entries
            .iter()
            .filter(|(_, e)| e.is_consumer)
            .map(|(label, e)| (label, e.key.consumer_hash()))
    }

    /// Every `is_iou`-flagged entry's label + verifying key -- the
    /// "populated by the keyring" half of `IouKeys::effective`.
    pub fn iou_keys(&self) -> impl Iterator<Item = (&String, VerifyingKey)> {
        self.entries
            .iter()
            .filter(|(_, e)| e.is_iou)
            .map(|(label, e)| (label, e.key.verifying_key()))
    }

    /// Stores an existing signing key under `label`, flagged for the
    /// `consumer`/`iou_key` roles given. Rejects a label or a key value
    /// already held here, *and* a flagged role whose derived identity
    /// already exists as an independent `consumers`/`iou_keys` entry --
    /// the full "no duplicate keys and/or labels" invariant, checked
    /// eagerly here rather than left for `Consumers`/`IouKeys` to
    /// discover later.
    pub fn insert(
        &mut self,
        label: String,
        key: SigningKeyHex,
        is_consumer: bool,
        is_iou: bool,
        consumers: &Consumers,
        iou_keys: &IouKeys,
    ) -> Result<(), Error> {
        if self.entries.contains_key(&label) {
            return Err(Error::LabelTaken(label));
        }
        if let Some(existing) = self.label_holding(&key) {
            return Err(Error::KeyTaken(existing.clone()));
        }
        if is_consumer && consumers.contains(&key.consumer_hash()) {
            return Err(Error::KeyTaken(label));
        }
        if is_iou && iou_keys.contains(&key.verifying_key()) {
            return Err(Error::KeyTaken(label));
        }
        self.entries.insert(
            label,
            Entry {
                key,
                is_consumer,
                is_iou,
            },
        );
        Ok(())
    }

    /// Mints a fresh ed25519 keypair from OS randomness (`rand::rng()`,
    /// wasm32-backed by `getrandom`'s `wasm_js` feature -- see
    /// plan-subbit-console.md's open item on this needing a real
    /// build+run smoke test, not just `cargo check`), flagged for the
    /// `consumer`/`iou_key` roles given, and stores it under `label`.
    /// Returns the new key so callers can offer it for Stamp export
    /// immediately. Same cross-container checks as `insert` -- a fresh
    /// key can't collide with another *signing* key by construction,
    /// but its derived identity can still collide with an independently
    /// added `consumers`/`iou_keys` entry.
    pub fn generate(
        &mut self,
        label: String,
        is_consumer: bool,
        is_iou: bool,
        consumers: &Consumers,
        iou_keys: &IouKeys,
    ) -> Result<SigningKeyHex, Error> {
        if self.entries.contains_key(&label) {
            return Err(Error::LabelTaken(label));
        }
        let sk = SigningKey::generate(&mut rand::rng());
        let key = SigningKeyHex(sk.to_bytes());
        if is_consumer && consumers.contains(&key.consumer_hash()) {
            return Err(Error::KeyTaken(label));
        }
        if is_iou && iou_keys.contains(&key.verifying_key()) {
            return Err(Error::KeyTaken(label));
        }
        self.entries.insert(
            label,
            Entry {
                key: key.clone(),
                is_consumer,
                is_iou,
            },
        );
        Ok(key)
    }

    pub fn remove(&mut self, label: &str) -> Option<SigningKeyHex> {
        self.entries.remove(label).map(|e| e.key)
    }

    pub fn get(&self, label: &str) -> Option<&SigningKeyHex> {
        self.entries.get(label).map(|e| &e.key)
    }

    /// The held signing key whose derived `consumer_hash()` matches
    /// `vkh`, if any -- `submit`'s own witness-aggregation step uses
    /// this to sign a required signatory this console already holds
    /// locally, without needing the connected submit wallet's key to
    /// cover it too (see `ui::wasm::submit`'s own doc).
    pub fn signing_key_for_consumer(
        &self,
        vkh: cardano_sdk::Hash<28>,
    ) -> Option<cardano_sdk::SigningKey> {
        let target = Hash28::from(<[u8; 28]>::from(vkh));
        self.entries
            .values()
            .find(|e| e.is_consumer && e.key.consumer_hash() == target)
            .map(|e| cardano_sdk::SigningKey::from(e.key.to_bytes()))
    }

    pub fn labels(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    pub fn iter(&self) -> impl Iterator<Item = (&String, &SigningKeyHex)> {
        self.entries.iter().map(|(label, e)| (label, &e.key))
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duplicate_label_is_rejected() {
        let mut keyring = Keyring::default();
        keyring
            .generate(
                "alice".to_string(),
                false,
                false,
                &Consumers::default(),
                &IouKeys::default(),
            )
            .unwrap();
        assert!(matches!(
            keyring.generate("alice".to_string(), false, false, &Consumers::default(), &IouKeys::default()),
            Err(Error::LabelTaken(label)) if label == "alice"
        ));
    }

    #[test]
    fn duplicate_key_value_is_rejected_under_a_different_label() {
        let mut keyring = Keyring::default();
        let key = keyring
            .generate(
                "alice".to_string(),
                false,
                false,
                &Consumers::default(),
                &IouKeys::default(),
            )
            .unwrap();
        assert!(matches!(
            keyring.insert("bob".to_string(), key, false, false, &Consumers::default(), &IouKeys::default()),
            Err(Error::KeyTaken(label)) if label == "alice"
        ));
    }

    #[test]
    fn flagged_consumer_colliding_with_an_independent_entry_is_rejected() {
        // Mint a key via a throwaway keyring, never actually registered
        // in the real one, just to learn its derived vkh -- then
        // register that vkh independently, simulating a prior import
        // that the real keyring's own insert now collides with.
        let key = Keyring::default()
            .generate(
                "scratch".to_string(),
                false,
                false,
                &Consumers::default(),
                &IouKeys::default(),
            )
            .unwrap();
        let mut consumers = Consumers::default();
        let empty_keyring = Keyring::default();
        consumers
            .insert("existing".to_string(), key.consumer_hash(), &empty_keyring)
            .unwrap();

        let mut keyring = Keyring::default();
        assert!(matches!(
            keyring.insert(
                "alice".to_string(),
                key,
                true,
                false,
                &consumers,
                &IouKeys::default()
            ),
            Err(Error::KeyTaken(label)) if label == "alice"
        ));
    }

    #[test]
    fn consumers_and_iou_keys_are_populated_from_flagged_entries() {
        let mut keyring = Keyring::default();
        let key = keyring
            .generate(
                "alice".to_string(),
                true,
                true,
                &Consumers::default(),
                &IouKeys::default(),
            )
            .unwrap();

        assert_eq!(
            keyring.consumers().collect::<Vec<_>>(),
            vec![(&"alice".to_string(), key.consumer_hash())]
        );
        assert_eq!(
            keyring.iou_keys().collect::<Vec<_>>(),
            vec![(&"alice".to_string(), key.verifying_key())]
        );
    }

    #[test]
    fn unflagged_entry_populates_neither() {
        let mut keyring = Keyring::default();
        keyring
            .generate(
                "alice".to_string(),
                false,
                false,
                &Consumers::default(),
                &IouKeys::default(),
            )
            .unwrap();

        assert_eq!(keyring.consumers().count(), 0);
        assert_eq!(keyring.iou_keys().count(), 0);
    }

    #[test]
    fn signing_key_hex_roundtrips_through_hex() {
        let mut keyring = Keyring::default();
        let key = keyring
            .generate(
                "alice".to_string(),
                false,
                false,
                &Consumers::default(),
                &IouKeys::default(),
            )
            .unwrap();
        let hex = key.to_bytes();
        let parsed = SigningKeyHex::from(hex);
        assert_eq!(parsed, key);
    }
}
