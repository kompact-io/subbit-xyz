use std::collections::BTreeMap;

use cardano_connector::CardanoConnector;
use cardano_connector_direct::Blockfrost;
use cardano_sdk::{
    Address, ChangeStrategy, Hash, Input, NetworkId, Output, PlutusScript, ProtocolParameters,
    Transaction, address::kind::Shelley, transaction::state::ReadyForSigning,
};

use crate::{Config, NetworkParameters, Tip, Waiter, Wallet, waiter};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // TODO: CardanoConnector doesn't expose a nameable error type
    // yet. Tighten once it does.
    #[error("chain connector error: {0}")]
    Connector(String),
    #[error("failed to build tx: {0}")]
    BuildTx(String),
    #[error("waiter: {0}")]
    Timeout(#[from] waiter::Error),
    #[error("no ref script tracked for hash {0:?}")]
    RefScriptNotFound(Hash<28>),
    #[error("refusing to untrack a protected address (wallet/script host)")]
    ProtectedAddress,
    #[error("teardown requires script_host to be the wallet's own address")]
    UnsupportedScriptHost,
}

/// Everything needed to build and submit a transaction on this chain,
/// independent of any dapp/validator running on top of it. Rule of thumb:
/// a field/method belongs here iff it'd still make sense with a different
/// dapp - otherwise it belongs one layer up, in the dapp's own code.
pub struct Session<C> {
    wallet: Wallet,
    waiter: Waiter,
    cardano: C,
    network_parameters: NetworkParameters,
    // Every address the session tracks, including the wallet's own.
    // `fuel`/`ref_script`/`ref_scripts` below are just filtered views into
    // tip's entry for the wallet address - not separate state.
    tip: Tip,
}

impl Session<Blockfrost> {
    // Tied to `Blockfrost` because `connector::Config::build()` currently
    // only ever constructs one. Everything else on `Session<C>` is generic.
    pub async fn init(config: Config) -> Result<Self, Error> {
        let cardano = config.cardano.build();
        let network_id: NetworkId = cardano.network().into();
        let protocol_parameters = cardano
            .protocol_parameters()
            .await
            .map_err(|e| Error::Connector(e.to_string()))?;
        Ok(Self {
            wallet: Wallet::new(config.wallet),
            waiter: Waiter::new(config.wait),
            cardano,
            network_parameters: NetworkParameters {
                network_id,
                protocol_parameters,
            },
            tip: Tip::empty(),
        })
    }
}

impl<C: CardanoConnector> Session<C> {
    fn wallet_address(&self) -> Address<Shelley> {
        self.wallet.address(self.network_parameters.network_id)
    }

    /// Refreshes the wallet's own entry in `tip`. `fuel`/`ref_script`s read
    /// from this, so nothing else needs to change after calling it.
    pub async fn reload(&mut self) -> Result<(), Error> {
        self.refresh_at(self.wallet_address()).await
    }

    /// Where `upload` deposits reference scripts. Always the wallet's own
    /// address for now - add a settable override field on `Session` if a
    /// dapp ever needs scripts to live elsewhere.
    pub fn script_host(&self) -> Address<Shelley> {
        self.wallet_address()
    }

    pub fn change_address(&self) -> Address<Shelley> {
        self.wallet_address()
    }

    pub fn verification_key(&self) -> cardano_sdk::VerificationKey {
        self.wallet.verification_key()
    }

    pub fn network_id(&self) -> NetworkId {
        self.network_parameters.network_id
    }

    pub fn protocol_parameters(&self) -> &ProtocolParameters {
        &self.network_parameters.protocol_parameters
    }

    pub fn network_parameters(&self) -> &NetworkParameters {
        &self.network_parameters
    }

    /// The wallet's non-script UTXOs, as of the last `reload`.
    pub fn fuel(&self) -> BTreeMap<Input, Output> {
        self.utxos_at(&self.wallet_address())
            .into_iter()
            .flatten()
            .filter(|(_, output)| output.script().is_none())
            .map(|(input, output)| (input.clone(), output.clone()))
            .collect()
    }

    pub fn ref_script(&self, hash: &Hash<28>) -> Option<(Input, Output)> {
        self.utxos_at(&self.wallet_address())?
            .iter()
            .find_map(|(input, output)| {
                output
                    .script()
                    .filter(|script| &Hash::<28>::from(*script) == hash)
                    .map(|_| (input.clone(), output.clone()))
            })
    }

