//! CIP-30 as a `subbit_wallet::Wallet` implementor -- the "complement"
//! that crate's own doc names but deliberately doesn't build itself,
//! since it's inherently wasm/JS-dependent (see `subbit_wallet::
//! Wallet`'s own doc). Everything here is JS interop glue (`js_sys::
//! Reflect` + `Function` + `JsFuture`) over an already-`enable()`d
//! CIP-30 `api` object -- `app.js`'s `wallet-connect` handler still
//! does the connect/network-check/address-derivation dance exactly as
//! before (none of that moved here); it just hands the resulting `api`
//! object to `JsConsole::connectCip30` once connected, and this struct
//! becomes that session's `Wallet` from then on.
//!
//! **Always requests a partial signature** (`signTx(tx, true)`) --
//! CIP-30 returns only a `transaction_witness_set` either way (never a
//! complete transaction, regardless of this flag), so asking for
//! partial is also the honest request: this wallet never assumes it's
//! the only signer a transaction needs. `submit::submit`'s own
//! `add_witness` step is what actually assembles a submittable
//! transaction from that witness, the same step `Embedded`'s own
//! (already-complete, single-signer) witness goes through too.

use std::collections::BTreeMap;
use std::str::FromStr;

use cardano_sdk::{
    Address, Hash, Input, NetworkId, Output, Signature, Transaction, Value, VerificationKey,
    address::kind, cbor::ToCbor, transaction::state,
};
use js_sys::{Function, Promise, Reflect};
use subbit_wallet::Wallet;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("CIP-30 call failed: {0}")]
    Js(String),
    #[error("hex decode error: {0}")]
    Hex(#[from] hex::FromHexError),
    #[error("witness decode error: {0}")]
    Witness(#[from] subbit_wallet::witness::Error),
    #[error("CIP-30 returned an address this wallet can't parse: {0}")]
    Address(String),
    #[error("CIP-30 wallet reported no addresses")]
    NoAddress,
    #[error("CIP-30 getUtxos returned a non-string entry")]
    BadUtxoEntry,
    #[error("CIP-30 getUtxos entry didn't decode as a TransactionUnspentOutput: {0}")]
    BadUtxoCbor(String),
}

fn js_display(v: JsValue) -> String {
    v.as_string().unwrap_or_else(|| format!("{v:?}"))
}

fn as_array(v: JsValue, context: &str) -> Result<js_sys::Array, Error> {
    v.dyn_into()
        .map_err(|_| Error::Js(format!("{context} didn't return an array")))
}

/// One `TransactionUnspentOutput = [transaction_input, transaction_output]`
/// CBOR entry, same shape `getUtxos()` returns -- a local decode (not
/// `ui::wasm`'s own `decode_unspent_output`) so this module's errors
/// stay this module's own `Error`, not a second, unrelated `JsError`
/// threaded through.
fn decode_unspent_output(hex_cbor: &str) -> Result<(Input, Output), Error> {
    let bytes = hex::decode(hex_cbor)?;
    let mut decoder = cardano_sdk::cbor::Decoder::new(&bytes);
    decoder
        .array()
        .map_err(|e| Error::BadUtxoCbor(e.to_string()))?;
    let input: Input = decoder
        .decode()
        .map_err(|e| Error::BadUtxoCbor(e.to_string()))?;
    let output: Output = decoder
        .decode()
        .map_err(|e| Error::BadUtxoCbor(e.to_string()))?;
    Ok((input, output))
}

pub struct Cip30 {
    api: JsValue,
}

impl Cip30 {
    pub fn new(api: JsValue) -> Self {
        Self { api }
    }

    async fn call(&self, method: &str, args: &[JsValue]) -> Result<JsValue, Error> {
        let f: Function = Reflect::get(&self.api, &JsValue::from_str(method))
            .map_err(|e| Error::Js(js_display(e)))?
            .dyn_into()
            .map_err(|_| Error::Js(format!("{method} is not a function")))?;
        let args_array = js_sys::Array::new();
        for a in args {
            args_array.push(a);
        }
        let promise: Promise = Reflect::apply(&f, &self.api, &args_array)
            .map_err(|e| Error::Js(js_display(e)))?
            .dyn_into()
            .map_err(|_| Error::Js(format!("{method} didn't return a promise")))?;
        JsFuture::from(promise)
            .await
            .map_err(|e| Error::Js(js_display(e)))
    }

    async fn call_str(&self, method: &str, args: &[JsValue]) -> Result<String, Error> {
        self.call(method, args)
            .await?
            .as_string()
            .ok_or_else(|| Error::Js(format!("{method} didn't return a string")))
    }
}

impl Wallet for Cip30 {
    type Error = Error;

    async fn network_id(&self) -> Result<NetworkId, Self::Error> {
        let v = self.call("getNetworkId", &[]).await?;
        let id = v
            .as_f64()
            .ok_or_else(|| Error::Js("getNetworkId didn't return a number".into()))?;
        // CIP-30 only ever distinguishes testnet (0) from mainnet (1) --
        // it has no notion of preprod vs preview (that's the chain's
        // own network magic, never surfaced to a dApp). Same limit
        // `app.js`'s own `networkMismatch` already lives with.
        let network = if id as u64 == 1 {
            cardano_sdk::Network::Mainnet
        } else {
            cardano_sdk::Network::Preprod
        };
        Ok(NetworkId::from(network))
    }

    async fn change_address(&self) -> Result<Address<kind::Any>, Self::Error> {
        let used = as_array(
            self.call("getUsedAddresses", &[]).await?,
            "getUsedAddresses",
        )?;
        let hex_addr = if used.length() > 0 {
            used.get(0).as_string().ok_or(Error::NoAddress)?
        } else {
            let unused = as_array(
                self.call("getUnusedAddresses", &[]).await?,
                "getUnusedAddresses",
            )?;
            if unused.length() == 0 {
                return Err(Error::NoAddress);
            }
            unused.get(0).as_string().ok_or(Error::NoAddress)?
        };
        let bytes = hex::decode(&hex_addr)?;
        Address::<kind::Any>::try_from(bytes.as_slice()).map_err(|_| Error::Address(hex_addr))
    }

    async fn utxos(
        &mut self,
        _value: Option<Value<u64>>,
    ) -> Result<Option<BTreeMap<Input, Output>>, Self::Error> {
        // No pagination/value-hint argument -- matches `app.js`'s own
        // prior `wallet.api.getUtxos()` call exactly (every utxo, every
        // time); `Embedded::utxos` is the one that actually honors a
        // value hint, since CIP-30 wallets are free to ignore theirs
        // anyway.
        let v = self.call("getUtxos", &[]).await?;
        if v.is_null() || v.is_undefined() {
            return Ok(None);
        }
        let entries = as_array(v, "getUtxos")?;
        let mut out = BTreeMap::new();
        for entry in entries.iter() {
            let hex_cbor = entry.as_string().ok_or(Error::BadUtxoEntry)?;
            let (input, output) = decode_unspent_output(&hex_cbor)?;
            out.insert(input, output);
        }
        Ok(Some(out))
    }

    async fn sign_tx(
        &self,
        tx: &Transaction<state::ReadyForSigning>,
    ) -> Result<(VerificationKey, Signature), Self::Error> {
        let tx_hex = hex::encode(tx.to_cbor());
        let witness_hex = self
            .call_str(
                "signTx",
                &[JsValue::from_str(&tx_hex), JsValue::from_bool(true)],
            )
            .await?;
        let bytes = hex::decode(&witness_hex)?;
        Ok(subbit_wallet::decode_single_vkey_witness(&bytes)?)
    }

    async fn submit(
        &mut self,
        tx: &Transaction<state::ReadyForSigning>,
    ) -> Result<Hash<32>, Self::Error> {
        let tx_hex = hex::encode(tx.to_cbor());
        let hash_hex = self
            .call_str("submitTx", &[JsValue::from_str(&tx_hex)])
            .await?;
        Hash::<32>::from_str(&hash_hex).map_err(Error::Js)
    }
}
