//! Owns the funding session, every registered signer, and both HTTP
//! round trips against the provider (`index`, `claim_tick`). Actors read
//! `can()`, decide a `Want`, hand it to `want()`; `flush()` batches and
//! submits.
//!
//! Generic over `C: CardanoConnector` -- `MockChain` for `--mode mock`,
//! `cardano_connector_direct::Blockfrost` for `--mode preprod`.

use std::collections::BTreeMap;
use std::sync::Arc;

use anyhow::Context;
use cardano_connector::CardanoConnector;
use cardano_connector_direct::Blockfrost;
use cardano_sdk::{Hash, Input, Output, SigningKey, Value, cbor::ToCbor};
use cardano_session::{Session as CardanoSession, SubmitVia, Waiter, waiter};
use cardano_wallet::Embedded;
use subbit_core::{Duration, VerifyingKey};
use subbit_l1_dto::{Constants, Stage};
use subbit_session::{Keyring, Session};
use subbit_tx::step::{Can, Want};
use subbit_tx::{Channel, Open, Variables, tx::Tx};

use crate::mock_chain::MockChain;

type ChainSession<C> = Session<C, Embedded<C>>;

/// Generous one-off "genesis" deposit, large enough to cover every
/// channel a scenario opens plus fees.
const GENESIS_LOVELACE: u64 = 1_000_000_000_000;

pub struct L1<C: CardanoConnector> {
    session: ChainSession<C>,
    signers: Vec<SigningKey>,
    /// Decided, not yet submitted.
    wills: Vec<(Input, Want)>,
    /// Submitted, not yet confirmed.
    pending: Vec<Hash<32>>,
}

impl L1<MockChain> {
    pub async fn build_mock() -> anyhow::Result<(Arc<MockChain>, Self)> {
        let chain = Arc::new(MockChain::new());
        let wallet = Embedded::new(chain.clone(), cardano_wallet::Config::default(), None);
        let cardano = CardanoSession::new(
            chain.clone(),
            wallet,
            Waiter::new(waiter::Config::default()),
            SubmitVia::default(),
        )
        .await
        .map_err(|e| anyhow::anyhow!("building mock cardano session: {e}"))?;

        let input = Input::new(Hash::<32>::from([0u8; 32]), 0);
        let output = Output::new(
            cardano.change_address().into(),
            Value::new(GENESIS_LOVELACE),
        );
        chain.seed(input, output);

        let mut session = Session::new(cardano, None, Default::default());
        session
            .refresh_all()
            .await
            .context("pulling the seeded genesis UTxO into the session's tip cache")?;
        upload_validator_if_missing(&mut session).await?;

        Ok((chain, Self::new(session)))
    }
}

impl L1<Blockfrost> {
    /// `signing_key` is the funding wallet's own raw ed25519 key -- an
    /// already-funded preprod wallet, supplied by the caller, not
    /// generated here. `project_id` is a Blockfrost preprod project id.
    pub async fn build_preprod(project_id: String, signing_key: [u8; 32]) -> anyhow::Result<Self> {
        let connector = Arc::new(Blockfrost::new(project_id));
        let wallet_config: cardano_wallet::Config =
            toml::from_str(&format!("key = \"{}\"", hex::encode(signing_key)))
                .context("building wallet config from the supplied signing key")?;
        let wallet = Embedded::new(connector.clone(), wallet_config, None);
        // The default `Waiter` (5s x 12 = 60s) assumes mock-chain instant
        // confirmation; real preprod blocks land every ~20s. This
        // `Waiter` (from the vendored `cardano-session` dependency) only
        // supports a fixed poll interval -- `wait_for_confirmation`
        // below adds growing backoff on top, retrying this whole 5min
        // window with longer pauses if it's a genuine stall rather than
        // a long block.
        let waiter = Waiter::new(waiter::Config {
            interval: std::time::Duration::from_secs(10),
            max_attempts: 30,
        });
        let cardano = CardanoSession::new(connector, wallet, waiter, SubmitVia::default())
            .await
            .map_err(|e| anyhow::anyhow!("building preprod cardano session: {e}"))?;

        let mut session = Session::new(cardano, None, Default::default());
        session
            .refresh_all()
            .await
            .context("fetching the funding wallet's current UTxOs from preprod")?;
        upload_validator_if_missing(&mut session).await?;

        let mut l1 = Self::new(session);
        // Wallet and validator addresses index separately and can lag
        // each other; settle that once here, not per caller.
        for _ in 0..2 {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            l1.refresh().await?;
        }

        Ok(l1)
    }
}

