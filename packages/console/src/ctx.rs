//! `Ctx`: the full application model every UI (wasm, this crate's own
//! CLI, and whatever comes next) sits on. No separate `Store` type --
//! its fields live directly here, and `Ctx` derives `Serialize`/
//! `Deserialize` itself, nothing more: `Ctx` has no target-gated code
//! and no persistence methods of its own. *Where* to persist (a file,
//! `localStorage`, anything else) is a UI-layer decision, same as
//! *how* to format a read (JSON, or not) already was -- see
//! `ui::cli`/`ui::wasm`, which each pick their own storage and call
//! plain `serde_json` on `Ctx` directly. Every method here takes/
//! returns already-typed domain values, never raw hex/JSON strings:
//! parsing those belongs to whichever frontend is crossing its own
//! boundary (mirrors `Basket`'s own rule, applied crate-wide). A UI's
//! own marshalling layer (`ui::wasm`, `ui::cli`)
//! should be boilerplate: parse args, call a `Ctx` method, (de)
//! serialize the result -- never reach into `Ctx`'s fields directly
//! from outside this module.

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use cardano_sdk::{
    Address, Credential, Hash, Input, NetworkId, Output, Transaction, address::kind,
    transaction::state::ReadyForSigning,
};
use floe_dto_http::query::Utxo;
use subbit_core::{Duration, Hash28, Tag, VerifyingKey};
use subbit_l1_dto::{Asset, Constants};
use subbit_tx::{
    Channel, Open,
    step::{Want, Will},
};

use crate::channels::{Match, Status};
use crate::{
    Basket, Consumers, Error, IouKeys, Keyring, ProviderEntry, Providers, SigningKeyHex,
    WalletView, Whitelist,
};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Ctx {
    keyring: Keyring,
    consumers: Consumers,
    iou_keys: IouKeys,
    providers: Providers,
    basket: Basket,
    /// Gates `basket_build`/`basket_build_batch`: neither will run at
    /// all until this is set (see `set_reference_utxo`'s own doc) --
    /// `Basket`/`Tx` themselves are unchanged, still perfectly willing
    /// to build an Opens-only tx with no reference script; this is a
    /// stricter policy enforced one layer up, here, not a change to
    /// what the chain itself requires. `#[serde(default)]` so a `Ctx`
    /// persisted before this field existed still deserializes (as
    /// `None` -- building is then forbidden until it's set, same as
    /// if it had never existed).
    #[serde(default)]
    reference_utxo: Option<Utxo>,
    #[serde(default)]
    whitelist: Whitelist,
    /// The embedded wallet's own state (`plan-embedded-wallet.md`): at
    /// most one signing key, at most one delegation credential. This
    /// is `subbit_wallet::Store` -- a whole separate crate's `Store`,
    /// not console's `Keyring` -- embedded as a field, not merged into
    /// it. Stays independently (de)serializable too (see
    /// `settings::wallet`'s own doc), so exporting *just* the wallet
    /// piece never needs `Ctx::export`'s own whole-app path.
    #[serde(default)]
    wallet: subbit_wallet::Store,
}

impl Ctx {
    // --- Keyring -----------------------------------------------------

    pub fn keyring_add(
        &mut self,
        label: String,
        key: SigningKeyHex,
        is_consumer: bool,
        is_iou: bool,
    ) -> Result<(), Error> {
        self.keyring.insert(
            label,
            key,
            is_consumer,
            is_iou,
            &self.consumers,
            &self.iou_keys,
        )
    }

    pub fn keyring_generate(
        &mut self,
        label: String,
        is_consumer: bool,
        is_iou: bool,
    ) -> Result<SigningKeyHex, Error> {
        self.keyring
            .generate(label, is_consumer, is_iou, &self.consumers, &self.iou_keys)
    }

    pub fn keyring_remove(&mut self, label: &str) -> bool {
        self.keyring.remove(label).is_some()
    }

