use subbit_core::Duration;

pub fn now() -> Duration {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    Duration::from_millis(now_ms as u64)
}
