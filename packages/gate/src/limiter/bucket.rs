use minicbor::{Decode, Encode};
use serde::{Deserialize, Serialize};

/// Leaky bucket rate limiter. All time and all costs are in ms. Rate is
/// fixed at 1 unit/ms. All math is integer.
#[derive(Debug, Clone, Serialize, Deserialize, Encode, Decode, PartialEq)]
pub struct Bucket {
    #[n(0)]
    capacity: u64,
    #[n(1)]
    level: u64,
    #[n(2)]
    last_update: u64, // unix ms
}

impl Default for Bucket {
    fn default() -> Self {
        Self {
            capacity: 30000,
            level: 0,
            last_update: 0,
        }
    }
}

impl Bucket {
    pub fn new(capacity: u64, now: u64) -> Self {
        Self {
            capacity,
            level: 0,
            last_update: now,
        }
    }

    fn leak(&mut self, now: u64) {
        let elapsed = now.saturating_sub(self.last_update);
        self.level = self.level.saturating_sub(elapsed);
        self.last_update = now;
    }

    pub fn add(&mut self, cost: u64, now: u64) -> bool {
        self.leak(now);
        self.level = self.level.saturating_add(cost);
        self.level <= self.capacity
    }

    pub fn sub(&mut self, amount: u64, now: u64) {
        self.leak(now);
        self.level = self.level.saturating_sub(amount);
    }
}
