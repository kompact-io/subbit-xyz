//! A logical Consumer: an ephemeral on-chain identity (for
//! `Constants.consumer`/`.iou_key`, unrelated to the shared funding
//! wallet `l1.rs` owns) plus a real `subbit_stamp::Stamp` driving real
//! HTTP spend/refund against a Provider, in either wire dialect -- the
//! same `Arc<Stamp<C>>` + forwarding-`Middleware` pattern
//! `examples/echo/client` and the old `packages/e2e` harness used.
//!
//! `persona`/`roster` own everything about turning a `--persona` roster
//! into real consumers (behavior sampling, identity/tag derivation) --
//! `Orchestrator::build` just calls `roster::plan_roster` and builds
//! what it gets back.

pub mod persona;
mod roster;

pub(crate) use roster::{identity_for, plan_roster};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use anyhow::Context;
use http::Extensions;
use reqwest::{Request, Response};
use reqwest_middleware::{
    ClientBuilder, ClientWithMiddleware, Middleware, Next, Result as MwResult,
};
use subbit_core::{Duration, Tag, VerifyingKey};
use subbit_l2_dto::{Catalog, CatalogIndex, Terms};
use subbit_stamp::reqwest::vanilla::Stamp as VanillaMiddleware;
use subbit_stamp::reqwest::x402::Stamp as X402Middleware;
use subbit_tx::step::{Can, Want};

use crate::identity::hash28_from_seed;

#[derive(Debug, Clone, Copy, PartialEq, Eq, clap::ValueEnum)]
pub enum Dialect {
    Vanilla,
    X402,
}

enum Driver {
    Vanilla(Arc<VanillaMiddleware<CatalogIndex>>),
    X402(Arc<X402Middleware<CatalogIndex>>),
}

pub struct Consumer {
    /// Never exposed beyond this module: `connect` needs it to build
    /// the real signing `Account` once it has a real catalog to pair it
    /// with.
    seed: [u8; 32],
    /// Where this consumer spends against -- just the provider's own
    /// base URL, nothing resolved from it. `connect` fetches `/v1/terms`
    /// from here; `spend` hits `/v1/spend/echo` here.
    provider_base_url: String,
    /// Set once by `tidy`, never cleared: from then on `want` always
    /// heads for the exit (`Close`, then `Elapse`) instead of consulting
    /// `add_amounts`/`spend_counts`. `AtomicBool` rather than `&mut
    /// self` since every other method here takes `&self` -- consumers
    /// are shared via `Arc` across concurrent spend tasks.
    tidying: AtomicBool,
    /// Set by `tidy_once_closed`: `want` only actually starts `tidy`ing
    /// once this consumer's own `Close` lands, not before -- see its own
    /// doc comment.
    tidy_on_close: AtomicBool,
    pub tag: Vec<u8>,
    /// consumer's `Constants.consumer` identity.
    pub l1_key: subbit_core::Hash28,
    pub iou_key: VerifyingKey,
    pub dialect: Dialect,
    /// Built by `connect`, from the provider's own real catalog -- `None`
    /// until then (e.g. for a consumer that's never spent against at
    /// all, like tidy's placeholders, which skip `connect` entirely).
    driver: Option<Driver>,
    client: Option<ClientWithMiddleware>,
    /// Behavior plan, as resolved by `ConsumerBuilder` (or a `Persona` it
    /// was built from). Unset (`deposit: 0`, no `Add`s, one spend round)
    /// for a plain `Consumer::new`, which carries no behavior of its own
    /// -- the driving scenario decides what to do in that case, same as
    /// before.
    pub deposit: u64,
    /// One entry per `Want::Add` this consumer should perform
    /// mid-lifecycle, in order -- also what `rounds` counts from: round
    /// count is an L1 scheduling concern (how many Add/Close cycles),
    /// not something the spend side needs to expose separately.
    pub add_amounts: Vec<u64>,
    /// One entry per spend round -- `spend_counts.len() ==
    /// add_amounts.len() + 1`. See `firehose_l2`.
    spend_counts: Vec<usize>,
    /// Delay between each spend request within a round -- see
    /// `persona::Behavior::pace`.
    pace: std::time::Duration,
    /// See `persona::Behavior::burst` -- overrides `firehose_l2`'s
    /// fixed-count loop entirely when set.
    burst: Option<persona::BurstStop>,
    /// The persona name this consumer was built from, if any -- for
    /// logging only.
    pub persona: String,
}

struct ForwardVanilla(Arc<VanillaMiddleware<CatalogIndex>>);

#[async_trait::async_trait]
impl Middleware for ForwardVanilla {
    async fn handle(
        &self,
        req: Request,
        ext: &mut Extensions,
        next: Next<'_>,
    ) -> MwResult<Response> {
        self.0.handle(req, ext, next).await
    }
}

