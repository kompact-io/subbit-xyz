//! `Wallet` backed by a raw signing key held in-process, plus a
//! `floe_client::Backend` for chain I/O -- shared, not owned: see this
//! struct's own field doc. Single address (no HD derivation, no
//! multiple accounts), single optional delegation credential, per
//! `plan-embedded-wallet.md`'s scope.
//!
//! Where the key/delegation come from is `crate::store::Store`'s job,
//! not this module's -- `Embedded::from_store` is the only place the
//! two meet, and it's a thin, fallible lookup, not a dependency in
//! either direction (`Store` doesn't know `Embedded` exists).

use std::collections::BTreeMap;

use cardano_sdk::{
    Address, Credential, Hash, Input, NetworkId, Output, Signature, SigningKey, Transaction, Value,
    VerificationKey, address::kind, cbor::ToCbor, transaction::state,
};
use floe_client::Backend;

use crate::store::Store;
use crate::wallet::Wallet;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no signing key configured in this wallet's Store")]
    NoSigningKey,
    #[error("floe error: {0}")]
    Floe(#[from] floe_client::Error),
}

/// Pure address derivation -- no `Backend`, no key material beyond a
/// payment credential, so callers that only need "what address does
/// this key have" (e.g. `subbit-console`'s own wallet tab, previewing
/// an address before any `Embedded`/`Backend` exists at all) can use
/// this directly instead of duplicating it. `Embedded::change_address`
/// is this plus nothing else.
pub fn derive_address(
    network: NetworkId,
    credential: &Credential,
    delegation: Option<&Credential>,
) -> Address<kind::Any> {
    let mut address = Address::<kind::Shelley>::new(network, credential.clone());
    if let Some(delegation) = delegation {
        address = address.with_delegation(delegation.clone());
    }
    address.into()
}

/// This key's payment credential -- `Credential::from_key(Hash::<28>::
/// new(verification_key))`, named so callers don't have to remember the
/// exact hash construction (same reasoning as `derive_address` above).
pub fn credential_of(verification_key: VerificationKey) -> Credential {
    Credential::from_key(Hash::<28>::new(verification_key))
}

pub struct Embedded<B: Backend> {
    /// Shared, caller-supplied -- this struct never constructs its own
    /// connection. Whoever integrates `Embedded` (console, a CLI, a
    /// test) is responsible for reusing one `Backend` instance across
    /// every Floe-talking thing it owns, not reconnecting per call --
    /// this field being taken by value at construction, rather than
    /// e.g. a base-URL string `Embedded` would connect from itself, is
    /// what enforces that by construction.
    backend: B,
    signing_key: SigningKey,
    verification_key: VerificationKey,
    credential: Credential,
    delegation: Option<Credential>,
    network: NetworkId,
}

impl<B: Backend> Embedded<B> {
    pub fn new(
        backend: B,
        signing_key: SigningKey,
        network: NetworkId,
        delegation: Option<Credential>,
    ) -> Self {
        let verification_key = signing_key.to_verification_key();
        let credential = credential_of(verification_key);
        Self {
            backend,
            signing_key,
            verification_key,
            credential,
            delegation,
            network,
        }
    }

    /// Fails only when `store.signing_key` is unset -- a `Store` with
    /// no key configured yet has nothing to build an `Embedded` wallet
    /// from.
    pub fn from_store(store: &Store, backend: B, network: NetworkId) -> Result<Self, Error> {
        let signing_key = store
            .signing_key
            .clone()
            .ok_or(Error::NoSigningKey)?
            .into_signing_key();
        Ok(Self::new(
            backend,
            signing_key,
            network,
            store.delegation.clone(),
        ))
    }

    fn pay_hash(&self) -> [u8; 28] {
        <[u8; 28]>::from(Hash::<28>::new(self.verification_key))
    }
}

impl<B: Backend> Wallet for Embedded<B> {
    type Error = Error;

    async fn network_id(&self) -> Result<NetworkId, Self::Error> {
        Ok(self.network)
    }

    async fn change_address(&self) -> Result<Address<kind::Any>, Self::Error> {
        Ok(derive_address(
            self.network,
            &self.credential,
            self.delegation.as_ref(),
        ))
    }

    async fn utxos(
        &mut self,
        value: Option<Value<u64>>,
    ) -> Result<Option<BTreeMap<Input, Output>>, Self::Error> {
        let pay_hash = self.pay_hash();
        let utxos = crate::floe::search_live(&mut self.backend, pay_hash).await?;
        let Some(target) = value else {
            return Ok(Some(utxos));
        };
        let entries: Vec<(Input, Output)> = utxos.into_iter().collect();
        Ok(
            Value::cover(&target, &entries, |pair: &(Input, Output)| pair.1.value())
                .map(|selection| selection.inputs.into_iter().cloned().collect()),
        )
    }

