//! Consumer-side L1 actions via `subbit_console::Ctx` + `submit`/`upload_reference_script`/
//! `send_payment` over a `subbit_wallet::Embedded` wallet.

use anyhow::Context;
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

/// Which keyring namespace `Chain::register` flags an identity under --
/// `Consumer` for every consumer (and the claim wallet, which
/// only ever needs to sign its own fuel inputs), `Provider` for the one
/// provider identity, so `Chain::stage_settle`'s admin Will can actually
/// find a local key covering its required signer.
pub enum Role {
    Consumer,
    Provider,
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

    /// Registers `seed`'s derived identity as a keyring entry under `label`, flagged
    /// `is_consumer` or `is_provider` per `role` -- never both, since no canary identity is
    /// ever genuinely both at once.
    pub fn register(
        &mut self,
        label: impl Into<String>,
        seed: [u8; 32],
        role: Role,
    ) -> anyhow::Result<()> {
        let (is_consumer, is_provider) = match role {
            Role::Consumer => (true, false),
            Role::Provider => (false, true),
        };
        self.ctx
            .keyring_add(
                label.into(),
                SigningKeyHex::from(seed),
                is_consumer,
                false,
                is_provider,
            )
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Admin-only: stages a `Settle` for `keytag` using an already-signed `iou` -- the one
    /// chain action canary performs directly as the provider, rather than via the spawned
    /// `subbit-claim`. Used by `tidy`'s own sweep, which can't rely on `subbit-claim`/
    /// `subbit-gate`'s IOU record (ephemeral, per-run -- see `simplify.md`) once those
    /// processes have already exited. Submitting is the caller's job, same as `want`/`flush`.
    pub fn stage_settle(&mut self, keytag: &str, iou: subbit_core::Iou) -> anyhow::Result<()> {
        self.ctx
            .settle(keytag, None, iou.amount(), *iou.signature())
            .map(|_| ())
            .map_err(|e| anyhow::anyhow!("{e}"))
    }

    /// Re-reads the validator address's live txos, plus the funding wallet's own reference-script utxo.
    pub async fn refresh(&mut self) -> anyhow::Result<()> {
        let pay_hash: [u8; 28] = <[u8; 28]>::from(VALIDATOR.hash);
        let items = retry_transient!(
            subbit_index::cardano::search_all(&mut self.discovery, pay_hash, None).await
        );
        self.ctx.basket_refresh_txos(self.network, items);

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

    /// `fund_address`'s reverse: sweeps whatever lovelace `wallet_seed`'s own address
    /// currently holds back into the main funding wallet, signed by `wallet_seed` itself
    /// (`Chain`'s own wallet has no spending authority over it). Opens a fresh connection to
    /// `floe_endpoint` to build that one-off signer -- `Chain`'s own `wallet`/`discovery`
    /// connections are tied to its own signing key. Batches across as many transactions as
    /// it takes (`MAX_INPUTS_PER_TX` utxos at a time) -- a long-running deployment's claim
    /// wallet can easily accumulate far more utxos (one change output per Sub/Settle batch)
    /// than fit in a single tx. Returns how many sweep transactions it took, `0` if there
    /// was nothing worth reclaiming.
    pub async fn reclaim_wallet(
        &mut self,
        floe_endpoint: &str,
        wallet_seed: [u8; 32],
    ) -> anyhow::Result<usize> {
        // Generous headroom under `default_max_tx_size` for plain (datum-free) inputs --
        // each costs ~40B, so even 150 of them (~6KB) leaves plenty of room for the one
        // output, one witness, and fee.
        const MAX_INPUTS_PER_TX: usize = 150;
        const RECLAIM_MARGIN: u64 = 1_000_000;

        let signing_key = cardano_sdk::SigningKey::from(wallet_seed);
        let backend = Backend::connect(floe_endpoint).await?;
        let mut wallet = Embedded::new(backend, signing_key, self.network_id, None);
        let target = self.change_address_shelley().into();
        Ok(subbit_console::sweep(
            &mut wallet,
            self.network,
            target,
            MAX_INPUTS_PER_TX,
            RECLAIM_MARGIN,
            subbit_wallet::LandingPolicy::PATIENT,
        )
        .await?)
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

    /// Stages `want` against whichever utxo currently backs the channel
    /// identified by `keytag` -- re-resolved fresh at pick time, robust to
    /// that channel's own utxo moving underneath it between this round's
    /// `refresh` and this flush/submit (e.g. a concurrent `subbit-claim`
    /// Sub/Settle landing on the same channel). `true` if accepted.
    pub fn want_by_id(&mut self, keytag: impl Into<String>, want: Want) -> bool {
        self.ctx.basket_propose_by_id(keytag, want).is_ok()
    }

    /// Submits every currently staged Open/Will, batched/retried, then refreshes.
    pub async fn flush(&mut self) -> anyhow::Result<()> {
        if self.ctx.basket_opens().is_empty() && self.ctx.basket_wills().is_empty() {
            return Ok(());
        }
        // Snapshot the channel inputs this flush is about to target *before*
        // `submit` consumes/rolls back the basket. `submit`'s own internal
        // `wait_for_landing` only ever watches the one hash it itself submitted --
        // it has no way to notice a genuinely contending tx (most likely
        // `subbit-claim`'s own concurrent `Sub`/`Settle` on the very same
        // channel, which is expected to happen, not a bug to prevent) consuming
        // one of these same inputs and landing its effect under a different
        // hash entirely. `subbit-claim` already handles this for its own
        // submissions via `Pending::advance`'s Dead-on-missing-input rule
        // (`packages/claim/src/pending.rs`) -- this mirrors that: report,
        // rollback (nothing more to do -- `submit` already rolled back its own
        // attempt on error), and let the *next* round's fresh `refresh`/`want`
        // re-pick and resubmit, rather than treating "my tx didn't
        // land" as proof the operation never happened.
        let targeted_channel_inputs: std::collections::BTreeSet<cardano_sdk::Input> =
            self.ctx.basket_wills().keys().cloned().collect();
        // `NothingStaged` can surface *mid-submit* too, not just from the empty-basket
        // check above: a staged want can turn out stale (its channel moved under it,
        // racing a concurrent step) and get dropped once `submit` is already underway,
        // leaving nothing left to actually send. That's the race resolving itself, not
        // a failure -- same as the precondition above, just discovered later.
        if let Err(e) = subbit_console::submit(&mut self.wallet, &mut self.ctx).await {
            if !matches!(e, subbit_console::Error::NothingStaged) {
                self.refresh()
                    .await
                    .context("refreshing to check for contention after a lifecycle-step error")?;
                let live = self.channels();
                let contended: Vec<cardano_sdk::Input> = targeted_channel_inputs
                    .iter()
                    .filter(|i| !live.contains_key(i))
                    .cloned()
                    .collect();
                if contended.is_empty() {
                    return Err(e.into());
                }
                println!(
                    "canary: a lifecycle step didn't confirm landing under its own tx, but \
                     {}/{} targeted channel input(s) were already spent by something \
                     else (most likely subbit-claim's own concurrent activity) -- treating this \
                     step's effect as resolved rather than erroring ({e:#})",
                    contended.len(),
                    targeted_channel_inputs.len(),
                );
                // `submit` errored out before ever calling `ctx.basket_finalize` for this
                // attempt, so these wants are still sitting in the basket, now stale (their
                // own input is confirmed gone). Left alone, the *next* `flush` would re-pick
                // them and `Queue::pick` would legitimately find nothing eligible to build --
                // a hard, uncaught `BuildError::Empty` (confirmed live: this exact crash
                // happened immediately after an earlier version of this recovery path logged
                // success but never dropped the stale intent). Drop them explicitly, same as
                // `submit`'s own stale-intent-drop branch does for a rejection caught before
                // the landing wait.
                for input in &contended {
                    self.ctx.basket_drop_intent(input);
                }
            }
        }
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

    /// Polls `wallet.utxos(None)` until `hash`'s own utxo appears. Canary is not a
    /// latency-sensitive caller, so it opts into the patient policy rather than the
    /// shared default.
    async fn wait_for_landing(&mut self, hash: cardano_sdk::Hash<32>) -> anyhow::Result<()> {
        Ok(subbit_wallet::wait_for_landing_with(
            &mut self.wallet,
            hash,
            tokio::time::sleep,
            subbit_wallet::LandingPolicy::PATIENT,
        )
        .await?)
    }
}
