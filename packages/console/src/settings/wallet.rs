//! The embedded wallet's own core logic (`plan-embedded-wallet.md` +
//! this session's console-views resumption): a thin layer over
//! `subbit_wallet::Store` -- generate/import a key, set/clear a
//! delegation credential, scrub, reveal, and a small read-only `View`
//! for the Settings tab to render. No I/O here, same rule as every
//! other core module in this crate: constructing a real
//! `subbit_wallet::Embedded` (which needs a live `floe_client::Backend`)
//! happens in `wasm.rs`, using the key material `Ctx::wallet_*`
//! exposes -- this module only ever touches the `Store` that's already
//! in memory. Lives under `settings/` alongside `floe.rs`/`keys.rs`/
//! `providers.rs`/`whitelist.rs` -- see `settings.rs`'s own doc for
//! why.
//!
//! Deliberately a free-function module over `&Store`/`&mut Store`
//! (mirroring `crate::tx::basket`'s own shape, not a `Keyring`-style
//! type with its own methods) since there's exactly one
//! `subbit_wallet::Store` per `Ctx` -- nothing here needs a label or a
//! collection.

use std::str::FromStr;

use cardano_sdk::{Credential, LeakableSigningKey, NetworkId, SigningKey, address::kind};

use crate::Error;

pub use subbit_wallet::Store;

/// What the Settings tab renders -- whether a key/delegation are
/// configured, never the key material itself (see `reveal` for that,
/// a separate, deliberately-named action).
#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct View {
    pub has_key: bool,
    pub has_delegation: bool,
}

pub fn view(store: &Store) -> View {
    View {
        has_key: !store.is_empty(),
        has_delegation: store.delegation.is_some(),
    }
}

/// Mints a fresh key, stores it, and returns it once -- same "shown
/// once, save it now" contract as `Keyring::generate`. Returns the
/// serializable key itself, not a pre-formatted string: hex-encoding it
/// (or any other representation) is the caller's decision, same as
/// `Keyring::generate` returning `SigningKeyHex` rather than a string.
pub fn generate(store: &mut Store) -> LeakableSigningKey {
    let key = LeakableSigningKey::from(SigningKey::new());
    store.signing_key = Some(key.clone());
    key
}

pub fn import(store: &mut Store, hex: &str) -> Result<(), Error> {
    let key = LeakableSigningKey::from_str(hex).map_err(|e| Error::Key(e.to_string()))?;
    store.signing_key = Some(key);
    Ok(())
}

pub fn set_delegation(store: &mut Store, credential: Credential) {
    store.delegation = Some(credential);
}

pub fn clear_delegation(store: &mut Store) {
    store.delegation = None;
}

pub fn scrub(store: &mut Store) {
    store.scrub();
}

/// The held signing key, for backup -- `None` if no key is configured.
/// Returns the serializable key itself (see `generate`'s own doc on why
/// this isn't a pre-formatted string); `subbit_wallet::reveal_hex` is
/// the sanctioned way to turn it into displayable hex, same as every
/// other caller of a `LeakableSigningKey`. Named distinctly from `view`
/// (which never exposes key material) to keep this one, deliberately
/// sensitive, call site easy to grep for.
pub fn reveal(store: &Store) -> Option<LeakableSigningKey> {
    store.signing_key.clone()
}

/// This wallet's address on `network` -- pure derivation, no `Backend`
/// needed (see `subbit_wallet::derive_address`'s own doc). `None` if no
/// key is configured yet.
pub fn address(store: &Store, network: NetworkId) -> Option<cardano_sdk::Address<kind::Any>> {
    let key = store.signing_key.as_ref()?.clone().into_signing_key();
    let credential = subbit_wallet::credential_of(key.to_verification_key());
    Some(subbit_wallet::derive_address(
        network,
        &credential,
        store.delegation.as_ref(),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn generate_then_import_round_trips_the_same_key() {
        let mut store = Store::default();
        assert!(!view(&store).has_key);

        let key = generate(&mut store);
        assert!(view(&store).has_key);

        let mut other = Store::default();
        import(&mut other, &subbit_wallet::reveal_hex(&key)).unwrap();
        assert_eq!(store.signing_key.is_some(), other.signing_key.is_some());
        assert_eq!(
            reveal(&store).map(|k| subbit_wallet::reveal_hex(&k)),
            reveal(&other).map(|k| subbit_wallet::reveal_hex(&k)),
        );
    }

    #[test]
    fn import_rejects_garbage_hex() {
        let mut store = Store::default();
        assert!(import(&mut store, "not hex").is_err());
        assert!(!view(&store).has_key);
    }

    #[test]
    fn scrub_clears_key_and_delegation() {
        let mut store = Store::default();
        generate(&mut store);
        set_delegation(
            &mut store,
            Credential::from_key(cardano_sdk::Hash::<28>::from([1u8; 28])),
        );
        assert!(view(&store).has_key && view(&store).has_delegation);

        scrub(&mut store);
        let v = view(&store);
        assert!(!v.has_key && !v.has_delegation);
    }

    #[test]
    fn address_is_none_until_a_key_exists() {
        let mut store = Store::default();
        assert!(address(&store, NetworkId::from(cardano_sdk::Network::Preprod)).is_none());
        generate(&mut store);
        assert!(address(&store, NetworkId::from(cardano_sdk::Network::Preprod)).is_some());
    }
}
