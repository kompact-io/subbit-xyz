//! Isolates where `benches/firehose.rs` vs `packages/canary`'s ~60x
//! latency gap actually comes from (see `plan-fix-canary.md`): both
//! drive the real `subbit-gate` binary over real HTTP with a real
//! per-request signature, so the gap has to be in whatever differs
//! between them on the client side. The one structural difference:
//! canary's `Consumer` sends every spend through
//! `subbit_stamp::reqwest::vanilla::Stamp` wrapped in a
//! `reqwest_middleware::ClientWithMiddleware`; `firehose.rs` builds
//! and sends its own envelope through a plain `reqwest::Client`
//! directly, no middleware crate involved.
//!
//! This bench measures both paths back to back, against the same
//! spawned server, same request shape (`Request::ByPopMac` liveness-
//! only, no commit), same signing algorithm (`ed25519-dalek`, to rule
//! out a signing-library difference as a confound) -- the only thing
//! that differs between the two rows is whether `subbit_stamp` +
//! `reqwest_middleware` sit in the path.
//!
//! Run with: `cargo bench --bench stamp_overhead -p subbit-gate`

use std::{
    net::SocketAddr,
    path::PathBuf,
    process::Stdio,
    time::{Duration, Instant},
};

use ed25519_dalek::{Signer, SigningKey};
use reqwest_middleware::ClientBuilder;
use subbit_core::{Duration as CoreDuration, Signature, Tag, VerifyingKey};
use subbit_gate::{Backing, Config, Keytag, now};
use subbit_l2_dto::{CatalogIndex, Cost, KeyTagTtl, PopMacTtl, PopTtl, Request, Terms};
use subbit_stamp::reqwest::vanilla::Stamp as VanillaMiddleware;
use tokio::process::{Child, Command};

const REQUESTS: usize = 300;
const PRIME_COMMIT: u64 = u64::MAX / 2;

fn main() {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(run());
}

async fn run() {
    let server = Server::spawn().await;
    let client = reqwest::Client::new();

    let seed_a = [11u8; 32];
    let seed_b = [22u8; 32];
    let tag_a = Tag::from(b"overhead-a".to_vec());
    let tag_b = Tag::from(b"overhead-b".to_vec());
    let key_a = VerifyingKey::from(SigningKey::from_bytes(&seed_a).verifying_key().to_bytes());
    let key_b = VerifyingKey::from(SigningKey::from_bytes(&seed_b).verifying_key().to_bytes());

    // Identity A spends liveness-only (`commit: None`), which needs a
    // committed balance already on the channel. Identity B goes
    // through the real `Stamp`, which tracks and sends its own `Iou`
    // from zero on its first call -- priming it out-of-band here
    // would desync the `Stamp`'s own belief about `committed` from
    // the server's, since `apply_iou` requires a strictly increasing
    // amount (see `Channel::apply_iou`).
    seed_backings(
        &client,
        &server.base_url(),
        &[key_a, key_b],
        &[tag_a.clone(), tag_b.clone()],
    )
    .await;
    prime_liveness(&client, &server.base_url(), key_a, tag_a.clone(), &seed_a).await;

    let url = format!("{}/v1/spend/overhead", server.base_url());

    // --- Row A: hand-rolled envelope, plain reqwest, no middleware crate ---
    let mut hand_rolled = Vec::with_capacity(REQUESTS);
    for _ in 0..REQUESTS {
        let body = KeyTagTtl {
            key: key_a,
            tag: tag_a.clone(),
            ttl: now() + CoreDuration::from_secs(3600),
        };
        let sig = Signature::from(SigningKey::from_bytes(&seed_a).sign(&body.tbs()).to_bytes());
        let payload = Request::ByPopMac {
            auth: PopMacTtl::Pop(PopTtl { body, sig }),
            commit: None,
        };
        let header =
            http::HeaderValue::from_str(&subbit_core::base64::to_base64(&payload)).unwrap();
        let start = Instant::now();
        let resp = client
            .get(&url)
            .header("subbit", header)
            .send()
            .await
            .expect("send");
        assert!(
            resp.status().is_success(),
            "hand-rolled request failed: {}",
            resp.status()
        );
        hand_rolled.push(start.elapsed());
    }
    report(
        "hand-rolled (plain reqwest, no middleware crate)",
        &hand_rolled,
    );

    // --- Row B: the real subbit_stamp + reqwest_middleware stack canary uses ---
    let terms: Terms = reqwest::get(format!("{}/v1/terms", server.base_url()))
        .await
        .unwrap()
        .json()
        .await
        .unwrap();
    let costing = CatalogIndex::new(terms.catalog).unwrap();
    let account = subbit_stamp::Account::new(SigningKey::from_bytes(&seed_b), tag_b.clone());
    let stamp = subbit_stamp::Stamp::new(account, subbit_core::Duration::from_secs(3600));
    let middleware = VanillaMiddleware::new(stamp, costing);
    let stamp_client = ClientBuilder::new(reqwest::Client::new())
        .with(middleware)
        .build();

    let mut via_stamp = Vec::with_capacity(REQUESTS);
    for _ in 0..REQUESTS {
        let start = Instant::now();
        let resp = stamp_client.get(&url).send().await.expect("send");
        assert!(
            resp.status().is_success(),
            "stamp-stack request failed: {}",
            resp.status()
        );
        via_stamp.push(start.elapsed());
    }
    report(
        "subbit_stamp + reqwest_middleware (canary's real stack)",
        &via_stamp,
    );
}

