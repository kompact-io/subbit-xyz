//! Spawns the `subbit-gate` binary.

use std::net::SocketAddr;
use std::time::Duration;

use anyhow::bail;
use tokio::process::Child;

use crate::process::spawn_with_config;

pub(crate) struct Process {
    _child: Child,
    _dir: tempfile::TempDir,
    addr: SocketAddr,
}

impl Process {
    pub(crate) async fn spawn(mut config: subbit_gate::Config) -> anyhow::Result<Self> {
        let addr = free_local_addr()?;
        let addr_arg = addr.to_string();
        // Created ahead of spawn_with_config so db_path can point inside it too.
        let dir = tempfile::tempdir()?;
        config.db.db_path = dir.path().join("db.redb").to_string_lossy().into_owned();
        let child = spawn_with_config(
            &dir,
            "subbit-gate",
            &config,
            &["serve", "--listen", &addr_arg],
        )?;

        wait_ready(addr).await?;

        Ok(Self {
            _child: child,
            _dir: dir,
            addr,
        })
    }

    pub(crate) fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }
}

fn free_local_addr() -> anyhow::Result<SocketAddr> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    Ok(listener.local_addr()?)
}

async fn wait_ready(addr: SocketAddr) -> anyhow::Result<()> {
    let url = format!("http://{addr}/v1/a/ious");
    let client = reqwest::Client::new();
    for _ in 0..100 {
        if client.get(&url).send().await.is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    bail!("server at {addr} did not become ready in time")
}
