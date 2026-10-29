//! Persisted wallet material: at most one signing key, at most one
//! delegation credential (single-address wallet, single optional
//! delegation -- see `plan-embedded-wallet.md`'s scope note). This is
//! the *only* thing `Store` holds, which is what makes "export" and
//! "view" trivial below: there's no larger app blob to carve a subset
//! out of, unlike e.g. `subbit-console::Store` (which holds a whole
//! app's worth of data and has to pick this kind of thing back out of
//! a bigger Export). Serializing this `Store` IS the export.
//!
//! `cardano_sdk::LeakableSigningKey` (not a custom hex wrapper) is the
//! exact, already-existing type for this: a `SigningKey` that opts
//! into hex-string `serde` deliberately, named to make the risk
//! explicit at every use site, not a type this crate invents.

use std::path::Path;

use cardano_sdk::{Credential, LeakableSigningKey};

/// `key`'s raw hex seed, for display/backup. Routes through
/// `LeakableSigningKey`'s own serde impl (which does the actual
/// `unsafe` leak internally, already named/audited for exactly this)
/// rather than any caller duplicating that `unsafe` block themselves.
/// A free function, not a `Store` method: it operates on a key a
/// caller already has in hand (e.g. straight out of `Store::
/// signing_key`, or one `Embedded::new` hasn't been given to yet),
/// not on `Store` itself.
pub fn reveal_hex(key: &LeakableSigningKey) -> String {
    match serde_json::to_value(key) {
        Ok(serde_json::Value::String(hex)) => hex,
        _ => unreachable!("LeakableSigningKey always serializes to a JSON string"),
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("toml decode error: {0}")]
    TomlDecode(#[from] toml::de::Error),
    #[error("toml encode error: {0}")]
    TomlEncode(#[from] toml::ser::Error),
    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Store {
    pub signing_key: Option<LeakableSigningKey>,
    pub delegation: Option<Credential>,
}

impl Store {
    pub fn is_empty(&self) -> bool {
        self.signing_key.is_none()
    }

    /// Clears both fields. Named distinctly from a plain `clear`/
    /// `Default::default()` to make the intent ("the user asked for
    /// this key to be gone") explicit at every call site -- see
    /// `plan-embedded-wallet.md`'s scrubbable requirement. Does not
    /// (and, short of a `zeroize`-style dependency this crate
    /// deliberately doesn't take on to keep scope tight, cannot fully)
    /// guarantee the dropped key's bytes are wiped from memory --
    /// dropping `self.signing_key`/`self.delegation` is the extent of
    /// what this method does; persisting the result (`save`/`to_json`)
    /// is the caller's job, same as every other mutation here.
    pub fn scrub(&mut self) {
        self.signing_key = None;
        self.delegation = None;
    }

    // --- wasm / browser: plain JSON string, caller owns localStorage ---

    pub fn to_json(&self) -> Result<String, Error> {
        Ok(serde_json::to_string(self)?)
    }

    pub fn from_json(json: &str) -> Result<Self, Error> {
        Ok(serde_json::from_str(json)?)
    }

    // --- native: a file, same shape as every other Store in this workspace ---

    pub fn load(path: &Path) -> Result<Self, Error> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        Ok(toml::from_str(&text)?)
    }

    pub fn save(&self, path: &Path) -> Result<(), Error> {
        let text = toml::to_string_pretty(self)?;
        std::fs::write(path, text)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_round_trip() {
        let mut store = Store::default();
        assert!(store.is_empty());
        store.signing_key = Some(LeakableSigningKey::from(cardano_sdk::SigningKey::from(
            [7u8; 32],
        )));
        store.delegation = Some(Credential::from_key(cardano_sdk::Hash::<28>::from(
            [9u8; 28],
        )));
        assert!(!store.is_empty());

        let json = store.to_json().unwrap();
        let restored = Store::from_json(&json).unwrap();
        assert!(!restored.is_empty());
        assert!(restored.delegation.is_some());
    }

    #[test]
    fn toml_round_trip() {
        let dir = tempdir();
        let path = dir.join("subbit-wallet.toml");

        let store = Store {
            signing_key: Some(LeakableSigningKey::from(cardano_sdk::SigningKey::from(
                [1u8; 32],
            ))),
            delegation: None,
        };
        store.save(&path).unwrap();

        let restored = Store::load(&path).unwrap();
        assert!(!restored.is_empty());

        std::fs::remove_file(&path).ok();
    }

    #[test]
    fn scrub_clears_both_fields() {
        let mut store = Store {
            signing_key: Some(LeakableSigningKey::from(cardano_sdk::SigningKey::from(
                [2u8; 32],
            ))),
            delegation: Some(Credential::from_key(cardano_sdk::Hash::<28>::from(
                [3u8; 28],
            ))),
        };
        store.scrub();
        assert!(store.is_empty());
        assert!(store.delegation.is_none());
    }

    #[test]
    fn load_missing_file_is_default() {
        let dir = tempdir();
        let path = dir.join("does-not-exist.toml");
        let store = Store::load(&path).unwrap();
        assert!(store.is_empty());
    }

    fn tempdir() -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("subbit-wallet-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }
}