    pub fn ref_scripts(&self) -> impl Iterator<Item = (Hash<28>, Input, Output)> + '_ {
        self.utxos_at(&self.wallet_address())
            .into_iter()
            .flatten()
            .filter_map(|(input, output)| {
                output
                    .script()
                    .map(|script| (Hash::<28>::from(script), input.clone(), output.clone()))
            })
    }

    /// Cached UTXOs at `address`. `None` if it's never been refreshed -
    /// call `refresh_at`/`refresh_many`/`refresh_all` first. This is a
    /// plain lookup into `tip`, not a network call.
    /// FIXME :: Make this a result?!
    pub fn utxos_at(&self, address: &Address<Shelley>) -> Option<&BTreeMap<Input, Output>> {
        self.tip.utxos_at(address)
    }

    pub fn tip(&self) -> &Tip {
        &self.tip
    }

    /// Replaces `tip` wholesale - e.g. with one loaded from a cache file.
    /// Anything currently tracked that isn't in the replacement stops
    /// being tracked.
    pub fn load_tip(&mut self, tip: Tip) {
        self.tip = tip;
    }

    /// Fetches `address`'s current UTXOs from the connector and updates
    /// `tip`. `CardanoConnector::utxos_at` wants the payment and delegation
    /// parts separately, not a single credential.
    pub async fn refresh_at(&mut self, address: Address<Shelley>) -> Result<(), Error> {
        let utxos = self
            .cardano
            .utxos_at(&address.payment(), address.delegation().as_ref())
            .await
            .map_err(|e| Error::Connector(e.to_string()))?;
        self.tip.refresh(address, utxos);
        Ok(())
    }

    /// Sequential for now - switch to `futures::future::try_join_all` if
    /// this becomes a bottleneck.
    pub async fn refresh_many(
        &mut self,
        addresses: impl IntoIterator<Item = Address<Shelley>>,
    ) -> Result<(), Error> {
        for address in addresses {
            self.refresh_at(address).await?;
        }
        Ok(())
    }

    /// No-op until `refresh_at`/`refresh_many` has tracked something -
    /// `tip` has no way to guess what a dapp wants tracked.
    pub async fn refresh_all(&mut self) -> Result<(), Error> {
        let addresses: Vec<_> = self.tip.addresses().cloned().collect();
        self.refresh_many(addresses).await
    }

    pub async fn upload(&mut self, script: PlutusScript) -> Result<Hash<32>, Error> {
        let script_host = self.script_host();
        let change_address = self.change_address();
        let outputs = vec![Output::to(script_host.into()).with_plutus_script(script)];
        let fuel = self.fuel();
        let tx = Transaction::build(&self.network_parameters.protocol_parameters, &fuel, |tx| {
            tx.with_inputs(fuel.keys().cloned().map(|i| (i, None)))
                .with_outputs(outputs.to_owned())
                .with_change_strategy(ChangeStrategy::as_last_output(
                    change_address.to_owned().into(),
                ))
                .ok()
        })
        .map_err(|e| Error::BuildTx(e.to_string()))?;

        self.sign_and_submit(tx).await
    }

    pub async fn sign_and_submit(
        &mut self,
        mut tx: Transaction<ReadyForSigning>,
    ) -> Result<Hash<32>, Error> {
        tx.sign(self.wallet.cardano_signing_key());
        let id = tx.id();
        self.cardano
            .submit(&tx)
            .await
            .map_err(|e| Error::Connector(e.to_string()))?;
        Ok(id)
    }

    /// Watches `address`'s UTXOs for `id`, polling the connector directly
    /// each attempt. Caches the confirming fetch into `tip` before
    /// returning, so callers don't need a separate refresh after.
    pub async fn wait_until(
        &mut self,
        address: &Address<Shelley>,
        id: &Hash<32>,
    ) -> Result<(), Error> {
        for attempt in 0..self.waiter.max_attempts() {
            tracing::warn!("awaiting {} at {:?}, attempt {}", id, address, attempt);
            let utxos = self
                .cardano
                .utxos_at(&address.payment(), address.delegation().as_ref())
                .await
                .map_err(|e| Error::Connector(e.to_string()))?;
            if utxos.keys().any(|i| i.transaction_id() == *id) {
                self.tip.refresh(address.clone(), utxos);
                return Ok(());
            }
            if attempt + 1 == self.waiter.max_attempts() {
                break;
            }
            self.waiter.wait().await;
        }
        Err(self.waiter.timed_out(id).into())
    }

    /// Watches the wallet's own address for `id`. Thin wrapper over
    /// `wait_until` - use that directly if the tx might not touch the
    /// wallet.
    pub async fn wait_wallet(&mut self, id: &Hash<32>) -> Result<(), Error> {
        let wallet_address = self.wallet_address();
        self.wait_until(&wallet_address, id).await
    }

    pub async fn teardown(&mut self, hash: &Hash<28>) -> Result<Hash<32>, Error> {
        if self.script_host() != self.wallet_address() {
            return Err(Error::UnsupportedScriptHost);
        }
        let ref_utxo = self
            .ref_script(hash)
            .ok_or(Error::RefScriptNotFound(*hash))?;
        let change_address = self.change_address();
        let mut utxos = self.fuel();
        let ref_input = ref_utxo.0.clone();
        utxos.insert(ref_utxo.0, ref_utxo.1);
        let tx = Transaction::build(&self.network_parameters.protocol_parameters, &utxos, |tx| {
            tx.with_inputs(vec![(ref_input.clone(), None)])
                .with_change_strategy(ChangeStrategy::as_last_output(
                    change_address.to_owned().into(),
                ))
                .ok()
        })
        .map_err(|e| Error::BuildTx(e.to_string()))?;
        self.sign_and_submit(tx).await
    }

    /// Stops tracking `address` and drops its cached UTXOs from `tip`.
    /// Refuses to untrack the wallet's own address - `fuel`/`ref_script`s
    /// depend on it always being tracked. `script_host` is the same
    /// address for now, so this covers it too; revisit if `script_host`
    /// ever becomes independently settable.
    pub fn untrack(&mut self, address: &Address<Shelley>) -> Result<(), Error> {
        if *address == self.wallet_address() {
            return Err(Error::ProtectedAddress);
        }
        self.tip.untrack(address);
        Ok(())
    }
}
