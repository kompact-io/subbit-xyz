//! Reads live chain state straight from Floe (`SearchUtxos`/`ReadTx`) and
//! submits through it (`SubmitTx`) -- Floe is this service's *only* chain
//! backend now (no cardano-connector/cardano-session involved at all).
//! Floe doesn't serve protocol parameters (see its own README), so those
//! come from a hardcoded per-network snapshot instead -- see
//! `crate::config::Config::network_parameters`.

use std::str::FromStr;
use std::time::Duration;

use amaru_kernel::{Address as KernelAddress, Credential};
use floe_client::{Backend, grpc};
use floe_dto_http::query::{ReadTxRequest, SearchUtxosRequest, TxoRef, Utxo};
use serde::{Deserialize, Serialize};
use subbit_l1_dto::{Asset, Datum};
use subbit_tx::{Channel, MIN_ADA_BUFFER, VALIDATOR, Variables};

use cardano_sdk::{
    Address, Hash, Input, Output, PlutusData, Transaction, Value, address::kind, cbor::ToCbor,
    transaction::state::ReadyForSigning,
};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    /// Floe's gRPC address, e.g. `http://127.0.0.1:50051`.
    pub endpoint: String,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            endpoint: "http://127.0.0.1:50051".to_string(),
        }
    }
}

impl Config {
    pub async fn connect(&self) -> anyhow::Result<grpc::Client> {
        Ok(grpc::Client::connect(&self.endpoint).await?)
    }
}

/// Paginates `SearchUtxos` until exhausted, filtered to the live
/// (unspent) set matching `pay_hash`/`stake_hash` -- the one primitive
/// every other search in this module builds on.
async fn search(
    client: &mut grpc::Client,
    pay_hash: [u8; 28],
    stake_hash: Option<[u8; 28]>,
) -> anyhow::Result<Vec<Utxo>> {
    let mut items = Vec::new();
    let mut start_token = None;
    loop {
        let resp = client
            .search_utxos(SearchUtxosRequest {
                pay_hash: Some(pay_hash),
                stake_hash,
                start_token,
                ..Default::default()
            })
            .await?;
        items.extend(resp.items);
        start_token = resp.next_token;
        if start_token.is_none() {
            break;
        }
    }
    Ok(items)
}

/// Every live channel UTxO (any delegation -- `stake_hash` is left
/// unset, so this matches the subbit validator's payment credential
/// regardless of which stake credential a given channel delegates to).
pub async fn search_all(client: &mut grpc::Client) -> anyhow::Result<Vec<Utxo>> {
    search(client, <[u8; 28]>::from(VALIDATOR.hash), None).await
}

/// Every live UTxO at a plain (non-channel) address, identified by its
/// payment credential hash (and, optionally, a specific stake
/// credential) -- used for the fee-wallet's own utxos (`fuel`), for
/// locating the validator's reference script, and (via
/// `canary::floe_connector`) for a generic `CardanoConnector::utxos_at`.
pub async fn search_by_pay_hash(
    client: &mut grpc::Client,
    pay_hash: [u8; 28],
    stake_hash: Option<[u8; 28]>,
) -> anyhow::Result<Vec<Utxo>> {
    search(client, pay_hash, stake_hash).await
}

#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("non-Shelley address, or payment credential isn't a script")]
    Address,
    #[error("payment credential does not match the subbit validator")]
    Credential,
    #[error("missing or unresolved inline datum")]
    Datum,
    #[error("failed to parse datum: {0}")]
    ParseDatum(String),
    #[error("datum own_hash does not match the subbit validator")]
    OwnHash,
}

/// Payment part must specifically be a *Script* credential -- see
/// `subbit_index::cardano::shelley_credentials`'s own doc for why a
/// bare hash match (ignoring Key vs Script) isn't safe to trust here.
fn script_pay_hash(addr: &KernelAddress) -> Option<[u8; 28]> {
    let KernelAddress::Shelley(s) = addr else {
        return None;
    };
    match s.payment() {
        Credential::ScriptHash(h) => h.as_ref().try_into().ok(),
        Credential::KeyHash(_) => None,
    }
}