impl<C: CardanoConnector> L1<C> {
    fn new(session: ChainSession<C>) -> Self {
        Self {
            session,
            signers: Vec::new(),
            wills: Vec::new(),
            pending: Vec::new(),
        }
    }

    /// Registers a signing key so `flush`/`open` can produce whatever
    /// witness a proposed `Want`/`Open` ends up requiring. Every actor
    /// (provider, each consumer) registers once, up front.
    pub fn register(&mut self, signing_key: SigningKey) {
        self.signers.push(signing_key);
    }

    /// For ownership checks against this run's own registered signers
    /// (e.g. `subbit_claim::keytag::is_ours`) -- `flush`/`open` also use
    /// this internally to sign.
    pub fn keyring(&self) -> Keyring {
        Keyring::new(self.signers.iter().cloned())
    }

    pub async fn refresh(&mut self) -> anyhow::Result<()> {
        self.session
            .refresh_channels()
            .await
            .context("refreshing tracked channels")
    }

    pub fn channels(&self) -> BTreeMap<Input, Output> {
        self.session.channels()
    }

    /// Every channel matching `(iou_key, tag)`, with its current
    /// options. 0 results means no channel yet; 1 is the expected case;
    /// more than 1 isn't refused here (unlike a tag-only lookup would
    /// have to) -- `can` just reports what's there, the caller decides
    /// what a collision means.
    pub fn can(&self, iou_key: &VerifyingKey, tag: &[u8]) -> Vec<(Input, Vec<Can>)> {
        self.session
            .channels()
            .into_iter()
            .filter_map(|(input, output)| {
                let channel = Channel::try_from(&output).ok()?;
                let constants = channel.constants();
                (constants.iou_key() == iou_key && constants.tag().as_ref() == tag)
                    .then(|| (input, channel.can()))
            })
            .collect()
    }

    /// `can` resolved to at most the one expected match: refuses
    /// outright on more than one (e.g. a stale channel from an earlier
    /// run under the same `L2_SEED_HEX`) rather than guessing. Zero
    /// matches is `Ok(None)`, not an error -- a live tracked channel
    /// legitimately disappears once it's fully wound down (`Elapse`,
    /// `End`), so "not found" is an expected outcome every caller here
    /// already has to handle, not a failure.
    pub fn at_most_one(
        &self,
        iou_key: &VerifyingKey,
        tag: &[u8],
    ) -> anyhow::Result<Option<(Input, Vec<Can>)>> {
        let results = self.can(iou_key, tag);
        match results.len() {
            0 => Ok(None),
            1 => Ok(Some(results.into_iter().next().unwrap())),
            n => anyhow::bail!(
                "{n} tracked channels share tag {tag:?} -- refusing to guess which one; \
                 likely an old channel from a previous run with the same L2_SEED_HEX is \
                 still open on this chain (see `canary --tidy` to reclaim it first)"
            ),
        }
    }

    /// Queues `want` against `input` for `flush`. Returns `false`
    /// without queuing an `Elapse{lower}` whose window hasn't opened
    /// yet -- a real chain would refuse it anyway; the caller just
    /// offers it again next round.
    pub fn want(&mut self, input: Input, want: Want) -> bool {
        if let Want::Elapse { lower } = &want
            && crate::time::now() < *lower
        {
            return false;
        }
        self.wills.push((input, want));
        true
    }