fn report(label: &str, samples: &[Duration]) {
    let mut sorted = samples.to_vec();
    sorted.sort_unstable();
    let total: Duration = sorted.iter().sum();
    let avg = total / sorted.len() as u32;
    let pct = |p: f64| sorted[((sorted.len() as f64 * p) as usize).min(sorted.len() - 1)];
    println!(
        "stamp_overhead: {label:<55} avg={avg:>8.1?} p50={:>8.1?} p99={:>8.1?}",
        pct(0.50),
        pct(0.99),
    );
}

// ---------------------------------------------------------------------------
// Spawning the real binary + priming channels (same approach as firehose.rs)
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
        let child = Command::new(sibling_bin("subbit-gate"))
            .arg("--config")
            .arg(&config_path)
            .arg("serve")
            .arg("--listen")
            .arg(addr.to_string())
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("spawn subbit-gate");

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

async fn seed_backings(
    client: &reqwest::Client,
    base_url: &str,
    keys: &[VerifyingKey],
    tags: &[Tag],
) {
    let backings: std::collections::BTreeMap<Keytag, Option<Backing>> = keys
        .iter()
        .zip(tags)
        .map(|(key, tag)| (Keytag::new(*key, tag.clone()), None))
        .collect();
    let body = minicbor::to_vec(&backings).unwrap();
    client
        .post(format!("{base_url}/v1/a/backings"))
        .body(body)
        .send()
        .await
        .expect("seed backings")
        .error_for_status()
        .expect("seed backings status");
}

async fn prime_liveness(
    client: &reqwest::Client,
    base_url: &str,
    key: VerifyingKey,
    tag: Tag,
    seed: &[u8; 32],
) {
    let tbs = subbit_core::TagTbs::new(tag.clone(), PRIME_COMMIT).to_vec();
    let sig = Signature::from(SigningKey::from_bytes(seed).sign(&tbs).to_bytes());
    let payload = Request::ById {
        channel_id: subbit_l2_dto::ChannelId {
            iou_key: key,
            tag: tag.clone(),
        },
        commit: subbit_l2_dto::Commit::Iou(subbit_core::Iou::new(PRIME_COMMIT, sig)),
    };
    let header = http::HeaderValue::from_str(&subbit_core::base64::to_base64(&payload)).unwrap();
    client
        .get(format!("{base_url}/v1/spend/overhead"))
        .header("subbit", header)
        .send()
        .await
        .expect("priming spend")
        .error_for_status()
        .expect("priming spend status");
}
