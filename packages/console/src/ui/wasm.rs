//! `packages/console/www`'s wasm-bindgen bundle: one `JsConsole`
//! wrapping a `Ctx`. Part of `ui` (alongside `ui::cli`) -- this is *a*
//! UI-layer adapter
//! over `Ctx`, not the only one, and `Ctx` itself has no idea this one
//! exists.
//!
//! **This file holds no domain logic and no tests, ever.** Every
//! method here does nothing but (a) parse hex/JSON off the wasm
//! boundary into the typed value a `Ctx` method expects, (b) call that
//! `Ctx` method, (c) map any error to `JsError`, and (d) serialize the
//! result back out. `Ctx` defines the API; this file is pure
//! boilerplate + error handling over it, never its own organizing
//! structure -- if a change here needs a test to be confident it's
//! correct, that logic belongs on `Ctx` instead. The one exception is
//! *where* to persist (`load`/`save`, below): `Ctx` has no opinion on
//! storage location at all, so picking `localStorage` specifically --
//! as opposed to `ui::cli`'s file -- is this adapter's own call, not
//! domain logic creeping in.
//!
//! Floe access goes through `floe_client::http::json::Client` (forwarded
//! by this crate's own `wasm` feature) -- no `Backend`-over-`fetch` impl
//! of our own.
//!
//! `JsConsole` also carries a `crate::Logs` alongside its `Ctx` -- see
//! that module's own doc for why it's a sibling field, not part of
//! `Ctx`. Only a handful of methods push into it (the ones with no
//! other success-signal `app.js` can render -- an add/remove that
//! fails already throws a `JsError` app.js catches and logs itself,
//! same as ever); `logsEntriesJson`/`logsClear` let `app.js` drain
//! whatever landed there into its own log view, same wording the
//! equivalent `ui::cli` command narrates.

mod cip30;

use std::collections::BTreeMap;
use std::str::FromStr;

use wasm_bindgen::prelude::*;

use cardano_sdk::{
    Address, Credential, Hash, Input, Network, NetworkId, Output, Signature, Transaction, Value,
    VerificationKey, address::kind, cbor::ToCbor, transaction::state,
};
use floe_dto_http::query::Utxo;
use subbit_core::{Hash28, VerifyingKey};
use subbit_l2_dto::Terms;
use subbit_wallet::Wallet;

use crate::{Ctx, Logs, ProviderEntry, SigningKeyHex};

fn js_err<E: std::fmt::Display>(e: E) -> JsError {
    JsError::new(&e.to_string())
}

const STORAGE_KEY: &str = "subbit-console.store";

fn local_storage() -> Result<web_sys::Storage, JsError> {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .ok_or_else(|| js_err("localStorage unavailable"))
}

/// This build's own validator hash (included in the whitelist by
/// default, on first run).
#[wasm_bindgen(js_name = defaultValidatorHash)]
pub fn default_validator_hash() -> String {
    subbit_tx::VALIDATOR.hash.to_string()
}

/// Every txo (live or spent) at any of `validator_hexes` (hex-encoded
/// 28-byte script hashes) over `floe_base_url`, as a JSON
/// `floe_dto_http::query::SearchUtxosResponse` -- feed straight into
/// `channelsLineageJson`. Every live-only need (`basketRefreshTxosJson`,
/// `findReferenceScriptJson`) is a client-side filter over this same
/// response's `items` (`spent_by == null`), not a second fetch -- see
/// `settings::floe`'s own doc.
#[wasm_bindgen(js_name = fetchHistory)]
pub async fn fetch_history(
    floe_base_url: String,
    validator_hexes: Vec<String>,
) -> Result<String, JsError> {
    let validators = parse_validators(&validator_hexes)?;
    let mut client = floe_client::http::json::Client::connect(&floe_base_url)
        .await
        .map_err(js_err)?;
    let (items, ledger_tip) = crate::discover_history(&mut client, &validators)
        .await
        .map_err(js_err)?;
    let response = floe_dto_http::query::SearchUtxosResponse {
        items,
        ledger_tip,
        next_token: None,
    };
    serde_json::to_string(&response).map_err(js_err)
}

fn parse_validators(hexes: &[String]) -> Result<std::collections::BTreeSet<Hash<28>>, JsError> {
    hexes
        .iter()
        .map(|h| Hash::<28>::from_str(h))
        .collect::<Result<_, _>>()
        .map_err(js_err)
}