    /// Drains queued wills into however many transactions the real
    /// per-tx budget takes, backing off (see `back_off`) on rejection.
    /// Waits for each batch to confirm before building the next: today's
    /// single funding wallet means the next tx's own fee input doesn't
    /// exist until the previous one lands. Finer-grained wallet
    /// management could dispatch batches without waiting between them
    /// and confirm them all via one later `wait_til` -- that would
    /// change this loop's internals, not its signature.
    pub async fn flush(&mut self) -> anyhow::Result<()> {
        while !self.wills.is_empty() {
            let mut attempt = self.wills.len();
            loop {
                let batch = self.wills[..attempt].to_vec();
                match self.submit_steps(batch).await {
                    Ok(id) => {
                        println!("canary: submitted {id} with {attempt} want(s)");
                        self.wills.drain(..attempt);
                        self.pending.push(id);
                        self.wait_til().await?;
                        break;
                    }
                    Err(e) if attempt > 1 => {
                        let next = back_off(attempt, &e);
                        println!(
                            "canary: batch of {attempt} want(s) rejected ({e:#}) -- retrying with {next}"
                        );
                        attempt = next;
                    }
                    Err(e) => return Err(e).context("a single-want batch was still rejected"),
                }
            }
        }
        Ok(())
    }

    /// Waits until every tx dispatched so far (by `flush` or `open`) is
    /// confirmed.
    pub async fn wait_til(&mut self) -> anyhow::Result<()> {
        while let Some(id) = self.pending.pop() {
            self.wait_for_confirmation(&id).await?;
            // `wait_til` confirms via the funding wallet's own address;
            // the script address's own UTxO set (what `can`/`channels`
            // read) is a separate index that can lag a few seconds
            // behind. Refresh again after this sleep, not just rely on
            // `wait_til`'s own refresh (which ran before the wait).
            tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            self.session
                .refresh_channels()
                .await
                .context("refreshing tracked channels after the post-confirmation buffer")?;
        }
        Ok(())
    }

    /// Confirms `id`, retrying the whole poll window (the preprod
    /// `Session`'s own `Waiter`, 10s x 30 attempts ~= 5min) with growing
    /// backoff between retries if it times out: 30s, 60s, 120s, capped
    /// at `max_backoff`, up to `MAX_CONFIRM_RETRIES` whole windows (worst
    /// case: 4 x 5min inner windows plus ~3.5min of backoff, ~24min
    /// total). A single timed-out window isn't necessarily a dead tx --
    /// Blockfrost indexing can lag behind the tip, or a run is competing
    /// with other preprod traffic -- so this gives a stuck confirmation
    /// real runway before surfacing the error, instead of the old
    /// behavior of giving up after one fixed 5min window.
    async fn wait_for_confirmation(&mut self, id: &Hash<32>) -> anyhow::Result<()> {
        const MAX_CONFIRM_RETRIES: u32 = 4;
        let max_backoff = std::time::Duration::from_secs(120);
        let mut backoff = std::time::Duration::from_secs(30);
        for retry in 0..MAX_CONFIRM_RETRIES {
            match self.session.wait_til(id).await {
                Ok(()) => return Ok(()),
                Err(e) if retry + 1 < MAX_CONFIRM_RETRIES => {
                    println!(
                        "canary: {id} still unconfirmed after one poll window ({e}) -- \
                         backing off {backoff:?} before retrying (attempt {}/{MAX_CONFIRM_RETRIES})",
                        retry + 1
                    );
                    tokio::time::sleep(backoff).await;
                    backoff = (backoff * 2).min(max_backoff);
                }
                Err(e) => {
                    return Err(e).context("waiting for tx and refreshing channels");
                }
            }
        }
        unreachable!("loop above always returns by its last iteration")
    }

