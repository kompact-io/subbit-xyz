//! Whitelisted validator script hashes (plan-subbit-console.md section
//! J/K): which on-chain scripts `Ctx` is willing to treat as `subbit`
//! channels at all, before any `Constants`-level checks apply. Used to
//! scope every Floe discovery call (`settings::floe::discover_history`)
//! -- a validator not on this list is never queried, let alone shown.
//! Its own type, same reason `Providers`/`Keyring` already have one --
//! `Ctx` should hold named, independently testable pieces, not raw
//! collections.

use std::collections::BTreeSet;

use cardano_sdk::Hash;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Whitelist {
    hashes: BTreeSet<Hash<28>>,
}

impl Whitelist {
    pub fn add(&mut self, script_hash: Hash<28>) {
        self.hashes.insert(script_hash);
    }

    pub fn remove(&mut self, script_hash: &Hash<28>) -> bool {
        self.hashes.remove(script_hash)
    }

    pub fn hashes(&self) -> &BTreeSet<Hash<28>> {
        &self.hashes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_remove_roundtrip() {
        let mut whitelist = Whitelist::default();
        let hash = Hash::<28>::from([7u8; 28]);
        assert!(whitelist.hashes().is_empty());

        whitelist.add(hash);
        assert!(whitelist.hashes().contains(&hash));

        assert!(whitelist.remove(&hash));
        assert!(!whitelist.remove(&hash));
        assert!(whitelist.hashes().is_empty());
    }
}