/// Accepts either bech32 text or raw hex bytes (CIP-30's own
/// `getUsedAddresses`/`getRewardAddresses`/`getChangeAddress` shape) --
/// tries bech32 first, falls back to hex.
fn parse_address(s: &str) -> Result<Address<kind::Any>, JsError> {
    if let Ok(address) = Address::<kind::Any>::from_str(s) {
        return Ok(address);
    }
    let bytes = hex::decode(s).map_err(js_err)?;
    Address::<kind::Any>::try_from(bytes.as_slice()).map_err(js_err)
}

/// Decodes a wallet-advertised address into its payment credential's
/// vkh -- `Constants.consumer`'s own type. Errors on a non-Shelley or
/// script-credential address.
#[wasm_bindgen(js_name = decodeAddressVkh)]
pub fn decode_address_vkh(address: &str) -> Result<String, JsError> {
    let shelley = parse_address(address)?
        .as_shelley()
        .ok_or_else(|| js_err("not a Shelley address"))?;
    let vkh = shelley
        .payment()
        .as_key()
        .ok_or_else(|| js_err("payment credential is a script, not a key"))?;
    Ok(vkh.to_string())
}

/// Finds the validator's reference-script UTxO among `utxos_json`
/// (Floe wire JSON, `fetchLiveTxos`'s own shape) -- feed the result
/// straight into `basketBuild`'s `reference_utxo_json`. `None` if none
/// of the given utxos carries it.
#[wasm_bindgen(js_name = findReferenceScriptJson)]
pub fn find_reference_script_json(utxos_json: &str) -> Result<Option<String>, JsError> {
    let utxos: Vec<floe_dto_http::query::Utxo> =
        serde_json::from_str(utxos_json).map_err(js_err)?;
    let typed: BTreeMap<Input, Output> = utxos
        .iter()
        .filter_map(|u| subbit_tx::floe::to_input_output(u).ok())
        .collect();
    let Some((input, _)) = subbit_tx::validator::find_reference_script(&typed) else {
        return Ok(None);
    };
    let matching = utxos.iter().find(|u| {
        Hash::<32>::from(u.txo_ref.hash) == input.transaction_id()
            && u.txo_ref.index as u64 == input.output_index()
    });
    matching
        .map(|u| serde_json::to_string(u).map_err(js_err))
        .transpose()
}

/// Decodes one of CIP-30 `getUtxos()`'s own hex-CBOR entries -- a bare
/// `TransactionUnspentOutput = [transaction_input, transaction_output]`
/// array.
fn decode_unspent_output(hex_cbor: &str) -> Result<(Input, Output), JsError> {
    let bytes = hex::decode(hex_cbor).map_err(js_err)?;
    let mut decoder = cardano_sdk::cbor::Decoder::new(&bytes);
    decoder.array().map_err(js_err)?;
    let input: Input = decoder.decode().map_err(js_err)?;
    let output: Output = decoder.decode().map_err(js_err)?;
    Ok((input, output))
}

/// This session's submit wallet, whichever backend is currently
/// connected -- `WalletHandleError` wraps `Cip30::Error` and
/// `subbit_wallet::wallet::embedded::Error`, two unrelated types;
/// unifying them here is what lets `subbit_console::submit` stay generic over
/// `impl Wallet` instead of this crate branching on backend itself.
/// Never serialized -- a live connection (a CIP-30 `api` handle, or
/// `Embedded`'s own live `Backend`) can't survive a reload either way,
/// so there's nothing to persist here, same as `logs`.
enum WalletHandle {
    Cip30(cip30::Cip30),
    Embedded(subbit_wallet::Embedded<floe_client::http::json::Client>),
}

