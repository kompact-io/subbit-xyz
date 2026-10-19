//! Deterministic L1 identity derivation: given a root seed (`L2_SEED_HEX`),
//! every provider/consumer identity a run needs, plus their on-chain
//! channel tags.

use cardano_sdk::{Hash, SigningKey};
use subbit_core::Hash28;

/// One identity's L1 material, derived once from a seed: the public key
/// hash (`Constants.consumer`/`.provider`, tags, logging) and the
/// signing key `Keyring` needs for the tx-level witness `Tx::signers`
/// requires.
#[derive(Debug, Clone, Copy)]
pub struct Identity {
    pub hash28: Hash28,
    seed: [u8; 32],
}

impl Identity {
    pub fn from_seed(seed: [u8; 32]) -> Self {
        Self {
            hash28: hash28_from_seed(&seed),
            seed,
        }
    }

    pub fn signing_key(&self) -> SigningKey {
        signing_key_from_seed(&self.seed)
    }
}

/// `Constants.consumer`/`.provider`'s real Cardano key hash for `seed`.
/// Must use the same hash algorithm real Cardano credentials use
/// (`Hash::<28>::new(verification_key)`, same as `Credential::from_key`):
/// `Tx::signers` reads these back out as required tx-level signers,
/// checked by the ledger itself (`MissingVKeyWitnessesUTXOW`).
pub fn hash28_from_seed(seed: &[u8; 32]) -> Hash28 {
    let vk = SigningKey::from(*seed).to_verification_key();
    Hash28::from(<[u8; 28]>::from(Hash::<28>::new(vk)))
}

pub fn signing_key_from_seed(seed: &[u8; 32]) -> SigningKey {
    SigningKey::from(*seed)
}

/// Derives a labelled 32-byte seed from one root seed (`base`) -- lets
/// one `L2_SEED_HEX` reproduce every identity a run needs.
pub fn derive_seed(base: &[u8; 32], label: &str) -> [u8; 32] {
    *blake3::hash(&[base.as_slice(), label.as_bytes()].concat()).as_bytes()
}

/// The one provider identity every run derives from `l2_seed`.
pub fn provider_seed(l2_seed: &[u8; 32]) -> [u8; 32] {
    derive_seed(l2_seed, "provider")
}

/// Consumer `i`'s identity, derived from `l2_seed`. Every module that
/// drives or recognizes consumer channels (`scenario`, `tidy`) goes
/// through this one function, so they can never disagree on what
/// "consumer `i`" means under a given `l2_seed`.
pub fn consumer_seed(l2_seed: &[u8; 32], i: usize) -> [u8; 32] {
    derive_seed(l2_seed, &format!("consumer-{i}"))
}

/// Consumer `i`'s on-chain channel tag, namespaced by both `l2_seed` and
/// `namespace` so separate drivers (or separate runs under the same
/// `L2_SEED_HEX`) can't collide on tags -- channels are looked up by tag
/// (`L1::can`).
///
/// Kept short (2 seed bytes, no separator beyond `namespace`): the tag
/// lives in every channel's own datum, so its length eats into how many
/// channels fit in one transaction before the real 16384-byte
/// `maxTxSize`. `hex::encode` only ever produces `0-9a-f`, so a non-hex
/// `namespace` can't collide with the empty one.
pub fn tag_for(l2_seed: &[u8; 32], namespace: &str, i: usize) -> Vec<u8> {
    format!("{namespace}{}{i}", hex::encode(&l2_seed[..2])).into_bytes()
}