struct ForwardX402(Arc<X402Middleware<CatalogIndex>>);

#[async_trait::async_trait]
impl Middleware for ForwardX402 {
    async fn handle(
        &self,
        req: Request,
        ext: &mut Extensions,
        next: Next<'_>,
    ) -> MwResult<Response> {
        self.0.handle(req, ext, next).await
    }
}

impl Consumer {
    /// `seed`/`tag` name this consumer deterministically.
    /// `provider_base_url` is where it spends against (a teardown-only
    /// consumer that's never spent against can pass any placeholder).
    /// `catalog` is optional: a real consumer leaves it `None` here and
    /// calls `connect` to fetch the provider's real one before spending;
    /// a consumer that never spends at all (tidy's placeholders) can
    /// skip `connect` entirely and just stay disconnected.
    pub fn new(
        seed: [u8; 32],
        tag: impl Into<Vec<u8>>,
        catalog: Option<Catalog>,
        dialect: Dialect,
        provider_base_url: impl Into<String>,
    ) -> anyhow::Result<Self> {
        let tag: Vec<u8> = tag.into();
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&seed);
        let iou_key = VerifyingKey::from(signing_key.verifying_key().to_bytes());

        let mut consumer = Self {
            seed,
            l1_key: hash28_from_seed(&seed),
            iou_key,
            provider_base_url: provider_base_url.into(),
            tidying: AtomicBool::new(false),
            tidy_on_close: AtomicBool::new(false),
            tag,
            dialect,
            driver: None,
            client: None,
            deposit: 0,
            add_amounts: Vec::new(),
            spend_counts: vec![1],
            pace: std::time::Duration::ZERO,
            burst: None,
            persona: String::new(),
        };
        if let Some(catalog) = catalog {
            consumer.build_driver(catalog)?;
        }
        Ok(consumer)
    }

    /// Fetches the provider's own published `Terms` from
    /// `{provider_base_url}/v1/terms` (the same endpoint regardless of
    /// `dialect` -- Terms discovery isn't dialect-specific) and builds
    /// this consumer's own HTTP driver from the real catalog it returns,
    /// rather than being handed one it never asked for. Needed before
    /// `spend`/`firehose_l2`; not needed for a consumer that never
    /// spends at all (tidy's placeholders skip this entirely).
    pub async fn connect(&mut self) -> anyhow::Result<()> {
        let terms: Terms = reqwest::get(format!("{}/v1/terms", self.provider_base_url))
            .await
            .context("fetching provider terms")?
            .json()
            .await
            .context("parsing provider terms")?;
        self.build_driver(terms.catalog)
    }

    fn build_driver(&mut self, catalog: Catalog) -> anyhow::Result<()> {
        let signing_key = ed25519_dalek::SigningKey::from_bytes(&self.seed);
        let account = subbit_stamp::Account::new(signing_key, Tag::from(self.tag.clone()));
        let stamp = subbit_stamp::Stamp::new(account, Duration::from_secs(3600));
        let costing = CatalogIndex::new(catalog).context("indexing provider catalog")?;

        let (driver, client) = match self.dialect {
            Dialect::Vanilla => {
                let middleware = Arc::new(VanillaMiddleware::new(stamp, costing));
                let client = ClientBuilder::new(reqwest::Client::new())
                    .with(ForwardVanilla(middleware.clone()))
                    .build();
                (Driver::Vanilla(middleware), client)
            }
            Dialect::X402 => {
                let middleware = Arc::new(X402Middleware::new(stamp, costing));
                let client = ClientBuilder::new(reqwest::Client::new())
                    .with(ForwardX402(middleware.clone()))
                    .build();
                (Driver::X402(middleware), client)
            }
        };
        self.driver = Some(driver);
        self.client = Some(client);
        Ok(())
    }

    /// Spends against `{provider_base_url}/v1/spend/echo`. x402's first
    /// request against any given path it hasn't paid under yet always
    /// comes back as an `Err` from the middleware itself
    /// (`apply_required` always returns `Err`,
    /// even though it learns `accepted` as a side effect -- see
    /// `subbit_stamp::dialect::x402::apply_required`'s own doc
    /// comment): that's expected, not a real failure, so retry once now
    /// that terms are learned, the same way `examples/echo/client`
    /// does. Vanilla has no such dance, so it's a single request.
    pub async fn spend(&self) -> anyhow::Result<Response> {
        let client = self
            .client
            .as_ref()
            .context("spend called before connect -- no HTTP driver to spend with")?;
        let url = format!("{}/v1/spend/echo", self.provider_base_url);
        match self.dialect {
            Dialect::Vanilla => client.get(&url).send().await.context("spend request"),
            Dialect::X402 => match client.get(&url).send().await {
                Ok(resp) => Ok(resp),
                Err(_first_discovers_terms) => client.get(&url).send().await.context("spend retry"),
            },
        }
    }

    /// This consumer's own decision for `round`: `End` once `Settled`,
    /// `tidy_want` if tidying, else an `Add` due this round, a `Close`
    /// on the last round, or nothing. States intent only -- `L1::want`
    /// decides if it's actually submittable yet.
    pub fn want(&self, round: usize, can: &[Can], close_upper: Duration) -> Option<Want> {
        if can.contains(&Can::End) {
            return Some(Want::End);
        }
        if self.tidying.load(Ordering::Relaxed) {
            return self.tidy_want(can, close_upper);
        }
        if let Some(&amount) = self.add_amounts.get(round) {
            return can.contains(&Can::Add).then_some(Want::Add { amount });
        }
        if round + 1 == self.rounds() {
            let want = can
                .contains(&Can::Close)
                .then_some(Want::Close { upper: close_upper });
            // tidy from here on, once it lands -- same spend/Add
            // schedule as any other consumer right up to this point, it
            // just skips the real provider's `subbit-claim` Settle
            // afterward (see `tidy_on_close`).
            if want.is_some() && self.tidy_on_close.load(Ordering::Relaxed) {
                self.tidy();
            }
            return want;
        }
        None
    }

    /// From now on, `want` always heads for the exit: `Close` once
    /// `Opened`, `Elapse` once `Closed`. Skips `Settle` entirely --
    /// that's a live provider's own real claim decision
    /// (`subbit-claim`), which a tidying consumer doesn't wait around
    /// for (teardown has no provider at all).
    pub fn tidy(&self) {
        self.tidying.store(true, Ordering::Relaxed);
    }

    /// Marks this consumer to `tidy` the moment its own `Close` lands --
    /// not immediately: a demo straggler still spends its full persona
    /// schedule like anyone else, it just skips the real provider's
    /// `Settle` once it's done, to demonstrate `Elapse` instead.
    pub fn tidy_once_closed(&self) {
        self.tidy_on_close.store(true, Ordering::Relaxed);
    }

    pub fn is_tidying(&self) -> bool {
        self.tidying.load(Ordering::Relaxed)
    }

    /// `Close` while `Opened`; once `Closed`, `Elapse`. See `want`.
    fn tidy_want(&self, can: &[Can], close_upper: Duration) -> Option<Want> {
        for c in can {
            match c {
                Can::Close => return Some(Want::Close { upper: close_upper }),
                // 1s, then 20s, both measured too tight on real preprod
                // (seen live: a submission still rejected 33 slots short
                // of `lower` even with the 20s margin) -- queued wants
                // ahead of this one in the same batch delay actual
                // submission past the readiness check in `L1::want`, and
                // preprod's own block production can stall well past
                // either margin. 120s matches `close_upper`'s own
                // established margin elsewhere in this module for the
                // same reason (measured confirmation lag tops out around
                // 70-80s) -- tidy/teardown has no urgency to shave this
                // down, only to not fail.
                // (hit live: `OutsideValidityIntervalUTxO`, 4 slots
                // short).
                Can::Elapse { after } => {
                    return Some(Want::Elapse {
                        lower: *after + Duration::from_secs(120),
                    });
                }
                _ => {}
            }
        }
        None
    }

    /// How many rounds this consumer's own lifecycle has -- an L1
    /// scheduling fact (`add_amounts.len() + 1`: every `Add` plus one
    /// final round), not something the caller needs to know the spend
    /// side's own shape to derive.
    pub fn rounds(&self) -> usize {
        self.add_amounts.len() + 1
    }

    /// Fires this consumer's own planned spends for `round`, in order,
    /// paced by `pace`. Stops at the first failure, returns how many
    /// landed. `burst` overrides this entirely (see `spend_burst`).
    pub async fn firehose_l2(&self, round: usize) -> usize {
        if self.tidying.load(Ordering::Relaxed) {
            return 0;
        }
        if let Some(stop) = self.burst {
            return self.spend_burst(stop).await;
        }
        let Some(&count) = self.spend_counts.get(round) else {
            return 0;
        };
        let mut done = 0;
        for i in 0..count {
            match self.spend().await {
                Ok(resp) if resp.status().is_success() => done += 1,
                _ => break,
            }
            if !self.pace.is_zero() && i + 1 < count {
                tokio::time::sleep(self.pace).await;
            }
        }
        done
    }

    /// Fires requests back to back, no pacing, until `committed` reaches
    /// `stop.commit_max` or `stop.deadline` passes, whichever first. A
    /// stop by deadline (not already at `commit_max`) fires exactly one
    /// more request as a top-up before returning -- the catalog only
    /// offers a flat per-request cost, not an arbitrary IOU amount, so
    /// this lands within one request's cost of `commit_max`, not
    /// exactly on it.
    async fn spend_burst(&self, stop: persona::BurstStop) -> usize {
        let deadline = std::time::Instant::now() + stop.deadline;
        let mut done = 0;
        loop {
            if self.committed().unwrap_or(0) >= stop.commit_max {
                break;
            }
            if std::time::Instant::now() >= deadline {
                if self.committed().unwrap_or(0) < stop.commit_max
                    && let Ok(resp) = self.spend().await
                    && resp.status().is_success()
                {
                    done += 1;
                }
                break;
            }
            match self.spend().await {
                Ok(resp) if resp.status().is_success() => done += 1,
                _ => break,
            }
        }
        done
    }

    /// The consumer's current off-chain view: cumulative spend and
    /// committed balance, as tracked by its own `Stamp` cache. `None`
    /// before `connect` (no cache to read yet).
    pub fn committed(&self) -> Option<u64> {
        self.cache()?.committed()
    }

    pub fn spent(&self) -> u64 {
        self.cache().map_or(0, |c| c.spent())
    }

    fn cache(&self) -> Option<subbit_stamp::Cache> {
        Some(match self.driver.as_ref()? {
            Driver::Vanilla(m) => m.cache(),
            Driver::X402(m) => m.cache(),
        })
    }
}

