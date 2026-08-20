//! Throughput benchmarks for `Db` under the relaxed-durability design.
//!
//! Run with: `cargo bench --bench db_throughput`
use std::time::Duration;

use criterion::{BatchSize, Criterion, criterion_group, criterion_main};
use tempfile::NamedTempFile;

use subbit_server::db::{self, Db, ExposureConfig};
use subbit_server::{Aux, Backing, Bucket, Channel, Keytag};

// ---------------------------------------------------------------------------
// Fixtures
// ---------------------------------------------------------------------------

fn sample_key(i: u64) -> Keytag {
    let mut buf = vec![0u8; 35];
    buf[..8].copy_from_slice(&i.to_le_bytes());
    Keytag::from(buf)
}

fn sample_channel(spent: u64) -> Channel {
    Channel::new(
        Some(Backing::new(1_000_000, 0)),
        None,
        spent,
        Bucket::default(),
        Aux::default(),
    )
}

/// Mostly-small amounts with occasional larger ones, so the exposure
/// policy exercises both untouched `None` commits and forced
/// checkpoints. Tune to your real traffic shape.
fn mixed_amount(i: u64) -> u64 {
    if i % 50 == 0 { 1_000 } else { 5 }
}

fn open_db(exposure: ExposureConfig) -> (NamedTempFile, Db) {
    let file = NamedTempFile::new().expect("create temp file");
    let config = db::Config {
        db_path: file.path().to_str().unwrap().to_string(),
        exposure,
    };
    let db = Db::open(config).expect("open db");
    (file, db)
}

/// A config whose thresholds are effectively unreachable within a single
/// benchmark run — `update_exposed` never forces a checkpoint on its own.
fn config_never_checkpoints() -> ExposureConfig {
    ExposureConfig {
        max_cumulative_exposure: u64::MAX,
        single_write_threshold: u64::MAX,
        max_checkpoint_interval: Duration::from_secs(3600),
    }
}

/// A config where every single exposed write exceeds the cap, forcing a
/// checkpoint every time — the exposure-path equivalent of "always
/// durable."
fn config_always_checkpoints() -> ExposureConfig {
    ExposureConfig {
        max_cumulative_exposure: 0,
        single_write_threshold: 0,
        max_checkpoint_interval: Duration::from_secs(3600),
    }
}

// ---------------------------------------------------------------------------
// upsert / update: never touch the exposure lock, always `Durability::None`.
// No config axis to vary — these measure the relaxed-write floor, and a
// manual-checkpoint variant to show the durable ceiling for comparison.
// ---------------------------------------------------------------------------

fn bench_upsert(c: &mut Criterion) {
    let mut group = c.benchmark_group("upsert");
    group.measurement_time(Duration::from_secs(10));

    let mut i: u64 = 0;
    group.bench_function("relaxed", |b| {
        let (_file, db) = open_db(config_never_checkpoints());
        b.iter_batched(
            || {
                i += 1;
                (sample_key(i), sample_channel(i))
            },
            |(key, value)| db.upsert(value, &key, |ch| Ok(ch)).expect("upsert"),
            BatchSize::SmallInput,
        );
    });

    // Manually forces a checkpoint after every write — simulates the old
    // "always immediate" baseline now that durability isn't a per-write
    // config choice for `upsert`.
    i = 0;
    group.bench_function("checkpoint_every_write", |b| {
        let (_file, db) = open_db(config_never_checkpoints());
        b.iter_batched(
            || {
                i += 1;
                (sample_key(i), sample_channel(i))
            },
            |(key, value)| {
                db.upsert(value, &key, |ch| Ok(ch)).expect("upsert");
                db.checkpoint().expect("checkpoint");
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

fn bench_update(c: &mut Criterion) {
    let mut group = c.benchmark_group("update");
    group.measurement_time(Duration::from_secs(10));

    const POOL_SIZE: u64 = 10_000;
    fn seed_pool(db: &Db) {
        for i in 0..POOL_SIZE {
            db.upsert(sample_channel(0), &sample_key(i), |ch| Ok(ch))
                .expect("seed");
        }
    }

    let (_file, db) = open_db(config_never_checkpoints());
    seed_pool(&db);

    let mut i: u64 = 0;
    group.bench_function("relaxed", |b| {
        b.iter_batched(
            || {
                let key = sample_key(i % POOL_SIZE);
                i += 1;
                key
            },
            |key| {
                db.update(&key, |mut ch| {
                    ch.apply_refund(0); // no-op; replace with representative work
                    Ok(ch)
                })
                .expect("update")
            },
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// update_exposed: the one path that touches the exposure lock and can
// force a checkpoint on its own. This is where the policy config matters.
// ---------------------------------------------------------------------------

fn bench_update_exposed(c: &mut Criterion) {
    let mut group = c.benchmark_group("update_exposed");
    group.measurement_time(Duration::from_secs(10));

    const POOL_SIZE: u64 = 10_000;
    fn seed_pool(db: &Db) {
        for i in 0..POOL_SIZE {
            db.upsert(sample_channel(0), &sample_key(i), |ch| Ok(ch))
                .expect("seed");
        }
    }

    let configs = [
        ("always_checkpoints", config_always_checkpoints()),
        ("never_checkpoints", config_never_checkpoints()),
        ("production_default", ExposureConfig::default()),
    ];

    for (name, config) in configs {
        let (_file, db) = open_db(config);
        seed_pool(&db);

        let mut i: u64 = 0;
        group.bench_function(name, |b| {
            b.iter_batched(
                || {
                    let key = sample_key(i % POOL_SIZE);
                    let amount = mixed_amount(i);
                    i += 1;
                    (key, amount)
                },
                |(key, amount)| {
                    db.update_exposed(&key, |mut ch| {
                        ch.apply_refund(0); // no-op; replace with representative work
                        Ok((ch, amount))
                    })
                    .expect("update_exposed")
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// checkpoint: cost of the bare fsync-only transaction in isolation.
// ---------------------------------------------------------------------------

fn bench_checkpoint(c: &mut Criterion) {
    let mut group = c.benchmark_group("checkpoint");
    group.measurement_time(Duration::from_secs(5));

    group.bench_function("bare", |b| {
        let (_file, db) = open_db(config_never_checkpoints());
        b.iter(|| db.checkpoint().expect("checkpoint"));
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_upsert,
    bench_update,
    bench_update_exposed,
    bench_checkpoint
);
criterion_main!(benches);
