use std::time::{SystemTime, UNIX_EPOCH};

use subbit_core::Instant;

pub fn now() -> Instant {
    Instant::from_secs(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}
