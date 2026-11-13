//! Plain, queue-free transactions a single wallet can build and submit
//! on its own: `upload_reference_script` (needed before staging a
//! `Want` against an existing channel) and `send_payment`.

use std::collections::BTreeMap;

use cardano_sdk::{
    ChangeStrategy, Hash, Input, Output, Transaction, address::kind,
    transaction::state::ReadyForSigning,
};
use subbit_tx::{NetworkParameters, VALIDATOR};
use subbit_wallet::{LandingPolicy, Wallet, wait_for_spent_with};

use crate::Error;

/// Reference-script outputs can't be spent as ordinary fuel -- same
/// filter `cardano-wallet`'s own `txs::scriptless` applied.
fn scriptless(utxos: &BTreeMap<Input, Output>) -> BTreeMap<Input, Output> {
    utxos
        .iter()
        .filter(|(_, o)| o.script().is_none())
        .map(|(i, o)| (i.clone(), o.clone()))
        .collect()
}

/// Shared build -> sign -> submit plumbing for `upload_reference_script`,
/// `send_payment`, and `sweep` -- all three are a single-wallet, no-`Ctx`-
/// involved plain tx (no consumer/provider witnesses to aggregate, unlike
/// `submit`), so none needs `submit`'s own multi-signatory logic. Takes
/// `sender_utxos` explicitly rather than always querying `wallet.utxos(None)`
/// itself -- `sweep`'s own batching needs to build against one chunk of a
/// larger utxo set at a time, not everything the wallet holds.
#[tracing::instrument(skip(wallet, sender_utxos, outputs), ret(Debug), err(Debug))]
async fn build_sign_submit_with<W: Wallet>(
    wallet: &mut W,
    network: &str,
    sender_utxos: &BTreeMap<Input, Output>,
    change_address: cardano_sdk::Address<kind::Any>,
    outputs: Vec<Output>,
) -> Result<Hash<32>, Error> {
    let network_parameters =
        NetworkParameters::for_network(network).map_err(|e| Error::Network(e.to_string()))?;

    let inputs = sender_utxos.keys().map(|input| (input.clone(), None));
    let mut tx: Transaction<ReadyForSigning> = Transaction::build(
        &network_parameters.protocol_parameters,
        sender_utxos,
        |tx| {
            tx.with_inputs(inputs.to_owned())
                .with_outputs(outputs.to_owned())
                .with_change_strategy(ChangeStrategy::as_last_output(change_address.to_owned()))
                .ok()
        },
    )
    .map_err(|e| Error::Basket(e.to_string()))?;

    let (vkey, sig) = wallet
        .sign_tx(&tx)
        .await
        .map_err(|e| Error::Wallet(e.to_string()))?;
    tx.add_witness(vkey, sig);
    wallet
        .submit(&tx)
        .await
        .map_err(|e| Error::Wallet(e.to_string()))
}

async fn build_sign_submit<W: Wallet>(
    wallet: &mut W,
    network: &str,
    outputs: Vec<Output>,
) -> Result<Hash<32>, Error> {
    let change_address: cardano_sdk::Address<kind::Any> = wallet
        .change_address()
        .await
        .map_err(|e| Error::Wallet(e.to_string()))?;
    let utxos = wallet
        .utxos(None)
        .await
        .map_err(|e| Error::Wallet(e.to_string()))?
        .ok_or_else(|| Error::Wallet("wallet reported no coverable utxos".to_string()))?;
    let sender_utxos = scriptless(&utxos);
    build_sign_submit_with(wallet, network, &sender_utxos, change_address, outputs).await
}

/// A swept-output value below this is rejected by `Transaction::build` as under the real
/// protocol minUTxO (a plain lovelace-only output needs ~978_370 on preprod/mainnet as
/// measured live) -- kept comfortably above that rather than exact, same spirit as
/// `subbit_tx::queue`'s own size margins. Below this, there's nothing left worth sweeping.
const MIN_SWEEP_OUTPUT: u64 = 1_500_000;

