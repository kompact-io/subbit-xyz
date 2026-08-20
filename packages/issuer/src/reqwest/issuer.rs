//! reqwest middleware for `Issuer`: injects a request envelope (costed via
//! `Cost`) on the way out, applies the response envelope on the way back.
//! Gated behind the `reqwest` feature.

use std::sync::Mutex;

use http::Extensions;
use reqwest::header::HeaderValue;
use reqwest::{Request, Response};
use reqwest_middleware::{Error as MwError, Middleware, Next, Result as MwResult};

use crate::{Cache, HEADER};

/// `reqwest_middleware::Middleware` that injects a request envelope
/// (via `Issuer::request`, costed by `C: Cost`) and applies the
/// response envelope on the way back.
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
            issuer.request(cost)
        };
        let value = HeaderValue::from_str(&envelope).map_err(|e| MwError::Middleware(e.into()))?;
        req.headers_mut().insert(HEADER, value);
        // --- send ---
        let resp = next.run(req, extensions).await?;

        // --- extract on response ---
        if let Some(header_value) = resp.headers().get(HEADER)
            && let Ok(s) = header_value.to_str()
        {
            let mut issuer = self.issuer.lock().unwrap();
            if let Err(e) = issuer.response(s) {
                return Err(MwError::Middleware(anyhow::anyhow!(
                    "issuer failed to process response envelope: {e}"
                )));
            }
        }
        // FIXME :: handle unexpect no-reply

        Ok(resp)
    }
}