    async fn submit_steps(&mut self, wants: Vec<(Input, Want)>) -> anyhow::Result<Hash<32>> {
        let mut tx = self.session.stage_tx().context("staging batched tx")?;
        for (input, want) in wants {
            tx.propose(input, want).context("proposing batched step")?;
        }
        self.submit(&mut tx).await
    }

    /// Opens every `(Constants, amount)` pair, batched and saturated the
    /// same way `flush` is -- doesn't go through `wills`, since there's
    /// no existing channel yet to read `Can` from.
    pub async fn open(&mut self, mut channels: Vec<(Constants, u64)>) -> anyhow::Result<()> {
        while !channels.is_empty() {
            let mut attempt = channels.len();
            loop {
                match self.open_batch(channels[..attempt].to_vec()).await {
                    Ok(id) => {
                        println!("canary: submitted {id} with {attempt} want(s)");
                        channels.drain(..attempt);
                        self.pending.push(id);
                        self.wait_til().await?;
                        break;
                    }
                    Err(e) if attempt > 1 => {
                        let next = back_off(attempt, &e);
                        println!(
                            "canary: Open batch of {attempt} rejected ({e:#}) -- retrying with {next}"
                        );
                        attempt = next;
                    }
                    Err(e) => return Err(e).context("a single-channel Open was still rejected"),
                }
            }
        }
        Ok(())
    }

    async fn open_batch(&mut self, channels: Vec<(Constants, u64)>) -> anyhow::Result<Hash<32>> {
        let mut tx = self
            .session
            .stage_tx()
            .context("staging tx for batched Open")?;
        for (constants, amount) in channels {
            let channel = Channel::new(
                constants,
                Variables::new(amount, Stage::Opened { subbed: 0 }),
            );
            tx.add_open(Open::new(channel, None));
        }
        self.submit(&mut tx).await
    }

    async fn submit(&mut self, tx: &mut Tx) -> anyhow::Result<Hash<32>> {
        let mut built = self.session.build_tx(tx).context("building tx")?;
        self.keyring()
            .sign(&mut built)
            .map_err(|e| anyhow::anyhow!("signing required consumer/provider witnesses: {e}"))?;
        // Measured after signing, not before: consumer/provider witnesses
        // scale with batch size (the funding wallet's own witness, added
        // by `sign_and_submit` below, doesn't) and are what actually
        // determines the real per-channel byte budget.
        println!(
            "canary: built tx is {} bytes (CBOR, consumer/provider witnesses included) -- real preprod maxTxSize is 16384",
            built.to_cbor().len()
        );
        self.session
            .sign_and_submit(built)
            .await
            .context("signing and submitting tx")
    }

    /// Mirrors the real `naive-index` binary
    /// (`subbit_index::naive::rows_from_channels` +
    /// `subbit_index::client::Client`), reading this run's own tracked
    /// channels directly instead of spawning it as a separate process.
    pub async fn index(
        &self,
        provider_base_url: &str,
        provider_identity: subbit_core::Hash28,
        close_period: Duration,
    ) -> anyhow::Result<()> {
        let cfg = subbit_index::naive::Config {
            cardano: Default::default(),
            delegations: Default::default(),
            endpoint: format!("{provider_base_url}/v1/a/backings").parse()?,
            provider: provider_identity,
            asset: subbit_l1_dto::Asset::Lovelace,
            close_period,
            max_tag_len: 64,
            poll_interval_secs: 30,
        };
        let rows = subbit_index::naive::rows_from_channels(&self.session.channels(), &cfg);
        subbit_index::client::Client::new(cfg.endpoint.clone())
            .send(rows)
            .await
            .context("posting backing rows")?;
        Ok(())
    }