/// Sweeps every scriptless utxo `wallet` holds to `target`, batching across as many
/// transactions as it takes (`max_inputs_per_tx` at a time) -- a wallet that's accumulated
/// far more utxos than fit in one tx (e.g. `subbit-claim`'s own change output, repeated
/// across many Sub/Settle batches over a long-running deployment) can still be fully
/// reclaimed, rather than `send_payment`'s single unbounded attempt blowing the tx size
/// budget outright. Waits for each batch to land before re-querying the wallet's utxos for
/// the next one, so a not-yet-confirmed spend is never mistaken for still-live and double-
/// spent into the next batch. `margin` is subtracted from each batch's own swept total as a
/// safety buffer, purely to decide (via `MIN_SWEEP_OUTPUT`) whether a batch is worth
/// building at all -- not used to size the real output. There's no explicit output: the
/// whole chunk's value (minus the real, measured fee) becomes the tx's *only* output, via
/// `ChangeStrategy::as_last_output(target)` -- an explicit `Output` alongside that change
/// strategy would split the sweep into two outputs at the same address, and whichever one
/// absorbed the fee's own slack would land under minUTxO on its own regardless of which
/// address it went to. `landing` controls how long each batch is waited for -- cleanup
/// callers can opt into a more patient policy than the shared default.
/// Returns the number of sweep transactions it took.
pub async fn sweep<W: Wallet>(
    wallet: &mut W,
    network: &str,
    target: cardano_sdk::Address<kind::Any>,
    max_inputs_per_tx: usize,
    margin: u64,
    landing: LandingPolicy,
) -> Result<usize, Error> {
    let mut batches = 0usize;
    loop {
        let utxos = wallet
            .utxos(None)
            .await
            .map_err(|e| Error::Wallet(e.to_string()))?
            .ok_or_else(|| Error::Wallet("wallet reported no coverable utxos".to_string()))?;
        let scriptless_utxos = scriptless(&utxos);
        if scriptless_utxos.is_empty() {
            break;
        }

        let chunk: BTreeMap<Input, Output> = scriptless_utxos
            .into_iter()
            .take(max_inputs_per_tx)
            .collect();
        let balance: u64 = chunk.values().map(|o| o.value().lovelace()).sum();
        if balance <= margin || balance - margin < MIN_SWEEP_OUTPUT {
            break;
        }

        let hash =
            build_sign_submit_with(wallet, network, &chunk, target.clone(), Vec::new()).await?;
        let spent = chunk.keys().cloned().collect();
        wait_for_spent_with(wallet, hash, &spent, crate::sleep::sleep, landing)
            .await
            .map_err(|e| Error::Wallet(e.to_string()))?;
        batches += 1;
    }
    Ok(batches)
}

/// Uploads the subbit validator as a reference script, at `address`
/// if given, else `wallet`'s own change address -- the common case,
/// a wallet hosting a reference script for its own future use.
pub async fn upload_reference_script<W: Wallet>(
    wallet: &mut W,
    network: &str,
    address: Option<cardano_sdk::Address<kind::Any>>,
) -> Result<Hash<32>, Error> {
    let address = match address {
        Some(address) => address,
        None => wallet
            .change_address()
            .await
            .map_err(|e| Error::Wallet(e.to_string()))?,
    };
    let output = Output::to(address).with_plutus_script(VALIDATOR.script.clone());
    build_sign_submit(wallet, network, vec![output]).await
}

/// `upload_reference_script`'s reverse: reclaims the reference-script
/// utxo `wallet` itself holds (found the same way `scriptless` finds
/// what to exclude -- any script-bearing utxo among what the wallet
/// can spend) back into an ordinary output at `wallet`'s own change
/// address, destroying the reference script in the process. Unlike
/// `upload`, there's no address to choose: this only ever reclaims a
/// utxo `wallet` already has spending authority over, so it can only
/// ever be one `wallet` itself previously uploaded (or otherwise
/// controls), not an arbitrary address.
pub async fn teardown_reference_script<W: Wallet>(
    wallet: &mut W,
    network: &str,
) -> Result<Hash<32>, Error> {
    let network_parameters =
        NetworkParameters::for_network(network).map_err(|e| Error::Network(e.to_string()))?;
    let change_address = wallet
        .change_address()
        .await
        .map_err(|e| Error::Wallet(e.to_string()))?;
    let utxos = wallet
        .utxos(None)
        .await
        .map_err(|e| Error::Wallet(e.to_string()))?
        .ok_or_else(|| Error::Wallet("wallet reported no coverable utxos".to_string()))?;
    let script_utxo: BTreeMap<Input, Output> = utxos
        .into_iter()
        .filter(|(_, o)| o.script().is_some())
        .take(1)
        .collect();
    if script_utxo.is_empty() {
        return Err(Error::Wallet(
            "wallet holds no reference-script utxo to tear down".to_string(),
        ));
    }

    let inputs = script_utxo.keys().map(|input| (input.clone(), None));
    let mut tx: Transaction<ReadyForSigning> = Transaction::build(
        &network_parameters.protocol_parameters,
        &script_utxo,
        |tx| {
            tx.with_inputs(inputs.to_owned())
                .with_change_strategy(ChangeStrategy::as_last_output(change_address.to_owned()))
                .ok()
        },
    )
    .map_err(|e| Error::Basket(e.to_string()))?;

    let (vkey, sig) = wallet
        .sign_tx(&tx)
        .await
        .map_err(|e| Error::Wallet(e.to_string()))?;
    tx.add_witness(vkey, sig);
    wallet
        .submit(&tx)
        .await
        .map_err(|e| Error::Wallet(e.to_string()))
}

