//! Consumer-side L1 actions via `subbit_console::Ctx` + `submit`/`upload_reference_script`/
//! `send_payment` over a `subbit_wallet::Embedded` wallet.

use subbit_console::{Ctx, SigningKeyHex};
use subbit_core::Hash28;
use subbit_tx::{
    VALIDATOR,
    step::{Can, Want},
};
use subbit_wallet::wallet::embedded::Embedded;

type Backend = floe_client::grpc::Client;

/// Collapses a credential (key or script) to its raw hash bytes.
fn credential_hash28(credential: &cardano_sdk::Credential) -> [u8; 28] {
    <[u8; 28]>::from(credential.select(|h| h, |h| h))
}

// A single transient 502 from Floe used to abort an entire multi-minute run; retry first.
const DISCOVERY_RETRY_ATTEMPTS: u32 = 5;

/// Retry wrapper around one in-flight discovery call (inlined per call site, not a generic combinator).
macro_rules! retry_transient {
    ($call:expr) => {{
        let mut delay = std::time::Duration::from_secs(2);
        let mut attempt = 0;
        loop {
            match $call {
                Ok(value) => break value,
                Err(e) if attempt + 1 < DISCOVERY_RETRY_ATTEMPTS => {
                    attempt += 1;
                    eprintln!(
                        "canary: transient discovery error (attempt {attempt}/{DISCOVERY_RETRY_ATTEMPTS}), \
                         retrying in {delay:?}: {e:#}"
                    );
                    tokio::time::sleep(delay).await;
                    delay *= 2;
                }
                Err(e) => return Err(e.into()),
            }
        }
    }};
}

pub struct Chain {
    ctx: Ctx,
    wallet: Embedded<Backend>,
    /// The funding wallet's own payment credential, kept as a `kind::Shelley` address.
    funding_credential: cardano_sdk::Credential,
    discovery: Backend,
    network: &'static str,
    network_id: cardano_sdk::NetworkId,
}

impl Chain {
    /// Connects to `floe_endpoint` and uploads a reference script if none is tracked yet.
    pub async fn build_preprod(
        floe_endpoint: String,
        signing_key: [u8; 32],
    ) -> anyhow::Result<Self> {
        let network_id = cardano_sdk::NetworkId::from(cardano_sdk::Network::Preprod);
        let signing_key_typed = cardano_sdk::SigningKey::from(signing_key);
        let funding_credential =
            subbit_wallet::wallet::embedded::credential_of(signing_key_typed.to_verification_key());
        let wallet_backend = Backend::connect(&floe_endpoint).await?;
        let wallet = Embedded::new(wallet_backend, signing_key_typed, network_id, None);
        let discovery = Backend::connect(&floe_endpoint).await?;
        let mut chain = Self {
            ctx: Ctx::default(),
            wallet,
            funding_credential,
            discovery,
            network: "preprod",
            network_id,
        };
        chain.refresh().await?;
        if chain.ctx.basket_reference_utxo().is_none() {
            println!("canary: uploading subbit validator as a reference script");
            let hash =
                subbit_console::upload_reference_script(&mut chain.wallet, chain.network, None)
                    .await?;
            chain.wait_for_landing(hash).await?;
            chain.refresh().await?;
            anyhow::ensure!(
                chain.ctx.basket_reference_utxo().is_some(),
                "reference script upload confirmed but still isn't recognized -- check \
                 VALIDATOR.hash matches what was actually uploaded"
            );
        }
        Ok(chain)
    }

