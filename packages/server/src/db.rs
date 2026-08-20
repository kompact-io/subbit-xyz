//! Next optimization: Sharding?
//!
use std::{
    path::Path,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

use minicbor::{Decode, Encode};
use redb::{
    Database, Durability, ReadableDatabase, ReadableTable, TableDefinition, WriteTransaction,
};
use serde::{Deserialize, Serialize};
use subbit_core::envelope;

use crate::{Channel, Keytag, channel};

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

impl From<Error> for envelope::Error {
    fn from(value: Error) -> Self {
        match value {
            Error::Backend(_) => Self::Other,
            Error::NotFound => Self::NoChannel,
            Error::Channel(error) => error.into(),
        }
    }
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
}

impl Default for Config {
    fn default() -> Self {
        Self {
            db_path: "/tmp/subbit.redb".to_string(),
            exposure: Default::default(),
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
    /// Max staleness before a checkpoint is forced. NOT self-enforcing —
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
/// (2) is load-bearing, not optional: `upsert`/`upsert_batch`/`remove`/
/// `update` never check staleness themselves. Without a periodic external
/// call to `checkpoint()`, a workload that avoids `update_exposed` can
/// stay non-durable indefinitely.
#[derive(Clone)]
pub struct Db {
    inner: Arc<Database>,
    config: ExposureConfig,
    exposure: Arc<Mutex<Exposure>>,
}

impl Db {
    pub fn open(config: Config) -> Result<Self, Error> {
        Ok(Self::open_inner(config.db_path, config.exposure)?)
    }

    fn open_inner(path: impl AsRef<Path>, config: ExposureConfig) -> Result<Self, BackendError> {
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
        })
    }

    fn begin_relaxed_txn(&self) -> Result<WriteTransaction, BackendError> {
        let mut write_txn = self.inner.begin_write()?;
        write_txn.set_durability(Durability::None)?;
        Ok(write_txn)
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

    /// Empty `Immediate` commit — flushes all pending `None` writes.
    /// Counters reset only on success, so a failed checkpoint leaves
    /// state such that the next write will retry it.
    fn force_checkpoint(&self) -> Result<(), BackendError> {
        let mut write_txn = self.inner.begin_write()?;
        write_txn.set_durability(Durability::Immediate)?;
        write_txn.commit()?;
        let mut exposure = self.exposure.lock().unwrap();
        exposure.at_risk = 0;
        exposure.last_checkpoint = Instant::now();
        Ok(())
    }

    /// Call on a periodic timer (interval <= `max_checkpoint_interval`).
    /// This is the only staleness backstop for writes that never go
    /// through `update_exposed`.
    pub fn checkpoint(&self) -> Result<(), Error> {
        Ok(self.force_checkpoint()?)
    }

    pub fn keys(&self) -> Result<Vec<Keytag>, Error> {
        Ok(self.keys_inner()?)
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
    pub fn upsert<F>(&self, default: Channel, key: &Keytag, f: F) -> Result<Channel, Error>
    where
        F: FnOnce(Channel) -> Result<Channel, channel::Error>,
    {
        let write_txn = self.begin_relaxed_txn().map_err(Error::from)?;
        let result;
        {
            let mut table = write_txn.open_table(TABLE).map_err(BackendError::from)?;
            let existing = table
                .get(key)
                .map_err(BackendError::from)?
                .map(|g| g.value())
                .unwrap_or(default);
            result = f(existing).map_err(Error::Channel)?;
            table.insert(key, &result).map_err(BackendError::from)?;
        }
        write_txn.commit().map_err(BackendError::from)?;
        Ok(result)
    }

    pub fn upsert_batch<I, F>(&self, default: Channel, items: I) -> Result<Vec<Channel>, Error>
    where
        I: IntoIterator<Item = (Keytag, F)>,
        F: FnOnce(Channel) -> Result<Channel, channel::Error>,
        Channel: Clone,
    {
        let write_txn = self.begin_relaxed_txn().map_err(Error::from)?;
        let mut results = Vec::new();
        {
            let mut table = write_txn.open_table(TABLE).map_err(BackendError::from)?;
            for (key, f) in items {
                let existing = table
                    .get(&key)
                    .map_err(BackendError::from)?
                    .map(|g| g.value())
                    .unwrap_or_else(|| default.clone());
                let result = f(existing).map_err(Error::Channel)?;
                table.insert(&key, &result).map_err(BackendError::from)?;
                results.push(result);
            }
        }
        write_txn.commit().map_err(BackendError::from)?;
        Ok(results)
    }

    pub fn get(&self, key: &Keytag) -> Result<Option<Channel>, Error> {
        Ok(self.get_inner(key)?)
    }

    fn get_inner(&self, key: &Keytag) -> Result<Option<Channel>, BackendError> {
        let read_txn = self.inner.begin_read()?;
        let table = read_txn.open_table(TABLE)?;
        Ok(table.get(key)?.map(|guard| guard.value()))
    }

    pub fn remove(&self, key: &Keytag) -> Result<Option<Channel>, Error> {
        Ok(self.remove_inner(key)?)
    }

    fn remove_inner(&self, key: &Keytag) -> Result<Option<Channel>, BackendError> {
        let write_txn = self.begin_relaxed_txn()?;
        let removed;
        {
            let mut table = write_txn.open_table(TABLE)?;
            removed = table.remove(key)?.map(|g| g.value());
        }
        write_txn.commit()?;
        Ok(removed)
    }

    /// Requires `key` to exist — use `upsert` to create.
    pub fn update<F>(&self, key: &Keytag, f: F) -> Result<Channel, Error>
    where
        F: FnOnce(Channel) -> Result<Channel, channel::Error>,
    {
        let write_txn = self.begin_relaxed_txn().map_err(Error::from)?;
        let result;
        {
            let mut table = write_txn.open_table(TABLE).map_err(BackendError::from)?;
            let existing = table
                .get(key)
                .map_err(BackendError::from)?
                .map(|g| g.value())
                .ok_or(Error::NotFound)?;
            result = f(existing).map_err(Error::Channel)?;
            table.insert(key, &result).map_err(BackendError::from)?;
        }
        write_txn.commit().map_err(BackendError::from)?;
        Ok(result)
    }

    /// The one write path with exposure. `f` returns the new state and
    /// the amount at risk for this specific change (e.g. an applied
    /// `Iou`) — `Db` doesn't infer it from the value itself. Requires
    /// `key` to exist.
    pub fn update_exposed<F>(&self, key: &Keytag, f: F) -> Result<Channel, Error>
    where
        F: FnOnce(Channel) -> Result<(Channel, u64), channel::Error>,
    {
        let write_txn = self.begin_relaxed_txn().map_err(Error::from)?;
        let result;
        let amount;
        {
            let mut table = write_txn.open_table(TABLE).map_err(BackendError::from)?;
            let existing = table
                .get(key)
                .map_err(BackendError::from)?
                .map(|g| g.value())
                .ok_or(Error::NotFound)?;
            (result, amount) = f(existing).map_err(Error::Channel)?;
            table.insert(key, &result).map_err(BackendError::from)?;
        }
        write_txn.commit().map_err(BackendError::from)?;
        self.note_exposed_write(amount).map_err(Error::from)?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    // TODO: upsert/get/update/update_exposed/remove/checkpoint tests
    // once Iou/Backing/Bucket/Aux exist.
}
