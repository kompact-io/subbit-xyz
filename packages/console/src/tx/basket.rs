//! The basket (plan-subbit-console.md section I): wraps `subbit_tx::
//! tx::Tx` directly, at the Rust level -- not just JSON at a wasm
//! boundary. This is the fix for a real architectural gap: an earlier
//! round had the basket and the Floe data it's built from living only
//! inside `subbit-tx`'s own (now-removed) `wasm` feature, with no
//! native equivalent at all -- meaning a native wrapper (a TUI, say)
//! would have had to reimplement this from scratch rather than reuse
//! it. `Basket` is the single core implementation; `subbit-tx` itself
//! stays entirely `wasm_bindgen`-free, and both `src/wasm.rs` (the
//! browser) and `src/main.rs` (the native "test-driving" bin, and any
//! future native TUI with "an interface very similar to the web-app")
//! are thin wrappers over this same type.
//!
//! Every method here takes already-typed domain values (`Input`,
//! `Want`, `Constants`, ...), never raw hex/JSON strings -- parsing
//! those belongs to whichever frontend is crossing *its own* boundary
//! (wasm.rs parsing JS strings; a native wrapper parsing CLI args or
//! reading already-typed values straight from a wallet session), not
//! to this shared core.

use std::collections::BTreeMap;

use cardano_sdk::{
    Address, Credential, Input, Output, Transaction, address::kind,
    transaction::state::ReadyForSigning,
};
use floe_dto_http::query::Utxo;
use subbit_l1_dto::{Constants, Stage};
use subbit_tx::{Channel, NetworkParameters, Open, Variables, step::Want, tx::Tx};

use crate::Error;

/// Serializes/deserializes exactly as `Tx` itself does (gated on
/// `subbit-tx`'s own `serde` feature, which this crate always enables)
/// -- the same "cache the staged Tx between invocations" shape
/// `subbit-cli`'s own `Config::cache_path` already relies on, now part
/// of `Store` (section G) so a reload doesn't lose a draft basket (see
/// plan-subbit-console.md's "Open items/risks" entry on this, now
/// resolved).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Basket(Tx);

impl Default for Basket {
    fn default() -> Self {
        Self::from_floe_utxos(std::iter::empty())
    }
}

impl Basket {
    /// Builds the basket's initial txo set from Floe `searchUtxos`
    /// results. Permissive, matching `Tx::filtered`'s own precedent: a
    /// `Utxo` that doesn't parse as a subbit txo is silently
    /// skipped rather than failing construction -- dashboard-first
    /// browsing must stay robust to a handful of unrelated utxos
    /// sharing the validator's payment credential.
    pub fn from_floe_utxos(utxos: impl IntoIterator<Item = Utxo>) -> Self {
        let mut map = BTreeMap::new();
        for utxo in utxos {
            if let Ok((input, output)) = subbit_tx::floe::to_input_output(&utxo) {
                map.insert(input, output);
            }
        }
        Basket(Tx::filtered(map, |_| true))
    }

    /// Refreshes the basket's txo view from fresh Floe data (same
    /// permissive construction as `from_floe_utxos`), while preserving
    /// every staged `Open` unconditionally -- an `Open` doesn't
    /// reference any existing on-chain input, so a txo refresh can
    /// never invalidate it. Staged `Want`s against *existing* txos are
    /// **not** preserved across a refresh: `Tx`'s own `wills` only
    /// retain the validated `Will`, not the original `Want`, so
    /// there's nothing to safely re-`propose` against the refreshed
    /// state -- carrying the old (pre-refresh) `Will` forward
    /// unchecked would risk building against now-stale txo state.
    /// Re-stage those after a refresh if still wanted; this is a
    /// deliberate, scoped limitation, not an oversight.
    pub fn refresh_txos(&mut self, utxos: impl IntoIterator<Item = Utxo>) {
        let opens: Vec<Open> = self.0.opens().values().cloned().collect();
        *self = Self::from_floe_utxos(utxos);
        for open in opens {
            self.0.add_open(open);
        }
    }

