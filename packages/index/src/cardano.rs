use amaru_kernel::{Address as KernelAddress, Credential, StakeReference};
use floe_client::Backend;
#[cfg(feature = "grpc")]
use floe_client::grpc;
use floe_dto_http::query::{ChainPoint, SearchUtxosRequest, Utxo};
use serde::{Deserialize, Serialize};
use subbit_l1_dto::{Asset, Constants, Datum, Stage};
use subbit_tx::{Channel, MIN_ADA_BUFFER, VALIDATOR, Variables};

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

#[cfg(feature = "grpc")]
impl Config {
    pub async fn connect(&self) -> floe_client::Result<grpc::Client> {
        grpc::Client::connect(&self.endpoint).await
    }
}

/// Every live (unspent) UTxO matching `pay_hash`/`stake_hash`, paginating
/// Floe's `SearchUtxos` until `next_token` is exhausted. No `since` is
/// passed, so Floe returns only the current live set (no `spent_by`
/// items) -- exactly what the naive (non-lineage) poller needs.
///
/// Generic over `impl Backend`, not tied to `grpc::Client` -- the same
/// code already works against any transport implementing it, native
/// (`grpc`, a future `http-json`/`http-cbor`) or wasm32
/// (`subbit_console::wasm::transport::Client`, Console's own fetch
/// impl -- not `floe-client`'s own `wasm` feature, see this crate's
/// `[features]` doc).
pub async fn search_all(
    client: &mut impl Backend,
    pay_hash: [u8; 28],
    stake_hash: Option<[u8; 28]>,
) -> floe_client::Result<Vec<Utxo>> {
    let (items, _ledger_tip) = search(client, pay_hash, stake_hash, None).await?;
    Ok(items)
}

/// Every TxO (live or spent) ever matching `pay_hash`/`stake_hash`, as
/// far back as Floe's own retention goes -- unlike `search_all`, this
/// includes `spent_by` items, which lineage reconstruction needs.
/// Also returns the ledger tip observed while paginating.
pub async fn search_history(
    client: &mut impl Backend,
    pay_hash: [u8; 28],
    stake_hash: Option<[u8; 28]>,
) -> floe_client::Result<(Vec<Utxo>, ChainPoint)> {
    // `since_slot: Some(0)` is a permissive "slot > 0" filter (no
    // chain-validity check, see `SearchUtxosRequest::since_hash`'s own
    // doc) -- the cheapest way to ask for "every row, from genesis"
    // while still setting `since` so Floe includes spent items.
    search(client, pay_hash, stake_hash, Some(0)).await
}

async fn search(
    client: &mut impl Backend,
    pay_hash: [u8; 28],
    stake_hash: Option<[u8; 28]>,
    since_slot: Option<u64>,
) -> floe_client::Result<(Vec<Utxo>, ChainPoint)> {
    let mut items = Vec::new();
    let mut start_token = None;
    let ledger_tip = loop {
        let resp = client
            .search_utxos(SearchUtxosRequest {
                pay_hash: Some(pay_hash),
                stake_hash,
                since_slot,
                start_token,
                ..Default::default()
            })
            .await?;
        items.extend(resp.items);
        start_token = resp.next_token;
        if start_token.is_none() {
            break resp.ledger_tip;
        }
    };
    Ok((items, ledger_tip))
}

#[derive(Debug, thiserror::Error)]
pub enum ChannelError {
    #[error("non-Shelley address, or no payment credential")]
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

/// The fields a channel's well-formedness check yields, before they're
/// assembled into a `subbit_tx::Channel` (which drops `stake_hash` --
/// `lineage` needs it, `channel_from_utxo`'s callers don't).
pub struct ParsedChannel {
    pub stake_hash: Option<[u8; 28]>,
    pub constants: Constants,
    pub stage: Stage,
    pub amount: u64,
}

fn hash28(h: impl AsRef<[u8]>) -> Option<[u8; 28]> {
    h.as_ref().try_into().ok()
}

struct ShelleyCredentials {
    /// `None` unless the payment part is specifically a *Script*
    /// credential -- see `shelley_credentials`.
    pay_hash: Option<[u8; 28]>,
    stake_hash: Option<[u8; 28]>,
}

/// Payment and delegation credential hashes of `addr`. Unlike
/// `floe_client::address_hashes` (which returns a bare hash either
/// way), the payment part must specifically be a *Script* credential:
/// our validator's address always locks funds behind its script hash,
/// never a key. Collapsing that distinction would let someone
/// construct an address whose payment part is flagged `Key` but
/// numerically equal to our validator's script hash -- the ledger
/// would then never invoke the validator to spend it, so it isn't
/// really a backing no matter what its datum claims. Cheap to
/// exclude, so we do; the delegation part has no such requirement
/// (a consumer's own stake credential may legitimately be either).
fn shelley_credentials(addr: &KernelAddress) -> Option<ShelleyCredentials> {
    let KernelAddress::Shelley(s) = addr else {
        return None;
    };
    let pay_hash = match s.payment() {
        Credential::ScriptHash(h) => hash28(h),
        Credential::KeyHash(_) => None,
    };
    let stake_hash = match s.delegation() {
        Some(StakeReference::Credential(Credential::KeyHash(h) | Credential::ScriptHash(h))) => {
            hash28(h)
        }
        Some(StakeReference::Pointer(_)) | None => None,
    };
    Some(ShelleyCredentials {
        pay_hash,
        stake_hash,
    })
}

/// Parses channel fields out of a Floe `Utxo`. Mirrors
/// `subbit_tx::Channel`'s `TryFrom<&cardano_sdk::Output>`, reading the
/// same fields off Floe's wire DTO instead.
pub fn parse_channel_utxo(utxo: &Utxo) -> Result<ParsedChannel, ChannelError> {
    let ShelleyCredentials {
        pay_hash,
        stake_hash,
    } = shelley_credentials(&utxo.output.address.0).ok_or(ChannelError::Address)?;
    if pay_hash != Some(<[u8; 28]>::from(VALIDATOR.hash)) {
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
    Ok(ParsedChannel {
        stake_hash,
        constants,
        stage,
        amount,
    })
}

/// Parses a channel out of a Floe `Utxo`, dropping the stake hash --
/// see `parse_channel_utxo`.
pub fn channel_from_utxo(utxo: &Utxo) -> Result<Channel, ChannelError> {
    let ParsedChannel {
        constants,
        stage,
        amount,
        ..
    } = parse_channel_utxo(utxo)?;
    Ok(Channel::new(constants, Variables::new(amount, stage)))
}

#[cfg(all(test, feature = "grpc"))]
mod preprod_smoke {
    use floe_client::Backend;

    #[tokio::test]
    #[ignore = "hits the real preprod Floe instance; run manually with --ignored"]
    async fn connects_and_reports_health() {
        let mut client = super::grpc::Client::connect("https://grpc.floe.kompact.io")
            .await
            .expect("connect");
        let health = client.get_health().await.expect("get_health");
        println!("{health:?}");
    }
}
