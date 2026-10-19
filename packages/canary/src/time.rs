//! Wall-clock time as a validity-interval bound: `Close`/`Elapse` bounds
//! and `subbit-claim`'s `panic_settle` window are all measured against
//! this one clock, shared rather than redefined per caller.

use subbit_core::Duration;

pub(crate) fn now() -> Duration {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis() as u64;
    Duration::from_millis(ms)
}