    async fn sign_tx(
        &self,
        tx: &Transaction<state::ReadyForSigning>,
    ) -> Result<(VerificationKey, Signature), Self::Error> {
        Ok((self.verification_key, self.signing_key.sign(tx.id())))
    }

    async fn submit(
        &mut self,
        tx: &Transaction<state::ReadyForSigning>,
    ) -> Result<Hash<32>, Self::Error> {
        self.backend.submit_tx(tx.to_cbor()).await?;
        Ok(tx.id())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use floe_dto_http::health::{HealthResponse, MetricsResponse};
    use floe_dto_http::query::{
        ChainPoint, ReadDataRequest, ReadDataResponse, ReadTxRequest, ReadTxResponse,
        ReadUtxosRequest, ReadUtxosResponse, SearchUtxosRequest, SearchUtxosResponse, TxoRef,
    };
    use floe_dto_http::submit::SubmitTxResponse;

    struct MockBackend {
        by_pay_hash: std::collections::BTreeMap<[u8; 28], SearchUtxosResponse>,
        submitted: Vec<Vec<u8>>,
    }

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
                .by_pay_hash
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
        async fn submit_tx(&mut self, raw: Vec<u8>) -> floe_client::Result<SubmitTxResponse> {
            self.submitted.push(raw);
            Ok(SubmitTxResponse { r#ref: vec![] })
        }
        async fn get_health(&mut self) -> floe_client::Result<HealthResponse> {
            unimplemented!()
        }
        async fn get_metrics(&mut self) -> floe_client::Result<MetricsResponse> {
            unimplemented!()
        }
    }

    fn utxo(index: u16, coin: u64) -> floe_dto_http::query::Utxo {
        floe_dto_http::query::Utxo {
            txo_ref: TxoRef { hash: [0u8; 32], index },
            output: floe_dto_http::cardano::TxOutput {
                address: floe_dto_http::query::Address(
                    "addr1qxu84ftxpzh3zd8p9awp2ytwzk5exj0fxcj7paur4kd4ytun36yuhgl049rxhhuckm2lpq3rmz5dcraddyl45d6xgvqqsp504c"
                        .parse()
                        .unwrap(),
                ),
                coin,
                assets: vec![],
                datum: None,
                script: None,
            },
            block_ref: ChainPoint::default(),
            spent_by: None,
        }
    }

    fn embedded(backend: MockBackend) -> Embedded<MockBackend> {
        Embedded::new(
            backend,
            SigningKey::from([1u8; 32]),
            NetworkId::from(cardano_sdk::Network::Preprod),
            None,
        )
    }

    #[test]
    fn address_is_deterministic_for_a_fixed_seed() {
        let wallet = embedded(MockBackend {
            by_pay_hash: Default::default(),
            submitted: vec![],
        });
        let addr1 = wallet.credential.clone();
        let wallet2 = embedded(MockBackend {
            by_pay_hash: Default::default(),
            submitted: vec![],
        });
        assert_eq!(addr1, wallet2.credential);
    }

    #[tokio::test]
    async fn utxos_only_returns_entries_for_this_wallets_own_pay_hash() {
        let wallet = embedded(MockBackend {
            by_pay_hash: Default::default(),
            submitted: vec![],
        });
        let pay_hash = wallet.pay_hash();

        let mut by_pay_hash = std::collections::BTreeMap::new();
        by_pay_hash.insert(
            pay_hash,
            SearchUtxosResponse {
                items: vec![utxo(0, 5_000_000)],
                ledger_tip: ChainPoint::default(),
                next_token: None,
            },
        );
        by_pay_hash.insert(
            [9u8; 28],
            SearchUtxosResponse {
                items: vec![utxo(1, 1_000_000)],
                ledger_tip: ChainPoint::default(),
                next_token: None,
            },
        );
        let mut wallet = embedded(MockBackend {
            by_pay_hash,
            submitted: vec![],
        });

        let utxos = wallet.utxos(None).await.unwrap().unwrap();
        assert_eq!(utxos.len(), 1);
    }

    #[tokio::test]
    async fn utxos_with_a_value_hint_returns_none_when_uncoverable() {
        let mut by_pay_hash = std::collections::BTreeMap::new();
        let wallet = embedded(MockBackend {
            by_pay_hash: Default::default(),
            submitted: vec![],
        });
        let pay_hash = wallet.pay_hash();
        by_pay_hash.insert(
            pay_hash,
            SearchUtxosResponse {
                items: vec![utxo(0, 1_000_000)],
                ledger_tip: ChainPoint::default(),
                next_token: None,
            },
        );
        let mut wallet = embedded(MockBackend {
            by_pay_hash,
            submitted: vec![],
        });

        let covered = wallet.utxos(Some(Value::new(1_000_000))).await.unwrap();
        assert!(covered.is_some());

        let uncoverable = wallet.utxos(Some(Value::new(10_000_000))).await.unwrap();
        assert!(uncoverable.is_none());
    }
}
