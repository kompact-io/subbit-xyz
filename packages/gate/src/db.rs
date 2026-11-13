use std::{
    collections::BTreeSet,
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use minicbor::{Decode, Encode};
use redb::{Database, Durability, ReadableDatabase, ReadableTable, TableDefinition};
use serde::{Deserialize, Serialize};

use crate::{Channel, Keytag, buffer::Buffer, channel};

const TABLE: TableDefinition<'static, Keytag, Channel> = TableDefinition::new("values");

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

#[derive(thiserror::Error, Debug)]
pub enum BackendError {
    #[error(transparent)]
    Database(#[from] redb::DatabaseError),
    #[error(transparent)]
    Transaction(#[from] redb::TransactionError),
    #[error(transparent)]
    Table(#[from] redb::TableError),
    #[error(transparent)]
    Storage(#[from] redb::StorageError),
    #[error(transparent)]
    Commit(#[from] redb::CommitError),
    #[error(transparent)]
    SetDurability(#[from] redb::SetDurabilityError),
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Backend(#[from] BackendError),
    #[error("key not found; use `upsert` for a new value")]
    NotFound,
    #[error("channel update failed")]
    Channel(channel::Error),
}

// ---------------------------------------------------------------------------
// Config
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode)]
pub struct Config {
    #[n(0)]
    pub db_path: String,
    #[n(1)]
    pub exposure: ExposureConfig,
    #[n(2)]
    pub buffer: BufferConfig,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db_path: "/tmp/subbit.redb".to_string(),
            exposure: Default::default(),
            buffer: Default::default(),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Encode, Decode)]
pub struct BufferConfig {
    /// Cumulative pending amount across every buffered channel that
    /// forces a flush (same units as `ExposureConfig`'s thresholds).
    #[n(0)]
    pub max_pending: u64,
    /// Max time a write may sit buffered, unflushed.
    #[n(1)]
    pub max_buffer_interval: Duration,
}

impl Default for BufferConfig {
    fn default() -> Self {
        Self {
            max_pending: 2_000,
            max_buffer_interval: Duration::from_millis(50),
        }
    }
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, Encode, Decode)]
pub struct ExposureConfig {
    /// Cumulative exposure above which the next `update_exposed` call
    /// forces a checkpoint.
    #[n(0)]
    pub max_cumulative_exposure: u64,
    /// Single-write exposure at or under this is never accumulated.
    #[n(1)]
    pub single_write_threshold: u64,
    /// Max staleness before a checkpoint is forced. Not self-enforcing,
    /// see `checkpoint()`.
    #[n(2)]
    pub max_checkpoint_interval: Duration,
}

impl Default for ExposureConfig {
    fn default() -> Self {
        Self {
            max_cumulative_exposure: 500, // e.g. $5.00
            single_write_threshold: 5,    // e.g. $0.05
            max_checkpoint_interval: Duration::from_secs(30),
        }
    }
}

// ---------------------------------------------------------------------------
// Db
// ---------------------------------------------------------------------------

struct Exposure {
    at_risk: u64,
    last_checkpoint: Instant,
}

/// All writes commit at `Durability::None`. Durability is instead
/// enforced by periodic `Immediate` checkpoints, triggered by:
///   1. `update_exposed` pushing cumulative exposure over
///      `max_cumulative_exposure`, or
///   2. an external caller invoking `checkpoint()` on a timer.
///
/// (2) is load-bearing, not optional: a buffered write only forces a
/// flush on its own via `BufferConfig`'s own thresholds, which bound
/// staleness, not durability. Without a periodic external call to
/// `checkpoint()`, a workload that never crosses `max_cumulative_exposure`
/// can stay non-durable indefinitely.
///
/// Every write funnels through `buffer` instead of `redb` directly:
/// `upsert`/`upsert_batch`/`update`/`update_exposed` all read their
/// baseline from the pending buffer (falling back to the last
/// committed `redb` value), merge in memory, and store the result back
/// into the buffer. `redb`'s write table is only ever touched by a
/// flush, draining the buffer in one transaction. This is what lets a
/// channel with many requests in one flush window cost exactly one
/// `redb` write for that window, not one per request.
#[derive(Clone)]
pub struct Db {
    inner: Arc<Database>,
    config: ExposureConfig,
    exposure: Arc<Mutex<Exposure>>,
    buffer: Arc<Buffer>,
    buffer_config: BufferConfig,
    /// Serializes flushes against each other (not against `merge`,
    /// which only ever contends with the brief swap inside
    /// `Buffer::take`). Needed because `Buffer::draining` can only
    /// safely hold one not-yet-committed generation at a time -- a
    /// second, concurrent flush calling `take` before the first's
    /// commit finishes would silently overwrite it. See `buffer.rs`'s
    /// own doc comment.
    flush_lock: Arc<Mutex<()>>,
}

impl Db {
    pub fn open(config: Config) -> Result<Self, Error> {
        Ok(Self::open_inner(
            config.db_path,
            config.exposure,
            config.buffer,
        )?)
    }

    fn open_inner(
        path: impl AsRef<Path>,
        config: ExposureConfig,
        buffer_config: BufferConfig,
    ) -> Result<Self, BackendError> {
        let db = Database::create(path)?;
        {
            let write_txn = db.begin_write()?;
            {
                let _table = write_txn.open_table(TABLE)?;
            } // table handle dropped before commit
            write_txn.commit()?;
        }
        Ok(Self {
            inner: Arc::new(db),
            config,
            exposure: Arc::new(Mutex::new(Exposure {
                at_risk: 0,
                last_checkpoint: Instant::now(),
            })),
            buffer: Arc::new(Buffer::new()),
            buffer_config,
            flush_lock: Arc::new(Mutex::new(())),
        })
    }

    /// Only call site that touches the exposure lock. Every other write
    /// path is exposure-blind by construction.
    fn note_exposed_write(&self, amount: u64) -> Result<(), BackendError> {
        let should_checkpoint = {
            let mut exposure = self.exposure.lock().unwrap();
            if amount > self.config.single_write_threshold {
                exposure.at_risk += amount;
            }
            let stale = exposure.last_checkpoint.elapsed() > self.config.max_checkpoint_interval;
            exposure.at_risk > self.config.max_cumulative_exposure || stale
        };
        if should_checkpoint {
            self.force_checkpoint()?;
        }
        Ok(())
    }

    /// Drain the buffer's current generation into one `redb` write
    /// transaction at `durability`, atomically swapping in a fresh
    /// empty generation first so nothing landing after the swap point
    /// is at risk of being dropped. Runs even when the buffer is empty:
    /// an `Immediate` flush with nothing to drain is still how a stale
    /// earlier `None` write actually becomes durable.
    ///
    /// Holds `flush_lock` for the whole call, not just the swap: while
    /// this flush's commit is in flight, the drained generation sits
    /// in `Buffer::draining` as the one and only fallback a concurrent
    /// `merge` has between the (now-empty) live generation and `redb`
    /// itself (which doesn't have this data yet). `mark_drained` only
    /// runs after a successful commit -- on failure `draining` is left
    /// populated, since it's the only place this flush's data still
    /// exists; `flush_lock`'s guard still drops on that `Err` return
    /// though, same as on success, so a later, overlapping `take` is
    /// always possible here -- `Buffer::take` itself (not this lock)
    /// is what guarantees that one carries this flush's still-unwritten
    /// data forward instead of discarding it (see its own doc).
    fn flush_with(&self, durability: Durability) -> Result<(), BackendError> {
        let _flush_guard = self.flush_lock.lock().unwrap();
        let drained = self.buffer.take();
        let mut write_txn = self.inner.begin_write()?;
        write_txn.set_durability(durability)?;
        {
            let mut table = write_txn.open_table(TABLE)?;
            for entry in drained.iter() {
                table.insert(entry.key(), entry.value())?;
            }
        }
        write_txn.commit()?;
        self.buffer.mark_drained();
        Ok(())
    }

    /// Flush if this request's write pushed the buffer past its
    /// size or time trigger. Always `Durability::None`: durability is
    /// `note_exposed_write`/`checkpoint`'s job, not this one's.
    fn maybe_flush(&self) -> Result<(), BackendError> {
        if self.buffer.should_flush(
            self.buffer_config.max_pending,
            self.buffer_config.max_buffer_interval,
        ) {
            self.flush_with(Durability::None)?;
        }
        Ok(())
    }

    /// Flush the buffer at `Immediate` durability. Counters reset only
    /// on success, so a failed checkpoint leaves state such that the
    /// next write will retry it.
    fn force_checkpoint(&self) -> Result<(), BackendError> {
        self.flush_with(Durability::Immediate)?;
        let mut exposure = self.exposure.lock().unwrap();
        exposure.at_risk = 0;
        exposure.last_checkpoint = Instant::now();
        Ok(())
    }

    /// Call on a periodic timer (interval <= `max_checkpoint_interval`).
    /// This is the only staleness backstop for writes that never push
    /// `update_exposed`'s cumulative exposure over its own threshold.
    pub fn checkpoint(&self) -> Result<(), Error> {
        Ok(self.force_checkpoint()?)
    }

    /// Every key with committed or buffered state.
    pub fn keys(&self) -> Result<Vec<Keytag>, Error> {
        let mut keys: BTreeSet<Keytag> = self.keys_inner()?.into_iter().collect();
        keys.extend(self.buffer.keys());
        Ok(keys.into_iter().collect())
    }

    fn keys_inner(&self) -> Result<Vec<Keytag>, BackendError> {
        let tx = self.inner.begin_read()?;
        let table = tx.open_table(TABLE)?;
        table
            .iter()?
            .map(|r| {
                let (k, _v) = r?;
                Ok(k.value())
            })
            .collect()
    }

    /// The last committed `redb` value for `key`, or `default` if it
    /// was never created. Only ever called from inside a
    /// `Buffer::merge` closure, i.e. with that key's buffer slot
    /// locked -- see `Buffer::merge`'s own doc comment for why that
    /// matters.
    fn committed_or(&self, key: &Keytag, default: Option<Channel>) -> Result<Channel, Error> {
        match self.get_inner(key).map_err(Error::from)? {
            Some(channel) => Ok(channel),
            None => default.ok_or(Error::NotFound),
        }
    }

    pub fn upsert<F>(&self, default: Channel, key: &Keytag, f: F) -> Result<Channel, Error>
    where
        F: FnOnce(Channel) -> Result<Channel, channel::Error>,
    {
        let (result, _) = self.buffer.merge(key, |existing| {
            let existing = match existing {
                Some(channel) => channel,
                None => self.committed_or(key, Some(default))?,
            };
            Ok::<_, Error>((f(existing).map_err(Error::Channel)?, 0))
        })?;
        self.maybe_flush().map_err(Error::from)?;
        Ok(result)
    }

    pub fn upsert_batch<I, F>(&self, default: Channel, items: I) -> Result<Vec<Channel>, Error>
    where
        I: IntoIterator<Item = (Keytag, F)>,
        F: FnOnce(Channel) -> Result<Channel, channel::Error>,
        Channel: Clone,
    {
        let mut results = Vec::new();
        for (key, f) in items {
            let (result, _) = self.buffer.merge(&key, |existing| {
                let existing = match existing {
                    Some(channel) => channel,
                    None => self.committed_or(&key, Some(default.clone()))?,
                };
                Ok::<_, Error>((f(existing).map_err(Error::Channel)?, 0))
            })?;
            results.push(result);
        }
        self.maybe_flush().map_err(Error::from)?;
        Ok(results)
    }

    pub fn get(&self, key: &Keytag) -> Result<Option<Channel>, Error> {
        if let Some(channel) = self.buffer.get(key) {
            return Ok(Some(channel));
        }
        Ok(self.get_inner(key)?)
    }

    fn get_inner(&self, key: &Keytag) -> Result<Option<Channel>, BackendError> {
        let read_txn = self.inner.begin_read()?;
        let table = read_txn.open_table(TABLE)?;
        Ok(table.get(key)?.map(|guard| guard.value()))
    }

    /// No live caller today. Flushes first so nothing buffered for
    /// `key` resurrects it on the next flush after this removes it.
    pub fn remove(&self, key: &Keytag) -> Result<Option<Channel>, Error> {
        self.flush_with(Durability::None).map_err(Error::from)?;
        Ok(self.remove_inner(key)?)
    }

    fn remove_inner(&self, key: &Keytag) -> Result<Option<Channel>, BackendError> {
        let mut write_txn = self.inner.begin_write()?;
        write_txn.set_durability(Durability::None)?;
        let removed;
        {
            let mut table = write_txn.open_table(TABLE)?;
            removed = table.remove(key)?.map(|g| g.value());
        }
        write_txn.commit()?;
        Ok(removed)
    }

    /// Requires `key` to exist, use `upsert` to create.
    pub fn update<F>(&self, key: &Keytag, f: F) -> Result<Channel, Error>
    where
        F: FnOnce(Channel) -> Result<Channel, channel::Error>,
    {
        let (result, _) = self.buffer.merge(key, |existing| {
            let existing = match existing {
                Some(channel) => channel,
                None => self.committed_or(key, None)?,
            };
            Ok::<_, Error>((f(existing).map_err(Error::Channel)?, 0))
        })?;
        self.maybe_flush().map_err(Error::from)?;
        Ok(result)
    }

    /// The one write path with exposure. `f` returns the new state and
    /// the amount at risk for this specific change (e.g. an applied
    /// `Iou`), `Db` doesn't infer it from the value itself. Requires
    /// `key` to exist.
    pub fn update_exposed<F>(&self, key: &Keytag, f: F) -> Result<Channel, Error>
    where
        F: FnOnce(Channel) -> Result<(Channel, u64), channel::Error>,
    {
        let (result, amount) = self.buffer.merge(key, |existing| {
            let existing = match existing {
                Some(channel) => channel,
                None => self.committed_or(key, None)?,
            };
            f(existing).map_err(Error::Channel)
        })?;
        self.note_exposed_write(amount).map_err(Error::from)?;
        self.maybe_flush().map_err(Error::from)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use subbit_core::{Iou, Signature};
    use tempfile::NamedTempFile;

    use super::*;
    use crate::{Aux, Backing, Bucket, channel_ops::apply_spend};

    fn never_flushes() -> BufferConfig {
        BufferConfig {
            max_pending: u64::MAX,
            max_buffer_interval: Duration::from_secs(3600),
        }
    }

    fn never_checkpoints() -> ExposureConfig {
        ExposureConfig {
            max_cumulative_exposure: u64::MAX,
            single_write_threshold: u64::MAX,
            max_checkpoint_interval: Duration::from_secs(3600),
        }
    }

    fn open(exposure: ExposureConfig, buffer: BufferConfig) -> (NamedTempFile, Db) {
        let file = NamedTempFile::new().unwrap();
        let config = Config {
            db_path: file.path().to_str().unwrap().to_string(),
            exposure,
            buffer,
        };
        (file, Db::open(config).unwrap())
    }

    fn key(i: u8) -> Keytag {
        Keytag::from(vec![i; 35])
    }

    fn channel_with_iou(spent: u64, committed: u64) -> Channel {
        Channel::new(
            Some(Backing::new(1_000_000, 0)),
            Some(Iou::new(committed, Signature::from([0u8; 64]))),
            spent,
            Bucket::default(),
            Aux::default(),
            None,
        )
    }

    #[test]
    fn upsert_is_visible_through_get_before_any_flush() {
        let (_file, db) = open(never_checkpoints(), never_flushes());
        let channel = channel_with_iou(0, 100);
        db.upsert(channel.clone(), &key(1), Ok).unwrap();
        assert_eq!(db.get(&key(1)).unwrap(), Some(channel));
    }

    #[test]
    fn update_exposed_merges_against_the_buffered_baseline_not_a_stale_read() {
        let (_file, db) = open(never_checkpoints(), never_flushes());
        db.upsert(channel_with_iou(0, 1_000), &key(1), Ok).unwrap();

        // Two spends in a row, neither flushed: the second must see the
        // first's buffered result as its baseline, not the committed
        // pre-spend state, or it would double-charge (or under-charge).
        db.update_exposed(&key(1), apply_spend(30)).unwrap();
        let after = db.update_exposed(&key(1), apply_spend(20)).unwrap();
        assert_eq!(after.spent(), 50);
        assert_eq!(db.get(&key(1)).unwrap().unwrap().spent(), 50);
    }

    #[test]
    fn checkpoint_flushes_buffered_writes_into_redb() {
        let (file, db) = open(never_checkpoints(), never_flushes());
        db.upsert(channel_with_iou(0, 1_000), &key(1), Ok).unwrap();
        db.update_exposed(&key(1), apply_spend(40)).unwrap();

        db.checkpoint().unwrap();
        drop(db); // release the file lock so it can be reopened below

        // A fresh `Db` over the same file sees nothing buffered, so this
        // only passes if `checkpoint` actually drained the buffer into
        // the committed table rather than just resetting counters.
        let config = Config {
            db_path: file.path().to_str().unwrap().to_string(),
            exposure: never_checkpoints(),
            buffer: never_flushes(),
        };
        let reopened = Db::open(config).unwrap();
        assert_eq!(reopened.get(&key(1)).unwrap().unwrap().spent(), 40);
    }

    #[test]
    fn size_triggered_flush_persists_without_an_explicit_checkpoint() {
        let tiny_buffer = BufferConfig {
            max_pending: 10,
            max_buffer_interval: Duration::from_secs(3600),
        };
        let (_file, db) = open(never_checkpoints(), tiny_buffer);
        db.upsert(channel_with_iou(0, 1_000), &key(1), Ok).unwrap();
        // This spend's exposure (20) pushes cumulative pending (also 20,
        // since `update_exposed` feeds the same amount to both) over
        // `max_pending` (10), triggering a `maybe_flush` with nothing
        // left buffered afterward.
        db.update_exposed(&key(1), apply_spend(20)).unwrap();
        assert_eq!(db.buffer.keys(), Vec::<Keytag>::new());
    }

    #[test]
    fn update_requires_key_to_already_exist() {
        let (_file, db) = open(never_checkpoints(), never_flushes());
        let err = db.update(&key(1), Ok).unwrap_err();
        assert!(matches!(err, Error::NotFound));
    }

    /// Reproduces the race `l2_throughput`'s bench first caught: many
    /// threads spending against their own channels while flushes keep
    /// firing concurrently (small `max_pending` so a flush interleaves
    /// with nearly every spend) must never drop a spend or return
    /// `NotFound` for a key a flush is mid-drain on.
    #[test]
    fn concurrent_spends_across_many_channels_survive_concurrent_flushes() {
        let tiny_buffer = BufferConfig {
            max_pending: 3,
            max_buffer_interval: Duration::from_millis(1),
        };
        let (_file, db) = open(never_checkpoints(), tiny_buffer);

        const CHANNELS: u8 = 50;
        const SPENDS_PER_CHANNEL: u64 = 200;
        for i in 0..CHANNELS {
            db.upsert(channel_with_iou(0, u64::MAX / 2), &key(i), Ok)
                .unwrap();
        }

        std::thread::scope(|scope| {
            for i in 0..CHANNELS {
                let db = &db;
                scope.spawn(move || {
                    let mut history: Vec<(u64, u64, u64)> = Vec::new();
                    for n in 1..=SPENDS_PER_CHANNEL {
                        let ch = db
                            .update_exposed(&key(i), |mut ch| {
                                let baseline = ch.spent();
                                ch.apply_spend(1)?;
                                history.push((n, baseline, ch.spent()));
                                Ok((ch, 1))
                            })
                            .unwrap();
                        if ch.spent() != n {
                            eprintln!(
                                "channel {i} call #{n} returned spent={}, expected {n}; last 10 history entries: {:?}",
                                ch.spent(),
                                &history[history.len().saturating_sub(10)..]
                            );
                        }
                        assert_eq!(
                            ch.spent(),
                            n,
                            "channel {i} call #{n} returned spent={}, expected {n}",
                            ch.spent()
                        );
                    }
                });
            }
        });

        for i in 0..CHANNELS {
            assert_eq!(
                db.get(&key(i)).unwrap().unwrap().spent(),
                SPENDS_PER_CHANNEL,
                "channel {i} lost a spend to a racing flush"
            );
        }
    }
}
