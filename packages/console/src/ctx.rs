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
//! boundary (mirrors `Basket`'s own long-standing rule, now applied
//! crate-wide). A UI's own marshalling layer (`ui::wasm`, `ui::cli`)
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

    pub fn basket_build(
        &mut self,
        network: &str,
        reference_utxo: Option<Utxo>,
        change_address: Address<kind::Any>,
        fuel: &BTreeMap<Input, Output>,
    ) -> Result<Transaction<ReadyForSigning>, Error> {
        self.basket
            .build(network, reference_utxo, change_address, fuel)
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
}

#[cfg(test)]
mod tests {
    use super::*;

    /// File/`localStorage` round-trips live in `ui::cli`/`ui::wasm`
    /// now -- that's where the storage decision lives. This just
    /// checks `Ctx`'s own `Serialize`/`Deserialize` derive round-trips
    /// its fields correctly, with nothing storage-specific involved.
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
}
