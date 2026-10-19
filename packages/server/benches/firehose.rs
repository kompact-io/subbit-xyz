//! Portable firehose load test for `subbit-server`'s real request
//! path, meant to be run on real hardware, not necessarily wherever
//! this was first written (see `plan-fix-canary.md`: a same-session
//! investigation found numbers that didn't add up, traced to a
//! sandboxed container whose real CPU budget isn't visible from
//! inside it at all -- no `/sys/fs/cgroup`, `nproc` reporting a count
//! that may not reflect real availability). This bench prints what it
//! can see about its own hardware up front so its numbers are
//! self-documenting about where they came from.
//!
//! Spawns the real `subbit-server` binary as a separate OS process
//! (the same technique `packages/canary/src/provider.rs` uses) and
//! drives it over real HTTP with a real `reqwest` client. No in-
//! process shortcuts, no shared tokio runtime with the server under
//! test.
//!
//! Each simulated issuer: one real ed25519 keypair, one real signed
//! priming spend carrying a large `Iou` (materializes and funds its
//! channel -- `Ctx::spend`'s no-commit path requires the channel to
//! already exist, see `Db::update_exposed`'s own doc comment, so the
//! channel has to be created first via the admin backing endpoint,
//! then actually funded via one real signed commit), then fires its
//! own liveness-only spends sequentially, **signing a fresh envelope
//! on every single request** -- the same real per-request cost
//! `subbit_issuer`'s middleware pays in `packages/canary`, not a
//! cached header reused for the whole run (unlike `benches/
//! worker_threads.rs`, which deliberately reuses one to isolate the
//! server's own tokio/write-path behavior from client-side crypto
//! cost -- this bench deliberately does not, since the point here is
//! reproducing canary's real cost profile).
//!
//! All `N` issuers in a sweep level fire concurrently, one tokio task
//! each; sequential within each issuer's own loop. Same shape
//! `packages/canary`'s `spend_round_concurrently` already uses, just
//! without any L1/channel-lifecycle machinery riding along with it.
//!
//! Run with: `cargo bench --bench firehose -p subbit-server`
//! Override the concurrency sweep: `cargo bench --bench firehose -p subbit-server -- 1 50 500 2000`

use std::{
    collections::BTreeMap,
    net::SocketAddr,
    path::PathBuf,
    process::Stdio,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use subbit_core::{Duration as CoreDuration, Iou, Signature, Tag, TagTbs, VerifyingKey};
use subbit_l2_dto::{ChannelId, Commit, Cost, KeyTagTtl, PopMacTtl, PopTtl, Request};
use subbit_server::{Backing, Config, Keytag, now};
use tokio::process::{Child, Command};

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

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        for &n in &sweep {
            run_level(n).await;
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

// ---------------------------------------------------------------------------
// Spawning the real binary
// ---------------------------------------------------------------------------

fn sibling_bin(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let mut dir = exe.clone();
    for _ in 0..4 {
        dir.pop();
        let candidate = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
        if candidate.exists() {
            return candidate;
        }
    }
    panic!(
        "{name} binary not found near {}; run `cargo build --workspace` first",
        exe.display()
    );
}

struct Server {
    _child: Child,
    _dir: tempfile::TempDir,
    addr: SocketAddr,
}

impl Server {
    fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    async fn spawn() -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut config = Config::default();
        config.db.db_path = dir.path().join("db.redb").to_string_lossy().into_owned();
        config.terms.catalog.default.cost = Cost::ByRequest(1);
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();

        let addr: SocketAddr = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let child = Command::new(sibling_bin("subbit-server"))
            .arg("--config")
            .arg(&config_path)
            .arg("serve")
            .arg("--listen")
            .arg(addr.to_string())
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn subbit-server");

        let client = reqwest::Client::new();
        let ready_url = format!("http://{addr}/v1/a/ious");
        for _ in 0..200 {
            if client.get(&ready_url).send().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Self {
            _child: child,
            _dir: dir,
            addr,
        }
    }
}

// ---------------------------------------------------------------------------
// Issuers
// ---------------------------------------------------------------------------

struct Issuer {
    sk: [u8; 64],
    key: VerifyingKey,
    tag: Tag,
}

fn build_issuer(i: usize) -> Issuer {
    let mut seed = [0u8; 32];
    seed[..8].copy_from_slice(&(i as u64).to_le_bytes());
    let (sk, pk) = cryptoxide::ed25519::keypair(&seed);
    Issuer {
        sk,
        key: VerifyingKey::from(pk),
        tag: Tag::from(format!("fh-{i}").into_bytes()),
    }
}

impl Issuer {
    fn keytag(&self) -> Keytag {
        Keytag::new(self.key, self.tag.clone())
    }

    /// A fresh liveness-only envelope, signed right now -- not cached.
    fn liveness_header(&self) -> http::HeaderValue {
        let body = KeyTagTtl {
            key: self.key,
            tag: self.tag.clone(),
            ttl: now() + CoreDuration::from_secs(3600),
        };
        let sig = Signature::from(cryptoxide::ed25519::signature(&body.tbs(), &self.sk));
        let payload = Request::ByPopMac {
            auth: PopMacTtl::Pop(PopTtl { body, sig }),
            commit: None,
        };
        http::HeaderValue::from_str(&subbit_core::base64::to_base64(&payload)).unwrap()
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

async fn seed_and_prime(client: &reqwest::Client, base_url: &str, issuers: &[Issuer]) {
    let backings: BTreeMap<Keytag, Option<Backing>> =
        issuers.iter().map(|i| (i.keytag(), None)).collect();
    let body = minicbor::to_vec(&backings).unwrap();
    client
        .post(format!("{base_url}/v1/a/backings"))
        .body(body)
        .send()
        .await
        .expect("seed backings")
        .error_for_status()
        .expect("seed backings status");

    let mut primes = Vec::with_capacity(issuers.len());
    for issuer in issuers {
        let client = client.clone();
        let url = format!("{base_url}/v1/spend/bench");
        let header = issuer.priming_header();
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

async fn run_level(n: usize) {
    let server = Server::spawn().await;
    let client = reqwest::Client::new();
    let url = format!("{}/v1/spend/bench", server.base_url());

    let issuers: Vec<Issuer> = (0..n).map(build_issuer).collect();
    seed_and_prime(&client, &server.base_url(), &issuers).await;

    let total_ops = Arc::new(AtomicU64::new(0));
    let latencies: Arc<Mutex<Vec<Duration>>> = Arc::new(Mutex::new(Vec::new()));
    let deadline = Instant::now() + RUN_DURATION;

    let mut tasks = Vec::with_capacity(n);
    for issuer in issuers {
        let client = client.clone();
        let url = url.clone();
        let total_ops = Arc::clone(&total_ops);
        let latencies = Arc::clone(&latencies);
        tasks.push(tokio::spawn(async move {
            let mut local = Vec::new();
            let mut ops = 0u64;
            while Instant::now() < deadline {
                let header = issuer.liveness_header();
                let start = Instant::now();
                let _ = client.get(&url).header(SUBBIT_HEADER, header).send().await;
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
        "firehose: issuers={n:<5} ops={ops:<8} req/s={rate:<8.0} p50={:>6.1?} p90={:>6.1?} p99={:>6.1?}",
        pct(0.50),
        pct(0.90),
        pct(0.99),
    );
}
