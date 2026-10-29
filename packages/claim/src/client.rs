//! Thin read-only client for `subbit-gate`'s `GET /v1/a/channels`
//! (`{channel_id: {is_active, iou}}`). This service never writes
//! `is_active` -- see `decide`'s doc comment for why.

use std::collections::BTreeMap;

use subbit_index_dto::OutputReference;
use url::Url;

use crate::keytag::Keytag;

/// Mirrors `subbit_gate::channel::Backing`'s wire shape.
#[derive(Debug, Clone, minicbor::Decode)]
pub struct Backing {
    #[n(0)]
    pub amount: u64,
    #[n(1)]
    pub subbed: u64,
}

/// Mirrors `subbit_gate::ChannelSummary`'s wire shape, decoded
/// independently here rather than depending on `subbit-gate` itself.
#[derive(Debug, Clone, minicbor::Decode)]
pub struct ChannelSummary {
    #[n(0)]
    pub is_active: bool,
    #[n(1)]
    pub iou: Option<subbit_core::Iou>,
    /// TxO refs the server has recorded as safe to settle for this
    /// channel -- see `subbit_gate::index_output`'s own doc.
    #[n(2)]
    pub safe_to_settle: Vec<OutputReference>,
    /// `None` means the index's lineage found no open line backing
    /// this channel -- see `decide`'s own use of this.
    #[n(3)]
    pub backing: Option<Backing>,
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
