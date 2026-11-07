//! A logical Consumer: an ephemeral on-chain identity plus a real `subbit_stamp::Stamp`
//! driving real HTTP spend/refund against a Provider, in either wire dialect.

pub mod persona;
mod roster;

pub(crate) use roster::{identity_for, plan_roster};

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, OnceLock};

use anyhow::Context;
use http::Extensions;
use reqwest::{Request, Response};
use reqwest_middleware::{
    ClientBuilder, ClientWithMiddleware, Middleware, Next, Result as MwResult,
};
use subbit_core::{Duration, Instant, Tag, VerifyingKey};
use subbit_l2_dto::dialect::x402::SubbitAccept;
use subbit_l2_dto::{Catalog, Terms};
use subbit_stamp::catalog;
use subbit_stamp::dialect::x402::{self as x402_dialect, TermsDecision};
use subbit_stamp::reqwest::vanilla::Stamp as VanillaMiddleware;
use subbit_stamp::reqwest::x402::Stamp as X402Middleware;
use subbit_tx::step::{Can, Want};

use crate::identity::hash28_from_seed;

/// A catalog miss is a hard failure -- no consent loop here.
type Policy = fn(&str) -> catalog::MissDecision;
const POLICY: Policy = catalog::reject;

/// Accept the first x402 terms challenge, never a later change.
type AcceptPolicy = fn(&SubbitAccept, Option<&SubbitAccept>) -> TermsDecision;
const ACCEPT_POLICY: AcceptPolicy = x402_dialect::accept_once;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Vanilla,
    X402,
}

/// Per-request round-trip latency from `spend_burst`, pooled across the process.
static SPEND_LATENCIES: OnceLock<Mutex<Vec<std::time::Duration>>> = OnceLock::new();

fn record_latencies(mut d: Vec<std::time::Duration>) {
    SPEND_LATENCIES
        .get_or_init(|| Mutex::new(Vec::new()))
        .lock()
        .unwrap()
        .append(&mut d);
}

/// `(p50, p90, p99, count)` over every `spend_burst` round-trip recorded so far.
pub fn latency_percentiles() -> Option<(
    std::time::Duration,
    std::time::Duration,
    std::time::Duration,
    usize,
)> {
    let guard = SPEND_LATENCIES.get()?.lock().unwrap();
    if guard.is_empty() {
        return None;
    }
    let mut all = guard.clone();
    all.sort_unstable();
    let pct = |p: f64| all[((all.len() as f64 * p) as usize).min(all.len() - 1)];
    Some((pct(0.50), pct(0.90), pct(0.99), all.len()))
}

enum Driver {
    Vanilla(Arc<VanillaMiddleware<Policy>>),
    X402(Arc<X402Middleware<Policy, AcceptPolicy>>),
}

pub struct Consumer {
    seed: [u8; 32],
    provider_base_url: String,
    /// Set once by `tidy`, never cleared.
    tidying: AtomicBool,
    /// Set by `tidy_once_closed`: tidying starts once `Close` lands, not before.
    tidy_on_close: AtomicBool,
    pub tag: Vec<u8>,
    pub l1_key: subbit_core::Hash28,
    pub iou_key: VerifyingKey,
    pub dialect: Dialect,
    /// Built by `connect`; `None` for a consumer that never spends (tidy's placeholders).
    driver: Option<Driver>,
    client: Option<ClientWithMiddleware>,
    pub deposit: u64,
    /// One entry per `Want::Add` this consumer performs mid-lifecycle.
    pub add_amounts: Vec<u64>,
    /// One entry per spend round -- `spend_counts.len() == add_amounts.len() + 1`.
    spend_counts: Vec<usize>,
    pace: std::time::Duration,
    /// Overrides `firehose_l2`'s fixed-count loop entirely when set.
    burst: Option<persona::BurstStop>,
    pub persona: String,
    /// `want`'s burst branch measures `close_after` from here, not round index.
    started: std::time::Instant,
}

struct ForwardVanilla(Arc<VanillaMiddleware<Policy>>);

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