#[derive(Debug, thiserror::Error)]
enum WalletHandleError {
    #[error(transparent)]
    Cip30(#[from] cip30::Error),
    #[error(transparent)]
    Embedded(#[from] subbit_wallet::wallet::embedded::Error),
}

impl Wallet for WalletHandle {
    type Error = WalletHandleError;

    async fn network_id(&self) -> Result<NetworkId, Self::Error> {
        Ok(match self {
            WalletHandle::Cip30(w) => w.network_id().await?,
            WalletHandle::Embedded(w) => w.network_id().await?,
        })
    }

    async fn change_address(&self) -> Result<Address<kind::Any>, Self::Error> {
        Ok(match self {
            WalletHandle::Cip30(w) => w.change_address().await?,
            WalletHandle::Embedded(w) => w.change_address().await?,
        })
    }

    async fn utxos(
        &mut self,
        value: Option<Value<u64>>,
    ) -> Result<Option<BTreeMap<Input, Output>>, Self::Error> {
        Ok(match self {
            WalletHandle::Cip30(w) => w.utxos(value).await?,
            WalletHandle::Embedded(w) => w.utxos(value).await?,
        })
    }

    async fn sign_tx(
        &self,
        tx: &Transaction<state::ReadyForSigning>,
    ) -> Result<(VerificationKey, Signature), Self::Error> {
        Ok(match self {
            WalletHandle::Cip30(w) => w.sign_tx(tx).await?,
            WalletHandle::Embedded(w) => w.sign_tx(tx).await?,
        })
    }

    async fn submit(
        &mut self,
        tx: &Transaction<state::ReadyForSigning>,
    ) -> Result<Hash<32>, Self::Error> {
        Ok(match self {
            WalletHandle::Cip30(w) => w.submit(tx).await?,
            WalletHandle::Embedded(w) => w.submit(tx).await?,
        })
    }
}

#[wasm_bindgen]
pub struct JsConsole {
    ctx: Ctx,
    /// Never serialized, never persisted -- a fresh page load (or
    /// `new`/`load` call) always starts with an empty log, same as
    /// `ui::cli` starting a fresh `Logs` on every invocation. See
    /// `crate::logs`'s own doc.
    logs: Logs,
    /// This session's submit wallet -- `None` until `connectCip30`/
    /// `connectEmbedded` is called. See `WalletHandle`'s own doc on why
    /// it's never persisted.
    wallet: Option<WalletHandle>,
    /// `connectCip30`'s own externally-discovered vkh. CIP-30 never
    /// hands its signing key to this console, only the live `api`
    /// object `wallet` already wraps -- so unlike the embedded
    /// wallet's own vkh (`Ctx::wallet_vkh`, always derivable straight
    /// from key material this console already holds), this is the one
    /// fact about the connected signer `app.js` has to hand in itself.
    /// Session-only, same as `wallet` above -- never serialized into
    /// `Ctx`, and cleared by `disconnectWallet` right alongside it.
    cip30_vkh: Option<Hash28>,
}

#[wasm_bindgen]
impl JsConsole {
    /// `json`, when given, is an explicit backup blob (the Data
    /// Management "Import" button's uploaded file text) -- *not* how
    /// routine startup loads; see `load`/`save` for that. Deliberately
    /// a plain `serde_json` call here, not a `Ctx` method: the format
    /// (JSON, for a downloadable/uploadable file) is this boundary's
    /// own decision, same reasoning as every other `*Json` method in
    /// this file.
    #[wasm_bindgen(constructor)]
    pub fn new(json: Option<String>) -> Result<JsConsole, JsError> {
        let ctx = match json {
            Some(json) => serde_json::from_str(&json).map_err(js_err)?,
            None => Ctx::default(),
        };
        Ok(JsConsole {
            ctx,
            logs: Logs::default(),
            wallet: None,
            cip30_vkh: None,
        })
    }

    /// Routine startup: reads directly from `localStorage` -- a static
    /// call (`JsConsole.load()` in JS), not the constructor, since it
    /// takes no payload at all. `Ctx` itself has no opinion on *where*
    /// this comes from; that choice (and the JSON format) lives here,
    /// this boundary's own decision. Missing key -> a fresh default,
    /// not an error (first run).
    #[wasm_bindgen(js_name = load)]
    pub fn load() -> Result<JsConsole, JsError> {
        let storage = local_storage()?;
        let ctx = match storage
            .get_item(STORAGE_KEY)
            .map_err(|_| js_err("localStorage.getItem failed"))?
        {
            Some(text) => serde_json::from_str(&text).map_err(js_err)?,
            None => Ctx::default(),
        };
        Ok(JsConsole {
            ctx,
            logs: Logs::default(),
            wallet: None,
            cip30_vkh: None,
        })
    }

    /// Routine persistence: writes directly to `localStorage` -- call
    /// this after every mutation.
    #[wasm_bindgen(js_name = save)]
    pub fn save(&self) -> Result<(), JsError> {
        let storage = local_storage()?;
        let text = serde_json::to_string(&self.ctx).map_err(js_err)?;
        storage
            .set_item(STORAGE_KEY, &text)
            .map_err(|_| js_err("localStorage.setItem failed"))
    }

    /// The whole store, JSON-encoded -- for the Data Management
    /// "Export" button's downloadable backup file. Not the routine
    /// persistence path (`save`) -- see `new`'s own doc.
    #[wasm_bindgen(js_name = toJson)]
    pub fn to_json(&self) -> Result<String, JsError> {
        serde_json::to_string(&self.ctx).map_err(js_err)
    }

    /// Whatever narration has accumulated since the last `logsClear`
    /// -- `app.js` drains this after calling a method that might have
    /// pushed into it and renders each entry into the page's log view,
    /// same wording `ui::cli`'s equivalent command narrates.
    #[wasm_bindgen(js_name = logsEntriesJson)]
    pub fn logs_entries_json(&self) -> String {
        serde_json::to_string(self.logs.entries()).expect("Entry serialization is infallible")
    }

    #[wasm_bindgen(js_name = logsClear)]
    pub fn logs_clear(&mut self) {
        self.logs.clear();
    }

    // --- Keyring -----------------------------------------------------

    #[wasm_bindgen(js_name = keyringAdd)]
    pub fn keyring_add(
        &mut self,
        label: &str,
        key_hex: &str,
        is_consumer: bool,
        is_iou: bool,
    ) -> Result<(), JsError> {
        let key = SigningKeyHex::from_str(key_hex).map_err(js_err)?;
        self.ctx
            .keyring_add(label.to_string(), key, is_consumer, is_iou)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = keyringGenerate)]
    pub fn keyring_generate(
        &mut self,
        label: &str,
        is_consumer: bool,
        is_iou: bool,
    ) -> Result<String, JsError> {
        let key = self
            .ctx
            .keyring_generate(label.to_string(), is_consumer, is_iou)
            .map_err(js_err)?;
        self.logs.info(format!("generated key under label {label}"));
        Ok(hex::encode(key.to_bytes()))
    }

    #[wasm_bindgen(js_name = keyringRemove)]
    pub fn keyring_remove(&mut self, label: &str) -> bool {
        self.ctx.keyring_remove(label)
    }

    #[wasm_bindgen(js_name = keyringLabelsJson)]
    pub fn keyring_labels_json(&self) -> String {
        serde_json::to_string(&self.ctx.keyring_labels())
            .expect("String serialization is infallible")
    }

    // --- Consumers -----------------------------------------------------

    #[wasm_bindgen(js_name = consumersAdd)]
    pub fn consumers_add(&mut self, label: &str, vkh_hex: &str) -> Result<(), JsError> {
        let vkh = Hash28::from_str(vkh_hex).map_err(js_err)?;
        self.ctx
            .consumers_add(label.to_string(), vkh)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = consumersRemove)]
    pub fn consumers_remove(&mut self, label: &str) -> bool {
        self.ctx.consumers_remove(label)
    }

