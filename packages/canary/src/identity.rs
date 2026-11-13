//! Deterministic L1 identity derivation from the root seed (`L2_SEED_HEX`).

use cardano_sdk::{Address, NetworkId, address::kind};
use subbit_console::SigningKeyHex;
use subbit_core::{Hash28, Tag};
use subbit_wallet::wallet::embedded::{credential_of, derive_address};

/// One identity's L1 material: public key hash plus the raw seed.
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

    /// The raw 32-byte seed.
    pub fn seed(&self) -> [u8; 32] {
        self.seed
    }

    /// This identity's own plain payment address on `network_id`.
    pub fn address(&self, network_id: NetworkId) -> Address<kind::Any> {
        let verifying_key: [u8; 32] = SigningKeyHex::from(self.seed).verifying_key().to_bytes();
        let credential = credential_of(cardano_sdk::VerificationKey::from(verifying_key));
        derive_address(network_id, &credential, None)
    }
}

/// `Constants.consumer`/`.provider`'s Cardano key hash for `seed`.
pub fn hash28_from_seed(seed: &[u8; 32]) -> Hash28 {
    SigningKeyHex::from(*seed).consumer_hash()
}

/// Derives a labelled 32-byte seed from one root seed.
pub fn derive_seed(base: &[u8; 32], label: &str) -> [u8; 32] {
    *blake3::hash(&[base.as_slice(), label.as_bytes()].concat()).as_bytes()
}

/// The one provider identity every run derives from `l2_seed`.
pub fn provider_seed(l2_seed: &[u8; 32]) -> [u8; 32] {
    derive_seed(l2_seed, "provider")
}

/// Consumer `i`'s identity, derived from `l2_seed`.
pub fn consumer_seed(l2_seed: &[u8; 32], i: usize) -> [u8; 32] {
    derive_seed(l2_seed, &format!("consumer-{i}"))
}

/// A separately-funded wallet for the spawned `subbit-claim` process's own fee/collateral.
pub fn claim_wallet_seed(l2_seed: &[u8; 32]) -> [u8; 32] {
    derive_seed(l2_seed, "claim-fee-wallet")
}

/// Consumer `i`'s on-chain channel tag, namespaced by `l2_seed`.
pub fn tag_for(l2_seed: &[u8; 32], i: usize) -> Tag {
    Tag::from(format!("{}{i}", hex::encode(&l2_seed[..2])).into_bytes())
}
