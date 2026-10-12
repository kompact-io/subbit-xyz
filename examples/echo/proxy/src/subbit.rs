use std::net::SocketAddr;

use http::{HeaderMap, HeaderName};
use pingora::prelude::*;
use serde::{Deserialize, Serialize};
pub use subbit_client::{Client, Outcome};

#[derive(Serialize, Deserialize)]
pub struct Config {
    pub server: SocketAddr,
    pub timeout_ms: u64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            server: "0.0.0.0:7822".parse().unwrap(),
            timeout_ms: 200,
        }
    }
}

impl Config {
    pub fn into_client(self) -> Client {
        Client::new(
            self.server,
            std::time::Duration::from_millis(self.timeout_ms),
        )
        .expect("bad subbit client config")
    }
}

/// Either dialect's own request-side auth header: vanilla's `subbit`,
/// x402's `PAYMENT-SIGNATURE`. Whichever the consumer actually sent is
/// the one that matters; this doesn't decode or inspect either.
const REQUEST_HEADER_NAMES: &[&str] = &["subbit", "PAYMENT-SIGNATURE"];

/// Either dialect's own response-side header(s): vanilla's `subbit`,
/// x402's `PAYMENT-REQUIRED`/`PAYMENT-RESPONSE`.
const RESPONSE_HEADER_NAMES: &[&str] = &["subbit", "PAYMENT-REQUIRED", "PAYMENT-RESPONSE"];

fn select(headers: &HeaderMap, names: &[&str]) -> HeaderMap {
    let mut out = HeaderMap::new();
    for name in names {
        if let Some(value) = headers.get(*name) {
            out.insert(
                name.parse::<HeaderName>().expect("valid header name"),
                value.clone(),
            );
        }
    }
    out
}

/// The consumer's own dialect auth header(s), pulled off an incoming
/// request: forwarded to `subbit-server` for a spend, and kept around
/// to reuse unchanged for that same request's eventual refund.
pub fn auth_headers(headers: &HeaderMap) -> HeaderMap {
    select(headers, REQUEST_HEADER_NAMES)
}

/// Whichever of `subbit-server`'s own reply headers are worth relaying
/// back onto the real response to the consumer.
pub fn relay_headers(headers: &HeaderMap) -> HeaderMap {
    select(headers, RESPONSE_HEADER_NAMES)
}

/// `subbit-server` answers `Terms` at both `/v1/terms` and the
/// conventional `/.well-known/subbit`; the proxy relays whichever one
/// a consumer hits, verbatim, no metering check.
pub fn is_terms_path(path: &str) -> bool {
    path == "/v1/terms" || path == "/.well-known/subbit"
}

pub async fn terms(client: &Client) -> Result<(u16, Vec<u8>), ()> {
    client.terms().await
}

/// Reads the consumer's own auth header(s) + path off `session` and
/// asks `subbit-server` to spend. Hands the auth headers back too,
/// alongside the outcome, so the caller can reuse them for the
/// eventual refund without re-reading `session`.
pub async fn check(client: &Client, session: &Session) -> (HeaderMap, Outcome) {
    let path = session.req_header().uri.path();
    let headers = auth_headers(&session.req_header().headers);
    let outcome = client.spend(path, &headers).await;
    (headers, outcome)
}
