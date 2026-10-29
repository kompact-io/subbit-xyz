//! Floe I/O glue, generic over `floe_client::Backend` -- never tied to
//! a transport, so `floe_client::http::json::Client` (wasm, via
//! `"http-json"`'s wasm32 `gloo-net` substitute) and any future native
//! transport (same `Client`, via `reqwest`) both just work.
//!
//! Just `discover_history`, reusing `subbit_index::cardano::
//! search_history` per whitelisted validator (plan section K), not a
//! port. There used to be a second function, `discover_live` (`search_all`,
//! live-only), but every live-only need (the basket's own live-txo
//! cache, `basketBuild`'s reference-script lookup) is a strict subset
//! of what `discover_history` already returns -- `Utxo::spent_by ==
//! None` picks out exactly the live ones (see `subbit_index::cardano::
//! search_history`'s own doc: "unlike `search_all`, this includes
//! `spent_by` items"). One Floe round-trip now backs the Channels
//! tab's lineage view, the live-txo cache, *and* submit's
//! reference-script lookup -- `ui::wasm`/`app.js` filter the one
//! response rather than this crate fetching live/history separately.

use std::collections::BTreeSet;

use cardano_sdk::Hash;
use floe_client::Backend;
use floe_dto_http::query::{ChainPoint, Utxo};

/// Every TxO (live or spent) at any whitelisted validator's payment
/// credential (any stake credential), far back as Floe's retention
/// goes -- feeds the lineage view (`subbit_index::lineage::
/// reconstruct`'s own input) directly, and (filtered to `spent_by ==
/// None`) every live-only need too. Ledger tip is merged by taking the
/// tallest height across validators. A Floe-side error for one
/// validator doesn't abort the rest, matching `Basket::
/// from_floe_utxos`'s own permissive construction.
pub async fn discover_history(
    backend: &mut impl Backend,
    validators: &BTreeSet<Hash<28>>,
) -> anyhow::Result<(Vec<Utxo>, ChainPoint)> {
    let mut items = Vec::new();
    let mut tip = ChainPoint::default();
    for validator in validators {
        let pay_hash: [u8; 28] = (*validator).into();
        let (validator_items, validator_tip) =
            subbit_index::cardano::search_history(backend, pay_hash, None).await?;
        items.extend(validator_items);
        if validator_tip.height > tip.height {
            tip = validator_tip;
        }
    }
    Ok((items, tip))
}

#[cfg(test)]
mod tests {
    use super::*;
    use floe_dto_http::health::{HealthResponse, MetricsResponse};
    use floe_dto_http::query::{
        ReadDataRequest, ReadDataResponse, ReadTxRequest, ReadTxResponse, ReadUtxosRequest,
        ReadUtxosResponse, SearchUtxosRequest, SearchUtxosResponse, TxoRef,
    };
    use floe_dto_http::submit::SubmitTxResponse;

    /// Canned per-`pay_hash` responses, one page each -- enough to
    /// exercise multi-validator merging without a real server.
    struct MockBackend(std::collections::BTreeMap<[u8; 28], SearchUtxosResponse>);

    impl Backend for MockBackend {
        async fn read_utxos(
            &mut self,
            _: ReadUtxosRequest,
        ) -> floe_client::Result<ReadUtxosResponse> {
            unimplemented!()
        }
        async fn search_utxos(
            &mut self,
            req: SearchUtxosRequest,
        ) -> floe_client::Result<SearchUtxosResponse> {
            Ok(self
                .0
                .get(&req.pay_hash.unwrap())
                .cloned()
                .unwrap_or(SearchUtxosResponse {
                    items: vec![],
                    ledger_tip: ChainPoint::default(),
                    next_token: None,
                }))
        }
        async fn read_data(
            &mut self,
            _: ReadDataRequest,
        ) -> floe_client::Result<Option<ReadDataResponse>> {
            unimplemented!()
        }
        async fn read_tx(
            &mut self,
            _: ReadTxRequest,
        ) -> floe_client::Result<Option<ReadTxResponse>> {
            unimplemented!()
        }
        async fn submit_tx(&mut self, _: Vec<u8>) -> floe_client::Result<SubmitTxResponse> {
            unimplemented!()
        }
        async fn get_health(&mut self) -> floe_client::Result<HealthResponse> {
            unimplemented!()
        }
        async fn get_metrics(&mut self) -> floe_client::Result<MetricsResponse> {
            unimplemented!()
        }
    }

    fn utxo(index: u16) -> Utxo {
        Utxo {
            txo_ref: TxoRef { hash: [0u8; 32], index },
            output: floe_dto_http::cardano::TxOutput {
                address: floe_dto_http::query::Address(
                    "addr1qxu84ftxpzh3zd8p9awp2ytwzk5exj0fxcj7paur4kd4ytun36yuhgl049rxhhuckm2lpq3rmz5dcraddyl45d6xgvqqsp504c"
                        .parse()
                        .unwrap(),
                ),
                coin: 0,
                assets: vec![],
                datum: None,
                script: None,
            },
            block_ref: ChainPoint::default(),
            spent_by: None,
        }
    }

    #[tokio::test]
    async fn merges_history_and_takes_tallest_tip() {
        let v1 = Hash::<28>::from([1u8; 28]);
        let v2 = Hash::<28>::from([2u8; 28]);
        let mut backend = MockBackend(std::collections::BTreeMap::from([
            (
                <[u8; 28]>::from(v1),
                SearchUtxosResponse {
                    items: vec![utxo(0)],
                    ledger_tip: ChainPoint {
                        height: 10,
                        ..Default::default()
                    },
                    next_token: None,
                },
            ),
            (
                <[u8; 28]>::from(v2),
                SearchUtxosResponse {
                    items: vec![utxo(1)],
                    ledger_tip: ChainPoint {
                        height: 20,
                        ..Default::default()
                    },
                    next_token: None,
                },
            ),
        ]));

        let (items, tip) = discover_history(&mut backend, &BTreeSet::from([v1, v2]))
            .await
            .unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(tip.height, 20);
    }
}
