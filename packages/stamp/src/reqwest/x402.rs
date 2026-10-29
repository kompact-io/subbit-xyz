//! reqwest middleware for x402, v2: injects a `PAYMENT-SIGNATURE`
//! request header (costed via `Cost`) on the way out, once terms have
//! been learned from a prior `PAYMENT-REQUIRED`; applies whichever of
//! `PAYMENT-RESPONSE`/`PAYMENT-REQUIRED` the reply carries on the way
//! back. Gated behind `reqwest` + `x402`.

use std::sync::Mutex;

use http::Extensions;
use reqwest::header::HeaderValue;
use reqwest::{Request, Response, ResponseBuilderExt};
use reqwest_middleware::{Error as MwError, Middleware, Next, Result as MwResult};
use subbit_l2_dto::dialect::x402::SubbitAccept;

use crate::Cache;
use crate::dialect::x402::{
    self, PAYMENT_REQUIRED_HEADER, PAYMENT_RESPONSE_HEADER, PAYMENT_SIGNATURE_HEADER,
};

/// `reqwest_middleware::Middleware` that injects a request envelope
/// (via `dialect::x402::request`, costed by `C: Cost`) once
/// `accepted` has been learned, and applies the response header on
/// the way back. The very first request against a fresh `Stamp` (or
/// after terms change) carries no payment at all: there's nothing to
/// pay under yet, so it goes out bare, and the resulting
/// `PAYMENT-REQUIRED` challenge is what teaches `accepted` for the
/// caller's own retry.
pub struct Stamp<C> {
    stamp: Mutex<crate::Stamp>,
    accepted: Mutex<Option<SubbitAccept>>,
    costing: C,
}

impl<C: super::Cost> Stamp<C> {
    pub fn new(stamp: crate::Stamp, costing: C) -> Self {
        Self {
            stamp: Mutex::new(stamp),
            accepted: Mutex::new(None),
            costing,
        }
    }

    pub fn cache(&self) -> Cache {
        self.stamp.lock().unwrap().cache().clone()
    }

    pub fn into_stamp(self) -> crate::Stamp {
        self.stamp.into_inner().unwrap()
    }
}

#[async_trait::async_trait]
impl<C: super::Cost + 'static> Middleware for Stamp<C> {
    async fn handle(
        &self,
        mut req: Request,
        extensions: &mut Extensions,
        next: Next<'_>,
    ) -> MwResult<Response> {
        // --- inject on request, only once terms are known ---
        let accepted = self.accepted.lock().unwrap().clone();
        if let Some(accepted) = accepted {
            let cost = self.costing.cost(&req);
            let envelope = {
                let mut stamp = self.stamp.lock().unwrap();
                x402::request(&mut stamp, cost, accepted)
            };
            let value =
                HeaderValue::from_str(&envelope).map_err(|e| MwError::Middleware(e.into()))?;
            req.headers_mut().insert(PAYMENT_SIGNATURE_HEADER, value);
        }
        // --- send ---
        let resp = next.run(req, extensions).await?;

        // --- extract on response ---
        // Both x402 headers are read before the body is consumed:
        // nothing x402-specific rides in the body, see
        // `transports-v2/http.md`'s own "Response Body".
        let status = resp.status();
        let url = resp.url().clone();
        let headers = resp.headers().clone();
        let settlement = header_bytes(&headers, PAYMENT_RESPONSE_HEADER);
        let required = header_bytes(&headers, PAYMENT_REQUIRED_HEADER);
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| MwError::Middleware(e.into()))?;
        {
            let mut stamp = self.stamp.lock().unwrap();
            let mut accepted = self.accepted.lock().unwrap();
            if let Err(e) = x402::response(
                &mut stamp,
                &mut accepted,
                settlement.as_deref(),
                required.as_deref(),
            ) {
                return Err(MwError::Middleware(anyhow::anyhow!(
                    "stamp failed to process response header: {e}"
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

fn header_bytes(headers: &reqwest::header::HeaderMap, name: &str) -> Option<Vec<u8>> {
    headers
        .get(name)?
        .to_str()
        .ok()
        .map(|s| s.as_bytes().to_vec())
}