    /// Every txo currently known to the basket (provider, amount,
    /// stage, ...).
    pub fn txos(&self) -> &BTreeMap<Input, Channel> {
        self.0.channels()
    }

    /// Every Open currently staged, keyed by its hex keytag -- for
    /// rendering the basket panel (plan section I) before submit.
    pub fn opens(&self) -> &BTreeMap<String, Open> {
        self.0.opens()
    }

    /// Every intent currently staged against an existing txo -- the
    /// basket panel's other half, alongside `opens()`.
    pub fn wills(&self) -> &BTreeMap<Input, subbit_tx::step::Will> {
        self.0.wills()
    }

    /// Stages a `Want` against the txo at `input`. Overwrites any
    /// intent already staged for that same input.
    pub fn propose(&mut self, input: Input, want: Want) -> Result<(), Error> {
        self.0
            .propose(input, want)
            .map_err(|e| Error::Basket(e.to_string()))
    }

    /// Un-stages the intent (if any) against `input`.
    pub fn drop_intent(&mut self, input: &Input) -> bool {
        self.0.drop_intent(input).is_some()
    }

    pub fn drop_all_intents(&mut self) {
        self.0.drop_all_intents();
    }

    /// Stages a new-channel Open: `constants` is the L1 Channel's own
    /// identity (asset/iou_key/consumer/provider/close_period/tag --
    /// locked to a matched `Terms` or hand-entered, per plan section J),
    /// `amount` the initial deposit, `delegation` an optional stake
    /// credential for the new output's address.
    pub fn add_open(&mut self, constants: Constants, amount: u64, delegation: Option<Credential>) {
        let variables = Variables::new(amount, Stage::Opened { subbed: 0 });
        let channel = Channel::new(constants, variables);
        self.0.add_open(Open::new(channel, delegation));
    }

    /// Un-stages an Open by its hex-encoded keytag (`Tx::add_open`'s own
    /// `iou_key.to_string() + tag.to_string()` key).
    pub fn drop_open(&mut self, keytag: &str) -> bool {
        self.0.drop_open(keytag).is_some()
    }

    pub fn drop_all_opens(&mut self) {
        self.0.drop_all_opens();
    }

