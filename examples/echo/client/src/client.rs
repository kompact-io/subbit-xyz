//! HTTP client used to talk to the echo server, and the one request it
//! knows how to make: a plain `reqwest` client when no `[subbit]` is
//! configured, or one wrapped in `subbit_issuer`'s metering middleware when
//! there is.

use std::sync::Arc;

use anyhow::Context;
use http::Extensions;
use reqwest::{Request, Response, Url};
use reqwest_middleware::{
    ClientBuilder, ClientWithMiddleware, Middleware, Next, Result as MwResult,
};
use subbit_issuer::cost::url_lookup::UrlLookup;
use subbit_issuer::native_url_lookup;
use subbit_issuer::reqwest::issuer::Issuer as IssuerMiddleware;

use crate::config::Config;

pub enum Client {
    Plain(ClientWithMiddleware),
    Subbit {
        client: ClientWithMiddleware,
        // Kept alongside the client (rather than only inside it) so we can
        // read `.cache()` back out after the request completes.
        issuer: Arc<IssuerMiddleware<UrlLookup>>,
        cache_path: std::path::PathBuf,
    },
}

impl Client {
    pub fn build(config: &Config) -> anyhow::Result<Self> {
        match &config.subbit {
            None => Ok(Self::Plain(
                ClientBuilder::new(reqwest::Client::new()).build(),
            )),
            Some(subbit) => {
                // The same `Config::build` native_url_lookup::NativeUrlLookup
                // wraps for its own no-network driver — here we take the raw
                // pieces and wire them into the middleware instead.
                let (issuer, cost) = subbit.build().context("failed to build issuer")?;
                let issuer = Arc::new(IssuerMiddleware::new(issuer, cost));
                let client = ClientBuilder::new(reqwest::Client::new())
                    .with(SharedIssuer(issuer.clone()))
                    .build();
                Ok(Self::Subbit {
                    client,
                    issuer,
                    cache_path: subbit.cache.clone(),
                })
            }
        }
    }

    /// POST `body` to `{base_url}/echo` and return the response text.
    /// Persists the issuer's spend-state cache afterwards, if metered.
    pub async fn echo(&self, base_url: &Url, body: Vec<u8>) -> anyhow::Result<String> {
        let resp = self
            .inner()
            .post(base_url.join("echo")?)
            .body(body)
            .send()
            .await?
            .error_for_status()
            .context("server returned an error status")?
            .text()
            .await?;

        self.persist_cache()?;
        Ok(resp)
    }

    fn inner(&self) -> &ClientWithMiddleware {
        match self {
            Self::Plain(c) => c,
            Self::Subbit { client, .. } => client,
        }
    }

    fn persist_cache(&self) -> anyhow::Result<()> {
        if let Self::Subbit {
            issuer, cache_path, ..
        } = self
        {
            native_url_lookup::write_cache(cache_path, &issuer.cache())?;
        }
        Ok(())
    }
}

struct SharedIssuer(Arc<IssuerMiddleware<UrlLookup>>);

#[async_trait::async_trait]
impl Middleware for SharedIssuer {
    async fn handle(
        &self,
        req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> MwResult<Response> {
        self.0.handle(req, extensions, next).await
    }
}
