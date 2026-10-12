//! reqwest middleware for vanilla: injects a `subbit` request
//! envelope (costed via `Cost`) on the way out, applies the response
//! envelope on the way back. Gated behind `reqwest` + `vanilla`.

use std::sync::Mutex;

use http::Extensions;
use reqwest::header::HeaderValue;
use reqwest::{Request, Response, ResponseBuilderExt};
use reqwest_middleware::{Error as MwError, Middleware, Next, Result as MwResult};

use crate::Cache;
use crate::dialect::vanilla::{self, HEADER};

/// `reqwest_middleware::Middleware` that injects a request envelope
/// (via `dialect::vanilla::request`, costed by `C: Cost`) and applies
/// the response envelope on the way back.
pub struct Issuer<C> {
    issuer: Mutex<crate::Issuer>,
    costing: C,
}

impl<C: super::Cost> Issuer<C> {
    pub fn new(issuer: crate::Issuer, costing: C) -> Self {
        Self {
            issuer: Mutex::new(issuer),
            costing,
        }
    }

    // On (before) graceful teardown, persist cache.
    pub fn cache(&self) -> Cache {
        self.issuer.lock().unwrap().cache().clone()
    }

    /// Hand back ownership, e.g. to retrieve `issuer.cache()` one last time
    /// after the client is done and dropped.
    pub fn into_issuer(self) -> crate::Issuer {
        self.issuer.into_inner().unwrap()
    }
}

#[async_trait::async_trait]
impl<C: super::Cost + 'static> Middleware for Issuer<C> {
    async fn handle(
        &self,
        mut req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> MwResult<Response> {
        // --- inject on request ---
        let cost = self.costing.cost(&req);
        let envelope = {
            let mut issuer = self.issuer.lock().unwrap();
            vanilla::request(&mut issuer, cost)
        };
        let value = HeaderValue::from_str(&envelope).map_err(|e| MwError::Middleware(e.into()))?;
        req.headers_mut().insert(HEADER, value);
        // --- send ---
        let resp = next.run(req, extensions).await?;

        // --- extract on response ---
        // On success the envelope rides in the `subbit` header, the body
        // is spoken for by whatever this request is metering. On failure
        // it rides in the body instead (see `subbit-server`'s `respond`),
        // so reading it means consuming and reconstructing the response.
        if resp.status().is_success() {
            if let Some(header_value) = resp.headers().get(HEADER)
                && let Ok(s) = header_value.to_str()
            {
                let mut issuer = self.issuer.lock().unwrap();
                if let Err(e) = vanilla::response(&mut issuer, s) {
                    return Err(MwError::Middleware(anyhow::anyhow!(
                        "issuer failed to process response envelope: {e}"
                    )));
                }
            }
            // FIXME :: handle unexpect no-reply
            return Ok(resp);
        }

        let status = resp.status();
        let url = resp.url().clone();
        let headers = resp.headers().clone();
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| MwError::Middleware(e.into()))?;
        {
            let mut issuer = self.issuer.lock().unwrap();
            if let Err(e) = vanilla::response_bytes(&mut issuer, &bytes) {
                return Err(MwError::Middleware(anyhow::anyhow!(
                    "issuer failed to process response envelope: {e}"
                )));
            }
        }
        let mut builder = http::Response::builder().status(status).url(url);
        *builder.headers_mut().expect("builder has no error yet") = headers;
        let http_resp = builder
            .body(bytes)
            .map_err(|e| MwError::Middleware(e.into()))?;
        Ok(Response::from(http_resp))
    }
}
