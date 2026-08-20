use std::net::SocketAddr;

use pingora::prelude::*;
use serde::{Deserialize, Serialize};
pub use subbit_client::{Client, HEADER, Outcome};

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

/// Reads the `subbit` header + path off `session` and asks subbit-server.
pub async fn check(client: &Client, session: &Session) -> Outcome {
    let path = session.req_header().uri.path();
    let header = session
        .req_header()
        .headers
        .get(HEADER)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    client.spend(path, header).await
}
