use amaru_kernel::{Address, Credential, StakeReference};
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

pub async fn search_all(
    client: &mut impl Backend,
    pay_hash: [u8; 28],
    stake_hash: Option<[u8; 28]>,
) -> floe_client::Result<Vec<Utxo>> {
    let (items, _ledger_tip) = search(client, pay_hash, stake_hash, None).await?;
    Ok(items)
}

pub async fn search_history(
    client: &mut impl Backend,
    pay_hash: [u8; 28],
    stake_hash: Option<[u8; 28]>,
) -> floe_client::Result<(Vec<Utxo>, ChainPoint)> {
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

pub struct ParsedChannel {
    pub stake_hash: Option<[u8; 28]>,
    pub constants: Constants,
    pub stage: Stage,
    pub amount: u64,
}

fn hash28(h: impl AsRef<[u8]>) -> Option<[u8; 28]> {
    h.as_ref().try_into().ok()
}

struct Credentials {
    pay_hash: Option<[u8; 28]>,
    stake_hash: Option<[u8; 28]>,
}

fn shelley_credentials(addr: &Address) -> Option<Credentials> {
    let Address::Shelley(s) = addr else {
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
    Some(Credentials {
        pay_hash,
        stake_hash,
    })
}

pub fn parse_channel_utxo(utxo: &Utxo) -> Result<ParsedChannel, ChannelError> {
    let Credentials {
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
