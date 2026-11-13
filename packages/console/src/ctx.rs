//! `Ctx`: the full application model every UI sits on. Methods take/
//! return already-typed domain values, never raw hex/JSON -- parsing
//! is each UI's own job.

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
    Bounds, Channel, Open,
    queue::Queue,
    step::{Want, Will},
};

use crate::channels::{Match, Status};
use crate::{
    Consumers, Error, IouKeys, Keyring, ProviderEntry, Providers, SigningKeyHex, WalletView,
    Whitelist,
};

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize)]
pub struct Ctx {
    keyring: Keyring,
    consumers: Consumers,
    iou_keys: IouKeys,
    providers: Providers,
    #[serde(default)]
    queue: Queue,
    #[serde(default)]
    whitelist: Whitelist,
    #[serde(default)]
    wallet: subbit_wallet::Store,
}

impl Ctx {
    // --- Keyring -----------------------------------------------------

    #[allow(clippy::too_many_arguments)]
    pub fn keyring_add(
        &mut self,
        label: String,
        key: SigningKeyHex,
        is_consumer: bool,
        is_iou: bool,
        is_provider: bool,
    ) -> Result<(), Error> {
        self.keyring.insert(
            label,
            key,
            is_consumer,
            is_iou,
            is_provider,
            &self.consumers,
            &self.iou_keys,
        )
    }

    pub fn keyring_generate(
        &mut self,
        label: String,
        is_consumer: bool,
        is_iou: bool,
        is_provider: bool,
    ) -> Result<SigningKeyHex, Error> {
        self.keyring.generate(
            label,
            is_consumer,
            is_iou,
            is_provider,
            &self.consumers,
            &self.iou_keys,
        )
    }

    pub fn keyring_remove(&mut self, label: &str) -> bool {
        self.keyring.remove(label).is_some()
    }

    pub fn keyring_labels(&self) -> Vec<&String> {
        self.keyring.labels().collect()
    }

    pub fn keyring_signing_key_for_consumer(
        &self,
        vkh: cardano_sdk::Hash<28>,
    ) -> Option<cardano_sdk::SigningKey> {
        self.keyring.signing_key_for_consumer(vkh)
    }

