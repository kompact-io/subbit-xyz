use std::time::{SystemTime, UNIX_EPOCH};

mod action;
pub use action::Action;

mod config;
pub use config::Config;

mod bucket;
pub use bucket::Bucket;

/// Ties a Config and a clock function together to gate actions against a
/// Bucket. Doesn't own bucket state — callers own the Bucket (e.g. loaded
/// from redb inside a write txn) and pass it in per call.
pub struct Limiter {
    capacity: u64,
    actions: action::Config,
    clock: fn() -> u64,
}

impl Limiter {
    pub fn new(config: Config, clock: fn() -> u64) -> Self {
        let Config { capacity, actions } = config;
        Self {
            capacity,
            actions,
            clock,
        }
    }

    pub fn now(&self) -> u64 {
        (self.clock)()
    }

    pub fn bucket(&self) -> Bucket {
        Bucket::new(self.capacity, self.now())
    }

    pub fn cost(&self, action: Action) -> u64 {
        self.actions.cost(action)
    }

    /// Mirrors Bucket::add — attempt `action` against `bucket` at the
    /// current time. Returns true if admitted.
    pub fn add(&self, bucket: &mut Bucket, action: Action) -> bool {
        bucket.add(self.cost(action), self.now())
    }

    /// Mirrors Bucket::sub — credit back `action`'s cost (e.g. on a
    /// successful, revenue-generating verification).
    pub fn sub(&self, bucket: &mut Bucket, action: Action) {
        bucket.sub(self.cost(action), self.now())
    }
}

pub fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

impl Default for Limiter {
    fn default() -> Self {
        Self::new(Config::default(), now_ms)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    thread_local! {
        static TEST_NOW: Cell<u64> = const { Cell::new(0) };
    }

    fn test_clock() -> u64 {
        TEST_NOW.with(|c| c.get())
    }

    fn set_test_clock(t: u64) {
        TEST_NOW.with(|c| c.set(t));
    }

    fn test_limiter() -> Limiter {
        set_test_clock(0);
        Limiter::new(Config::default(), test_clock)
    }

    const CAPACITY: u64 = 30_000;

    #[test]
    fn pure_register_spam_blocks_at_3() {
        let limiter = test_limiter();
        let mut bucket = Bucket::new(CAPACITY, 0);
        for i in 0..3 {
            assert!(limiter.add(&mut bucket, Action::Register), "register {i}");
        }
        assert!(!limiter.add(&mut bucket, Action::Register));
    }

    #[test]
    fn pure_auth_spam_blocks_at_10() {
        let limiter = test_limiter();
        let mut bucket = Bucket::new(CAPACITY, 0);
        for i in 0..10 {
            assert!(limiter.add(&mut bucket, Action::Auth), "auth {i}");
        }
        assert!(!limiter.add(&mut bucket, Action::Auth));
    }

    #[test]
    fn pure_iou_spam_blocks_at_5() {
        let limiter = test_limiter();
        let mut bucket = Bucket::new(CAPACITY, 0);
        for i in 0..5 {
            assert!(limiter.add(&mut bucket, Action::Iou), "iou {i}");
        }
        assert!(!limiter.add(&mut bucket, Action::Iou));
    }

    #[test]
    fn mixed_actions_share_the_pool() {
        let limiter = test_limiter();
        let mut bucket = Bucket::new(CAPACITY, 0);
        assert!(limiter.add(&mut bucket, Action::Register));
        assert!(limiter.add(&mut bucket, Action::Register));
        for i in 0..3 {
            assert!(limiter.add(&mut bucket, Action::Auth), "auth {i}");
        }
        assert!(!limiter.add(&mut bucket, Action::Auth));
    }

    #[test]
    fn one_register_slot_recovers_after_10000ms() {
        let limiter = test_limiter();
        let mut bucket = Bucket::new(CAPACITY, 0);
        for _ in 0..3 {
            assert!(limiter.add(&mut bucket, Action::Register));
        } // level = 30,000, last_update = 0

        // probe on a clone so checking "not yet" doesn't itself accrue
        // debt against the bucket we're asserting recovery on
        let mut probe = bucket.clone();
        set_test_clock(9_999);
        assert!(!limiter.add(&mut probe, Action::Register));

        set_test_clock(10_000);
        assert!(limiter.add(&mut bucket, Action::Register));
    }

    #[test]
    fn one_auth_slot_recovers_after_3000ms() {
        let limiter = test_limiter();
        let mut bucket = Bucket::new(CAPACITY, 0);
        for _ in 0..10 {
            assert!(limiter.add(&mut bucket, Action::Auth));
        } // level = 30,000, last_update = 0

        let mut probe = bucket.clone();
        set_test_clock(2_999);
        assert!(!limiter.add(&mut probe, Action::Auth));

        set_test_clock(3_000);
        assert!(limiter.add(&mut bucket, Action::Auth));
    }

    #[test]
    fn sub_frees_headroom_for_a_later_action() {
        let limiter = test_limiter();
        let mut bucket = Bucket::new(CAPACITY, 0);
        for _ in 0..5 {
            assert!(limiter.add(&mut bucket, Action::Iou));
        } // level = 30,000, full
        limiter.sub(&mut bucket, Action::Iou); // level -> 24,000
        assert!(
            limiter.add(&mut bucket, Action::Auth),
            "6,000 of headroom should admit a 3,000-cost auth"
        );
    }

    #[test]
    fn rejected_attempts_still_accrue_and_extend_recovery() {
        let limiter = test_limiter();
        let mut bucket = Bucket::new(CAPACITY, 0);
        for _ in 0..3 {
            assert!(limiter.add(&mut bucket, Action::Register));
        } // level = 30,000

        assert!(!limiter.add(&mut bucket, Action::Register)); // rejected, level -> 40,000, last_update -> 0

        set_test_clock(10_000);
        assert!(
            !limiter.add(&mut bucket, Action::Register),
            "still over: this probe itself also accrues, resetting last_update to 10,000"
        );
        // after this probe: leak(10,000) -> level 30,000; +10,000 -> 40,000 (rejected); last_update -> 10,000

        set_test_clock(30_000); // 20,000ms after the *last* probe, not the last success
        assert!(
            limiter.add(&mut bucket, Action::Register),
            "debt from both rejected probes finally paid down"
        );
    }
}
