//! reqwest middleware for vanilla: injects a `subbit` request
//! envelope (costed via `Stamp::resolve_charge`, see `catalog`) on
//! the way out, applies the response envelope on the way back. Gated
//! behind `reqwest` + `vanilla`.

use std::sync::Mutex;

use http::Extensions;
use reqwest::header::HeaderValue;
use reqwest::{Request, Response, ResponseBuilderExt};
use reqwest_middleware::{Error as MwError, Middleware, Next, Result as MwResult};

use crate::Cache;
use crate::catalog::MissPolicy;
use crate::dialect::vanilla::{self, HEADER};
use crate::reqwest::{ChargeOutcome, charge_outcome};

/// `reqwest_middleware::Middleware` that injects a request envelope
/// (via `dialect::vanilla::request`, costed by `stamp`'s own catalog,
/// see `Stamp::resolve_charge`, falling back to `policy: P` on a
/// miss) and applies the response envelope on the way back.
pub struct Stamp<P> {
    stamp: Mutex<crate::Stamp>,
    policy: P,
}

impl<P: MissPolicy> Stamp<P> {
    pub fn new(stamp: crate::Stamp, policy: P) -> Self {
        Self {
            stamp: Mutex::new(stamp),
            policy,
        }
    }

    // On (before) graceful teardown, persist cache.
    pub fn cache(&self) -> Cache {
        self.stamp.lock().unwrap().cache().clone()
    }

    /// Hand back ownership, e.g. to retrieve `stamp.cache()` one last time
    /// after the client is done and dropped.
    pub fn into_stamp(self) -> crate::Stamp {
        self.stamp.into_inner().unwrap()
    }
}

#[async_trait::async_trait]
impl<P: MissPolicy + 'static> Middleware for Stamp<P> {
    async fn handle(
        &self,
        mut req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> MwResult<Response> {
        // --- inject on request ---
        let charge = {
            let mut stamp = self.stamp.lock().unwrap();
            stamp
                .resolve_charge(req.url().path(), &self.policy)
                .map_err(|e| MwError::Middleware(e.into()))?
        };
        let cost = match charge_outcome(charge, req.url().path())
            .map_err(|e| MwError::Middleware(e.into()))?
        {
            // Send as-is, no payment envelope at all: nothing for the
            // server to echo back, so skip straight to returning its
            // reply untouched.
            ChargeOutcome::Unpriced => return next.run(req, extensions).await,
            ChargeOutcome::Priced(cost) => cost,
        };
        let envelope = {
            let mut stamp = self.stamp.lock().unwrap();
            vanilla::request(&mut stamp, cost)
        };
        let value = HeaderValue::from_str(&envelope).map_err(|e| MwError::Middleware(e.into()))?;
        req.headers_mut().insert(HEADER, value);
        // --- send ---
        let resp = next.run(req, extensions).await?;

        // --- extract on response ---
        // On success the envelope rides in the `subbit` header, the body
        // is spoken for by whatever this request is metering. On failure
        // it rides in the body instead (see `subbit-gate`'s `respond`),
        // so reading it means consuming and reconstructing the response.
        if resp.status().is_success() {
            if let Some(header_value) = resp.headers().get(HEADER)
                && let Ok(s) = header_value.to_str()
            {
                let mut stamp = self.stamp.lock().unwrap();
                if let Err(e) = vanilla::response(&mut stamp, s) {
                    return Err(MwError::Middleware(e.into()));
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
            let mut stamp = self.stamp.lock().unwrap();
            if let Err(e) = vanilla::response_bytes(&mut stamp, &bytes) {
                return Err(MwError::Middleware(e.into()));
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