    pub fn keyring_signing_key_for_provider(
        &self,
        vkh: cardano_sdk::Hash<28>,
    ) -> Option<cardano_sdk::SigningKey> {
        self.keyring.signing_key_for_provider(vkh)
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

    pub fn providers_insert(&mut self, label: String, entry: ProviderEntry) -> Result<(), Error> {
        self.providers.insert(label, entry)
    }

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

    pub fn basket_refresh_txos(&mut self, network: &str, utxos: impl IntoIterator<Item = Utxo>) {
        let network_parameters =
            subbit_tx::NetworkParameters::for_network(network).unwrap_or_else(|_| {
                subbit_tx::NetworkParameters::for_network("preprod").expect("preprod always parses")
            });
        let old_settings = self.queue.settings().clone();
        let mut settings = subbit_tx::queue::Settings::new(
            network_parameters,
            old_settings.build_config().clone(),
        );
        if let Some((input, output)) = old_settings.reference_utxo().cloned() {
            let _ = settings.set_reference_utxo(input, output);
        }
        if let Some(change_address) = old_settings.change_address().cloned() {
            settings.set_change_address(change_address);
        }

        let opens: Vec<Open> = self.queue.opens().values().cloned().collect();
        // Drained by `Ref`, not a frozen `Input` -- a want staged
        // `propose_by_id` (`Ref::ById`) re-resolves against whichever
        // utxo backs its channel in the *new* set below, surviving
        // that channel's own utxo having moved since it was staged
        // (e.g. a concurrent `subbit-claim` Sub/Settle on the same
        // channel). A want staged plain `propose` (`Ref::ByInput`)
        // keeps the old behavior: gone if that exact utxo is gone.
        let wants = self.queue.drain_wants();

        let mut queue = Queue::from_settings(settings);
        let mut map = BTreeMap::new();
        for utxo in utxos {
            if let Ok((input, output)) = crate::floe::to_input_output(&utxo) {
                map.insert(input, output);
            }
        }
        let _ = queue.add_utxos(map);
        for open in opens {
            queue.add_open(open);
        }
        for (r, want) in wants {
            let _ = queue.propose_ref(r, want);
        }
        self.queue = queue;
    }

    pub fn basket_txos(&self) -> &BTreeMap<Input, Channel> {
        self.queue.channels()
    }

    pub fn dashboard(&self) -> Vec<Status> {
        crate::statuses(
            self.queue.channels(),
            &self.consumers,
            &self.iou_keys,
            &self.keyring,
        )
    }

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
            self.queue.channels(),
        )
    }

    pub fn basket_opens(&self) -> &BTreeMap<String, Open> {
        self.queue.opens()
    }

    pub fn basket_wills(&self) -> BTreeMap<Input, Will> {
        self.queue.wills()
    }

    pub fn basket_picked_wills(&self) -> BTreeMap<Input, Will> {
        self.queue.picked_wills()
    }

    pub fn basket_picked_opens(&self) -> &BTreeMap<String, Open> {
        self.queue.picked_opens()
    }

    pub fn basket_propose(&mut self, input: Input, want: Want) -> Result<(), Error> {
        self.queue
            .propose(input, want)
            .map_err(|e| Error::Basket(e.to_string()))
    }

    /// Pairs `want` to whichever channel's utxo currently backs
    /// `keytag` (`iou_key`+`tag` identity, see `Constants::keytag`),
    /// re-resolved fresh on every `basket_pick` -- and carried across
    /// a `basket_refresh_txos` -- rather than frozen to one input.
    pub fn basket_propose_by_id(
        &mut self,
        keytag: impl Into<String>,
        want: Want,
    ) -> Result<(), Error> {
        self.queue
            .propose_by_id(keytag, want)
            .map_err(|e| Error::Basket(e.to_string()))
    }

    pub fn basket_drop_intent(&mut self, input: &Input) -> bool {
        self.queue.drop_intent(input).is_some()
    }

    pub fn basket_drop_all_intents(&mut self) {
        self.queue.drop_all_intents();
    }

    pub fn basket_evict_invalid(&mut self) -> BTreeMap<subbit_tx::queue::Ref, Want> {
        self.queue.evict_invalid()
    }

    pub fn basket_evict_expired(
        &mut self,
        now: subbit_core::Instant,
    ) -> BTreeMap<subbit_tx::queue::Ref, Want> {
        self.queue.evict_expired(now)
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
        let variables =
            subbit_tx::Variables::new(amount, subbit_l1_dto::Stage::Opened { subbed: 0 });
        let channel = Channel::new(constants, variables);
        self.queue
            .add_open(subbit_tx::Open::new(channel, delegation));
    }

    pub fn basket_drop_open(&mut self, keytag: &str) -> bool {
        self.queue.drop_open(keytag).is_some()
    }

    pub fn basket_drop_all_opens(&mut self) {
        self.queue.drop_all_opens();
    }

    pub fn basket_set_reference_utxo(&mut self, utxo: Utxo) -> Result<(), Error> {
        let (input, output) =
            crate::floe::to_input_output(&utxo).map_err(|e| Error::Basket(e.to_string()))?;
        self.queue
            .settings_mut()
            .set_reference_utxo(input, output)
            .map_err(|e| Error::Basket(e.to_string()))
    }

    pub fn basket_reference_utxo(&self) -> Option<&(Input, Output)> {
        self.queue.settings().reference_utxo()
    }

    pub fn basket_clear_reference_utxo(&mut self) {
        self.queue.settings_mut().clear_reference_utxo();
    }

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

    pub fn basket_set_change_address(&mut self, change_address: Address<kind::Any>) {
        self.queue.settings_mut().set_change_address(change_address);
    }

    pub fn basket_change_address(&self) -> Option<&Address<kind::Any>> {
        self.queue.settings().change_address()
    }

    pub fn basket_pick(&mut self, bounds: Bounds) -> Result<(), Error> {
        self.queue
            .pick(bounds)
            .map_err(|e| Error::Basket(e.to_string()))
    }

    pub fn basket_unpick(&mut self) {
        self.queue.unpick();
    }

    pub fn basket_build(
        &mut self,
        fuel: &BTreeMap<Input, Output>,
    ) -> Result<Transaction<ReadyForSigning>, Error> {
        self.queue
            .build(fuel)
            .map_err(|e| Error::Basket(e.to_string()))
    }

    pub fn basket_pick_and_build(
        &mut self,
        bounds: Bounds,
        fuel: &BTreeMap<Input, Output>,
    ) -> Result<Transaction<ReadyForSigning>, Error> {
        self.queue
            .pick_and_build(bounds, fuel)
            .map_err(|e| Error::Basket(e.to_string()))
    }

    pub fn basket_build_config(&self) -> &subbit_tx::queue::BuildConfig {
        self.queue.settings().build_config()
    }

    pub fn basket_set_build_config(&mut self, config: subbit_tx::queue::BuildConfig) {
        self.queue.settings_mut().set_build_config(config);
    }

    /// Every outstanding batch's own tx id, still awaiting
    /// `basket_finalize`/`basket_rollback`.
    pub fn basket_in_flight_ids(&self) -> Vec<Hash<32>> {
        self.queue.in_flight_ids().collect()
    }

    /// Whether any batch is still awaiting `basket_finalize`/
    /// `basket_rollback`.
    pub fn basket_in_flight(&self) -> bool {
        self.queue.in_flight()
    }

    /// Confirms `id`'s batch actually landed -- drops it for good.
    pub fn basket_finalize(&mut self, id: Hash<32>) -> bool {
        self.queue.finalize(id)
    }

    /// `id`'s batch was rejected or abandoned -- restages it back into
    /// the basket's own pending Wills/Opens.
    pub fn basket_rollback(&mut self, id: Hash<32>) -> bool {
        self.queue.rollback(id)
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

    pub fn wallet_signing_key(&self) -> Option<cardano_sdk::SigningKey> {
        self.wallet
            .signing_key
            .clone()
            .map(|key| key.into_signing_key())
    }

    pub fn wallet_delegation(&self) -> Option<Credential> {
        self.wallet.delegation.clone()
    }

    pub fn wallet_vkh(&self) -> Option<Hash28> {
        let key = self.wallet_signing_key()?;
        let hash = Hash::<28>::new(key.to_verification_key());
        Some(Hash28::from(<[u8; 28]>::from(hash)))
    }

    pub fn wallet_export(&self) -> subbit_wallet::Store {
        self.wallet.clone()
    }

    #[cfg(test)]
    pub(crate) fn test_with_queue(queue: Queue) -> Self {
        Self {
            queue,
            ..Default::default()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn json_roundtrips() {
        let mut ctx = Ctx::default();
        ctx.keyring_generate("alice".to_string(), true, true, false)
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
        assert_eq!(
            ctx.basket_reference_utxo().unwrap().0.transaction_id(),
            Hash::<32>::from([9u8; 32])
        );
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