/// A one-off plain payment from `wallet`'s own address to `address` --
/// e.g. seeding a separately-derived wallet (a spawned `subbit-claim`
/// process's own fee wallet) without routing it through a `Ctx`/
/// `Keyring`/basket at all.
pub async fn send_payment<W: Wallet>(
    wallet: &mut W,
    network: &str,
    address: cardano_sdk::Address<kind::Any>,
    lovelace: u64,
) -> Result<Hash<32>, Error> {
    let output = Output::new(address, cardano_sdk::Value::new(lovelace));
    build_sign_submit(wallet, network, vec![output]).await
}

#[cfg(test)]
mod tests {
    use super::*;

    use cardano_sdk::{Credential, NetworkId, SigningKey};
    use floe_dto_http::health::{HealthResponse, MetricsResponse};
    use floe_dto_http::query::{
        ChainPoint, ReadDataRequest, ReadDataResponse, ReadTxRequest, ReadTxResponse,
        ReadUtxosRequest, ReadUtxosResponse, SearchUtxosRequest, SearchUtxosResponse,
    };
    use floe_dto_http::submit::SubmitTxResponse;
    use subbit_wallet::wallet::embedded::Embedded;

    struct StubBackend {
        utxos: SearchUtxosResponse,
    }

    impl floe_client::Backend for StubBackend {
        async fn read_utxos(
            &mut self,
            _: ReadUtxosRequest,
        ) -> floe_client::Result<ReadUtxosResponse> {
            unimplemented!()
        }
        async fn search_utxos(
            &mut self,
            _: SearchUtxosRequest,
        ) -> floe_client::Result<SearchUtxosResponse> {
            Ok(self.utxos.clone())
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
            Ok(SubmitTxResponse { r#ref: vec![] })
        }
        async fn get_health(&mut self) -> floe_client::Result<HealthResponse> {
            unimplemented!()
        }
        async fn get_metrics(&mut self) -> floe_client::Result<MetricsResponse> {
            unimplemented!()
        }
    }

    #[tokio::test]
    async fn upload_builds_signs_and_submits_a_reference_script_tx() {
        let signing_key = SigningKey::from([1u8; 32]);
        let credential = Credential::from_key(Hash::<28>::new(signing_key.to_verification_key()));
        let address: cardano_sdk::Address<kind::Any> =
            cardano_sdk::Address::new(NetworkId::from(cardano_sdk::Network::Preprod), credential)
                .into();

        let utxo = floe_dto_http::query::Utxo {
            txo_ref: floe_dto_http::query::TxoRef {
                hash: [7u8; 32],
                index: 0,
            },
            output: floe_dto_http::cardano::TxOutput {
                address: floe_dto_http::query::Address(address.to_string().parse().unwrap()),
                coin: 50_000_000,
                assets: vec![],
                datum: None,
                script: None,
            },
            block_ref: ChainPoint::default(),
            spent_by: None,
        };
        let backend = StubBackend {
            utxos: SearchUtxosResponse {
                items: vec![utxo],
                ledger_tip: ChainPoint::default(),
                next_token: None,
            },
        };
        let mut wallet = Embedded::new(
            backend,
            signing_key,
            NetworkId::from(cardano_sdk::Network::Preprod),
            None,
        );

        let hash = upload_reference_script(&mut wallet, "preprod", None)
            .await
            .expect("uploading the reference script should succeed with one plain fuel utxo");
        assert_eq!(hash.to_string().len(), 64);
    }

    #[tokio::test]
    async fn upload_targets_an_explicit_address_when_given() {
        let signing_key = SigningKey::from([1u8; 32]);
        let credential = Credential::from_key(Hash::<28>::new(signing_key.to_verification_key()));
        let own_address: cardano_sdk::Address<kind::Any> =
            cardano_sdk::Address::new(NetworkId::from(cardano_sdk::Network::Preprod), credential)
                .into();
        let other_credential = Credential::from_key(Hash::<28>::from([9u8; 28]));
        let other_address: cardano_sdk::Address<kind::Any> = cardano_sdk::Address::new(
            NetworkId::from(cardano_sdk::Network::Preprod),
            other_credential,
        )
        .into();

        let utxo = floe_dto_http::query::Utxo {
            txo_ref: floe_dto_http::query::TxoRef {
                hash: [7u8; 32],
                index: 0,
            },
            output: floe_dto_http::cardano::TxOutput {
                address: floe_dto_http::query::Address(own_address.to_string().parse().unwrap()),
                coin: 50_000_000,
                assets: vec![],
                datum: None,
                script: None,
            },
            block_ref: ChainPoint::default(),
            spent_by: None,
        };
        let backend = StubBackend {
            utxos: SearchUtxosResponse {
                items: vec![utxo],
                ledger_tip: ChainPoint::default(),
                next_token: None,
            },
        };
        let mut wallet = Embedded::new(
            backend,
            signing_key,
            NetworkId::from(cardano_sdk::Network::Preprod),
            None,
        );

        // Just needs to succeed with a different target address than
        // the wallet's own -- `upload_reference_script`'s own change
        // strategy still returns leftover value to the wallet.
        upload_reference_script(&mut wallet, "preprod", Some(other_address))
            .await
            .expect("uploading to an explicit address should still succeed");
    }

    #[tokio::test]
    async fn teardown_reclaims_the_held_reference_script() {
        let signing_key = SigningKey::from([1u8; 32]);
        let credential = Credential::from_key(Hash::<28>::new(signing_key.to_verification_key()));
        let address: cardano_sdk::Address<kind::Any> =
            cardano_sdk::Address::new(NetworkId::from(cardano_sdk::Network::Preprod), credential)
                .into();

        let script_utxo = floe_dto_http::query::Utxo {
            txo_ref: floe_dto_http::query::TxoRef {
                hash: [8u8; 32],
                index: 0,
            },
            output: floe_dto_http::cardano::TxOutput {
                address: floe_dto_http::query::Address(address.to_string().parse().unwrap()),
                coin: 5_000_000,
                assets: vec![],
                datum: None,
                script: Some(floe_dto_http::cardano::Script {
                    lang: floe_dto_http::cardano::ScriptLang::PlutusV3,
                    cbor: VALIDATOR.script.script().to_vec(),
                }),
            },
            block_ref: ChainPoint::default(),
            spent_by: None,
        };
        let backend = StubBackend {
            utxos: SearchUtxosResponse {
                items: vec![script_utxo],
                ledger_tip: ChainPoint::default(),
                next_token: None,
            },
        };
        let mut wallet = Embedded::new(
            backend,
            signing_key,
            NetworkId::from(cardano_sdk::Network::Preprod),
            None,
        );

        let hash = teardown_reference_script(&mut wallet, "preprod")
            .await
            .expect("tearing down the one held reference-script utxo should succeed");
        assert_eq!(hash.to_string().len(), 64);
    }

    #[tokio::test]
    async fn teardown_without_a_held_reference_script_errors() {
        let signing_key = SigningKey::from([1u8; 32]);
        let backend = StubBackend {
            utxos: SearchUtxosResponse {
                items: vec![],
                ledger_tip: ChainPoint::default(),
                next_token: None,
            },
        };
        let mut wallet = Embedded::new(
            backend,
            signing_key,
            NetworkId::from(cardano_sdk::Network::Preprod),
            None,
        );

        assert!(
            teardown_reference_script(&mut wallet, "preprod")
                .await
                .is_err()
        );
    }
}
