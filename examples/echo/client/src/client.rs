//! HTTP client used to talk to the echo server, and the one request it
//! knows how to make: a plain `reqwest` client when no `[subbit]` is
//! configured, or one wrapped in `subbit_stamp`'s metering middleware
//! when there is, under whichever dialect the config names.

use std::sync::Arc;

use anyhow::Context;
use http::Extensions;
use reqwest::{Request, Response, Url};
use reqwest_middleware::{
    ClientBuilder, ClientWithMiddleware, Middleware, Next, Result as MwResult,
};
use subbit_stamp::catalog;
use subbit_stamp::dialect::x402::{self as x402_dialect, TermsDecision};
use subbit_stamp::local;
use subbit_stamp::reqwest::vanilla::Stamp as VanillaMiddleware;
use subbit_stamp::reqwest::x402::Stamp as X402Middleware;

use crate::config::{Config, Dialect};

/// This throwaway client has no consent loop of its own to offer: a
/// catalog miss is a hard failure, matching this crate's pre-
/// `MissPolicy` behavior exactly.
type Policy = fn(&str) -> catalog::MissDecision;
const POLICY: Policy = catalog::reject;

/// Same reasoning for x402's own terms: accept the first challenge,
/// never a later change to it, rather than silently following
/// whatever a fresh `PAYMENT-REQUIRED` happens to name.
type Terms = fn(
    &subbit_l2_dto::dialect::x402::SubbitAccept,
    Option<&subbit_l2_dto::dialect::x402::SubbitAccept>,
) -> TermsDecision;
const TERMS: Terms = x402_dialect::accept_once;

pub enum Client {
    Plain(ClientWithMiddleware),
    Vanilla {
        client: ClientWithMiddleware,
        // Kept alongside the client (rather than only inside it) so we can
        // read `.cache()` back out after the request completes.
        stamp: Arc<VanillaMiddleware<Policy>>,
        cache_path: std::path::PathBuf,
    },
    X402 {
        client: ClientWithMiddleware,
        stamp: Arc<X402Middleware<Policy, Terms>>,
        cache_path: std::path::PathBuf,
    },
}

impl Client {
    pub fn build(config: &Config) -> anyhow::Result<Self> {
        let Some(subbit) = &config.subbit else {
            return Ok(Self::Plain(
                ClientBuilder::new(reqwest::Client::new()).build(),
            ));
        };
        // The same `Config::build` `local::Local` wraps for its own
        // no-network driver: here we take the stamp (its catalog
        // already loaded, see `Stamp::set_catalog`) and wire it into
        // whichever dialect's middleware instead.
        let stamp = subbit.build().context("failed to build stamp")?;
        match config.dialect {
            Dialect::Vanilla => {
                let stamp = Arc::new(VanillaMiddleware::new(stamp, POLICY));
                let client = ClientBuilder::new(reqwest::Client::new())
                    .with(ForwardVanilla(stamp.clone()))
                    .build();
                Ok(Self::Vanilla {
                    client,
                    stamp,
                    cache_path: subbit.cache.clone(),
                })
            }
            Dialect::X402 => {
                let stamp = Arc::new(X402Middleware::new(stamp, POLICY, TERMS));
                let client = ClientBuilder::new(reqwest::Client::new())
                    .with(ForwardX402(stamp.clone()))
                    .build();
                Ok(Self::X402 {
                    client,
                    stamp,
                    cache_path: subbit.cache.clone(),
                })
            }
        }
    }

    /// POST `body` to `{base_url}{path}` and return the response text.
    /// Persists the stamp's spend-state cache afterwards, if metered.
    pub async fn echo(&self, base_url: &Url, path: &str, body: Vec<u8>) -> anyhow::Result<String> {
        let url = base_url.join(path.trim_start_matches('/'))?;
        let resp = if matches!(self, Self::X402 { .. }) {
            // x402's first request against a path it hasn't paid under
            // yet always comes back a bare 402, surfaced as a
            // middleware error (see `subbit_stamp::reqwest::x402::
            // Stamp`'s own doc comment): that's expected, not a real
            // failure, so retry once now that terms are learned.
            match self.post(url.clone(), body.clone()).await {
                Ok(resp) => resp,
                Err(_first_discovers_terms) => self.post(url, body).await?,
            }
        } else {
            self.post(url, body).await?
        };

        let text = resp
            .error_for_status()
            .context("server returned an error status")?
            .text()
            .await?;

        self.persist_cache()?;
        Ok(text)
    }

    async fn post(&self, url: Url, body: Vec<u8>) -> reqwest_middleware::Result<Response> {
        self.inner().post(url).body(body).send().await
    }

    fn inner(&self) -> &ClientWithMiddleware {
        match self {
            Self::Plain(c) => c,
            Self::Vanilla { client, .. } => client,
            Self::X402 { client, .. } => client,
        }
    }

    fn persist_cache(&self) -> anyhow::Result<()> {
        match self {
            Self::Plain(_) => Ok(()),
            Self::Vanilla {
                stamp, cache_path, ..
            } => local::write_cache(cache_path, &stamp.cache()).map_err(Into::into),
            Self::X402 {
                stamp, cache_path, ..
            } => local::write_cache(cache_path, &stamp.cache()).map_err(Into::into),
        }
    }
}

struct ForwardVanilla(Arc<VanillaMiddleware<Policy>>);

#[async_trait::async_trait]
impl Middleware for ForwardVanilla {
    async fn handle(
        &self,
        req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> MwResult<Response> {
        self.0.handle(req, extensions, next).await
    }
}

struct ForwardX402(Arc<X402Middleware<Policy, Terms>>);

#[async_trait::async_trait]
impl Middleware for ForwardX402 {
    async fn handle(
        &self,
        req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> MwResult<Response> {
        self.0.handle(req, extensions, next).await
    }
}
