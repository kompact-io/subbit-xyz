//! Shared wall-clock source for validity-interval bounds, and `Round`

use subbit_core::Instant;

pub(crate) fn now() -> Instant {
    let ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_millis() as u64;
    Instant::from_millis(ms)
}

// Scenario's notion of time
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Round {
    Tidying,
    Active(u8),
}
