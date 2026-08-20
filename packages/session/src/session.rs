use std::{
    collections::{BTreeMap, BTreeSet},
    iter,
};

use cardano_connector::CardanoConnector;
use cardano_connector_direct::Blockfrost;
use cardano_sdk::{
    Address, Credential, Hash, Input, Output, Transaction, address::kind,
    transaction::state::ReadyForSigning,
};
use cardano_session::{Session as CardanoSession, session};
use subbit_tx::{VALIDATOR, tx::Tx, validator};

#[derive(Debug, Clone, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Config {
    pub cardano: cardano_session::Config,
    pub script_host: Option<Address<kind::Shelley>>,
    pub delegations: BTreeSet<Credential>,
}

impl Config {
    pub async fn build(self) -> Result<Session<Blockfrost>, session::Error> {
        let cardano = cardano_session::Session::init(self.cardano).await?;
        Ok(Session::new(cardano, self.script_host, self.delegations))
    }
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no ref script tracked for hash {0:?}")]
    RefScriptNotFound(Hash<28>),
    #[error("script host already set; operation not permitted")]
    ScriptHostSet,
    #[error(transparent)]
    Tx(#[from] subbit_tx::tx::Error),
    #[error(transparent)]
    Build(#[from] subbit_tx::tx::BuildError),
    #[error(transparent)]
    Cardano(#[from] session::Error),
}

/// Thin wrapper over `cardano::Session`, adding the subbit validator's
/// address space (validator + maybe delegations) and ref-script resolution.
pub struct Session<C> {
    cardano: CardanoSession<C>,
    /// Set if the ref script lives at an address other than the wallet's.
    /// Assumed already uploaded there - `upload`/`teardown` refuse to run
    /// while this is set, since both only know how to manage the wallet's
    /// own copy.
    ///
    /// TODO :: UNTESTED!!
    script_host: Option<Address<kind::Shelley>>,
    /// TODO :: UNTESTED!!
    delegations: BTreeSet<Credential>,
}

impl<C: CardanoConnector> Session<C> {
    pub fn new(
        cardano: CardanoSession<C>,
        script_host: Option<Address<kind::Shelley>>,
        delegations: BTreeSet<Credential>,
    ) -> Self {
        Self {
            cardano,
            script_host,
            delegations,
        }
    }

    pub async fn init(&mut self) -> Result<(), Error> {
        self.cardano.reload().await?;
        if let Some(host) = self.script_host.clone() {
            self.cardano.refresh_at(host).await?;
        }
        if self.ref_script().is_err() {
            self.upload().await?
        }
        self.reload_channels().await
    }

    // --- accessors ---

    pub fn cardano(&self) -> &CardanoSession<C> {
        &self.cardano
    }

    pub fn script_host(&self) -> Option<&Address<kind::Shelley>> {
        self.script_host.as_ref()
    }

    pub fn delegations(&self) -> &BTreeSet<Credential> {
        &self.delegations
    }

    // --- delegation management ---

    pub async fn add_delegation(&mut self, cred: Credential) -> Result<bool, Error> {
        if !self.delegations.insert(cred.clone()) {
            return Ok(false);
        }
        let addr = validator::address(self.cardano.network_id(), Some(&cred));
        self.cardano.refresh_at(addr).await?;
        Ok(true)
    }

    pub fn remove_delegation(&mut self, cred: &Credential) -> Result<bool, Error> {
        if !self.delegations.remove(cred) {
            return Ok(false);
        }
        let addr = validator::address(self.cardano.network_id(), Some(cred));
        self.cardano.untrack(&addr)?;
        Ok(true)
    }

    // --- channel state ---

    fn addresses(&self) -> Vec<Address<kind::Shelley>> {
        let network_id = self.cardano.network_id();
        iter::once(None)
            .chain(self.delegations.iter().cloned().map(Some))
            .map(|d| validator::address(network_id, d.as_ref()))
            .collect()
    }

    pub async fn reload_channels(&mut self) -> Result<(), Error> {
        let addresses = self.addresses();
        Ok(self.cardano.refresh_many(addresses).await?)
    }

    pub fn channels(&self) -> BTreeMap<Input, Output> {
        self.addresses()
            .iter()
            .filter_map(|a| self.cardano.utxos_at(a))
            .flat_map(|m| m.iter().map(|(i, o)| (i.clone(), o.clone())))
            .collect()
    }

    pub fn stage_tx(&self) -> Result<Tx, Error> {
        Ok(Tx::new(self.channels())?)
    }

    // --- ref script ---

    /// Resolves the subbit validator's ref script UTXO. Checked at the
    /// wallet's address unless `script_host` overrides it.
    fn ref_script(&self) -> Result<(Input, Output), Error> {
        let hash = VALIDATOR.hash;
        let address = self.script_host.as_ref();
        match address {
            None => self
                .cardano
                .ref_script(&hash)
                .ok_or(Error::RefScriptNotFound(hash)),
            Some(host) => self
                .cardano
                .utxos_at(host)
                .into_iter()
                .flatten()
                .find_map(|(input, output)| {
                    output
                        .script()
                        .filter(|s| Hash::<28>::from(*s) == hash)
                        .map(|_| (input.clone(), output.clone()))
                })
                .ok_or(Error::RefScriptNotFound(hash)),
        }
    }

    pub async fn upload(&mut self) -> Result<(), Error> {
        if self.script_host.is_some() {
            return Err(Error::ScriptHostSet);
        }
        let id = self.cardano.upload(VALIDATOR.script.clone()).await?;
        Ok(self.cardano.wait_wallet(&id).await?)
    }

    pub async fn teardown(&mut self, hash: &Hash<28>) -> Result<Hash<32>, Error> {
        if self.script_host.is_some() {
            return Err(Error::ScriptHostSet);
        }
        Ok(self.cardano.teardown(hash).await?)
    }

    pub async fn wait_til(&mut self, id: &Hash<32>) -> Result<(), Error> {
        self.cardano.wait_wallet(id).await?;
        self.reload_channels().await
    }

    // --- tx build/submit glue ---

    fn network_parameters(&self) -> subbit_tx::NetworkParameters {
        let cardano_session::network::NetworkParameters {
            network_id,
            protocol_parameters,
        } = self.cardano.network_parameters().clone();
        subbit_tx::NetworkParameters {
            network_id,
            protocol_parameters,
        }
    }

    /// Builds `tx` against current chain state. The result still needs
    /// every signature `tx.signers()` requires (via a `Keyring`, or
    /// however signing is done) before `sign_and_submit`.
    pub fn build_tx(&self, tx: &mut Tx) -> Result<Transaction<ReadyForSigning>, Error> {
        let ref_script = self.ref_script()?;
        Ok(tx.build(
            &self.network_parameters(),
            Some(&ref_script),
            self.cardano.change_address().into(),
            &self.cardano.fuel(),
        )?)
    }

    pub async fn sign_and_submit(
        &mut self,
        built: Transaction<ReadyForSigning>,
    ) -> Result<Hash<32>, Error> {
        Ok(self.cardano.sign_and_submit(built).await?)
    }
}