struct ForwardX402(Arc<X402Middleware<Policy, AcceptPolicy>>);

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
    /// `catalog: None` leaves this consumer disconnected until `connect`.
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
            started: std::time::Instant::now(),
        };
        if let Some(catalog) = catalog {
            consumer.build_driver(catalog)?;
        }
        Ok(consumer)
    }

    /// Fetches the provider's real `Terms`/catalog and builds this consumer's HTTP driver.
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
        let account = subbit_stamp::Id::new(signing_key, Tag::from(self.tag.clone()));
        let mut stamp = subbit_stamp::Stamp::new(account, Duration::from_secs(3600)).by_id();
        stamp
            .set_catalog(catalog)
            .context("indexing provider catalog")?;

        let (driver, client) = match self.dialect {
            Dialect::Vanilla => {
                let middleware = Arc::new(VanillaMiddleware::new(stamp, POLICY));
                let client = ClientBuilder::new(reqwest::Client::new())
                    .with(ForwardVanilla(middleware.clone()))
                    .build();
                (Driver::Vanilla(middleware), client)
            }
            Dialect::X402 => {
                let middleware = Arc::new(X402Middleware::new(stamp, POLICY, ACCEPT_POLICY));
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

    /// Spends against `{provider_base_url}/v1/spend/echo`.
    // x402's first request against an unpaid path always errs (apply_required's own design,
    // learning `accepted` as a side effect) -- retry once, same as examples/echo/client.
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

    /// This consumer's own decision for `round`.
    pub fn want(&self, round: usize, can: &[Can], close_upper: Instant) -> Option<Want> {
        if can.contains(&Can::End) {
            return Some(Want::End);
        }
        if self.tidying.load(Ordering::Relaxed) {
            return self.tidy_want(can, close_upper);
        }
        if let Some(stop) = self.burst {
            // Measured against close_after (not round index) so every burst consumer starts
            // competing for Close at the same real time, holding concurrency flat.
            if self.started.elapsed() < stop.close_after {
                return None;
            }
            return can
                .contains(&Can::Close)
                .then_some(Want::Close { upper: close_upper });
        }
        if let Some(&amount) = self.add_amounts.get(round) {
            return can.contains(&Can::Add).then_some(Want::Add { amount });
        }
        if round + 1 == self.rounds() {
            let want = can
                .contains(&Can::Close)
                .then_some(Want::Close { upper: close_upper });
            if want.is_some() && self.tidy_on_close.load(Ordering::Relaxed) {
                self.tidy();
            }
            return want;
        }
        None
    }

    /// From now on, `want` heads for the exit: `Close`, then `Elapse`. Skips `Settle`.
    pub fn tidy(&self) {
        self.tidying.store(true, Ordering::Relaxed);
    }

    /// Tidies once this consumer's own `Close` lands, not immediately.
    pub fn tidy_once_closed(&self) {
        self.tidy_on_close.store(true, Ordering::Relaxed);
    }

    /// `Close` while `Opened`; once `Closed`, `Elapse`.
    fn tidy_want(&self, can: &[Can], close_upper: Instant) -> Option<Want> {
        for c in can {
            match c {
                Can::Close => return Some(Want::Close { upper: close_upper }),
                // 120s margin: 1s/20s both measured too tight live (rejected submissions);
                // matches close_upper's own margin for the same reason.
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

    /// How many rounds this consumer's own lifecycle has.
    pub fn rounds(&self) -> usize {
        self.add_amounts.len() + 1
    }

    /// Fires this consumer's planned spends for `round`, paced by `pace`.
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

    /// Fires back-to-back requests until `commit_max` or `deadline`, whichever first.
    async fn spend_burst(&self, stop: persona::BurstStop) -> usize {
        let deadline = std::time::Instant::now() + stop.deadline;
        let mut done = 0;
        // Accumulated locally; a per-request lock measurably depresses throughput at scale.
        let mut local_latencies = Vec::new();
        loop {
            if self.committed().unwrap_or(0) >= stop.commit_max {
                break;
            }
            if std::time::Instant::now() >= deadline {
                if self.committed().unwrap_or(0) < stop.commit_max
                    && self.timed_spend(&mut local_latencies).await
                {
                    done += 1;
                }
                break;
            }
            if self.timed_spend(&mut local_latencies).await {
                done += 1;
            } else {
                break;
            }
        }
        record_latencies(local_latencies);
        done
    }

    /// One `spend`, timed and pushed onto `local_latencies`.
    async fn timed_spend(&self, local_latencies: &mut Vec<std::time::Duration>) -> bool {
        let start = std::time::Instant::now();
        let resp = self.spend().await;
        local_latencies.push(start.elapsed());
        resp.is_ok_and(|resp| resp.status().is_success())
    }

    /// The consumer's current off-chain view. `None` before `connect`.
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

/// Builds, connects, and applies `behavior` wholesale -- the only way a real Consumer is built.
pub async fn build_consumer(
    seed: [u8; 32],
    tag: impl Into<Vec<u8>>,
    provider_base_url: impl Into<String>,
    persona_name: impl Into<String>,
    behavior: persona::Behavior,
) -> anyhow::Result<Consumer> {
    let mut consumer = Consumer::new(seed, tag, None, behavior.dialect, provider_base_url)?;
    consumer.connect().await?;
    consumer.deposit = behavior.deposit;
    consumer.add_amounts = behavior.add_amounts;
    consumer.spend_counts = behavior.spend_counts;
    consumer.pace = behavior.pace;
    consumer.burst = behavior.burst;
    consumer.persona = persona_name.into();
    Ok(consumer)
}
