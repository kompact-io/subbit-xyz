use subbit_core::Duration;
use web_time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("time: system")]
    System,
}

pub fn now() -> Result<Duration, Error> {
    Ok(Duration::from_millis(
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::System)?
            .as_millis() as u64,
    ))
}
