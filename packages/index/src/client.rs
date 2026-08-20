use reqwest::StatusCode;
use serde::Serialize;
use tracing::debug;
use url::Url;

use crate::wire::{Row, rows_to_map};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Wire(#[from] crate::wire::Error),
    #[error("failed to encode rows: {0}")]
    Encode(String),
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error("endpoint returned {status}: {body}")]
    Status { status: StatusCode, body: String },
}

#[derive(Serialize)]
pub struct SendResult {
    pub count: usize,
    pub status: u16,
}

/// Thin client bound to one endpoint. Holds a `reqwest::Client` so its
/// connection pool is reused across repeated `send` calls (matters for
/// `naive`'s polling loop; a no-op for `mock`'s one-shot use).
#[derive(Clone)]
pub struct Client {
    endpoint: Url,
    http: reqwest::Client,
}

impl Client {
    pub fn new(endpoint: Url) -> Self {
        Self {
            endpoint,
            http: reqwest::Client::new(),
        }
    }

    pub async fn send(&self, rows: Vec<Row>) -> Result<SendResult, Error> {
        let count = rows.len();
        let backings = rows_to_map(rows)?;
        let body = minicbor::to_vec(&backings).map_err(|e| Error::Encode(e.to_string()))?;
        let resp = self
            .http
            .post(self.endpoint.clone())
            .header("content-type", "application/cbor")
            .body(body)
            .send()
            .await?;

        let status = resp.status();
        debug!(%status, endpoint = %self.endpoint, "response received");
        if !status.is_success() {
            let body = resp.text().await.unwrap_or_default();
            return Err(Error::Status { status, body });
        }
        Ok(SendResult {
            count,
            status: status.as_u16(),
        })
    }
}
