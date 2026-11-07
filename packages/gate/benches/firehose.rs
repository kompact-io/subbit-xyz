//! Portable firehose load test for `subbit-gate`'s real request
//! path, meant to be run on real hardware, not necessarily wherever
//! this was first written: a sandboxed container's real CPU budget
//! isn't visible from inside it at all (no `/sys/fs/cgroup`, `nproc`
//! reporting a count that may not reflect real availability), which
//! can produce throughput numbers that don't add up. This bench
//! prints what it can see about its own hardware up front so its
//! numbers are self-documenting about where they came from.
//!
//! Spawns the real `subbit-gate` binary as a separate OS process
//! (the same technique `packages/canary/src/provider.rs` uses) and
//! drives it over real HTTP with a real `reqwest` client. No in-
//! process shortcuts, no shared tokio runtime with the server under
//! test.
//!
//! Each simulated stamper: one real ed25519 keypair, one real signed
//! priming spend carrying a large `Iou` (materializes and funds its
//! channel -- `Ctx::spend`'s no-commit path requires the channel to
//! already exist, see `Db::update_exposed`'s own doc comment, so the
//! channel has to be created first via the admin backing endpoint,
//! then actually funded via one real signed commit), then fires its
//! own liveness-only spends sequentially. The first spend signs a
//! fresh `Pop`; every one after reuses the session `Mac` the server
//! hands back, exactly like `subbit_stamp::Stamp::auth` does in real
//! `packages/canary` traffic (see `Stamper::learn_mac`) -- this used
//! to always resign, overstating steady-state cost by paying a full
//! `ed25519` verify on every request instead of the ~1700x cheaper
//! cached-`Mac` check real traffic mostly pays (see `results-ii.md` for
//! the measured gap this caused).
//!
//! All `N` stampers in a sweep level fire concurrently, one tokio task
//! each; sequential within each stamper's own loop. Same shape
//! `packages/canary`'s `spend_round_concurrently` already uses, just
//! without any L1/channel-lifecycle machinery riding along with it.
//!
//! Run with: `cargo bench --bench firehose -p subbit-gate`
//! Override the concurrency sweep: `cargo bench --bench firehose -p subbit-gate -- 1 50 500 2000`

use std::{
    collections::BTreeMap,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use subbit_core::{Duration as CoreDuration, Iou, Signature, Tag, TagTbs, VerifyingKey};
use subbit_gate::{Backing, Config, Keytag, now};
use subbit_l2_dto::{ChannelId, Commit, Cost, KeyTagTtl, MacTtl, PopMacTtl, PopTtl, Request};

#[path = "support/mod.rs"]
mod support;
use support::Server;

const RUN_DURATION: Duration = Duration::from_secs(5);
const DEFAULT_SWEEP: &[usize] = &[1, 10, 50, 200, 1000];
const PRIME_COMMIT: u64 = u64::MAX / 2;

fn main() {
    print_environment();

    let sweep: Vec<usize> = std::env::args()
        .skip(1)
        .filter_map(|a| a.parse().ok())
        .collect();
    let sweep = if sweep.is_empty() {
        DEFAULT_SWEEP.to_vec()
    } else {
        sweep
    };

    // Diagnostic escape hatch, not the default path: set
    // `FIREHOSE_ALWAYS_BY_ID=1` to drive every spend as a bare
    // `Request::ById` (a fresh, strictly-increasing `Iou`, no Pop/Mac
    // auth at all) instead of the default Pop-once-then-Mac shape --
    // mirrors `Stamp::always_by_id`.
    let always_by_id = std::env::var("FIREHOSE_ALWAYS_BY_ID").is_ok_and(|v| v == "1");

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        for &n in &sweep {
            run_level(n, always_by_id).await;
        }
    });
}

fn print_environment() {
    let cores = std::thread::available_parallelism()
        .map(|n| n.get())
        .unwrap_or(0);
    let cgroup_visible = std::path::Path::new("/sys/fs/cgroup").exists();
    println!("firehose: available_parallelism={cores}, /sys/fs/cgroup visible={cgroup_visible}");
    println!(
        "firehose: if throughput plateaus well below what available_parallelism suggests, \
         suspect a host-enforced CPU quota that isn't visible from in here, not server/client logic.\n"
    );
}

