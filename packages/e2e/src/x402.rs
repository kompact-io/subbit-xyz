//! Drives x402 v2 over real HTTP: `PAYMENT-SIGNATURE` out, base64 JSON
//! `Request`; `PAYMENT-REQUIRED`/`PAYMENT-RESPONSE` in, base64 JSON
//! `Require`/`Response`. Which header a reply carries depends on the
//! outcome, not the status code alone, so [`X402Client::pay`] returns
//! whichever one was actually there.

use base64::Engine;
use reqwest::StatusCode;
use subbit_l2_dto::dialect::x402::{Payload, Request, Require, Response, SubbitAccept};

const PAYMENT_SIGNATURE_HEADER: &str = "PAYMENT-SIGNATURE";
const PAYMENT_REQUIRED_HEADER: &str = "PAYMENT-REQUIRED";
const PAYMENT_RESPONSE_HEADER: &str = "PAYMENT-RESPONSE";

pub struct X402Client {
    http: reqwest::Client,
    base_url: String,
}

/// Whichever header a reply actually carried.
pub enum Reply {
    Required(Require),
    Settlement(Response),
}

impl X402Client {
    pub fn new(base_url: impl Into<String>) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url: base_url.into(),
        }
    }

    fn url(&self, path: &str) -> String {
        let path = path.strip_prefix('/').unwrap_or(path);
        format!("{}/v1/spend/{path}", self.base_url)
    }

    /// No `PAYMENT-SIGNATURE` header: the initial challenge.
    pub async fn challenge(&self, path: &str) -> anyhow::Result<(StatusCode, Require)> {
        let resp = self.http.get(self.url(path)).send().await?;
        let status = resp.status();
        let required = decode_header(resp.headers(), PAYMENT_REQUIRED_HEADER)?;
        Ok((status, required))
    }

    /// Submit `payload` under `accepted` (the terms a prior
    /// `challenge`/`pay` handed back).
    pub async fn pay(
        &self,
        path: &str,
        accepted: SubbitAccept,
        payload: Payload,
    ) -> anyhow::Result<(StatusCode, Reply)> {
        let envelope = Request {
            x402_version: 2,
            resource: None,
            accepted,
            payload,
            extensions: None,
        };
        let bytes = serde_json::to_vec(&envelope)?;
        let header = base64::engine::general_purpose::STANDARD.encode(bytes);
        let resp = self
            .http
            .get(self.url(path))
            .header(PAYMENT_SIGNATURE_HEADER, header)
            .send()
            .await?;
        let status = resp.status();
        if let Ok(settlement) = decode_header(resp.headers(), PAYMENT_RESPONSE_HEADER) {
            return Ok((status, Reply::Settlement(settlement)));
        }
        let required = decode_header(resp.headers(), PAYMENT_REQUIRED_HEADER)?;
        Ok((status, Reply::Required(required)))
    }
}

fn decode_header<T: serde::de::DeserializeOwned>(
    headers: &reqwest::header::HeaderMap,
    name: &str,
) -> anyhow::Result<T> {
    let raw = headers
        .get(name)
        .ok_or_else(|| anyhow::anyhow!("missing {name} header"))?
        .to_str()?;
    let bytes = base64::engine::general_purpose::STANDARD.decode(raw)?;
    Ok(serde_json::from_slice(&bytes)?)
}
