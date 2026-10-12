//! Drives vanilla's dialect over real HTTP: encodes a `Request` into
//! the `subbit` header the same way a real client would
//! (`subbit_core::base64::to_base64`), using `subbit-client`'s own
//! "dumb" HTTP client for the request, then decodes the envelope back
//! out (`subbit_core::base64::from_base64` on success, a plain
//! `minicbor::decode` of the error body on failure — mirroring
//! `subbit-server`'s own `respond()`, see `packages/server/src/main.rs`).

use std::net::SocketAddr;
use std::time::Duration;

use http::{HeaderMap, HeaderValue};
use subbit_client::{Client, Outcome};
use subbit_core::base64::{from_base64, to_base64};
use subbit_l2_dto::Request;

pub type Response = subbit_l2_dto::dialect::vanilla::Response;

const HEADER: &str = "subbit";

pub struct VanillaClient {
    client: Client,
}

impl VanillaClient {
    pub fn new(addr: SocketAddr) -> anyhow::Result<Self> {
        Ok(Self {
            client: Client::new(addr, Duration::from_secs(5))?,
        })
    }

    pub async fn spend(&self, path: &str, payload: &Request) -> anyhow::Result<Response> {
        decode(self.client.spend(path, &envelope_header(payload)).await)
    }

    pub async fn refund(&self, path: &str, payload: &Request) -> anyhow::Result<Response> {
        decode(self.client.refund(path, &envelope_header(payload)).await)
    }
}

fn envelope_header(payload: &Request) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(HEADER, HeaderValue::from_str(&to_base64(payload)).unwrap());
    headers
}

fn decode(outcome: Outcome) -> anyhow::Result<Response> {
    match outcome {
        Outcome::Ok(headers) => {
            let raw = headers
                .get(HEADER)
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| anyhow::anyhow!("2xx response carried no subbit header"))?;
            from_base64::<Response>(raw).map_err(|e| anyhow::anyhow!("decoding subbit header: {e}"))
        }
        Outcome::Ko {
            status,
            body: Some(bytes),
            ..
        } => minicbor::decode::<Response>(&bytes)
            .map_err(|e| anyhow::anyhow!("decoding error body (status {status}): {e}")),
        Outcome::Ko {
            status, body: None, ..
        } => anyhow::bail!("status {status} with no body"),
    }
}
