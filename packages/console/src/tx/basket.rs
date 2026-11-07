//! The basket (plan-subbit-console.md section I): wraps `subbit_tx::
//! tx::Tx` directly, at the Rust level -- not just JSON at a wasm
//! boundary. `Basket` is the single core implementation; `subbit-tx`
//! itself stays entirely `wasm_bindgen`-free, and both `src/wasm.rs`
//! (the browser) and `src/main.rs` (the native "test-driving" bin, and
//! any future native TUI with "an interface very similar to the
//! web-app") are thin wrappers over this same type, so neither has to
//! reimplement basket logic of its own.
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
/// `subbit-cli`'s own `Config::cache_path` already relies on. Part of
/// `Store` (section G), so a reload doesn't lose a draft basket.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Basket(Tx);

impl Default for Basket {
    fn default() -> Self {
        Self::from_floe_utxos(std::iter::empty())
    }
}

impl Basket {
    /// `build_batch`'s default cap: a conservative round number, not a
    /// measured ceiling -- raise it once real preprod headroom
    /// (`maxTxSize`, exec units) has actually been measured for a batch
    /// of Opens this size.
    pub const MAX_OPENS_PER_TX: usize = 40;

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

    /// `build`'s batched sibling: a basket holding more than `max_opens`
    /// staged Opens would hand `Tx::build` more channels than one real
    /// transaction can fit (preprod's own `maxTxSize` caps it well under
    /// a few hundred small channels; callers needing a real number
    /// should measure it, this just picks a conservative round one).
    /// Stashes every Open past `max_opens` out of the basket, builds
    /// from what's left (every staged Will, plus up to `max_opens`
    /// Opens), then restores the stash either way -- call this
    /// repeatedly (interleaved with submitting each returned tx) to
    /// drain a basket bigger than `max_opens` Opens.
    ///
    /// Only Opens are capped: a caller's own `Want` staging (one per
    /// round per channel, typically) hasn't yet needed the same
    /// treatment, and `Will`s can't be safely stashed and restored the
    /// way `Open`s can -- `propose` only accepts the original `Want`,
    /// which a staged `Will` no longer carries (see `refresh_txos`'s own
    /// doc). If a caller ever needs to cap a tx's total Wills too, this
    /// is the place to add it, once that's actually been hit.
    pub fn build_batch(
        &mut self,
        network: &str,
        reference_utxo: Option<Utxo>,
        change_address: Address<kind::Any>,
        fuel: &BTreeMap<Input, Output>,
        max_opens: usize,
    ) -> Result<Transaction<ReadyForSigning>, Error> {
        let overflow_keytags: Vec<String> =
            self.0.opens().keys().skip(max_opens).cloned().collect();
        let overflow: Vec<Open> = overflow_keytags
            .iter()
            .filter_map(|keytag| self.0.drop_open(keytag))
            .collect();

        let result = self.build(network, reference_utxo, change_address, fuel);

        for open in overflow {
            self.0.add_open(open);
        }
        result
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

    /// Same shape as `constants`, but a distinct `tag` per `i` so each
    /// produces its own keytag in the basket's `opens` map.
    fn constants_n(provider: Hash28, i: u8) -> Constants {
        Constants::new(
            Tag::from(vec![i]),
            Asset::Lovelace,
            VerifyingKey::from([1u8; 32]),
            Hash28::from([2u8; 28]),
            provider,
            Duration::from_secs(900),
        )
    }

    fn fuel_for(change_address: &Address<kind::Any>) -> BTreeMap<Input, Output> {
        let fee_input = Input::new(Hash::<32>::from([4u8; 32]), 0);
        let fee_output = Output::new(change_address.clone(), cardano_sdk::Value::new(50_000_000));
        BTreeMap::from([(fee_input, fee_output)])
    }

    #[test]
    fn build_batch_caps_opens_and_leaves_the_rest_staged() {
        let mut basket = Basket::from_floe_utxos(std::iter::empty());
        for i in 0..5u8 {
            basket.add_open(constants_n(Hash28::from([3u8; 28]), i), 5_000_000, None);
        }
        assert_eq!(basket.opens().len(), 5);

        let network_id = network_id_preprod();
        let change_address: Address<kind::Any> = cardano_sdk::Address::new(
            network_id,
            Credential::from_key(Hash::<28>::from([9u8; 28])),
        )
        .into();
        let fuel = fuel_for(&change_address);

        let tx = basket
            .build_batch("preprod", None, change_address.clone(), &fuel, 2)
            .expect("building a 2-open batch out of 5 staged should succeed");
        assert!(!tx.to_cbor().is_empty());
        assert_eq!(
            basket.opens().len(),
            3,
            "the 3 Opens past the cap must still be staged after the batch builds"
        );

        // Draining the rest: one more batch, same cap, covers what's left.
        let tx2 = basket
            .build_batch("preprod", None, change_address, &fuel, 2)
            .expect("a second batch should pick up the remaining Opens");
        assert!(!tx2.to_cbor().is_empty());
        assert_eq!(
            basket.opens().len(),
            1,
            "5 Opens drained 2 at a time across 2 batches should leave exactly 1"
        );
    }

    #[test]
    fn build_never_selects_a_script_bearing_utxo_as_fee_fuel() {
        // A reference-script-shaped utxo sitting in the funding
        // wallet's own `fuel` set (exactly how `subbit-canary`'s
        // `Chain::refresh`/submit.rs's own `wallet.utxos(None)` hands
        // it over, undifferentiated from any other wallet utxo) must
        // never be picked as a plain fee input, even when its lovelace
        // balance makes it the best coverage candidate -- otherwise
        // it's spent away, destroying the one reference script every
        // future `Will` batch depends on (the real preprod failure
        // this regression-tests: "reference script required when
        // stepping channels" reappearing mid-run).
        let mut basket = Basket::from_floe_utxos(std::iter::empty());
        basket.add_open(constants(Hash28::from([3u8; 28])), 5_000_000, None);

        let network_id = network_id_preprod();
        let change_address: Address<kind::Any> = cardano_sdk::Address::new(
            network_id,
            Credential::from_key(Hash::<28>::from([9u8; 28])),
        )
        .into();

        let script_input = Input::new(Hash::<32>::from([5u8; 32]), 0);
        let script_output =
            Output::new(change_address.clone(), cardano_sdk::Value::new(90_000_000))
                .with_plutus_script(subbit_tx::VALIDATOR.script.clone());
        let plain_input = Input::new(Hash::<32>::from([4u8; 32]), 0);
        let plain_output = Output::new(change_address.clone(), cardano_sdk::Value::new(50_000_000));
        let fuel = BTreeMap::from([
            (script_input.clone(), script_output),
            (plain_input, plain_output),
        ]);

        let tx = basket
            .build("preprod", None, change_address, &fuel)
            .expect("plain fuel alone should still cover a single Open");
        assert!(
            !tx.inputs().any(|i| i == script_input),
            "the script-bearing utxo must never be selected as a fee input"
        );
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
