//! Thin read-only client for `subbit-server`'s `GET /v1/a/channels`
//! (`{channel_id: {is_active, iou}}`). This service never writes
//! `is_active` -- see `decide`'s doc comment for why.

use std::collections::BTreeMap;

use url::Url;

use crate::keytag::Keytag;

/// Mirrors `subbit_server::ChannelSummary`'s wire shape, decoded
/// independently here rather than depending on `subbit-server` itself.
#[derive(Debug, Clone, minicbor::Decode)]
pub struct ChannelSummary {
    #[n(0)]
    pub is_active: bool,
    #[n(1)]
    pub iou: Option<subbit_core::Iou>,
}

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Request(#[from] reqwest::Error),
    #[error("decode: {0}")]
    Decode(#[from] minicbor::decode::Error),
}

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

    pub async fn channel_summaries(&self) -> Result<BTreeMap<Keytag, ChannelSummary>, Error> {
        let bytes = self
            .http
            .get(self.endpoint.clone())
            .send()
            .await?
            .error_for_status()?
            .bytes()
            .await?;
        Ok(minicbor::decode(&bytes)?)
    }
}
