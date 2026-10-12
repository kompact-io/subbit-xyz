//! Spawns a real `subbit-server` binary against a temp db and an
//! ephemeral port, and tears it down on drop. Callers resolve the
//! binary path themselves via `env!("CARGO_BIN_EXE_subbit-server")` —
//! that macro only works from a test/bench/example target, not from
//! this library, so it can't be baked in here.

use std::net::SocketAddr;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};

pub struct Server {
    child: Child,
    pub addr: SocketAddr,
    _dir: tempfile::TempDir,
}

impl Server {
    /// Spawn `bin` (the `subbit-server` binary) with `config`, after
    /// pointing its db at a fresh temp dir. Blocks until the server
    /// answers its admin route, or returns an error after ~5s.
    pub async fn spawn(bin: &Path, mut config: subbit_server::Config) -> anyhow::Result<Self> {
        let dir = tempfile::tempdir()?;
        config.db.db_path = dir.path().join("db.redb").to_string_lossy().into_owned();

        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, toml::to_string_pretty(&config)?)?;

        let addr = free_local_addr()?;
        let child = Command::new(bin)
            .arg("--config")
            .arg(&config_path)
            .arg("serve")
            .arg("--listen")
            .arg(addr.to_string())
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()?;

        wait_ready(addr).await?;

        Ok(Self {
            child,
            addr,
            _dir: dir,
        })
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Kill the server and wait for it to exit. Not required for
    /// cleanup (`kill_on_drop` already handles that), only for tests
    /// that want to assert the process actually stops.
    pub async fn shutdown(mut self) -> anyhow::Result<()> {
        self.child.kill().await?;
        self.child.wait().await?;
        Ok(())
    }
}

fn free_local_addr() -> anyhow::Result<SocketAddr> {
    // Bind to an OS-assigned port, then drop the listener so the
    // child can bind it. A small TOCTOU window, acceptable for tests.
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
    anyhow::bail!("server at {addr} did not become ready in time")
}
