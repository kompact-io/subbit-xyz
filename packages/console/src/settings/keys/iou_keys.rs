//! Labelled store of `Constants.iou_key`-shaped identities (a raw
//! ed25519 `VerifyingKey`, used as-is -- not hashed). Mirrors `crate::
//! Consumers` exactly, just typed for the other role a `Keyring` entry
//! can be flagged for; see that module's doc for the shared design
//! ("always populate from the keyring", live, via `effective`).

use std::collections::BTreeMap;

use subbit_core::VerifyingKey;

use crate::{Error, Keyring};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct IouKeys {
    entries: BTreeMap<String, VerifyingKey>,
}

impl IouKeys {
    /// True iff `key` is already held here independently -- doesn't
    /// account for `Keyring`-derived entries; `insert` checks both, see
    /// its own doc.
    pub fn contains(&self, key: &VerifyingKey) -> bool {
        self.entries.values().any(|v| v == key)
    }

    /// Adds a verifying-only entry. Rejects a label or a key already
    /// held either independently here or via a `Keyring` entry flagged
    /// `is_iou` -- the full "no duplicate keys and/or labels" invariant,
    /// checked eagerly (mirrors `Keyring::insert`'s own symmetric checks
    /// against this container).
    pub fn insert(
        &mut self,
        label: String,
        key: VerifyingKey,
        keyring: &Keyring,
    ) -> Result<(), Error> {
        if self.entries.contains_key(&label) || keyring.is_iou_label(&label) {
            return Err(Error::LabelTaken(label));
        }
        if self.contains(&key) || keyring.iou_keys().any(|(_, v)| v == key) {
            return Err(Error::KeyTaken(label));
        }
        self.entries.insert(label, key);
        Ok(())
    }

    pub fn remove(&mut self, label: &str) -> Option<VerifyingKey> {
        self.entries.remove(label)
    }

    pub fn labels(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    /// Every iou_key identity currently known: every `Keyring` entry
    /// flagged `is_iou` (always included, live -- "always populate from
    /// the keyring"), plus every independent entry here whose label
    /// isn't also a flagged `Keyring` label. That filter should never
    /// actually trigger given `insert` already refuses to create the
    /// collision, but keeps this correct even if the two drift.
    pub fn effective<'a>(
        &'a self,
        keyring: &'a Keyring,
    ) -> impl Iterator<Item = (&'a String, VerifyingKey)> {
        keyring.iou_keys().chain(
            self.entries
                .iter()
                .filter(|(label, _)| !keyring.is_iou_label(label))
                .map(|(label, key)| (label, *key)),
        )
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn populated_from_a_flagged_keyring_entry() {
        let mut keyring = Keyring::default();
        let key = keyring
            .generate(
                "alice".to_string(),
                false,
                true,
                &crate::Consumers::default(),
                &IouKeys::default(),
            )
            .unwrap();
        let iou_keys = IouKeys::default();

        let effective: Vec<_> = iou_keys.effective(&keyring).collect();
        assert_eq!(effective, vec![(&"alice".to_string(), key.verifying_key())]);
    }

    #[test]
    fn independent_entry_is_included_when_unrelated_to_any_keyring_label() {
        let keyring = Keyring::default();
        let mut iou_keys = IouKeys::default();
        let vk = VerifyingKey::from([7u8; 32]);
        iou_keys.insert("bob".to_string(), vk, &keyring).unwrap();

        let effective: Vec<_> = iou_keys.effective(&keyring).collect();
        assert_eq!(effective, vec![(&"bob".to_string(), vk)]);
    }

    #[test]
    fn duplicate_label_against_a_flagged_keyring_entry_is_rejected() {
        let mut keyring = Keyring::default();
        keyring
            .generate(
                "alice".to_string(),
                false,
                true,
                &crate::Consumers::default(),
                &IouKeys::default(),
            )
            .unwrap();
        let mut iou_keys = IouKeys::default();

        assert!(matches!(
            iou_keys.insert("alice".to_string(), VerifyingKey::from([9u8; 32]), &keyring),
            Err(Error::LabelTaken(label)) if label == "alice"
        ));
    }

    #[test]
    fn duplicate_key_against_a_flagged_keyring_entry_is_rejected() {
        let mut keyring = Keyring::default();
        let key = keyring
            .generate(
                "alice".to_string(),
                false,
                true,
                &crate::Consumers::default(),
                &IouKeys::default(),
            )
            .unwrap();
        let mut iou_keys = IouKeys::default();

        assert!(matches!(
            iou_keys.insert("bob".to_string(), key.verifying_key(), &keyring),
            Err(Error::KeyTaken(label)) if label == "bob"
        ));
    }

    #[test]
    fn duplicate_label_or_key_against_an_independent_entry_is_rejected() {
        let keyring = Keyring::default();
        let mut iou_keys = IouKeys::default();
        let vk = VerifyingKey::from([7u8; 32]);
        iou_keys.insert("bob".to_string(), vk, &keyring).unwrap();

        assert!(matches!(
            iou_keys.insert("bob".to_string(), VerifyingKey::from([8u8; 32]), &keyring),
            Err(Error::LabelTaken(label)) if label == "bob"
        ));
        assert!(matches!(
            iou_keys.insert("carol".to_string(), vk, &keyring),
            Err(Error::KeyTaken(label)) if label == "carol"
        ));
    }
}