    pub fn keyring_labels(&self) -> Vec<&String> {
        self.keyring.labels().collect()
    }

    /// The held signing key whose derived consumer vkh matches `vkh`,
    /// if any -- `ui::wasm::submit`'s own witness-aggregation step.
    pub fn keyring_signing_key_for_consumer(
        &self,
        vkh: cardano_sdk::Hash<28>,
    ) -> Option<cardano_sdk::SigningKey> {
        self.keyring.signing_key_for_consumer(vkh)
    }

    // --- Consumers -----------------------------------------------------

    pub fn consumers_add(&mut self, label: String, vkh: Hash28) -> Result<(), Error> {
        self.consumers.insert(label, vkh, &self.keyring)
    }

    pub fn consumers_remove(&mut self, label: &str) -> bool {
        self.consumers.remove(label).is_some()
    }

    pub fn consumers_effective(&self) -> Vec<(&String, Hash28)> {
        self.consumers.effective(&self.keyring).collect()
    }

    // --- IouKeys ---------------------------------------------------------

    pub fn iou_add(&mut self, label: String, key: VerifyingKey) -> Result<(), Error> {
        self.iou_keys.insert(label, key, &self.keyring)
    }

    pub fn iou_remove(&mut self, label: &str) -> bool {
        self.iou_keys.remove(label).is_some()
    }

    pub fn iou_effective(&self) -> Vec<(&String, VerifyingKey)> {
        self.iou_keys.effective(&self.keyring).collect()
    }

    // --- Providers ---------------------------------------------------

    /// Fails on a label already in use -- the native CLI's own "add"
    /// semantics.
    pub fn providers_insert(&mut self, label: String, entry: ProviderEntry) -> Result<(), Error> {
        self.providers.insert(label, entry)
    }

    /// Upserts -- a re-fetch or manual edit should always win (plan
    /// section F).
    pub fn providers_set(&mut self, label: String, entry: ProviderEntry) {
        self.providers.set(label, entry);
    }

    pub fn providers_remove(&mut self, label: &str) -> bool {
        self.providers.remove(label).is_some()
    }

    pub fn providers_list(&self) -> Vec<(&String, &ProviderEntry)> {
        self.providers.iter().collect()
    }

    pub fn providers_by_provider(&self, provider: &Hash28) -> Vec<(&String, &ProviderEntry)> {
        self.providers.by_provider(provider)
    }