/// Ergonomic `Consumer` construction: chain overrides onto a sensible
/// default (10 ADA deposit, vanilla dialect, one spend round, no `Add`s),
/// or apply a whole `persona::Behavior` at once via `behavior`.
pub struct ConsumerBuilder {
    seed: [u8; 32],
    tag: Vec<u8>,
    provider_base_url: String,
    dialect: Dialect,
    deposit: u64,
    add_amounts: Vec<u64>,
    spend_counts: Vec<usize>,
    pace: std::time::Duration,
    burst: Option<persona::BurstStop>,
    persona: String,
}

impl ConsumerBuilder {
    pub fn new(
        seed: [u8; 32],
        tag: impl Into<Vec<u8>>,
        provider_base_url: impl Into<String>,
    ) -> Self {
        Self {
            seed,
            tag: tag.into(),
            provider_base_url: provider_base_url.into(),
            dialect: Dialect::Vanilla,
            deposit: 10_000_000,
            add_amounts: Vec::new(),
            spend_counts: vec![1],
            pace: std::time::Duration::ZERO,
            burst: None,
            persona: String::new(),
        }
    }

    pub fn dialect(mut self, dialect: Dialect) -> Self {
        self.dialect = dialect;
        self
    }

    pub fn deposit(mut self, deposit: u64) -> Self {
        self.deposit = deposit;
        self
    }

