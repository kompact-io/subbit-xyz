use cardano_sdk::{
    Credential, Hash, NetworkId, SigningKey, VerificationKey, address::kind::Shelley,
};
use serde::{Deserialize, Serialize};

/// On-disk wallet config: just the raw signing key material. `Wallet`
/// derives everything else from this once, at construction, instead of
/// re-deriving on every call - see `Wallet` below.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    #[serde(with = "hex::serde")]
    key: [u8; 32],
}

impl Default for Config {
    fn default() -> Self {
        // Deterministic, dev-only default so a fresh checkout has a
        // working key without any setup. It's derived from a fixed seed,
        // so it's exactly as public as a hardcoded zero key would be -
        // never use it for anything holding real value.
        Self {
            key: hash32(b"wallet"),
        }
    }
}

/// The wallet's signing key and everything derived from it, computed once
/// at construction. `Session` holds this, not `Config` - `Config` is just
/// the serializable seed.
pub struct Wallet {
    signing_key: SigningKey,
    verification_key: VerificationKey,
    credential: Credential,
}

impl Wallet {
    pub fn new(config: Config) -> Self {
        let signing_key = SigningKey::from(config.key);
        let verification_key = signing_key.to_verification_key();
        // ASSUMPTION: `VerificationKey` is `Clone`. It's public key
        // material, so this should be uncontroversial - unlike
        // `SigningKey`, which cardano_sdk deliberately makes awkward to
        // copy/print/serialize. `cardano_signing_key` below respects that
        // by returning a reference rather than assuming `SigningKey: Clone`.
        let credential = Credential::from_key(Hash::<28>::new(verification_key));
        Self {
            signing_key,
            verification_key,
            credential,
        }
    }

    pub fn cardano_signing_key(&self) -> &SigningKey {
        &self.signing_key
    }

    pub fn verification_key(&self) -> VerificationKey {
        self.verification_key
    }

    pub fn credential(&self) -> Credential {
        self.credential.clone()
    }

    pub fn address(&self, network_id: NetworkId) -> cardano_sdk::Address<Shelley> {
        cardano_sdk::Address::new(network_id, self.credential())
    }
}

impl From<Config> for Wallet {
    fn from(config: Config) -> Self {
        Self::new(config)
    }
}

// Deliberately no `#[derive(Debug)]` on `Wallet` - the default would
// print `signing_key` verbatim. This prints only what's already public.
impl std::fmt::Debug for Wallet {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Wallet")
            .field("credential", &self.credential)
            .finish_non_exhaustive()
    }
}

/// Deterministic, non-cryptographic 32-byte expansion of `seed`. Only
/// good for the insecure dev default above - not a real KDF.
fn hash32(seed: &[u8]) -> [u8; 32] {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash as _, Hasher};

    let mut out = [0u8; 32];
    for (i, chunk) in out.chunks_mut(8).enumerate() {
        let mut hasher = DefaultHasher::new();
        seed.hash(&mut hasher);
        i.hash(&mut hasher);
        chunk.copy_from_slice(&hasher.finish().to_le_bytes());
    }
    out
}
