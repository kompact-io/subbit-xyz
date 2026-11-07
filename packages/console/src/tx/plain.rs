//! Plain, `Ctx`/basket-free transactions a single wallet can build and
//! submit entirely on its own: `upload_reference_script` (the one
//! on-chain prerequisite `Basket::build`'s own `reference_utxo`
//! argument needs before any `Will` can be staged against an
//! *existing* channel -- staging an `Open` needs none) and
//! `send_payment` (a plain transfer, e.g. seeding a separately-derived
//! fee wallet). Part of the core logic engine (`lib.rs`'s own doc): any
//! caller (`ui::cli`, `subbit-canary`, a future `ui::wasm` Settings-tab
//! action) that needs either calls these directly, rather than
//! reimplementing them.
//!
//! Ported in shape (not as a dependency) from the `cardano-wallet`
//! git crate's own `txs::upload`/`txs::send` -- that crate is on this
//! workspace's own remove-list for everything except `canary`'s
//! not-yet-migrated `l1.rs`, so this is the shared engine's own copy,
//! built on `subbit_wallet::Wallet` instead of `cardano-session`.
//!
//! No confirmation-wait here: same as `submit`, the caller decides
//! whether/how to wait for this to land (e.g. polling via
//! `floe_client::Backend::read_tx`, same pattern `submit`'s own
//! multi-batch loop uses).

use std::collections::BTreeMap;

use cardano_sdk::{
    ChangeStrategy, Hash, Input, Output, Transaction, address::kind,
    transaction::state::ReadyForSigning,
};
use subbit_tx::{NetworkParameters, VALIDATOR};
use subbit_wallet::Wallet;

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

/// Shared build -> sign -> submit plumbing for `upload_reference_script`
/// and `send_payment` -- both are a single-wallet, no-`Ctx`-involved
/// plain tx (no consumer/provider witnesses to aggregate, unlike
/// `submit`), so neither needs `submit`'s own multi-signatory logic.
#[tracing::instrument(skip(wallet, outputs), ret(Debug), err(Debug))]
async fn build_sign_submit<W: Wallet>(
    wallet: &mut W,
    network: &str,
    outputs: Vec<Output>,
) -> Result<Hash<32>, Error> {
    let network_parameters =
        NetworkParameters::for_network(network).map_err(|e| Error::Network(e.to_string()))?;
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

    let inputs = sender_utxos.keys().map(|input| (input.clone(), None));
    let mut tx: Transaction<ReadyForSigning> = Transaction::build(
        &network_parameters.protocol_parameters,
        &sender_utxos,
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
