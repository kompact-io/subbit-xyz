//! Floe wire-DTO -> `cardano_sdk` conversion. Duplicated from (not shared
//! with, not upstreamed from) `subbit-claim`'s own native-only
//! `src/floe.rs` -- see `plan-subbit-console.md` section B for why. This
//! copy keeps only the pure data conversions (`to_output`/`to_input`/
//! `to_input_output`); the networking (`grpc::Client`, `search_utxos`,
//! `submit`, `wait_for_tx`) stays in `claim`, which is the only consumer
//! that needs it -- Console talks to Floe over its own `Backend` impl
//! instead (`packages/console/src/wasm/transport.rs`), not `floe-client`'s
//! (`floe_dto_http` is a pure-DTO dependency here, no `floe-client`/
//! transport pulled in, so this stays wasm32-buildable).

use std::str::FromStr;

use floe_dto_http::query::{TxoRef, Utxo};

use cardano_sdk::{Address, Hash, Input, Output, PlutusData, Value, address::kind};

#[derive(Debug, thiserror::Error)]
pub enum OutputError {
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
/// `cardano_sdk::Output` the tx builder (`crate::tx::Tx`) actually needs
/// -- address, value (incl. native assets), datum and (when present) a
/// reference script, byte-for-byte off the wire. See `subbit_claim::
/// floe::to_output`'s own doc for why this must round-trip `script`
/// faithfully and why bech32 text (not `amaru_kernel::address::Address`
/// directly) is the bridge between Floe's wire DTOs and `cardano_sdk`'s
/// own (still pallas-backed) `Address`.
pub fn to_output(output: &floe_dto_http::cardano::TxOutput) -> Result<Output, OutputError> {
    let bech32 = output.address.0.to_bech32().ok_or(OutputError::Address)?;
    let address = Address::<kind::Any>::from_str(&bech32).map_err(|_| OutputError::Address)?;

    let assets = output
        .assets
        .iter()
        .map(|ma| {
            let policy: [u8; 28] = ma
                .policy_id
                .as_slice()
                .try_into()
                .map_err(|_| OutputError::PolicyId)?;
            let names = ma
                .assets
                .iter()
                .map(|a| (a.name.clone(), a.quantity))
                .collect::<Vec<_>>();
            Ok((Hash::<28>::from(policy), names))
        })
        .collect::<Result<Vec<_>, OutputError>>()?;

    let value = Value::new(output.coin).with_assets(assets);
    let mut out = Output::new(address, value);

    if let Some(datum) = &output.datum {
        out = match &datum.original_cbor {
            Some(bytes) => {
                let data: PlutusData =
                    minicbor_old::decode(bytes).map_err(|e| OutputError::Datum(e.to_string()))?;
                out.with_datum(data)
            }
            None => out.with_datum_hash(Hash::<32>::from(datum.hash)),
        };
    }

    if let Some(script) = &output.script {
        let version = cardano_sdk::PlutusVersion::try_from(script.lang as u8)
            .map_err(|_| OutputError::ScriptLang(script.lang))?;
        out = out.with_plutus_script(cardano_sdk::PlutusScript::new(version, script.cbor.clone()));
    }

    Ok(out)
}

pub fn to_input(txo_ref: &TxoRef) -> Input {
    Input::new(Hash::<32>::from(txo_ref.hash), txo_ref.index as u64)
}

pub fn to_input_output(utxo: &Utxo) -> Result<(Input, Output), OutputError> {
    Ok((to_input(&utxo.txo_ref), to_output(&utxo.output)?))
}