/// Parses a channel out of a Floe `Utxo`. Mirrors `subbit_tx::Channel`'s
/// `TryFrom<&cardano_sdk::Output>`, reading the same fields off Floe's
/// wire DTO instead. Used only to decide *what* to claim -- building the
/// actual transaction goes through `to_output`/`to_input_output` below,
/// which keeps the full `cardano_sdk::Output` instead of this crate's own
/// cut-down `Channel` view.
pub fn channel_from_utxo(utxo: &Utxo) -> Result<Channel, ChannelError> {
    let pay_hash = script_pay_hash(&utxo.output.address.0).ok_or(ChannelError::Address)?;
    if pay_hash != <[u8; 28]>::from(VALIDATOR.hash) {
        return Err(ChannelError::Credential);
    }
    let cbor = utxo
        .output
        .datum
        .as_ref()
        .and_then(|d| d.original_cbor.as_deref())
        .ok_or(ChannelError::Datum)?;
    let Datum {
        own_hash,
        constants,
        stage,
    } = minicbor::decode(cbor).map_err(|e| ChannelError::ParseDatum(e.to_string()))?;
    if <[u8; 28]>::from(own_hash) != <[u8; 28]>::from(VALIDATOR.hash) {
        return Err(ChannelError::OwnHash);
    }
    let amount = match constants.asset() {
        Asset::Lovelace => utxo.output.coin.saturating_sub(MIN_ADA_BUFFER),
        Asset::Native { hash, name } => utxo
            .output
            .assets
            .iter()
            .find(|ma| ma.policy_id.as_slice() == AsRef::<[u8]>::as_ref(hash))
            .and_then(|ma| ma.assets.iter().find(|a| &a.name == name))
            .map(|a| a.quantity)
            .unwrap_or(0),
    };
    Ok(Channel::new(constants, Variables::new(amount, stage)))
}

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
/// `cardano_sdk::Output` the tx builder (`subbit_tx::Tx`) actually needs
/// -- address, value (incl. native assets), datum and (when present) a
/// reference script, byte-for-byte off the wire. This *must* round-trip
/// `script` faithfully, not just for channel/fuel outputs (which never
/// carry one in practice): `FloeConnector` (`packages/canary`) reuses
/// this for every tracked address generically, including a wallet's own,
/// and `cardano_session::Session::ref_script`'s generic script-discovery
/// depends on `Output::script()` actually being populated -- dropping it
/// here silently broke that path (a live preprod ref-script upload
/// landed and indexed fine, but `ref_script()` never found it, because
/// every `utxos_at` call routed through this function first).
/// `floe_dto_http::cardano::ScriptLang`'s discriminant doubles as the
/// real blake2b-224 lang-tag byte, matching `cardano_sdk::PlutusVersion`'s
/// own (`TryFrom<u8>`) -- `Native`/`PlutusV4` have no `PlutusVersion`
/// equivalent (`cardano_sdk::Output`'s own doc: "Native scripts as
/// reference scripts aren't yet supported"), so those error out rather
/// than silently dropping the script again.
pub fn to_output(output: &floe_dto_http::cardano::TxOutput) -> Result<Output, OutputError> {
    // Floe's wire DTOs carry `amaru_kernel::address::Address`;
    // `cardano_sdk` still pins its own `pallas-addresses` -- the two
    // `Address` types can't convert directly, so bech32 text is the
    // crate-agnostic bridge (also how `query::Address`'s own
    // `Serialize` works, over the wire). `to_bech32` is `None` only for
    // a Byron address, which the validator never produces.
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

/// Locates the subbit validator's reference script among the live
/// utxos at `host_pay_hash` (by default the fee wallet's own address --
/// see `crate::config::Config::ref_script_host`). Recognised by an exact
/// match against the already-known `VALIDATOR.script` bytes, not by
/// re-hashing Floe's returned script -- see `to_output`'s own doc.
pub async fn find_ref_script(
    client: &mut grpc::Client,
    host_pay_hash: [u8; 28],
) -> anyhow::Result<Option<(Input, Output)>> {
    let utxos = search_by_pay_hash(client, host_pay_hash, None).await?;
    for utxo in &utxos {
        let is_ref_script = utxo
            .output
            .script
            .as_ref()
            .is_some_and(|s| s.cbor.as_slice() == VALIDATOR.script.script());
        if is_ref_script {
            let input = to_input(&utxo.txo_ref);
            let output = to_output(&utxo.output)?.with_plutus_script(VALIDATOR.script.clone());
            return Ok(Some((input, output)));
        }
    }
    Ok(None)
}

/// Submits a fully-signed transaction through Floe's `SubmitTx`, and
/// returns its id -- `SubmitTxResponse`'s own `ref` field is an opaque,
/// server-specific handle, not necessarily the tx hash, so the id is
/// computed locally instead (same as `cardano_wallet::Embedded::submit`
/// used to).
pub async fn submit(
    client: &mut grpc::Client,
    tx: &Transaction<ReadyForSigning>,
) -> anyhow::Result<Hash<32>> {
    submit_cbor(client, &tx.to_cbor()).await?;
    Ok(tx.id())
}

/// Submits already-serialized transaction bytes. Used both by `submit` and to
/// rebroadcast a still-pending tx byte-for-byte from `pending::Pending`.
pub async fn submit_cbor(client: &mut grpc::Client, cbor: &[u8]) -> anyhow::Result<()> {
    client.submit_tx(cbor.to_vec()).await?;
    Ok(())
}

/// Whether Floe has observed `hash` (i.e. seen it included in a block).
pub async fn read_tx(client: &mut grpc::Client, hash: Hash<32>) -> anyhow::Result<bool> {
    Ok(client
        .read_tx(ReadTxRequest {
            hash: <[u8; 32]>::from(hash),
        })
        .await?
        .is_some())
}

/// Polls `ReadTx` until Floe has indexed `hash` (i.e. seen it included
/// in a block -- `ReadTx` only ever resolves a tx that's actually
/// landed, see `floe-server`'s own `Ctx::read_tx`), or gives up after
/// `max_attempts`.
pub async fn wait_for_tx(
    client: &mut grpc::Client,
    hash: Hash<32>,
    max_attempts: u32,
    interval: Duration,
) -> anyhow::Result<()> {
    for attempt in 0..max_attempts {
        if read_tx(client, hash).await? {
            return Ok(());
        }
        if attempt + 1 == max_attempts {
            break;
        }
        tokio::time::sleep(interval).await;
    }
    anyhow::bail!("timed out waiting for Floe to observe tx {hash}")
}
