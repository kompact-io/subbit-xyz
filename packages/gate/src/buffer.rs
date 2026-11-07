//! In-memory write buffer sitting in front of `Db`'s `redb` table.
//!
//! `Channel` only ever stores a running total, so only the latest
//! state per key is ever worth writing. This collapses many requests
//! for the same `Keytag` into one pending entry; `Db` drains it into a
//! single `redb` write transaction per flush instead of one per
//! request.
//!
//! `map` is an `RwLock<Arc<DashMap<..>>>`, not a bare `ArcSwap`: a
//! flush needs to swap in a fresh generation and know, by the time it
//! starts draining the old one, that no `merge` call is still
//! concurrently inserting into it -- otherwise that insert can land
//! after the drain already read the table and vanish when the old
//! generation is dropped. A plain `ArcSwap` can't give that guarantee
//! (a `merge` can load the old `Arc` just before the swap and insert
//! into it just after), so `take` swaps under the *write* lock, which
//! can't proceed while any `merge`'s read-locked critical section is
//! still in flight. Readers (`merge`/`get`/`keys`) never contend with
//! each other, only with the rare flush.
//!
//! That still isn't the whole story: `take` only hands the drained
//! generation to `Db`, which then has to actually write it to `redb`
//! and commit -- real I/O, not instantaneous. Between `take` returning
//! and that commit landing, a key that was only ever written in the
//! drained batch is invisible to both `map` (already swapped away)
//! and `redb` (not committed yet). A concurrent `merge` for that same
//! key, falling back to `redb` in that window, would read `NotFound`
//! or a stale pre-update value and silently lose whatever the drained
//! batch was about to commit. `draining` exists to close exactly that
//! window: `Db::flush_with` parks the detached generation there for
//! as long as its commit is in flight, `merge`/`get` check it as a
//! fallback between `map` and `redb`, and `Db` is responsible for
//! never letting two flushes have a generation parked in `draining` at
//! once (see its own `flush_lock`) -- otherwise a second `take` could
//! overwrite the first flush's still-uncommitted generation here
//! before anyone reads it.

