//! Dumb http client of server. Knows nothing about what a header is.
use std::{net::SocketAddr, time::Duration};

// TODO : upstream
pub static HEADER: &str = "subbit";

pub enum Outcome {
    Ok(Option<String>),
    Ko { status: u16, header: Option<String> },
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

    pub async fn spend(&self, path: &str, subbit_header: &str) -> Outcome {
        self.call("spend", path, subbit_header).await
    }

    pub async fn refund(&self, path: &str, subbit_header: &str) -> Outcome {
        self.call("refund", path, subbit_header).await
    }

    async fn call(&self, action: &str, path: &str, subbit_header: &str) -> Outcome {
        let path = path.strip_prefix('/').unwrap_or(path);
        let url = format!("http://{}/v1/x/{action}/{path}", self.addr);
        match self
            .http
            .get(&url)
            .header(HEADER, subbit_header)
            .send()
            .await
        {
            Ok(resp) => Self::interpret(resp),
            Err(_err) => Outcome::Ko {
                status: 502,
                header: None,
            },
        }
    }

    // 2xx -> Ok(header); 4xx -> Ko with that status + header; anything else -> Ko 502, header dropped (out-of-spec response isn't trusted).
    fn interpret(resp: reqwest::Response) -> Outcome {
        let status = resp.status();
        if !status.is_success() && !status.is_client_error() {
            return Outcome::Ko {
                status: 502,
                header: None,
            };
        }
        let header = resp
            .headers()
            .get(HEADER)
            .and_then(|v| v.to_str().ok())
            .map(str::to_string);
        if status.is_client_error() {
            return Outcome::Ko {
                status: status.as_u16(),
                header,
            };
        }
        Outcome::Ok(header)
    }
}
