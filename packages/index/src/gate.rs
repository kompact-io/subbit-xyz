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
pub struct Response {
    pub count: usize,
    pub status: u16,
}

/// Thin subbit gate client bound to one endpoint.
#[derive(Clone)]
pub struct Gate {
    endpoint: Url,
    http: reqwest::Client,
}

impl Gate {
    pub fn new(endpoint: Url) -> Self {
        Self {
            endpoint,
            http: reqwest::Client::new(),
        }
    }

    async fn post(&self, body: Vec<u8>, count: usize) -> Result<Response, Error> {
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
        Ok(Response {
            count,
            status: status.as_u16(),
        })
    }

    /// Posts to the naive (flat, lineage-free) `/v1/a/backings` route.
    pub async fn send(&self, rows: Vec<Row>) -> Result<Response, Error> {
        let count = rows.len();
        let backings = rows_to_map(rows)?;
        let body = minicbor::to_vec(&backings).map_err(|e| Error::Encode(e.to_string()))?;
        self.post(body, count).await
    }

    /// Posts a full lineage report to the `/v1/a/index` route -- see
    /// `subbit_gate::index_output`'s own doc for what the server
    /// does with it.
    pub async fn send_output(
        &self,
        output: &subbit_index_dto::Channels,
    ) -> Result<Response, Error> {
        let count = output.channels.len();
        let body = minicbor::to_vec(output).map_err(|e| Error::Encode(e.to_string()))?;
        self.post(body, count).await
    }
}
