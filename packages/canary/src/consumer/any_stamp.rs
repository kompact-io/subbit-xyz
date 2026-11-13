//! Wrap stamp of either dialect direct to gate.
//!
//! TODO: Consider upstreaming this into `subbit_stamp`, at least the core.

use std::sync::Arc;

use anyhow::Context;
use reqwest_middleware::{ClientBuilder, ClientWithMiddleware, Error as MwError};
use subbit_core::Tag;
use subbit_l2_dto::Terms;
use subbit_l2_dto::dialect::x402::SubbitAccept;
use subbit_stamp::catalog;
use subbit_stamp::dialect::vanilla as vanilla_dialect;
use subbit_stamp::dialect::x402::{self as x402_dialect, TermsDecision};
use subbit_stamp::reqwest::vanilla::Stamp as VanillaMiddleware;
use subbit_stamp::reqwest::x402::Stamp as X402Middleware;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpendError {
    Protocol(subbit_l2_dto::Error),
    Other,
}

type Policy = fn(&str) -> catalog::MissDecision;
const POLICY: Policy = catalog::reject;

type AcceptPolicy = fn(&SubbitAccept, Option<&SubbitAccept>) -> TermsDecision;
const ACCEPT_POLICY: AcceptPolicy = x402_dialect::accept_once;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dialect {
    Vanilla,
    X402,
}

pub enum AnyStamp {
    Vanilla {
        middleware: Arc<VanillaMiddleware<Policy>>,
        client: ClientWithMiddleware,
        base_url: String,
        id: subbit_stamp::Id,
    },
    X402 {
        middleware: Arc<X402Middleware<Policy, AcceptPolicy>>,
        client: ClientWithMiddleware,
        base_url: String,
        id: subbit_stamp::Id,
    },
}

impl AnyStamp {
    pub async fn init(
        seed: &[u8; 32],
        tag: &[u8],
        dialect: Dialect,
        provider_base_url: String,
    ) -> anyhow::Result<Self> {
        let terms: Terms = reqwest::get(format!("{provider_base_url}/v1/terms"))
            .await
            .context("fetching provider terms")?
            .json()
            .await
            .context("parsing provider terms")?;
        let base_url = format!("{provider_base_url}/v1/spend");
        let catalog = terms.catalog;

        let signing_key = ed25519_dalek::SigningKey::from_bytes(seed);
        let account = subbit_stamp::Id::new(signing_key, Tag::from(tag.to_vec()));
        let id = account.clone();
        let mut stamp =
            subbit_stamp::Stamp::new(account, subbit_core::Duration::from_secs(3600)).by_id();
        stamp
            .set_catalog(catalog)
            .context("indexing provider catalog")?;

        Ok(match dialect {
            Dialect::Vanilla => {
                let middleware = Arc::new(VanillaMiddleware::new(stamp, POLICY));
                let client = ClientBuilder::new(reqwest::Client::new())
                    .with_arc(middleware.clone())
                    .build();
                Self::Vanilla {
                    middleware,
                    client,
                    base_url,
                    id,
                }
            }
            Dialect::X402 => {
                let middleware = Arc::new(X402Middleware::new(stamp, POLICY, ACCEPT_POLICY));
                let client = ClientBuilder::new(reqwest::Client::new())
                    .with_arc(middleware.clone())
                    .build();
                Self::X402 {
                    middleware,
                    client,
                    base_url,
                    id,
                }
            }
        })
    }

    pub fn cache(&self) -> subbit_stamp::Cache {
        match self {
            Self::Vanilla { middleware, .. } => middleware.cache(),
            Self::X402 { middleware, .. } => middleware.cache(),
        }
    }

    pub fn committed(&self) -> Option<u64> {
        self.cache().committed()
    }

    // Required when forced settles on tidy
    pub fn committed_iou(&self) -> Option<subbit_core::Iou> {
        let amount = self.committed()?;
        let iou = match self {
            Self::Vanilla { id, .. } => id.iou(amount),
            Self::X402 { id, .. } => id.iou(amount),
        };
        Some(iou)
    }

    // FIXME :: Support arbitrary paths.
    pub async fn spend(&self, level: char) -> Result<u64, SpendError> {
        let url = match self {
            Self::Vanilla { base_url, .. } | Self::X402 { base_url, .. } => {
                format!("{base_url}/{}", crate::catalog::path(level))
            }
        };
        let before = self.cache().committed().unwrap_or(0);
        let result = match self {
            Self::Vanilla { client, .. } => client.get(&url).send().await,
            // Handle the x402 roundtrip
            Self::X402 { client, .. } => match client.get(&url).send().await {
                Ok(resp) => Ok(resp),
                Err(_first_discovers_terms) => client.get(&url).send().await,
            },
        };
        match result {
            Ok(resp) if resp.status().is_success() => {
                let after = self.cache().committed().unwrap_or(before);
                Ok(after.saturating_sub(before))
            }
            Ok(_) => Err(SpendError::Other),
            Err(err) => Err(self.classify(&err)),
        }
    }

    fn classify(&self, err: &MwError) -> SpendError {
        let MwError::Middleware(any) = err else {
            return SpendError::Other;
        };
        let core = match self {
            Self::Vanilla { .. } => {
                any.downcast_ref::<vanilla_dialect::Error>()
                    .and_then(|e| match e {
                        vanilla_dialect::Error::Protocol(core) => Some(*core),
                        _ => None,
                    })
            }
            Self::X402 { .. } => any
                .downcast_ref::<x402_dialect::Error>()
                .and_then(|e| match e {
                    x402_dialect::Error::Protocol(core) => Some(*core),
                    _ => None,
                }),
        };
        core.map(SpendError::Protocol).unwrap_or(SpendError::Other)
    }
}
