use subbit_core::Instant;

pub fn now() -> Instant {
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis();
    Instant::from_millis(now_ms as u64)
}