    pub fn add_amounts(mut self, add_amounts: Vec<u64>) -> Self {
        self.add_amounts = add_amounts;
        self
    }

    pub fn spend_counts(mut self, spend_counts: Vec<usize>) -> Self {
        self.spend_counts = spend_counts;
        self
    }

    pub fn pace(mut self, pace: std::time::Duration) -> Self {
        self.pace = pace;
        self
    }

    pub fn burst(mut self, burst: Option<persona::BurstStop>) -> Self {
        self.burst = burst;
        self
    }

    pub fn persona(mut self, name: impl Into<String>) -> Self {
        self.persona = name.into();
        self
    }

    /// Applies a `persona::Behavior` wholesale -- what
    /// `persona::Persona::generate` produces -- rather than setting
    /// `dialect`/`deposit`/`add_amounts`/`spend_counts`/`pace` one at a
    /// time.
    pub fn behavior(self, name: impl Into<String>, behavior: persona::Behavior) -> Self {
        self.persona(name)
            .dialect(behavior.dialect)
            .deposit(behavior.deposit)
            .add_amounts(behavior.add_amounts)
            .spend_counts(behavior.spend_counts)
            .pace(behavior.pace)
            .burst(behavior.burst)
    }

    pub async fn build(self) -> anyhow::Result<Consumer> {
        let mut consumer = Consumer::new(
            self.seed,
            self.tag,
            None,
            self.dialect,
            self.provider_base_url,
        )?;
        consumer.connect().await?;
        consumer.deposit = self.deposit;
        consumer.add_amounts = self.add_amounts;
        consumer.spend_counts = self.spend_counts;
        consumer.pace = self.pace;
        consumer.burst = self.burst;
        consumer.persona = self.persona;
        Ok(consumer)
    }
}
