//! Dumb http client of server. Knows nothing about what a header is:
//! callers decide which headers go out and which of the reply's
//! headers matter, this just relays bytes both ways.
use http::HeaderMap;
use std::{net::SocketAddr, time::Duration};

pub enum Outcome {
    /// The full response header map, unfiltered: the caller picks out
    /// whichever headers it actually cares about.
    Ok(HeaderMap),
    /// `headers`: as `Ok`'s, unfiltered - some dialects carry a real
    /// corrective/challenge shape here even on failure (x402's
    /// `PAYMENT-REQUIRED`), not just in `body` (vanilla's own CBOR
    /// error envelope).
    Ko {
        status: u16,
        headers: HeaderMap,
        body: Option<Vec<u8>>,
    },
}

#[derive(Clone)]
pub struct Client {
    addr: SocketAddr,
    http: reqwest::Client,
}

impl Client {
    pub fn new(addr: SocketAddr, timeout: Duration) -> reqwest::Result<Self> {
        Ok(Self {
            addr,
            http: reqwest::Client::builder().timeout(timeout).build()?,
        })
    }

    pub async fn spend(&self, path: &str, headers: &HeaderMap) -> Outcome {
        self.call("spend", path, headers).await
    }

    pub async fn refund(&self, path: &str, headers: &HeaderMap) -> Outcome {
        self.call("refund", path, headers).await
    }

    /// Status + raw body of a plain, headerless `GET /v1/terms`: the
    /// caller relays the body verbatim, so this doesn't fit `Outcome`.
    pub async fn terms(&self) -> Result<(u16, Vec<u8>), ()> {
        let url = format!("http://{}/v1/terms", self.addr);
        let resp = self.http.get(&url).send().await.map_err(|_| ())?;
        let status = resp.status().as_u16();
        let body = resp.bytes().await.map_err(|_| ())?.to_vec();
        Ok((status, body))
    }

    async fn call(&self, action: &str, path: &str, headers: &HeaderMap) -> Outcome {
        let path = path.strip_prefix('/').unwrap_or(path);
        let url = format!("http://{}/v1/{action}/{path}", self.addr);
        let mut req = self.http.get(&url);
        for (name, value) in headers {
            req = req.header(name, value);
        }
        match req.send().await {
            Ok(resp) => Self::interpret(resp).await,
            Err(_err) => Outcome::Ko {
                status: 502,
                headers: HeaderMap::new(),
                body: None,
            },
        }
    }

    // 2xx -> Ok(headers); 4xx -> Ko with that status + headers + body; anything else -> Ko 502, headers/body dropped (out-of-spec response isn't trusted).
    async fn interpret(resp: reqwest::Response) -> Outcome {
        let status = resp.status();
        if !status.is_success() && !status.is_client_error() {
            return Outcome::Ko {
                status: 502,
                headers: HeaderMap::new(),
                body: None,
            };
        }
        if status.is_client_error() {
            let headers = resp.headers().clone();
            let body = resp.bytes().await.ok().map(|b| b.to_vec());
            return Outcome::Ko {
                status: status.as_u16(),
                headers,
                body,
            };
        }
        Outcome::Ok(resp.headers().clone())
    }
}