    #[wasm_bindgen(js_name = consumersEffectiveJson)]
    pub fn consumers_effective_json(&self) -> String {
        serde_json::to_string(&self.ctx.consumers_effective()).expect("serialization is infallible")
    }

    // --- IouKeys ---------------------------------------------------------

    #[wasm_bindgen(js_name = iouAdd)]
    pub fn iou_add(&mut self, label: &str, key_hex: &str) -> Result<(), JsError> {
        let key = VerifyingKey::from_str(key_hex).map_err(js_err)?;
        self.ctx.iou_add(label.to_string(), key).map_err(js_err)
    }

    #[wasm_bindgen(js_name = iouRemove)]
    pub fn iou_remove(&mut self, label: &str) -> bool {
        self.ctx.iou_remove(label)
    }

    #[wasm_bindgen(js_name = iouEffectiveJson)]
    pub fn iou_effective_json(&self) -> String {
        serde_json::to_string(&self.ctx.iou_effective()).expect("serialization is infallible")
    }

    // --- Providers ---------------------------------------------------

    #[wasm_bindgen(js_name = providersSet)]
    pub fn providers_set(
        &mut self,
        label: &str,
        base_url: &str,
        terms_json: Option<String>,
        fetched_at: Option<u64>,
    ) -> Result<(), JsError> {
        let terms: Option<Terms> = terms_json
            .map(|json| serde_json::from_str(&json))
            .transpose()
            .map_err(js_err)?;
        let fetched = terms.is_some();
        self.ctx.providers_set(
            label.to_string(),
            ProviderEntry {
                base_url: base_url.to_string(),
                terms,
                fetched_at,
            },
        );
        if fetched {
            self.logs.info(format!("fetched + saved provider {label}"));
        } else {
            self.logs.info(format!("saved provider {label} (no terms)"));
        }
        Ok(())
    }