    /// `index`'s complement: pulls the provider's channel summaries and
    /// submits whatever real `subbit-claim::decide` logic says this
    /// run's channels need. `exclude` skips a channel on purpose (e.g.
    /// to demonstrate `Elapse` instead). Returns wants submitted.
    pub async fn claim_tick(
        &mut self,
        provider_base_url: &str,
        close_period: Duration,
        exclude: &std::collections::BTreeSet<Vec<u8>>,
    ) -> anyhow::Result<usize> {
        let channels = self.owned_channels(exclude);
        let client = subbit_claim::client::Client::new(
            format!("{provider_base_url}/v1/a/channels")
                .parse()
                .expect("provider_base_url is already a valid URL"),
        );
        let summaries = client.channel_summaries().await?;
        // min_total/min_single=1, panic_settle generous: a demo, not a
        // deployment economizing on fees or tracking is_active itself.
        let cfg = subbit_claim::decide::DecisionConfig {
            min_total: 1,
            min_single: 1,
            panic_settle: close_period + Duration::from_secs(3600),
        };
        let wants = subbit_claim::decide::decide(&channels, &summaries, &cfg, crate::time::now());
        let count = wants.len();
        for (input, want) in wants {
            self.want(input, want);
        }
        self.flush().await?;
        Ok(count)
    }

    /// This run's own channels, minus any tag in `exclude`.
    fn owned_channels(
        &self,
        exclude: &std::collections::BTreeSet<Vec<u8>>,
    ) -> BTreeMap<Input, Channel> {
        let keyring = self.keyring();
        self.channels()
            .into_iter()
            .filter_map(|(input, output)| {
                let channel = Channel::try_from(&output).ok()?;
                let constants = channel.constants();
                if !subbit_claim::keytag::is_ours(constants, &keyring)
                    || exclude.contains(constants.tag().as_ref())
                {
                    return None;
                }
                Some((input, channel))
            })
            .collect()
    }
}

/// Reuses an already-uploaded reference script if one is tracked
/// (avoids paying to re-upload on every run against a real chain);
/// uploads fresh otherwise -- always the case for a brand-new mock.
async fn upload_validator_if_missing<C: CardanoConnector>(
    session: &mut ChainSession<C>,
) -> anyhow::Result<()> {
    if session.ref_script().is_ok() {
        println!("canary: reusing an already-tracked reference script");
        return Ok(());
    }
    session
        .upload()
        .await
        .context("uploading the subbit validator as a reference script")?;
    session
        .refresh_all()
        .await
        .context("pulling the uploaded reference script into the session's tip cache")?;
    Ok(())
}

/// Picks the next attempted count after `attempt` was rejected by `err`.
/// A `MaxTxSizeUTxO` rejection names its own exact `supplied`/`expected`
/// byte counts; per-channel cost is roughly uniform, so scaling `attempt`
/// down by `expected/supplied` (5% safety margin for the non-uniform
/// part: tag/IOU/redeemer sizes vary a little channel to channel) lands
/// much closer to the real ceiling in one step than blind halving.
/// Anything else (a local exec-unit overflow with no parseable numbers,
/// or an unrecognized rejection) falls back to halving.
fn back_off(attempt: usize, err: &anyhow::Error) -> usize {
    let message = format!("{err:#}");
    if let Some((supplied, expected)) = parse_max_tx_size_mismatch(&message) {
        if expected < supplied {
            let scaled =
                (attempt as u128 * expected as u128 * 95 / (supplied as u128 * 100)) as usize;
            return scaled.clamp(1, attempt.saturating_sub(1).max(1));
        }
    }
    (attempt / 2).max(1)
}

/// Pulls `supplied`/`expected` out of a `MaxTxSizeUTxO
/// Mismatch {supplied: N, expected: M}` rejection's own message.
fn parse_max_tx_size_mismatch(message: &str) -> Option<(u64, u64)> {
    let parse_after = |needle: &str| -> Option<u64> {
        let start = message.find(needle)? + needle.len();
        message[start..]
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .ok()
    };
    Some((parse_after("supplied: ")?, parse_after("expected: ")?))
}
