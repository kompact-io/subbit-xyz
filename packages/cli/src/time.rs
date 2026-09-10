use std::time::{SystemTime, UNIX_EPOCH};

use subbit_core::Duration;

pub fn now() -> Duration {
    Duration::from_secs(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs(),
    )
}