fn gate_config() -> Config {
    let mut config = Config::default();
    config.terms.catalog.resources[0].1.cost = Cost::ByRequest(1);
    config
}

// ---------------------------------------------------------------------------
// Stampers
// ---------------------------------------------------------------------------

struct Stamper {
    sk: [u8; 64],
    key: VerifyingKey,
    tag: Tag,
    /// Set once the server hands back a session `Mac` (after the first
    /// `Pop`-authenticated request); real clients (`subbit_stamp::Stamp`,
    /// see `packages/stamp/src/stamp.rs::auth`) reuse this on every
    /// subsequent request instead of resigning a fresh `Pop`. This bench
    /// used to always resign -- see `results-ii.md` for why that mattered:
    /// it made every request pay a full `ed25519` verify, overstating the
    /// real per-request cost.
    mac: Option<MacTtl>,
    /// Cumulative amount claimed so far -- only used by `by_id_header`
    /// (`FIREHOSE_ALWAYS_BY_ID=1`, see `Stamp::always_by_id`'s doc).
    spent: u64,
}

fn build_stamper(i: usize) -> Stamper {
    let mut seed = [0u8; 32];
    seed[..8].copy_from_slice(&(i as u64).to_le_bytes());
    let (sk, pk) = cryptoxide::ed25519::keypair(&seed);
    Stamper {
        sk,
        key: VerifyingKey::from(pk),
        tag: Tag::from(format!("fh-{i}").into_bytes()),
        mac: None,
        spent: 0,
    }
}

impl Stamper {
    fn keytag(&self) -> Keytag {
        Keytag::new(self.key, self.tag.clone())
    }

    /// The cached `Mac` if one's been issued yet, a fresh signed `Pop`
    /// otherwise -- same choice `subbit_stamp::Stamp::auth` makes.
    fn liveness_header(&self) -> http::HeaderValue {
        let auth = match &self.mac {
            Some(mac) => PopMacTtl::Mac(mac.clone()),
            None => {
                let body = KeyTagTtl {
                    key: self.key,
                    tag: self.tag.clone(),
                    expires_at: now() + CoreDuration::from_secs(3600),
                };
                let sig = Signature::from(cryptoxide::ed25519::signature(&body.tbs(), &self.sk));
                PopMacTtl::Pop(PopTtl { body, sig })
            }
        };
        let payload = Request::ByPopMac { auth, commit: None };
        http::HeaderValue::from_str(&subbit_core::base64::to_base64(&payload)).unwrap()
    }

    /// `Request::ById`: a fresh, strictly-increasing `Iou`, no auth at
    /// all -- the `Iou`'s own signature is the only authorization.
    /// Mirrors `Stamp::always_by_id`.
    fn by_id_header(&mut self, cost: u64) -> http::HeaderValue {
        self.spent += cost;
        let tbs = TagTbs::new(self.tag.clone(), self.spent).to_vec();
        let sig = Signature::from(cryptoxide::ed25519::signature(&tbs, &self.sk));
        let payload = Request::ById {
            channel_id: ChannelId {
                iou_key: self.key,
                tag: self.tag.clone(),
            },
            commit: Commit::Iou(Iou::new(self.spent, sig)),
        };
        http::HeaderValue::from_str(&subbit_core::base64::to_base64(&payload)).unwrap()
    }

    /// Reads a `Mac` off a response's own `subbit` envelope (present
    /// only after a `Pop`-authenticated request, see
    /// `subbit_l2_dto::Status::mac`'s own doc) and caches it for every
    /// subsequent `liveness_header` call. A no-op once already cached,
    /// or on a response with no `subbit` header/no `Mac` in it.
    fn learn_mac(&mut self, header_value: Option<&http::HeaderValue>) {
        if self.mac.is_some() {
            return;
        }
        let Some(value) = header_value.and_then(|v| v.to_str().ok()) else {
            return;
        };
        if let Ok(response) = subbit_core::base64::from_base64::<subbit_l2_dto::Response>(value) {
            self.mac = response.status.mac;
        }
    }

