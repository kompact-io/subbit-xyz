//! Floe wire-DTO -> `cardano_sdk` conversion. A third independent copy
//! of `packages/tx/src/floe.rs`'s own `to_output`/`to_input`/
//! `to_input_output` (itself already "duplicated from, not shared
//! with, `subbit-claim`'s own native-only `src/floe.rs`" --
//! `plan-subbit-console.md` section B) -- this crate is meant to have
//! no dependency on `subbit-tx`/`subbit-claim` at all (see
//! `plan-embedded-wallet.md`), and this conversion is small, generic
//! (no subbit-protocol knowledge -- no `Constants`/`Variables`/keytags
//! anywhere in it), and already proven safe to copy per-consumer rather
//! than share.
//!
//! Also carries `search_live`, this crate's own small pagination loop
//! over `Backend::search_utxos` -- a fresh ~15-line copy of
//! `subbit_index::cardano::search_all`'s own loop, for the same
//! no-shared-dependency reason, not a port of that function's code
//! (which isn't available to this crate either).

use std::collections::BTreeMap;
use std::str::FromStr;

use floe_client::Backend;
use floe_dto_http::query::{SearchUtxosRequest, TxoRef, Utxo};

use cardano_sdk::{Address, Hash, Input, Output, PlutusData, Value, address::kind};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("unsupported address (not Shelley/Byron)")]
    Address,
    #[error("asset policy id is not 28 bytes")]
    PolicyId,
    #[error("failed to decode inline datum: {0}")]
    Datum(String),
    #[error(
        "unsupported reference script language: {0:?} -- cardano_sdk::PlutusScript only represents V1/V2/V3"
    )]
    ScriptLang(floe_dto_http::cardano::ScriptLang),
}

/// Full-fidelity conversion of a Floe `TxOutput` into the
/// `cardano_sdk::Output` a tx builder needs -- address, value (incl.
/// native assets), datum and (when present) a reference script,
/// byte-for-byte off the wire.
pub fn to_output(output: &floe_dto_http::cardano::TxOutput) -> Result<Output, Error> {
    let bech32 = output.address.0.to_bech32().ok_or(Error::Address)?;
    let address = Address::<kind::Any>::from_str(&bech32).map_err(|_| Error::Address)?;

    let assets = output
        .assets
        .iter()
        .map(|ma| {
            let policy: [u8; 28] = ma
                .policy_id
                .as_slice()
                .try_into()
                .map_err(|_| Error::PolicyId)?;
            let names = ma
                .assets
                .iter()
                .map(|a| (a.name.clone(), a.quantity))
                .collect::<Vec<_>>();
            Ok((Hash::<28>::from(policy), names))
        })
        .collect::<Result<Vec<_>, Error>>()?;

    let value = Value::new(output.coin).with_assets(assets);
    let mut out = Output::new(address, value);

    if let Some(datum) = &output.datum {
        out = match &datum.original_cbor {
            Some(bytes) => {
                let data: PlutusData =
                    minicbor_old::decode(bytes).map_err(|e| Error::Datum(e.to_string()))?;
                out.with_datum(data)
            }
            None => out.with_datum_hash(Hash::<32>::from(datum.hash)),
        };
    }

    if let Some(script) = &output.script {
        let version = cardano_sdk::PlutusVersion::try_from(script.lang as u8)
            .map_err(|_| Error::ScriptLang(script.lang))?;
        out = out.with_plutus_script(cardano_sdk::PlutusScript::new(version, script.cbor.clone()));
    }

    Ok(out)
}

pub fn to_input(txo_ref: &TxoRef) -> Input {
    Input::new(Hash::<32>::from(txo_ref.hash), txo_ref.index as u64)
}

pub fn to_input_output(utxo: &Utxo) -> Result<(Input, Output), Error> {
    Ok((to_input(&utxo.txo_ref), to_output(&utxo.output)?))
}

/// Every *live* (unspent) UTxO matching `pay_hash`, paginating Floe's
/// `SearchUtxos` until `next_token` is exhausted. No `since` passed, so
/// only the current live set comes back (no `spent_by` items) -- this
/// crate only ever needs live UTXOs to fund a transaction, never
/// history.
pub async fn search_live(
    backend: &mut impl Backend,
    pay_hash: [u8; 28],
) -> floe_client::Result<BTreeMap<Input, Output>> {
    let mut out = BTreeMap::new();
    let mut start_token = None;
    loop {
        let response = backend
            .search_utxos(SearchUtxosRequest {
                pay_hash: Some(pay_hash),
                start_token,
                ..Default::default()
            })
            .await?;
        for utxo in &response.items {
            if let Ok(pair) = to_input_output(utxo) {
                out.insert(pair.0, pair.1);
            }
        }
        start_token = response.next_token;
        if start_token.is_none() {
            break;
        }
    }
    Ok(out)
}