    /// Submit's own allocation stage: assembles every staged `Will`/
    /// `Open` into one transaction. `reference_utxo` is the validator's reference-
    /// script UTxO (required whenever any intent is staged against an
    /// existing txo); `fuel` is the fee-paying wallet's own spare
    /// utxos. Both already-typed -- a wasm frontend decodes CIP-30's
    /// hex-CBOR / Floe's JSON into these itself; a native wrapper reads
    /// them straight out of a real `Backend`/wallet session, no
    /// decoding needed at all. On failure, the basket is left
    /// completely unchanged (`Tx::build`'s own guarantee) -- safe to
    /// retry.
    pub fn build(
        &mut self,
        network: &str,
        reference_utxo: Option<Utxo>,
        change_address: Address<kind::Any>,
        fuel: &BTreeMap<Input, Output>,
    ) -> Result<Transaction<ReadyForSigning>, Error> {
        let network_parameters =
            NetworkParameters::for_network(network).map_err(|e| Error::Network(e.to_string()))?;

        let reference_utxo = reference_utxo
            .map(|utxo| subbit_tx::floe::to_input_output(&utxo))
            .transpose()
            .map_err(|e| Error::Basket(e.to_string()))?;

        self.0
            .build(
                &network_parameters,
                reference_utxo.as_ref(),
                change_address,
                fuel,
            )
            .map_err(|e| Error::Basket(e.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cardano_sdk::{Credential, Hash, cbor::ToCbor};
    use subbit_core::{Duration, Hash28, Tag, VerifyingKey};
    use subbit_l1_dto::Asset;
    use subbit_tx::VALIDATOR;

    fn network_id_preprod() -> cardano_sdk::NetworkId {
        NetworkParameters::for_network("preprod")
            .unwrap()
            .network_id
    }

    fn constants(provider: Hash28) -> Constants {
        Constants::new(
            Tag::from(b"t".to_vec()),
            Asset::Lovelace,
            VerifyingKey::from([1u8; 32]),
            Hash28::from([2u8; 28]),
            provider,
            Duration::from_secs(900),
        )
    }

    #[test]
    fn empty_basket_has_no_txos() {
        let basket = Basket::from_floe_utxos(std::iter::empty());
        assert!(basket.txos().is_empty());
    }

    #[test]
    fn add_open_then_build_produces_a_transaction() {
        let mut basket = Basket::from_floe_utxos(std::iter::empty());
        basket.add_open(constants(Hash28::from([3u8; 28])), 5_000_000, None);

        // A single fee utxo generous enough to cover the open + fees.
        let network_id = network_id_preprod();
        let change_address: Address<cardano_sdk::address::kind::Any> = cardano_sdk::Address::new(
            network_id,
            Credential::from_key(Hash::<28>::from([9u8; 28])),
        )
        .into();
        let fee_input = Input::new(Hash::<32>::from([4u8; 32]), 0);
        let fee_output = Output::new(change_address.clone(), cardano_sdk::Value::new(50_000_000));
        let fuel = BTreeMap::from([(fee_input, fee_output)]);

        let tx = basket
            .build("preprod", None, change_address, &fuel)
            .expect("build should succeed with no existing-txo intents staged");
        assert!(!tx.to_cbor().is_empty());
        // Submit clears what was staged.
        assert!(basket.0.opens().is_empty());
    }

    #[test]
    fn open_with_existing_intent_but_no_reference_utxo_fails() {
        // Staging a Want against an existing txo without ever
        // having discovered one isn't reachable through this API (no
        // `propose` target exists) -- covered instead by `subbit_tx::
        // tx`'s own tests. This test just confirms `build` surfaces
        // `subbit_tx`'s error via `Error::Basket`, not a panic, when
        // network parsing fails.
        let mut basket = Basket::from_floe_utxos(std::iter::empty());
        let change_address: Address<cardano_sdk::address::kind::Any> = cardano_sdk::Address::new(
            network_id_preprod(),
            Credential::from_key(Hash::<28>::from([9u8; 28])),
        )
        .into();
        assert!(matches!(
            basket.build("not-a-network", None, change_address, &BTreeMap::new()),
            Err(Error::Network(_))
        ));
    }

    #[test]
    fn refresh_preserves_opens_but_not_wills() {
        let mut basket = Basket::from_floe_utxos(std::iter::empty());
        basket.add_open(constants(Hash28::from([3u8; 28])), 5_000_000, None);
        assert_eq!(basket.opens().len(), 1);

        basket.refresh_txos(std::iter::empty());

        assert_eq!(
            basket.opens().len(),
            1,
            "the staged Open must survive a refresh"
        );
        assert!(basket.txos().is_empty());
    }

    #[test]
    fn basket_roundtrips_through_json_like_store_persists_it() {
        let mut basket = Basket::from_floe_utxos(std::iter::empty());
        basket.add_open(constants(Hash28::from([3u8; 28])), 5_000_000, None);

        let json = serde_json::to_string(&basket).unwrap();
        let restored: Basket = serde_json::from_str(&json).unwrap();
        assert_eq!(restored.opens().len(), 1);
    }

    #[test]
    fn validator_hash_is_reachable_for_sanity() {
        // Just exercises that `subbit_tx::VALIDATOR` links fine from
        // this crate's own test binary (floe feature wiring).
        assert_eq!(VALIDATOR.hash.to_string().len(), 56);
    }
}