use std::{
    sync::{
        Arc, RwLock,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use dashmap::{DashMap, mapref::entry::Entry};

use crate::{Channel, Keytag};

pub struct Buffer {
    map: RwLock<Arc<DashMap<Keytag, Channel>>>,
    /// The generation `take` most recently detached, for as long as
    /// its commit to `redb` is still in flight. `None` once `Db` calls
    /// `mark_drained` after that commit succeeds.
    draining: RwLock<Option<Arc<DashMap<Keytag, Channel>>>>,
    pending: AtomicU64,
    start: Instant,
    last_flush_millis: AtomicU64,
}

impl Buffer {
    pub fn new() -> Self {
        Self {
            map: RwLock::new(Arc::new(DashMap::new())),
            draining: RwLock::new(None),
            pending: AtomicU64::new(0),
            start: Instant::now(),
            last_flush_millis: AtomicU64::new(0),
        }
    }

    /// The pending state for `key`: the live generation if a request
    /// has touched it since the last flush, else whatever's still
    /// mid-commit in `draining`, else nothing buffered at all.
    pub fn get(&self, key: &Keytag) -> Option<Channel> {
        if let Some(channel) = self.map.read().unwrap().get(key).map(|g| g.value().clone()) {
            return Some(channel);
        }
        self.draining
            .read()
            .unwrap()
            .as_ref()
            .and_then(|draining| draining.get(key).map(|g| g.value().clone()))
    }

    /// Every key currently buffered, unflushed, live or mid-commit.
    pub fn keys(&self) -> Vec<Keytag> {
        let mut keys: Vec<Keytag> = self
            .map
            .read()
            .unwrap()
            .iter()
            .map(|e| e.key().clone())
            .collect();
        if let Some(draining) = self.draining.read().unwrap().as_ref() {
            for entry in draining.iter() {
                if !keys.contains(entry.key()) {
                    keys.push(entry.key().clone());
                }
            }
        }
        keys
    }

    /// Read whatever's currently buffered for `key` (`None` if
    /// nothing has touched it since the last flush and nothing for it
    /// is still mid-commit, see `get`), hand it to `f`, and store the
    /// result back into the live generation -- all as one atomic step
    /// per key. `f` typically falls back to the last committed `redb`
    /// value when handed `None`; doing that read while still holding
    /// this lock is what stops two concurrent requests for the *same*
    /// key from both reading the same stale baseline and losing one of
    /// their updates, exactly the hazard `DashMap`'s own per-shard
    /// locking (reached via `entry`) exists to close. The outer
    /// `RwLock` read-guard is a separate, orthogonal guarantee: it
    /// only keeps a concurrent flush from swapping this generation out
    /// from under us mid-insert (see this module's doc comment).
    pub fn merge<F, E>(&self, key: &Keytag, f: F) -> Result<(Channel, u64), E>
    where
        F: FnOnce(Option<Channel>) -> Result<(Channel, u64), E>,
    {
        let guard = self.map.read().unwrap();
        let entry = guard.entry(key.clone());
        let existing = match &entry {
            Entry::Occupied(occupied) => Some(occupied.get().clone()),
            Entry::Vacant(_) => self
                .draining
                .read()
                .unwrap()
                .as_ref()
                .and_then(|draining| draining.get(key).map(|g| g.value().clone())),
        };
        let (channel, amount) = f(existing)?;
        match entry {
            Entry::Occupied(mut occupied) => {
                occupied.insert(channel.clone());
            }
            Entry::Vacant(vacant) => {
                vacant.insert(channel.clone());
            }
        }
        // `guard` (the read lock) is released when it drops at the end
        // of this function, after the insert above -- not explicitly
        // here, since the entry-level borrows above are tied to its
        // lifetime and an early `drop` would fight the borrow checker
        // over exactly when that borrow actually ends.
        if amount > 0 {
            self.pending.fetch_add(amount, Ordering::Relaxed);
        }
        Ok((channel, amount))
    }

    /// Whether a size- or time-based flush trigger has fired.
    pub fn should_flush(&self, max_pending: u64, max_interval: Duration) -> bool {
        self.pending.load(Ordering::Relaxed) > max_pending
            || self.elapsed_since_last_flush() > max_interval
    }

    fn elapsed_since_last_flush(&self) -> Duration {
        let now_millis = self.start.elapsed().as_millis() as u64;
        let last = self.last_flush_millis.load(Ordering::Relaxed);
        Duration::from_millis(now_millis.saturating_sub(last))
    }

    /// Swap in a fresh, empty generation under the write lock and park
    /// the detached old one in `draining` so `merge`/`get` keep seeing
    /// it until `mark_drained` is called. The write lock can't be
    /// taken while any `merge`/`get`/`keys` call is still holding the
    /// read lock, so nothing concurrently inserted into the old
    /// generation can be caught mid-write by the swap -- by the time
    /// this returns, the old generation is final and safe to iterate.
    ///
    /// Callers (just `Db::flush_with`) must serialize their own calls
    /// to `take`: this doesn't itself stop a second `take` from
    /// overwriting `draining` before a first flush's commit finishes
    /// and calls `mark_drained`.
    pub fn take(&self) -> Arc<DashMap<Keytag, Channel>> {
        // `draining` must be populated *before* the write guard drops:
        // once it drops, a `merge` can immediately see the fresh empty
        // live generation. If `draining` weren't set yet at that exact
        // point, such a `merge` would find a key vacant in both and
        // wrongly fall through to `redb`, which doesn't have this
        // generation's data yet either -- the very bug this module's
        // own doc comment describes `draining` as closing.
        let mut guard = self.map.write().unwrap();
        let drained = std::mem::replace(&mut *guard, Arc::new(DashMap::new()));
        *self.draining.write().unwrap() = Some(Arc::clone(&drained));
        drop(guard);
        self.pending.store(0, Ordering::Relaxed);
        let now_millis = self.start.elapsed().as_millis() as u64;
        self.last_flush_millis.store(now_millis, Ordering::Relaxed);
        drained
    }

    /// Call once the generation `take` most recently handed out has
    /// been durably written to `redb` -- never on failure, since
    /// `draining` is the only place that data still exists otherwise.
    pub fn mark_drained(&self) {
        *self.draining.write().unwrap() = None;
    }
}

#[cfg(test)]
mod tests {
    use std::thread::sleep;

    use super::*;
    use crate::{Aux, Backing, Bucket};

    fn key(i: u8) -> Keytag {
        Keytag::from(vec![i; 35])
    }

    fn channel(spent: u64) -> Channel {
        Channel::new(
            Some(Backing::new(1_000_000, 0)),
            None,
            spent,
            Bucket::default(),
            Aux::default(),
            None,
        )
    }

    fn merge_set(buffer: &Buffer, key: &Keytag, amount: u64, channel: Channel) {
        buffer
            .merge::<_, std::convert::Infallible>(key, |_existing| Ok((channel, amount)))
            .unwrap();
    }

    #[test]
    fn merge_collapses_to_latest() {
        let buffer = Buffer::new();
        merge_set(&buffer, &key(1), 5, channel(5));
        merge_set(&buffer, &key(1), 7, channel(12));
        assert_eq!(buffer.get(&key(1)), Some(channel(12)));
        assert_eq!(buffer.keys(), vec![key(1)]);
    }

    #[test]
    fn missing_key_reads_as_none() {
        let buffer = Buffer::new();
        assert_eq!(buffer.get(&key(1)), None);
    }

    #[test]
    fn merge_sees_its_own_prior_result_as_the_baseline() {
        let buffer = Buffer::new();
        merge_set(&buffer, &key(1), 0, channel(5));
        let (result, _) = buffer
            .merge::<_, std::convert::Infallible>(&key(1), |existing| {
                let spent = existing.map_or(0, |c| c.spent());
                Ok((channel(spent + 1), 0))
            })
            .unwrap();
        assert_eq!(result.spent(), 6);
    }

    #[test]
    fn should_flush_on_cumulative_amount() {
        let buffer = Buffer::new();
        assert!(!buffer.should_flush(100, Duration::from_secs(3600)));
        merge_set(&buffer, &key(1), 60, channel(60));
        merge_set(&buffer, &key(2), 60, channel(60));
        assert!(buffer.should_flush(100, Duration::from_secs(3600)));
    }

    #[test]
    fn should_flush_on_elapsed_time() {
        let buffer = Buffer::new();
        assert!(!buffer.should_flush(u64::MAX, Duration::from_millis(20)));
        sleep(Duration::from_millis(30));
        assert!(buffer.should_flush(u64::MAX, Duration::from_millis(20)));
    }

    #[test]
    fn take_drains_and_resets() {
        let buffer = Buffer::new();
        merge_set(&buffer, &key(1), 50, channel(50));

        let drained = buffer.take();
        assert_eq!(drained.get(&key(1)).map(|g| g.clone()), Some(channel(50)));

        // Still visible via `draining` until the (simulated) commit
        // finishes -- that's the whole point of that fallback.
        assert_eq!(buffer.get(&key(1)), Some(channel(50)));
        assert!(!buffer.should_flush(0, Duration::from_secs(3600)));

        buffer.mark_drained();
        assert_eq!(buffer.get(&key(1)), None);
    }

    #[test]
    fn insert_after_take_lands_in_the_new_generation_only() {
        let buffer = Buffer::new();
        merge_set(&buffer, &key(1), 1, channel(1));
        let drained = buffer.take();
        merge_set(&buffer, &key(2), 1, channel(1));

        assert!(drained.get(&key(2)).is_none());
        assert_eq!(buffer.get(&key(2)), Some(channel(1)));
    }

    #[test]
    fn concurrent_merges_on_the_same_key_lose_nothing() {
        use std::thread;

        let buffer = Buffer::new();
        merge_set(&buffer, &key(1), 0, channel(0));

        const THREADS: usize = 16;
        const PER_THREAD: usize = 200;
        thread::scope(|scope| {
            for _ in 0..THREADS {
                scope.spawn(|| {
                    for _ in 0..PER_THREAD {
                        buffer
                            .merge::<_, std::convert::Infallible>(&key(1), |existing| {
                                let spent = existing.map_or(0, |c| c.spent());
                                Ok((channel(spent + 1), 1))
                            })
                            .unwrap();
                    }
                });
            }
        });

        assert_eq!(
            buffer.get(&key(1)).unwrap().spent(),
            (THREADS * PER_THREAD) as u64
        );
    }

    // Concurrent merge-vs-flush correctness (a flushed generation must
    // never silently lose an insert that landed in it) is exercised at
    // the `Db` level instead, in `db::tests` -- `Buffer` alone has no
    // persistent fallback to reconcile a post-flush read against, so a
    // flush legitimately "forgets" a key's prior baseline here, same
    // as it would for any `Db::get` caller that only ever reads the
    // buffer and never falls back to `redb`.
}