    /// Looks up every provider entry sharing `constants.provider()`
    /// and reports, per entry, exactly which fields diverge from what
    /// its `Terms` declares. See `crate::channels::reconcile`.
    pub fn reconcile(&self, constants: &Constants) -> Vec<Match<'_>> {
        let matches = self.providers.by_provider(constants.provider());
        crate::reconcile(constants, matches)
    }

    // --- Whitelisted validators (plan section J/K) ----------------------

    pub fn whitelist_add(&mut self, script_hash: Hash<28>) {
        self.whitelist.add(script_hash);
    }

    pub fn whitelist_remove(&mut self, script_hash: &Hash<28>) -> bool {
        self.whitelist.remove(script_hash)
    }

    pub fn whitelist(&self) -> &BTreeSet<Hash<28>> {
        self.whitelist.hashes()
    }

    // --- Basket (plan section I) ----------------------------------------

    pub fn basket_refresh_txos(&mut self, utxos: impl IntoIterator<Item = Utxo>) {
        self.basket.refresh_txos(utxos);
    }

    pub fn basket_txos(&self) -> &BTreeMap<Input, Channel> {
        self.basket.txos()
    }

    /// Each `basket_txos` entry's `Status` -- "mine?" and available
    /// actions already decided, for the dashboard to render directly.
    pub fn dashboard(&self) -> Vec<Status> {
        crate::statuses(
            self.basket.txos(),
            &self.consumers,
            &self.iou_keys,
            &self.keyring,
        )
    }

    /// The full-history view: reconstructs lineage from `items`/
    /// `ledger_tip` (a Floe history search's own shape -- same two
    /// pieces `settings::floe::discover_history` already returns) and
    /// groups it by Constants/line, index-dto's own organization (see
    /// `channels::lineage`'s own doc on why that's kept as-is, not
    /// re-grouped by keytag). Each distinct `Constants`' confirmation
    /// depth comes from the matching `Providers` entry when one
    /// exists, else `default_confirmation_depth`; its `brief` (label/
    /// Terms-match widget, including `mine`/`can_sign`) comes from
    /// `channels::brief`. Each line's own tip is additionally joined
    /// against `self.basket` (the live txo cache `basket_refresh_txos`
    /// fills) to attach its `can` -- the one place this decision is
    /// made, so a frontend renders `LineView`/`Brief` directly instead
    /// of re-joining a separate dashboard fetch by txo itself.
    ///
    /// `active_signer`: the frontend's own currently-connected submit
    /// backend's vkh, if any -- a session fact this `Ctx` has no way
    /// to derive for a live CIP-30 connection (the signing key never
    /// crosses into here), so the caller (`wasm.rs`'s `JsConsole`)
    /// hands it in rather than this method reaching for it itself.
    /// Feeds `Brief::can_sign`, nothing else.
    pub fn channels_lineage(
        &self,
        items: Vec<Utxo>,
        ledger_tip: floe_dto_http::query::ChainPoint,
        default_confirmation_depth: u32,
        active_signer: Option<Hash28>,
    ) -> crate::LineageView {
        let output = subbit_index::lineage::reconstruct(items, ledger_tip);
        crate::channels::lineage::group(
            output,
            |constants| {
                self.providers
                    .by_provider(constants.provider())
                    .into_iter()
                    .find_map(|(_, entry)| entry.terms.as_ref().map(|t| t.confirmation_depth))
                    .unwrap_or(default_confirmation_depth)
            },
            |constants| {
                crate::channels::brief::brief(
                    constants,
                    &self.consumers,
                    &self.iou_keys,
                    &self.keyring,
                    &self.providers,
                    active_signer,
                )
            },
            self.basket.txos(),
        )
    }

    pub fn basket_opens(&self) -> &BTreeMap<String, Open> {
        self.basket.opens()
    }

    pub fn basket_wills(&self) -> &BTreeMap<Input, Will> {
        self.basket.wills()
    }

    pub fn basket_propose(&mut self, input: Input, want: Want) -> Result<(), Error> {
        self.basket.propose(input, want)
    }

    pub fn basket_drop_intent(&mut self, input: &Input) -> bool {
        self.basket.drop_intent(input)
    }

    pub fn basket_drop_all_intents(&mut self) {
        self.basket.drop_all_intents();
    }

    #[allow(clippy::too_many_arguments)]
    pub fn basket_add_open(
        &mut self,
        tag: Tag,
        asset: Asset,
        iou_key: VerifyingKey,
        consumer: Hash28,
        provider: Hash28,
        close_period: Duration,
        delegation: Option<Credential>,
        amount: u64,
    ) {
        let constants = Constants::new(tag, asset, iou_key, consumer, provider, close_period);
        self.basket.add_open(constants, amount, delegation);
    }

    pub fn basket_drop_open(&mut self, keytag: &str) -> bool {
        self.basket.drop_open(keytag)
    }

    pub fn basket_drop_all_opens(&mut self) {
        self.basket.drop_all_opens();
    }

    /// Verified once, here, on set: `utxo` must carry exactly the
    /// subbit validator's own script. Not re-verified on every build
    /// after that -- if it's since been spent elsewhere, `basket_build`/
    /// `basket_build_batch` will fail downstream (at actual submission)
    /// rather than here. Consumers should be invited to pick one a
    /// provider already publishes (`Terms::reference_script`) over
    /// uploading their own, though either works equally well once set.
    pub fn basket_set_reference_utxo(&mut self, utxo: Utxo) -> Result<(), Error> {
        let (_, output) =
            subbit_tx::floe::to_input_output(&utxo).map_err(|e| Error::Basket(e.to_string()))?;
        if output.script() != Some(&subbit_tx::VALIDATOR.script) {
            return Err(Error::Basket(
                "not the subbit validator's own reference script".to_string(),
            ));
        }
        self.reference_utxo = Some(utxo);
        Ok(())
    }

    pub fn basket_reference_utxo(&self) -> Option<&Utxo> {
        self.reference_utxo.as_ref()
    }

    pub fn basket_clear_reference_utxo(&mut self) {
        self.reference_utxo = None;
    }

    /// Resolves a provider's published `Terms.reference_script`
    /// against already-fetched `items` (e.g. `discover_history`'s own
    /// output -- no separate Floe round trip needed, same "one fetch,
    /// filter it" convention `settings::floe`'s own doc already
    /// describes) and sets it as the basket's reference script
    /// (`basket_set_reference_utxo`, verified there). The one piece of
    /// "adopt a provider's own reference script" logic every UI (cli,
    /// wasm, canary) shares, instead of each re-finding it itself.
    pub fn basket_set_reference_utxo_from_terms(
        &mut self,
        terms: &subbit_l2_dto::Terms,
        items: &[Utxo],
    ) -> Result<(), Error> {
        let output_ref = terms
            .reference_script
            .ok_or_else(|| Error::Basket("provider publishes no reference_script".to_string()))?;
        let hash = output_ref.tx_hash.to_bytes();
        let utxo = items
            .iter()
            .find(|u| u.txo_ref.hash == hash && u32::from(u.txo_ref.index) == output_ref.index)
            .cloned()
            .ok_or_else(|| {
                Error::Basket(
                    "provider's published reference_script wasn't found among the given utxos"
                        .to_string(),
                )
            })?;
        self.basket_set_reference_utxo(utxo)
    }

    pub fn basket_build(
        &mut self,
        network: &str,
        change_address: Address<kind::Any>,
        fuel: &BTreeMap<Input, Output>,
    ) -> Result<Transaction<ReadyForSigning>, Error> {
        let reference_utxo = self.reference_utxo.clone().ok_or_else(|| {
            Error::Basket(
                "no reference script set -- call basket_set_reference_utxo first".to_string(),
            )
        })?;
        self.basket
            .build(network, Some(reference_utxo), change_address, fuel)
    }

    /// `basket_build`'s batched sibling -- see `Basket::build_batch`.
    pub fn basket_build_batch(
        &mut self,
        network: &str,
        change_address: Address<kind::Any>,
        fuel: &BTreeMap<Input, Output>,
        max_opens: usize,
    ) -> Result<Transaction<ReadyForSigning>, Error> {
        let reference_utxo = self.reference_utxo.clone().ok_or_else(|| {
            Error::Basket(
                "no reference script set -- call basket_set_reference_utxo first".to_string(),
            )
        })?;
        self.basket.build_batch(
            network,
            Some(reference_utxo),
            change_address,
            fuel,
            max_opens,
        )
    }

    // --- Embedded wallet (plan-embedded-wallet.md) -----------------------
    //
    // No `Embedded`/`Backend` here -- constructing a real embedded
    // wallet needs a live Floe connection, which `Ctx` never holds (same
    // I/O-free rule as everywhere else in this module). `wasm.rs` builds
    // the actual `subbit_wallet::Embedded` at submit time, using
    // `wallet_signing_key`/`wallet_delegation` below.

    pub fn wallet_view(&self) -> WalletView {
        crate::settings::wallet::view(&self.wallet)
    }

    /// Mints a fresh key, stores it, and returns it once -- the caller
    /// must offer this for backup immediately, since it is never
    /// retrievable again except via `wallet_reveal` (which is this
    /// exact value, just re-derivable later, not a one-time secret --
    /// unlike `Keyring::generate`'s own contract, a wallet has only one
    /// key, so there's nothing wrong with being able to look at it
    /// again). Returns the serializable key itself, not a pre-formatted
    /// string -- same reasoning as `keyring_generate` returning
    /// `SigningKeyHex` rather than a string: encoding is the caller's
    /// decision (`wasm.rs`'s `walletGenerate` picks hex, via
    /// `subbit_wallet::reveal_hex`).
    pub fn wallet_generate(&mut self) -> cardano_sdk::LeakableSigningKey {
        crate::settings::wallet::generate(&mut self.wallet)
    }

    pub fn wallet_import(&mut self, hex: &str) -> Result<(), Error> {
        crate::settings::wallet::import(&mut self.wallet, hex)
    }

    pub fn wallet_set_delegation(&mut self, credential: Credential) {
        crate::settings::wallet::set_delegation(&mut self.wallet, credential);
    }

    pub fn wallet_clear_delegation(&mut self) {
        crate::settings::wallet::clear_delegation(&mut self.wallet);
    }

    /// Clears both the key and the delegation. See `subbit_wallet::
    /// Store::scrub`'s own doc for exactly what this does and doesn't
    /// guarantee.
    pub fn wallet_scrub(&mut self) {
        crate::settings::wallet::scrub(&mut self.wallet);
    }

    /// The held signing key, for backup -- `None` if no key is
    /// configured. Same "return the serializable value, not a string"
    /// reasoning as `wallet_generate`.
    pub fn wallet_reveal(&self) -> Option<cardano_sdk::LeakableSigningKey> {
        crate::settings::wallet::reveal(&self.wallet)
    }

    /// This wallet's address on `network` -- `None` if no key is
    /// configured yet.
    pub fn wallet_address(&self, network: NetworkId) -> Option<Address<kind::Any>> {
        crate::settings::wallet::address(&self.wallet, network)
    }

    /// The signing key `wasm.rs`'s submit path hands to
    /// `subbit_wallet::Embedded::new` -- `None` if no key is configured.
    /// Exposed as the raw key (not just a hex string) since the caller
    /// is Rust, not JS crossing a wasm boundary.
    pub fn wallet_signing_key(&self) -> Option<cardano_sdk::SigningKey> {
        self.wallet
            .signing_key
            .clone()
            .map(|key| key.into_signing_key())
    }

    pub fn wallet_delegation(&self) -> Option<Credential> {
        self.wallet.delegation.clone()
    }

    /// This embedded wallet's own vkh, if a key is configured --
    /// `channels::brief`'s own `active_signer` input when `wasm.rs`'s
    /// `WalletHandle::Embedded` is the currently connected backend.
    /// Unlike a connected CIP-30 session's vkh (the signing key never
    /// leaves the extension, so this console has to be *told* it),
    /// this one `Ctx` can always derive itself, straight from the key
    /// material it already holds -- no connection needed to know it.
    pub fn wallet_vkh(&self) -> Option<Hash28> {
        let key = self.wallet_signing_key()?;
        let hash = Hash::<28>::new(key.to_verification_key());
        Some(Hash28::from(<[u8; 28]>::from(hash)))
    }

    /// The embedded wallet's own data (just this key + delegation) -- a
    /// serializable value, not a pre-formatted string: encoding it as
    /// JSON (or anything else) is the caller's decision, not `Ctx`'s.
    /// Separate from `export`'s whole-app blob, since
    /// `subbit_wallet::Store` is independently serializable by design
    /// (see `settings::wallet`'s own doc).
    pub fn wallet_export(&self) -> subbit_wallet::Store {
        self.wallet.clone()
    }

    /// Test-only: seeds a fresh `Ctx`'s basket directly (see
    /// `Basket::from_outputs`).
    #[cfg(test)]
    pub(crate) fn test_with_basket(basket: Basket) -> Self {
        Self {
            basket,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// File/`localStorage` round-trips live in `ui::cli`/`ui::wasm` --
    /// that's where the storage decision lives. This just checks
    /// `Ctx`'s own `Serialize`/`Deserialize` derive round-trips its
    /// fields correctly, with nothing storage-specific involved.
    #[test]
    fn json_roundtrips() {
        let mut ctx = Ctx::default();
        ctx.keyring_generate("alice".to_string(), true, true)
            .unwrap();
        ctx.whitelist_add(Hash::<28>::from([1u8; 28]));

        let json = serde_json::to_string(&ctx).unwrap();
        let restored: Ctx = serde_json::from_str(&json).unwrap();

        assert_eq!(ctx.keyring_labels(), restored.keyring_labels());
        assert_eq!(ctx.whitelist(), restored.whitelist());
    }

    fn validator_utxo(tx_hash: [u8; 32], index: u32) -> Utxo {
        let address: Address<kind::Any> =
            subbit_tx::validator::address(NetworkId::from(cardano_sdk::Network::Preprod), None)
                .into();
        Utxo {
            txo_ref: floe_dto_http::query::TxoRef {
                hash: tx_hash,
                index: index as u16,
            },
            output: floe_dto_http::cardano::TxOutput {
                address: floe_dto_http::query::Address(address.to_string().parse().unwrap()),
                coin: 5_000_000,
                assets: vec![],
                datum: None,
                script: Some(floe_dto_http::cardano::Script {
                    lang: floe_dto_http::cardano::ScriptLang::PlutusV3,
                    cbor: subbit_tx::VALIDATOR.script.script().to_vec(),
                }),
            },
            block_ref: floe_dto_http::query::ChainPoint {
                slot: 0,
                hash: vec![],
                height: 0,
                timestamp: 0,
            },
            spent_by: None,
        }
    }

    fn terms_with_reference_script(
        output_ref: Option<subbit_l2_dto::OutputRef>,
    ) -> subbit_l2_dto::Terms {
        subbit_l2_dto::Terms {
            network: subbit_l2_dto::Network::Preprod,
            script: Hash28::from([1u8; 28]),
            asset: Asset::Lovelace,
            provider: Hash28::from([2u8; 28]),
            close_period: Duration::from_secs(900),
            confirmation_depth: 3,
            min_iou_amount: 500,
            catalog: subbit_l2_dto::Catalog { resources: vec![] },
            reference_script: output_ref,
        }
    }

    #[test]
    fn set_reference_utxo_from_terms_finds_and_sets_the_matching_utxo() {
        let output_ref = subbit_l2_dto::OutputRef {
            tx_hash: subbit_core::Hash32::from([9u8; 32]),
            index: 2,
        };
        let terms = terms_with_reference_script(Some(output_ref));
        let items = vec![validator_utxo([1u8; 32], 0), validator_utxo([9u8; 32], 2)];

        let mut ctx = Ctx::default();
        ctx.basket_set_reference_utxo_from_terms(&terms, &items)
            .expect("the matching utxo is right there in items");
        assert_eq!(ctx.basket_reference_utxo().unwrap().txo_ref.hash, [9u8; 32]);
    }

    #[test]
    fn set_reference_utxo_from_terms_errors_without_a_published_reference_script() {
        let terms = terms_with_reference_script(None);
        let mut ctx = Ctx::default();
        assert!(
            ctx.basket_set_reference_utxo_from_terms(&terms, &[])
                .is_err()
        );
    }

    #[test]
    fn set_reference_utxo_from_terms_errors_when_not_among_the_given_items() {
        let output_ref = subbit_l2_dto::OutputRef {
            tx_hash: subbit_core::Hash32::from([9u8; 32]),
            index: 2,
        };
        let terms = terms_with_reference_script(Some(output_ref));
        let items = vec![validator_utxo([1u8; 32], 0)];

        let mut ctx = Ctx::default();
        assert!(
            ctx.basket_set_reference_utxo_from_terms(&terms, &items)
                .is_err()
        );
    }
}
