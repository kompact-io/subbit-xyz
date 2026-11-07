//! Concurrent L2 write-path throughput, in process -- the same shape
//! canary's `:burst` persona drives against a real `subbit-gate`
//! (preprod, 200 consumers), reproduced here directly against `Db` so
//! it's a `cargo bench` instead of a live deployment. Canary still
//! owns verifying this against a real server/network; this owns
//! verifying the write-buffering mechanism itself, cheaply and
//! repeatably.
//!
//! Run with: `cargo bench --bench l2_throughput`
use std::{
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

use tempfile::NamedTempFile;

use subbit_core::{Iou, Signature};
use subbit_gate::db::{self, BufferConfig, Db, ExposureConfig};
use subbit_gate::{Aux, Backing, Bucket, Channel, Keytag};

const RUN_DURATION: Duration = Duration::from_secs(1);
const SPEND_COST: u64 = 1;
const CONCURRENCY_LEVELS: &[usize] = &[1, 10, 50, 200];

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn sample_key(i: usize) -> Keytag {
    let mut buf = vec![0u8; 35];
    buf[..8].copy_from_slice(&(i as u64).to_le_bytes());
    Keytag::from(buf)
}

/// Backed and committed for far more than `RUN_DURATION` could ever
/// spend at `SPEND_COST` per op, so no thread ever hits `Unspendable`
/// and skews the measured throughput.
fn funded_channel() -> Channel {
    Channel::new(
        Some(Backing::new(u64::MAX / 2, 0)),
        Some(Iou::new(u64::MAX / 2, Signature::from([0u8; 64]))),
        0,
        Bucket::default(),
        Aux::default(),
        None,
    )
}

fn open_db(buffer: BufferConfig) -> (NamedTempFile, Db) {
    let file = NamedTempFile::new().expect("create temp file");
    let config = db::Config {
        db_path: file.path().to_str().unwrap().to_string(),
        // Exposure's own checkpoint trigger is a separate durability
        // mechanism (see `db.rs`'s own doc comment) -- disabled here so
        // this measures only the buffering mechanism under test, not
        // exposure's independent forced-checkpoint behavior.
        exposure: ExposureConfig {
            max_cumulative_exposure: u64::MAX,
            single_write_threshold: u64::MAX,
            max_checkpoint_interval: Duration::from_secs(3600),
        },
        buffer,
    };
    let db = Db::open(config).expect("open db");
    (file, db)
}

/// Collapses to one `redb` write transaction per call -- any nonzero
/// `update_exposed` amount immediately exceeds `max_pending: 0`, so
/// `maybe_flush` fires every time. Reproduces the pre-buffering
/// design's cost profile as a baseline to compare against, not a
/// separate code path of its own.
fn unbuffered() -> BufferConfig {
    BufferConfig {
        max_pending: 0,
        max_buffer_interval: Duration::from_secs(3600),
    }
}

// ---------------------------------------------------------------------------
// Load
// ---------------------------------------------------------------------------

/// `consumers` threads, each hammering its own dedicated channel for
/// `RUN_DURATION` -- matching canary's `:burst` persona's "N concurrent
/// consumers" shape, but driving `Db` directly instead of real HTTP
/// requests through a running server. Returns total completed spends
/// across every thread.
fn run_concurrent_spends(db: &Db, consumers: usize) -> u64 {
    let keys: Vec<Keytag> = (0..consumers).map(sample_key).collect();
    for key in &keys {
        db.upsert(funded_channel(), key, Ok).expect("seed channel");
    }

    let total = Arc::new(AtomicU64::new(0));
    let deadline = Instant::now() + RUN_DURATION;

    thread::scope(|scope| {
        for key in &keys {
            let total = Arc::clone(&total);
            scope.spawn(move || {
                let mut ops = 0u64;
                while Instant::now() < deadline {
                    db.update_exposed(key, |mut ch| {
                        ch.apply_spend(SPEND_COST)?;
                        Ok((ch, SPEND_COST))
                    })
                    .expect("spend");
                    ops += 1;
                }
                total.fetch_add(ops, Ordering::Relaxed);
            });
        }
    });

    total.load(Ordering::Relaxed)
}

fn report_row(label: &str, consumers: usize, ops: u64) {
    let rate = ops as f64 / RUN_DURATION.as_secs_f64();
    println!("{label:<24} consumers={consumers:<4} ops={ops:<10} req/s={rate:.0}");
}

fn main() {
    println!(
        "l2_throughput: {} consumers x {:?} run each, cost={SPEND_COST}/spend\n",
        CONCURRENCY_LEVELS.len(),
        RUN_DURATION
    );

    for &consumers in CONCURRENCY_LEVELS {
        let (_file, db) = open_db(unbuffered());
        let ops = run_concurrent_spends(&db, consumers);
        report_row("unbuffered (baseline)", consumers, ops);
    }

    println!();

    for &consumers in CONCURRENCY_LEVELS {
        let (_file, db) = open_db(BufferConfig::default());
        let ops = run_concurrent_spends(&db, consumers);
        report_row("buffered (default)", consumers, ops);
    }
}
