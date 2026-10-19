//! Diagnose `plan-fix-throughput.md`'s hypothesis 1: that
//! `subbit-server`'s measured real-world throughput ceiling comes from
//! `Ctx::handle_spend` running synchronously, inline, on tokio's
//! worker threads (`main.rs`'s `handle_spend` calls it directly, no
//! `.await`, no `spawn_blocking`) -- not from write-transaction
//! contention, which `l2_throughput.rs`'s own buffered-vs-unbuffered
//! comparison already rules out.
//!
//! Mounts the real, unmodified `subbit_server::app::router` in
//! process (no canary/preprod dependency) and drives it over real
//! HTTP with a real async client, under:
//!
//! - **Test A**: fixed concurrency, swept tokio worker-thread count,
//!   against both the real router and a bare-axum no-op baseline
//!   (calibration: rules out "axum/tokio itself is slow on this
//!   hardware" as a confound).
//! - **Test B**: fixed (small) worker-thread count, swept
//!   concurrency, comparing the real router's inline handler against
//!   a `spawn_blocking`-wrapped variant of the same call.
//!
//! Run with: `cargo bench --bench worker_threads`

use std::{
    net::SocketAddr,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use axum::{Router, extract::State, http::StatusCode, response::IntoResponse, routing::get};
use cryptoxide::ed25519;

use subbit_core::{Duration as CoreDuration, Signature, Tag, VerifyingKey};
use subbit_l2_dto::{
    Asset, Cost, KeyTagTtl, Network, PopMacTtl, PopTtl, Request, ResourceInfo, Terms,
};

use subbit_server::{
    Aux, Backing, Bucket, Channel, Ctx, Keytag,
    app::{self, AppState},
    catalog::{Catalog, CatalogDto},
    crypto::mac,
    db::{Config as DbConfig, Db},
    dialect::vanilla::Vanilla,
    mutual::Unsupported,
    now,
};

const RUN_DURATION: Duration = Duration::from_secs(1);
const WORKER_COUNTS: &[usize] = &[1, 2, 4, 8];
const TEST_A_CONCURRENCY: usize = 200;
const TEST_B_WORKER_THREADS: usize = 2;
const CONCURRENCY_LEVELS: &[usize] = &[1, 10, 50, 200];
const SPEND_PATH: &str = "/v1/spend/resource";

// ---------------------------------------------------------------------------
// Fixtures: a real `Ctx`, real signed consumers
// ---------------------------------------------------------------------------

fn catalog_dto() -> CatalogDto {
    CatalogDto {
        default: ResourceInfo {
            cost: Cost::ByRequest(1),
            description: "bench resource".to_string(),
            mime_type: "application/octet-stream".to_string(),
            max_timeout_seconds: 60,
            service_name: None,
            tags: None,
            icon_url: None,
        },
        resources: Vec::new(),
    }
}

fn terms() -> Terms {
    Terms {
        network: Network::Preprod,
        script: subbit_core::Hash28::from([0u8; 28]),
        asset: Asset::Lovelace,
        provider: subbit_core::Hash28::from([0u8; 28]),
        close_period: CoreDuration::from_secs(900),
        confirmation_depth: 3,
        min_iou_amount: 0,
        catalog: catalog_dto(),
        reference_script: None,
    }
}

/// Backed and committed for far more than any run here could spend,
/// so no request ever hits `Unspendable`.
fn funded_channel() -> Channel {
    Channel::new(
        Some(Backing::new(u64::MAX / 2, 0)),
        Some(subbit_core::Iou::new(
            u64::MAX / 2,
            Signature::from([0u8; 64]),
        )),
        0,
        Bucket::default(),
        Aux::default(),
    )
}

/// One consumer: its own seeded channel plus a pre-signed, far-
/// future-TTL liveness proof, reused across every request it makes --
/// `Ctx::spend`'s no-commit path only checks liveness, not nonce
/// uniqueness, so this is safe and keeps per-request cost limited to
/// what `handle_spend` itself does, not request setup.
fn build_consumer(db: &Db, seed: u8) -> http::HeaderMap {
    let (sk, pk) = ed25519::keypair(&[seed; 32]);
    let key = VerifyingKey::from(pk);
    let tag = Tag::from(format!("bench-{seed}").into_bytes());
    let keytag = Keytag::new(key, tag.clone());
    db.upsert(funded_channel(), &keytag, Ok)
        .expect("seed channel");

    let body = KeyTagTtl {
        key,
        tag,
        ttl: now() + CoreDuration::from_secs(3600),
    };
    let sig = Signature::from(ed25519::signature(&body.tbs(), &sk));
    let payload = Request::ByPopMac {
        auth: PopMacTtl::Pop(PopTtl { body, sig }),
        commit: None,
    };

    let mut headers = http::HeaderMap::new();
    headers.insert(
        http::HeaderName::from_static("subbit"),
        http::HeaderValue::from_str(&subbit_core::base64::to_base64(&payload)).unwrap(),
    );
    headers
}

/// A fresh `Ctx` plus `consumers` pre-signed header sets. Rebuilt per
/// run (not shared across sweep cells) so no run's buffered/flush
/// state leaks into the next.
fn build_ctx(consumers: usize) -> (tempfile::TempPath, Arc<Ctx>, Vec<http::HeaderMap>) {
    let file = tempfile::NamedTempFile::new().unwrap().into_temp_path();
    let db = Arc::new(
        Db::open(DbConfig {
            db_path: file.to_string_lossy().to_string(),
            ..Default::default()
        })
        .unwrap(),
    );
    let headers: Vec<http::HeaderMap> = (0..consumers)
        .map(|i| build_consumer(&db, i as u8))
        .collect();
    let catalog = Arc::new(Catalog::new(catalog_dto()).unwrap());
    let mac = Arc::new(mac::Mac::new(mac::Key::default()));
    let ctx = Arc::new(Ctx::new(
        db,
        mac,
        catalog,
        Arc::new(terms()),
        Arc::new(Unsupported),
        vec![Arc::new(Vanilla)],
        None,
    ));
    (file, ctx, headers)
}

// ---------------------------------------------------------------------------
// Router variants
// ---------------------------------------------------------------------------

async fn noop() -> StatusCode {
    StatusCode::OK
}

fn bare_axum_router() -> Router {
    Router::new().route("/", get(noop))
}

/// `app::router`'s production handler now wraps this same call in
/// `spawn_blocking` (see `app.rs`'s own doc comment -- landed on the
/// strength of this bench's Test B). These two bench-local variants
/// exist so Test B keeps comparing inline-vs-blocking directly,
/// independent of whatever `app::router` currently does -- the whole
/// point of this bench is deciding that, not assuming it.
async fn inline_spend(State(ctx): State<Arc<Ctx>>, headers: http::HeaderMap) -> impl IntoResponse {
    let wire = ctx.handle_spend(&headers, "/resource");
    (wire.status, wire.body)
}

async fn blocking_spend(
    State(ctx): State<Arc<Ctx>>,
    headers: http::HeaderMap,
) -> impl IntoResponse {
    let wire = tokio::task::spawn_blocking(move || ctx.handle_spend(&headers, "/resource"))
        .await
        .unwrap();
    (wire.status, wire.body)
}

fn inline_router(ctx: Arc<Ctx>) -> Router {
    Router::new()
        .route("/spend", get(inline_spend))
        .with_state(ctx)
}

fn blocking_router(ctx: Arc<Ctx>) -> Router {
    Router::new()
        .route("/spend", get(blocking_spend))
        .with_state(ctx)
}

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

/// Bind `router` on an ephemeral loopback port, serve it on the
/// current (already-running) tokio runtime, and hammer `path` with
/// `concurrency` concurrent real async tasks for `RUN_DURATION`,
/// each one a fresh HTTP request over a real `reqwest::Client`
/// (connection-pooled, not reconnecting per request). Returns total
/// completed requests across every task.
async fn run_load(router: Router, path: &str, headers: http::HeaderMap, concurrency: usize) -> u64 {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr: SocketAddr = listener.local_addr().unwrap();
    let server = tokio::spawn(async move {
        axum::serve(listener, router).await.unwrap();
    });

    let client = reqwest::Client::new();
    let url = format!("http://{addr}{path}");
    let total = Arc::new(AtomicU64::new(0));
    let deadline = Instant::now() + RUN_DURATION;

    let mut tasks = Vec::with_capacity(concurrency);
    for _ in 0..concurrency {
        let client = client.clone();
        let url = url.clone();
        let headers = headers.clone();
        let total = Arc::clone(&total);
        tasks.push(tokio::spawn(async move {
            let mut ops = 0u64;
            while Instant::now() < deadline {
                let _ = client
                    .get(&url)
                    .headers(headers.clone())
                    .send()
                    .await
                    .and_then(|r| r.error_for_status());
                ops += 1;
            }
            total.fetch_add(ops, Ordering::Relaxed);
        }));
    }
    for t in tasks {
        let _ = t.await;
    }
    server.abort();
    total.load(Ordering::Relaxed)
}

fn report_row(label: &str, worker_threads: usize, concurrency: usize, ops: u64) {
    let rate = ops as f64 / RUN_DURATION.as_secs_f64();
    println!(
        "{label:<28} workers={worker_threads:<3} concurrency={concurrency:<4} ops={ops:<9} req/s={rate:.0}"
    );
}

fn sweep_worker_threads(
    label: &str,
    concurrency: usize,
    make: impl Fn() -> (Router, String, http::HeaderMap),
) {
    for &workers in WORKER_COUNTS {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(workers)
            .enable_all()
            .build()
            .unwrap();
        let (router, path, headers) = make();
        let ops = rt.block_on(run_load(router, &path, headers, concurrency));
        report_row(label, workers, concurrency, ops);
    }
}

fn sweep_concurrency(
    label: &str,
    worker_threads: usize,
    make: impl Fn() -> (Router, String, http::HeaderMap),
) {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(worker_threads)
        .enable_all()
        .build()
        .unwrap();
    for &concurrency in CONCURRENCY_LEVELS {
        let (router, path, headers) = make();
        let ops = rt.block_on(run_load(router, &path, headers, concurrency));
        report_row(label, worker_threads, concurrency, ops);
    }
}

fn main() {
    println!(
        "worker_threads: {} worker-thread level(s), {:?} run each\n",
        WORKER_COUNTS.len(),
        RUN_DURATION
    );

    println!("-- Test A: fixed concurrency={TEST_A_CONCURRENCY}, swept worker-thread count --");
    sweep_worker_threads("bare axum (calibration)", TEST_A_CONCURRENCY, || {
        (bare_axum_router(), "/".to_string(), http::HeaderMap::new())
    });
    sweep_worker_threads("real server (app::router)", TEST_A_CONCURRENCY, || {
        let (_file, ctx, headers) = build_ctx(TEST_A_CONCURRENCY);
        let router = app::router(AppState { ctx });
        (router, SPEND_PATH.to_string(), headers[0].clone())
    });

    // Test B compares two bench-local handlers, not `app::router`:
    // `app::router`'s production handler already always uses
    // `spawn_blocking` (see `app.rs`), so it can't supply an inline
    // baseline to compare against -- these two exist independent of
    // whatever's currently landed there.
    println!(
        "\n-- Test B: fixed worker-thread count={TEST_B_WORKER_THREADS}, swept concurrency --"
    );
    sweep_concurrency(
        "handler, inline (bench-local)",
        TEST_B_WORKER_THREADS,
        || {
            let (_file, ctx, headers) =
                build_ctx(CONCURRENCY_LEVELS.iter().copied().max().unwrap());
            let router = inline_router(ctx);
            (router, "/spend".to_string(), headers[0].clone())
        },
    );
    sweep_concurrency(
        "handler, spawn_blocking (bench-local)",
        TEST_B_WORKER_THREADS,
        || {
            let (_file, ctx, headers) =
                build_ctx(CONCURRENCY_LEVELS.iter().copied().max().unwrap());
            let router = blocking_router(ctx);
            (router, "/spend".to_string(), headers[0].clone())
        },
    );
}
