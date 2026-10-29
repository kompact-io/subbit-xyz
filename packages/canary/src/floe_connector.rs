//! `cardano_connector::CardanoConnector` backed by floe-client, standing in
//! for `cardano_connector_direct::Blockfrost` in `--mode preprod` -- see
//! `L1::build_preprod`. Floe doesn't serve protocol parameters (same gap
//! `subbit_claim::config::Config::network_parameters` already works
//! around), so those come from the same hardcoded per-network snapshot;
//! canary's preprod mode has never been anything but preprod, so that
//! network is hardcoded here too, not configurable.
//!
//! `CardanoConnector`'s methods all take `&self`, but
//! `floe_client::Backend`'s (what `subbit_claim::floe`'s helpers drive)
//! take `&mut self` -- the `tokio::sync::Mutex` below exists purely to
//! bridge that mismatch, not for any real contention (canary drives one
//! `L1` from a single task at a time).

use std::collections::BTreeMap;

use cardano_connector::CardanoConnector;
use cardano_sdk::{
    Credential, Input, Network, Output, ProtocolParameters, Transaction,
    transaction::state::ReadyForSigning,
};
use floe_client::grpc;
use tokio::sync::Mutex;

pub struct FloeConnector {
    client: Mutex<grpc::Client>,
}

impl FloeConnector {
    pub async fn connect(endpoint: &str) -> anyhow::Result<Self> {
        let client = subbit_index::cardano::Config {
            endpoint: endpoint.to_string(),
        }
        .connect()
        .await?;
        Ok(Self {
            client: Mutex::new(client),
        })
    }
}

/// Collapses a credential (key or script) to its raw hash bytes --
/// floe's `pay_hash`/`stake_hash` filters don't distinguish the two.
fn hash28(credential: &Credential) -> [u8; 28] {
    <[u8; 28]>::from(credential.select(|h| h, |h| h))
}

impl CardanoConnector for FloeConnector {
    fn network(&self) -> Network {
        Network::Preprod
    }

    async fn health(&self) -> anyhow::Result<String> {
        // Unused anywhere in canary/`L1` today (only `MockChain` ever
        // implements this meaningfully) -- a trivial stub is enough.
        Ok("floe".to_string())
    }

    async fn protocol_parameters(&self) -> anyhow::Result<ProtocolParameters> {
        // `ProtocolParameters::preprod()`'s own `fee_per_byte`/`fee_constant`
        // (44/155381) match live preprod's real `min_fee_a`/`min_fee_b`
        // exactly (checked against Koios) -- the live node still rejected a
        // real ref-script upload tx here as underfeed by ~15%
        // ("declared fee 303045 below minimum 348475"), so the shortfall is
        // somewhere else in fee estimation (tx-size accounting around a
        // script-bearing output, most likely), not stale parameter values.
        // Rather than chase that down inside the vendored `cardano-sdk`
        // dependency, apply a flat safety margin here -- canary burns free
        // testnet ADA, so overpaying fees is harmless; a real production
        // service (`subbit-claim`) should NOT copy this, and doesn't.
        // `preprod()`'s own base values (not exposed via a getter, so
        // these are copied from `cardano_sdk::ProtocolParameters::mainnet`,
        // which `preprod()` builds on): fee_per_byte=44, fee_constant=155381.
        const FEE_SAFETY_MARGIN_PCT: u64 = 150;
        Ok(ProtocolParameters::preprod()
            .with_fee_per_byte(44 * FEE_SAFETY_MARGIN_PCT / 100)
            .with_fee_constant(155381 * FEE_SAFETY_MARGIN_PCT / 100))
    }

    async fn utxos_at(
        &self,
        payment: &Credential,
        delegation: Option<&Credential>,
    ) -> anyhow::Result<BTreeMap<Input, Output>> {
        let pay_hash = hash28(payment);
        let stake_hash = delegation.map(hash28);
        let mut client = self.client.lock().await;
        let utxos =
            subbit_claim::floe::search_by_pay_hash(&mut client, pay_hash, stake_hash).await?;
        utxos
            .iter()
            .map(|u| {
                subbit_claim::floe::to_input_output(u)
                    .map_err(|e| anyhow::anyhow!("converting floe utxo: {e}"))
            })
            .collect()
    }

    async fn submit(&self, transaction: &Transaction<ReadyForSigning>) -> anyhow::Result<()> {
        let mut client = self.client.lock().await;
        subbit_claim::floe::submit(&mut client, transaction).await?;
        Ok(())
    }
}