    /// Pulls this provider's own `Terms` from its well-known discovery
    /// path (`subbit_l2_dto::WELL_KNOWN_PATH`) and saves it, stamping
    /// `fetched_at` itself -- the fetch (and the path it fetches from)
    /// lives here, a wasm-boundary I/O concern same as `fetchHistory`'s
    /// own Floe fetch, rather than `app.js` doing its own `fetch()`
    /// against a literal it'd have to keep in sync with the server's
    /// own route by hand.
    #[wasm_bindgen(js_name = providersFetch)]
    pub async fn providers_fetch(&mut self, label: &str, base_url: &str) -> Result<(), JsError> {
        let url = format!(
            "{}{}",
            base_url.trim_end_matches('/'),
            subbit_l2_dto::WELL_KNOWN_PATH
        );
        let response = gloo_net::http::Request::get(&url)
            .send()
            .await
            .map_err(js_err)?;
        if !response.ok() {
            return Err(js_err(format!("http {}", response.status())));
        }
        let text = response.text().await.map_err(js_err)?;
        let terms: Terms = serde_json::from_str(&text).map_err(js_err)?;
        let fetched_at = (js_sys::Date::now() / 1000.0) as u64;
        self.ctx.providers_set(
            label.to_string(),
            ProviderEntry {
                base_url: base_url.to_string(),
                terms: Some(terms),
                fetched_at: Some(fetched_at),
            },
        );
        self.logs.info(format!("fetched + saved provider {label}"));
        Ok(())
    }

    #[wasm_bindgen(js_name = providersRemove)]
    pub fn providers_remove(&mut self, label: &str) -> bool {
        self.ctx.providers_remove(label)
    }

    #[wasm_bindgen(js_name = providersJson)]
    pub fn providers_json(&self) -> String {
        serde_json::to_string(&self.ctx.providers_list())
            .expect("ProviderEntry serialization is infallible")
    }

    #[wasm_bindgen(js_name = providersByProviderJson)]
    pub fn providers_by_provider_json(&self, provider_hex: &str) -> Result<String, JsError> {
        let provider = Hash28::from_str(provider_hex).map_err(js_err)?;
        serde_json::to_string(&self.ctx.providers_by_provider(&provider)).map_err(js_err)
    }

    #[wasm_bindgen(js_name = reconcileJson)]
    pub fn reconcile_json(&self, constants_json: &str) -> Result<String, JsError> {
        let constants: subbit_l1_dto::Constants =
            serde_json::from_str(constants_json).map_err(js_err)?;
        serde_json::to_string(&self.ctx.reconcile(&constants)).map_err(js_err)
    }

    // --- Whitelisted validators -------------------------------------

    #[wasm_bindgen(js_name = whitelistAdd)]
    pub fn whitelist_add(&mut self, script_hash_hex: &str) -> Result<(), JsError> {
        let hash = Hash::<28>::from_str(script_hash_hex).map_err(js_err)?;
        self.ctx.whitelist_add(hash);
        Ok(())
    }

    #[wasm_bindgen(js_name = whitelistRemove)]
    pub fn whitelist_remove(&mut self, script_hash_hex: &str) -> Result<bool, JsError> {
        let hash = Hash::<28>::from_str(script_hash_hex).map_err(js_err)?;
        Ok(self.ctx.whitelist_remove(&hash))
    }

    #[wasm_bindgen(js_name = whitelistJson)]
    pub fn whitelist_json(&self) -> String {
        serde_json::to_string(self.ctx.whitelist()).expect("String serialization is infallible")
    }

    // --- Basket --------------------------------------------------------

    #[wasm_bindgen(js_name = basketRefreshTxosJson)]
    pub fn basket_refresh_txos_json(&mut self, utxos_json: &str) -> Result<(), JsError> {
        let utxos: Vec<floe_dto_http::query::Utxo> =
            serde_json::from_str(utxos_json).map_err(js_err)?;
        self.ctx.basket_refresh_txos(utxos);
        Ok(())
    }

    #[wasm_bindgen(js_name = basketTxosJson)]
    pub fn basket_txos_json(&self) -> String {
        let pairs: Vec<_> = self.ctx.basket_txos().iter().collect();
        serde_json::to_string(&pairs).expect("Channel serialization is infallible")
    }

    #[wasm_bindgen(js_name = dashboardJson)]
    pub fn dashboard_json(&self) -> String {
        serde_json::to_string(&self.ctx.dashboard()).expect("Status serialization is infallible")
    }

    /// The full-history view: `search_utxos_response_json` is
    /// `fetchHistory`'s own output. Grouped by Constants/line
    /// (index-dto's own organization), each line carrying `current`/
    /// `finalized` directly rather than a flat item list + boundary
    /// index -- see `Ctx::channels_lineage`'s own doc.
    #[wasm_bindgen(js_name = channelsLineageJson)]
    pub fn channels_lineage_json(
        &self,
        search_utxos_response_json: &str,
        default_confirmation_depth: u32,
    ) -> Result<String, JsError> {
        let response: floe_dto_http::query::SearchUtxosResponse =
            serde_json::from_str(search_utxos_response_json).map_err(js_err)?;
        let view = self.ctx.channels_lineage(
            response.items,
            response.ledger_tip,
            default_confirmation_depth,
            self.active_signer_vkh(),
        );
        serde_json::to_string(&view).map_err(js_err)
    }

