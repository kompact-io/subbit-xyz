//! Labelled store of `Constants.consumer`-shaped identities (a Cardano
//! vkh, `Hash28`), mirroring `crate::Providers`'s label-based CRUD, but
//! -- unlike `Providers` -- also populated from `crate::Keyring`'s own
//! `is_consumer`-flagged entries (`Keyring::consumers`), always, live:
//! see `effective`. An independently-added entry here has no signing
//! key behind it at all -- a pasted provider vkh, or a connected
//! wallet's own vkh (consumer-only, never usable as an `iou_key` -- see
//! plan-subbit-console.md section E's resolution on why).

use std::collections::BTreeMap;

use subbit_core::Hash28;

use crate::{Error, Keyring};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Consumers {
    entries: BTreeMap<String, Hash28>,
}

impl Consumers {
    /// True iff `vkh` is already held here independently -- doesn't
    /// account for `Keyring`-derived entries; `insert` checks both, see
    /// its own doc.
    pub fn contains(&self, vkh: &Hash28) -> bool {
        self.entries.values().any(|v| v == vkh)
    }

    /// Adds a verifying-only entry. Rejects a label or a vkh already
    /// held either independently here or via a `Keyring` entry flagged
    /// `is_consumer` -- the full "no duplicate keys and/or labels"
    /// invariant, checked eagerly (mirrors `Keyring::insert`'s own
    /// symmetric checks against this container).
    pub fn insert(&mut self, label: String, vkh: Hash28, keyring: &Keyring) -> Result<(), Error> {
        if self.entries.contains_key(&label) || keyring.is_consumer_label(&label) {
            return Err(Error::LabelTaken(label));
        }
        if self.contains(&vkh) || keyring.consumers().any(|(_, v)| v == vkh) {
            return Err(Error::KeyTaken(label));
        }
        self.entries.insert(label, vkh);
        Ok(())
    }

    pub fn remove(&mut self, label: &str) -> Option<Hash28> {
        self.entries.remove(label)
    }

    pub fn labels(&self) -> impl Iterator<Item = &String> {
        self.entries.keys()
    }

    /// Every consumer identity currently known: every `Keyring` entry
    /// flagged `is_consumer` (always included, live -- "always populate
    /// from the keyring"), plus every independent entry here whose
    /// label isn't also a flagged `Keyring` label. That filter should
    /// never actually trigger given `insert` already refuses to create
    /// the collision, but keeps this correct even if the two drift.
    pub fn effective<'a>(
        &'a self,
        keyring: &'a Keyring,
    ) -> impl Iterator<Item = (&'a String, Hash28)> {
        keyring.consumers().chain(
            self.entries
                .iter()
                .filter(|(label, _)| !keyring.is_consumer_label(label))
                .map(|(label, vkh)| (label, *vkh)),
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
                true,
                false,
                false,
                &Consumers::default(),
                &crate::IouKeys::default(),
            )
            .unwrap();
        let consumers = Consumers::default();

        let effective: Vec<_> = consumers.effective(&keyring).collect();
        assert_eq!(effective, vec![(&"alice".to_string(), key.consumer_hash())]);
    }

    #[test]
    fn independent_entry_is_included_when_unrelated_to_any_keyring_label() {
        let keyring = Keyring::default();
        let mut consumers = Consumers::default();
        let vkh = Hash28::from([7u8; 28]);
        consumers.insert("bob".to_string(), vkh, &keyring).unwrap();

        let effective: Vec<_> = consumers.effective(&keyring).collect();
        assert_eq!(effective, vec![(&"bob".to_string(), vkh)]);
    }

    #[test]
    fn duplicate_label_against_a_flagged_keyring_entry_is_rejected() {
        let mut keyring = Keyring::default();
        keyring
            .generate(
                "alice".to_string(),
                true,
                false,
                false,
                &Consumers::default(),
                &crate::IouKeys::default(),
            )
            .unwrap();
        let mut consumers = Consumers::default();

        assert!(matches!(
            consumers.insert("alice".to_string(), Hash28::from([9u8; 28]), &keyring),
            Err(Error::LabelTaken(label)) if label == "alice"
        ));
    }

    #[test]
    fn duplicate_vkh_against_a_flagged_keyring_entry_is_rejected() {
        let mut keyring = Keyring::default();
        let key = keyring
            .generate(
                "alice".to_string(),
                true,
                false,
                false,
                &Consumers::default(),
                &crate::IouKeys::default(),
            )
            .unwrap();
        let mut consumers = Consumers::default();

        assert!(matches!(
            consumers.insert("bob".to_string(), key.consumer_hash(), &keyring),
            Err(Error::KeyTaken(label)) if label == "bob"
        ));
    }

    #[test]
    fn duplicate_label_or_vkh_against_an_independent_entry_is_rejected() {
        let keyring = Keyring::default();
        let mut consumers = Consumers::default();
        let vkh = Hash28::from([7u8; 28]);
        consumers.insert("bob".to_string(), vkh, &keyring).unwrap();

        assert!(matches!(
            consumers.insert("bob".to_string(), Hash28::from([8u8; 28]), &keyring),
            Err(Error::LabelTaken(label)) if label == "bob"
        ));
        assert!(matches!(
            consumers.insert("carol".to_string(), vkh, &keyring),
            Err(Error::KeyTaken(label)) if label == "carol"
        ));
    }
}