    /// Registers `seed`'s derived identity as a consumer-flagged keyring entry under `label`.
    pub fn register(&mut self, label: impl Into<String>, seed: [u8; 32]) -> anyhow::Result<()> {
        self.ctx
            .keyring_add(label.into(), SigningKeyHex::from(seed), true, false)
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Re-reads the validator address's live txos, plus the funding wallet's own reference-script utxo.
    pub async fn refresh(&mut self) -> anyhow::Result<()> {
        let pay_hash: [u8; 28] = <[u8; 28]>::from(VALIDATOR.hash);
        let items = retry_transient!(
            subbit_index::cardano::search_all(&mut self.discovery, pay_hash, None).await
        );
        self.ctx.basket_refresh_txos(items);

        let funding_pay_hash = credential_hash28(&self.funding_credential);
        let funding_utxos = retry_transient!(
            subbit_index::cardano::search_all(&mut self.discovery, funding_pay_hash, None).await
        );
        match funding_utxos
            .into_iter()
            .find(|u| u.output.script.is_some())
        {
            Some(utxo) => self
                .ctx
                .basket_set_reference_utxo(utxo)
                .map_err(|e| anyhow::anyhow!("{e}"))?,
            None => self.ctx.basket_clear_reference_utxo(),
        }
        Ok(())
    }

    /// Every txo currently known at the validator address.
    pub fn channels(&self) -> &std::collections::BTreeMap<cardano_sdk::Input, subbit_tx::Channel> {
        self.ctx.basket_txos()
    }

    pub fn network_id(&self) -> cardano_sdk::NetworkId {
        self.network_id
    }

    /// The main funding wallet's own `kind::Shelley`-typed address.
    pub fn change_address_shelley(
        &self,
    ) -> cardano_sdk::Address<cardano_sdk::address::kind::Shelley> {
        cardano_sdk::Address::new(self.network_id, self.funding_credential.clone())
    }

    /// One-off plain payment from the main funding wallet.
    pub async fn fund_address(
        &mut self,
        address: cardano_sdk::Address<cardano_sdk::address::kind::Any>,
        lovelace: u64,
    ) -> anyhow::Result<cardano_sdk::Hash<32>> {
        let hash =
            subbit_console::send_payment(&mut self.wallet, self.network, address, lovelace).await?;
        self.wait_for_landing(hash).await?;
        Ok(hash)
    }

    /// The one channel matching `(iou_key, tag, consumer)` exactly.
    pub fn can_for_consumer(
        &self,
        iou_key: &subbit_core::VerifyingKey,
        tag: &[u8],
        consumer: &Hash28,
    ) -> anyhow::Result<Option<(cardano_sdk::Input, Vec<Can>)>> {
        let matches: Vec<_> = self
            .ctx
            .dashboard()
            .into_iter()
            .filter(|s| {
                s.constants.iou_key() == iou_key
                    && s.constants.tag().as_ref() == tag
                    && s.constants.consumer() == consumer
            })
            .collect();
        match matches.len() {
            0 => Ok(None),
            1 => Ok(Some((matches[0].input.clone(), matches[0].can.clone()))),
            _ => anyhow::bail!("more than one channel matches this exact (iou_key, tag, consumer)"),
        }
    }

    /// Tolerant of more than one match -- `tidy`'s own reclaim sweep.
    pub fn channels_for_consumer(
        &self,
        iou_key: &subbit_core::VerifyingKey,
        tag: &[u8],
        consumer: &Hash28,
    ) -> Vec<(cardano_sdk::Input, Vec<Can>)> {
        self.ctx
            .dashboard()
            .into_iter()
            .filter(|s| {
                s.constants.iou_key() == iou_key
                    && s.constants.tag().as_ref() == tag
                    && s.constants.consumer() == consumer
            })
            .map(|s| (s.input, s.can))
            .collect()
    }

    /// Stages `want` against `input` -- `true` if it was accepted.
    pub fn want(&mut self, input: cardano_sdk::Input, want: Want) -> bool {
        self.ctx.basket_propose(input, want).is_ok()
    }

    /// Submits every currently staged Open/Will, batched/retried, then refreshes.
    pub async fn flush(&mut self) -> anyhow::Result<()> {
        if self.ctx.basket_opens().is_empty() && self.ctx.basket_wills().is_empty() {
            return Ok(());
        }
        subbit_console::submit(&mut self.wallet, &mut self.ctx, self.network).await?;
        self.refresh().await
    }

    /// Opens every `(Constants, amount)` pair, batched/retried the same way `flush` is.
    pub async fn open(
        &mut self,
        channels: Vec<(subbit_l1_dto::Constants, u64)>,
    ) -> anyhow::Result<()> {
        for (constants, amount) in channels {
            self.ctx.basket_add_open(
                constants.tag().clone(),
                constants.asset().clone(),
                *constants.iou_key(),
                *constants.consumer(),
                *constants.provider(),
                *constants.close_period(),
                None,
                amount,
            );
        }
        self.flush().await
    }

    /// Polls `wallet.utxos(None)` until `hash`'s own utxo appears.
    async fn wait_for_landing(&mut self, hash: cardano_sdk::Hash<32>) -> anyhow::Result<()> {
        Ok(subbit_wallet::wait_for_landing(&mut self.wallet, hash, tokio::time::sleep).await?)
    }
}