    /// One real signed `Iou` claiming `PRIME_COMMIT`, to fund the
    /// channel after it's been materialized by the admin backing call.
    fn priming_header(&self) -> http::HeaderValue {
        let tbs = TagTbs::new(self.tag.clone(), PRIME_COMMIT).to_vec();
        let sig = Signature::from(cryptoxide::ed25519::signature(&tbs, &self.sk));
        let payload = Request::ById {
            channel_id: ChannelId {
                iou_key: self.key,
                tag: self.tag.clone(),
            },
            commit: Commit::Iou(Iou::new(PRIME_COMMIT, sig)),
        };
        http::HeaderValue::from_str(&subbit_core::base64::to_base64(&payload)).unwrap()
    }
}

const SUBBIT_HEADER: &str = "subbit";

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

async fn seed_and_prime(
    client: &reqwest::Client,
    base_url: &str,
    stampers: &[Stamper],
    always_by_id: bool,
) {
    let backings: BTreeMap<Keytag, Option<Backing>> =
        stampers.iter().map(|i| (i.keytag(), None)).collect();
    let body = minicbor::to_vec(&backings).unwrap();
    client
        .post(format!("{base_url}/v1/a/backings"))
        .body(body)
        .send()
        .await
        .expect("seed backings")
        .error_for_status()
        .expect("seed backings status");

    // `always_by_id` sends its own small, strictly-increasing `Iou`
    // amounts per request -- a `PRIME_COMMIT` this large ahead of them
    // would make every one of those look like an `OldIou` (amount not
    // strictly greater than what's already committed).
    if always_by_id {
        return;
    }

    let mut primes = Vec::with_capacity(stampers.len());
    for stamper in stampers {
        let client = client.clone();
        let url = format!("{base_url}/v1/spend/bench");
        let header = stamper.priming_header();
        primes.push(tokio::spawn(async move {
            client
                .get(&url)
                .header(SUBBIT_HEADER, header)
                .send()
                .await
                .expect("priming spend")
                .error_for_status()
                .expect("priming spend status");
        }));
    }
    for p in primes {
        p.await.unwrap();
    }
}

async fn run_level(n: usize, always_by_id: bool) {
    let server = Server::spawn(gate_config()).await;
    let client = reqwest::Client::new();
    let url = format!("{}/v1/spend/bench", server.base_url());

    let stampers: Vec<Stamper> = (0..n).map(build_stamper).collect();
    seed_and_prime(&client, &server.base_url(), &stampers, always_by_id).await;

    let total_ops = Arc::new(AtomicU64::new(0));
    let latencies: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
    let deadline = Instant::now() + RUN_DURATION;

    let mut tasks = Vec::with_capacity(n);
    for mut stamper in stampers {
        let client = client.clone();
        let url = url.clone();
        let total_ops = Arc::clone(&total_ops);
        let latencies = Arc::clone(&latencies);
        tasks.push(tokio::spawn(async move {
            let mut local = Vec::new();
            let mut ops = 0u64;
            while Instant::now() < deadline {
                let header = if always_by_id {
                    stamper.by_id_header(1)
                } else {
                    stamper.liveness_header()
                };
                let start = Instant::now();
                if let Ok(resp) = client.get(&url).header(SUBBIT_HEADER, header).send().await {
                    if !always_by_id {
                        stamper.learn_mac(resp.headers().get(SUBBIT_HEADER));
                    }
                }
                local.push(start.elapsed());
                ops += 1;
            }
            total_ops.fetch_add(ops, Ordering::Relaxed);
            latencies.lock().unwrap().extend(local);
        }));
    }
    for t in tasks {
        let _ = t.await;
    }

    let ops = total_ops.load(Ordering::Relaxed);
    let rate = ops as f64 / RUN_DURATION.as_secs_f64();
    let mut all = latencies.lock().unwrap();
    all.sort_unstable();
    let pct = |p: f64| -> Duration {
        if all.is_empty() {
            return Duration::ZERO;
        }
        all[((all.len() as f64 * p) as usize).min(all.len() - 1)]
    };
    println!(
        "firehose: stampers={n:<5} ops={ops:<8} req/s={rate:<8.0} p50={:>6.1?} p90={:>6.1?} p99={:>6.1?}",
        pct(0.50),
        pct(0.90),
        pct(0.99),
    );
}