    #[wasm_bindgen(js_name = basketOpensJson)]
    pub fn basket_opens_json(&self) -> String {
        serde_json::to_string(self.ctx.basket_opens()).expect("Open serialization is infallible")
    }

    #[wasm_bindgen(js_name = basketWillsJson)]
    pub fn basket_wills_json(&self) -> String {
        let pairs: Vec<_> = self.ctx.basket_wills().iter().collect();
        serde_json::to_string(&pairs).expect("Will serialization is infallible")
    }

    #[wasm_bindgen(js_name = basketPropose)]
    pub fn basket_propose(
        &mut self,
        tx_hash_hex: &str,
        index: u64,
        want_json: &str,
    ) -> Result<(), JsError> {
        let hash = Hash::<32>::from_str(tx_hash_hex).map_err(js_err)?;
        let want = serde_json::from_str(want_json).map_err(js_err)?;
        self.ctx
            .basket_propose(Input::new(hash, index), want)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = basketDropIntent)]
    pub fn basket_drop_intent(&mut self, tx_hash_hex: &str, index: u64) -> Result<bool, JsError> {
        let hash = Hash::<32>::from_str(tx_hash_hex).map_err(js_err)?;
        Ok(self.ctx.basket_drop_intent(&Input::new(hash, index)))
    }

    #[wasm_bindgen(js_name = basketDropAllIntents)]
    pub fn basket_drop_all_intents(&mut self) {
        self.ctx.basket_drop_all_intents();
    }

    #[wasm_bindgen(js_name = basketAddOpen)]
    #[allow(clippy::too_many_arguments)]
    pub fn basket_add_open(
        &mut self,
        tag_hex: &str,
        asset_json: &str,
        iou_key_hex: &str,
        consumer_hex: &str,
        provider_hex: &str,
        close_period_secs: u64,
        delegation_bech32: Option<String>,
        amount: u64,
    ) -> Result<(), JsError> {
        let tag = subbit_core::Tag::from_str(tag_hex).map_err(js_err)?;
        let asset = serde_json::from_str(asset_json).map_err(js_err)?;
        let iou_key = VerifyingKey::from_str(iou_key_hex).map_err(js_err)?;
        let consumer = Hash28::from_str(consumer_hex).map_err(js_err)?;
        let provider = Hash28::from_str(provider_hex).map_err(js_err)?;
        let close_period = subbit_core::Duration::from_secs(close_period_secs);
        let delegation = delegation_bech32
            .map(|s| Credential::from_str(&s))
            .transpose()
            .map_err(js_err)?;

        self.ctx.basket_add_open(
            tag,
            asset,
            iou_key,
            consumer,
            provider,
            close_period,
            delegation,
            amount,
        );
        self.logs.info(format!("staged open, tag={tag_hex}"));
        Ok(())
    }

    #[wasm_bindgen(js_name = basketDropOpen)]
    pub fn basket_drop_open(&mut self, keytag: &str) -> bool {
        self.ctx.basket_drop_open(keytag)
    }

    #[wasm_bindgen(js_name = basketDropAllOpens)]
    pub fn basket_drop_all_opens(&mut self) {
        self.ctx.basket_drop_all_opens();
    }

    /// Sets the basket's reference script (verified once, here --
    /// see `Ctx::basket_set_reference_utxo`'s own doc) -- `basketBuild`/
    /// `submit` refuse to run at all until this has been called.
    /// `utxo_json` is `findReferenceScriptJson`'s own output when
    /// self-hosting, or a provider's published `Terms.reference_script`
    /// resolved to a live utxo -- consumers should be invited to the
    /// latter over uploading their own, though either works.
    #[wasm_bindgen(js_name = basketSetReferenceUtxo)]
    pub fn basket_set_reference_utxo(&mut self, utxo_json: &str) -> Result<(), JsError> {
        let utxo = serde_json::from_str(utxo_json).map_err(js_err)?;
        self.ctx.basket_set_reference_utxo(utxo).map_err(js_err)
    }

    #[wasm_bindgen(js_name = basketClearReferenceUtxo)]
    pub fn basket_clear_reference_utxo(&mut self) {
        self.ctx.basket_clear_reference_utxo();
    }

    /// Resolves a provider's own published `Terms.reference_script`
    /// against already-fetched `items_json` (`fetchHistory`'s own
    /// output, same as `findReferenceScriptJson` filters -- no
    /// separate Floe round trip) and sets it as the basket's
    /// reference script. `terms_json` is one `providersJson()` entry's
    /// own `terms` field -- the preferred path over
    /// `basketSetReferenceUtxo`'s self-hosted one, per `Ctx::
    /// basket_set_reference_utxo_from_terms`'s own doc.
    #[wasm_bindgen(js_name = basketSetReferenceUtxoFromTerms)]
    pub fn basket_set_reference_utxo_from_terms(
        &mut self,
        terms_json: &str,
        items_json: &str,
    ) -> Result<(), JsError> {
        let terms = serde_json::from_str(terms_json).map_err(js_err)?;
        let items: Vec<Utxo> = serde_json::from_str(items_json).map_err(js_err)?;
        self.ctx
            .basket_set_reference_utxo_from_terms(&terms, &items)
            .map_err(js_err)
    }

    #[wasm_bindgen(js_name = basketBuild)]
    pub fn basket_build(
        &mut self,
        network: &str,
        change_address: &str,
        fuel_cbor_hex: Vec<String>,
    ) -> Result<String, JsError> {
        let change_address = parse_address(change_address)?;
        let fuel: BTreeMap<Input, Output> = fuel_cbor_hex
            .iter()
            .map(|hex| decode_unspent_output(hex))
            .collect::<Result<_, _>>()?;

        let tx = self
            .ctx
            .basket_build(network, change_address, &fuel)
            .map_err(js_err)?;
        Ok(hex::encode(tx.to_cbor()))
    }

    // --- Submit wallet (Tx tab) ---------------------------------------------
    //
    // `app.js`'s `wallet-connect` handler still does CIP-30's own
    // `enable()`/network-check/address-derivation dance itself (none of
    // that moved here) -- it just hands the resulting `api` object over
    // once connected. `submit` is the one path both backends share from
    // there on; see `submit`'s own module doc for its three stages
    // (allocation, sign + aggregate witnesses, submit) and why that
    // fixes a real bug, not just moves code around.

    /// `vkh_hex`: `decodeAddressVkh`'s own output against whichever
    /// address `app.js`'s connect handler already resolved -- handed
    /// in, not re-derived here, since Rust has no visibility into the
    /// CIP-30 `api` object's addresses beyond what `Wallet::
    /// change_address` can tell it (async, and not this vkh's own
    /// network-bearing form). See `cip30_vkh`'s own doc on why this is
    /// the one piece of signer identity `app.js` has to hand over.
    #[wasm_bindgen(js_name = connectCip30)]
    pub fn connect_cip30(&mut self, api: JsValue, vkh_hex: &str) -> Result<(), JsError> {
        let vkh = Hash28::from(<[u8; 28]>::from(
            Hash::<28>::from_str(vkh_hex).map_err(js_err)?,
        ));
        self.wallet = Some(WalletHandle::Cip30(cip30::Cip30::new(api)));
        self.cip30_vkh = Some(vkh);
        Ok(())
    }

    /// `channels_lineage`'s own `active_signer` input -- whichever
    /// backend is currently connected, resolved to its own vkh.
    /// `Embedded`'s is always derivable from `Ctx` itself (no
    /// connection needed to know it, see `Ctx::wallet_vkh`'s own doc);
    /// CIP-30's is `cip30_vkh`, the one fact `app.js` had to hand in at
    /// `connectCip30` time.
    fn active_signer_vkh(&self) -> Option<Hash28> {
        match self.wallet.as_ref()? {
            WalletHandle::Cip30(_) => self.cip30_vkh,
            WalletHandle::Embedded(_) => self.ctx.wallet_vkh(),
        }
    }

    /// Connects this session's own embedded key (`Ctx::wallet_*`) as
    /// the submit wallet -- fails if no key is configured yet
    /// (`Embedded::from_store`'s own contract). Reconnects a fresh
    /// `floe_client::http::json::Client` every call, same as
    /// `fetchHistory`'s own, pre-existing "no shared client yet" note;
    /// not fixed here, out of scope for this change.
    #[wasm_bindgen(js_name = connectEmbedded)]
    pub async fn connect_embedded(
        &mut self,
        floe_base_url: &str,
        network: &str,
    ) -> Result<(), JsError> {
        let network = NetworkId::from(Network::try_from(network).map_err(js_err)?);
        let client = floe_client::http::json::Client::connect(floe_base_url)
            .await
            .map_err(js_err)?;
        let store = self.ctx.wallet_export();
        let embedded =
            subbit_wallet::Embedded::from_store(&store, client, network).map_err(js_err)?;
        self.wallet = Some(WalletHandle::Embedded(embedded));
        Ok(())
    }

    #[wasm_bindgen(js_name = disconnectWallet)]
    pub fn disconnect_wallet(&mut self) {
        self.wallet = None;
        self.cip30_vkh = None;
    }

    /// Uploads the subbit validator as a reference script, at
    /// `address` if given (else the connected wallet's own change
    /// address -- the common case). See `upload_reference_script`'s
    /// own doc; doesn't set it as the basket's reference script
    /// itself -- `basketSetReferenceUtxo`/`FromTerms` is still a
    /// separate, explicit step after this lands.
    #[wasm_bindgen(js_name = uploadReferenceScript)]
    pub async fn upload_reference_script(
        &mut self,
        network: &str,
        address: Option<String>,
    ) -> Result<String, JsError> {
        let address = address.map(|a| parse_address(&a)).transpose()?;
        let wallet = self
            .wallet
            .as_mut()
            .ok_or_else(|| js_err("no wallet connected"))?;
        let hash = crate::upload_reference_script(wallet, network, address)
            .await
            .map_err(js_err)?;
        Ok(hash.to_string())
    }

    /// Reclaims the connected wallet's own held reference-script utxo
    /// back into an ordinary output at its own change address. See
    /// `teardown_reference_script`'s own doc -- only ever reclaims a
    /// utxo this wallet already has spending authority over.
    #[wasm_bindgen(js_name = teardownReferenceScript)]
    pub async fn teardown_reference_script(&mut self, network: &str) -> Result<String, JsError> {
        let wallet = self
            .wallet
            .as_mut()
            .ok_or_else(|| js_err("no wallet connected"))?;
        let hash = crate::teardown_reference_script(wallet, network)
            .await
            .map_err(js_err)?;
        Ok(hash.to_string())
    }

    /// Allocates (`Ctx::basket_build`), signs + aggregates every
    /// required witness, and submits -- against whichever wallet
    /// `connectCip30`/`connectEmbedded` last connected. See `submit`'s
    /// own module doc for the three stages. Fails immediately if
    /// `basketSetReferenceUtxo` hasn't been called yet.
    #[wasm_bindgen(js_name = submit)]
    pub async fn submit(&mut self, network: &str) -> Result<String, JsError> {
        let wallet = self
            .wallet
            .as_mut()
            .ok_or_else(|| js_err("no wallet connected"))?;
        let hash = crate::submit(wallet, &mut self.ctx, network)
            .await
            .map_err(js_err)?;
        Ok(hash.to_string())
    }

    // --- Embedded wallet (Settings tab) -----------------------------------

    #[wasm_bindgen(js_name = walletViewJson)]
    pub fn wallet_view_json(&self) -> String {
        serde_json::to_string(&self.ctx.wallet_view())
            .expect("WalletView serialization is infallible")
    }

    #[wasm_bindgen(js_name = walletGenerate)]
    pub fn wallet_generate(&mut self) -> String {
        subbit_wallet::reveal_hex(&self.ctx.wallet_generate())
    }

    #[wasm_bindgen(js_name = walletImport)]
    pub fn wallet_import(&mut self, key_hex: &str) -> Result<(), JsError> {
        self.ctx.wallet_import(key_hex).map_err(js_err)?;
        self.logs.info("embedded wallet key imported");
        Ok(())
    }

    #[wasm_bindgen(js_name = walletSetDelegation)]
    pub fn wallet_set_delegation(&mut self, credential_hex: &str) -> Result<(), JsError> {
        let hash = Hash::<28>::from_str(credential_hex).map_err(js_err)?;
        self.ctx.wallet_set_delegation(Credential::from_key(hash));
        Ok(())
    }

    #[wasm_bindgen(js_name = walletClearDelegation)]
    pub fn wallet_clear_delegation(&mut self) {
        self.ctx.wallet_clear_delegation();
    }

    #[wasm_bindgen(js_name = walletScrub)]
    pub fn wallet_scrub(&mut self) {
        self.ctx.wallet_scrub();
        self.logs.info("embedded wallet scrubbed");
    }

    #[wasm_bindgen(js_name = walletReveal)]
    pub fn wallet_reveal(&mut self) -> Option<String> {
        let key = self.ctx.wallet_reveal();
        if key.is_none() {
            self.logs.info("reveal: no key configured");
        }
        key.map(|key| subbit_wallet::reveal_hex(&key))
    }

    #[wasm_bindgen(js_name = walletExportJson)]
    pub fn wallet_export_json(&self) -> Result<String, JsError> {
        serde_json::to_string(&self.ctx.wallet_export()).map_err(js_err)
    }

    #[wasm_bindgen(js_name = walletAddress)]
    pub fn wallet_address(&self, network: &str) -> Result<Option<String>, JsError> {
        let network = NetworkId::from(Network::try_from(network).map_err(js_err)?);
        Ok(self.ctx.wallet_address(network).map(|a| a.to_string()))
    }
}
