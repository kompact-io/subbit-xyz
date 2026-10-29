//! Spawns the real `subbit-gate` binary as a child process, the same
//! technique `examples/echo/tests`' `support::sibling_bin` and the old
//! `packages/e2e::harness::Server` both used.

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{Context, bail};
use tokio::process::{Child, Command};

use crate::identity::Identity;

/// Every workspace binary lands in the same `target/<profile>/`
/// directory regardless of which package defines it; locate `name`
/// there relative to this running binary's own `current_exe()`.
fn sibling_bin(name: &str) -> anyhow::Result<PathBuf> {
    let mut dir = std::env::current_exe().context("resolving current_exe")?;
    dir.pop();
    let bin = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    if !bin.exists() {
        bail!(
            "{name} binary not found at {}; run `cargo build --workspace` first",
            bin.display()
        );
    }
    Ok(bin)
}

pub struct Provider {
    _child: Child,
    addr: SocketAddr,
    _dir: tempfile::TempDir,
}

impl Provider {
    pub async fn spawn(mut config: subbit_gate::Config) -> anyhow::Result<Self> {
        let dir = tempfile::tempdir().context("tempdir")?;
        config.db.db_path = dir.path().join("db.redb").to_string_lossy().into_owned();

        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, toml::to_string_pretty(&config)?)?;

        let addr = free_local_addr()?;
        let child = Command::new(sibling_bin("subbit-gate")?)
            .arg("--config")
            .arg(&config_path)
            .arg("serve")
            .arg("--listen")
            .arg(addr.to_string())
            // Default to `warn`, not the server's own `info` default:
            // an `info`-level line per request (two per spend) writes
            // synchronously to this inherited stderr on every request,
            // which is itself a real, measured source of per-request
            // latency under load, independent of anything server- or
            // client-logic-related. Still overridable by the
            // environment this canary process itself runs under.
            .env(
                "RUST_LOG",
                std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()),
            )
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .context("spawn subbit-gate")?;

        wait_ready(addr).await?;

        Ok(Self {
            _child: child,
            addr,
            _dir: dir,
        })
    }

    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    /// Spawns a real `subbit-gate` configured for this run's own demo
    /// `Terms`: `flat_cost` for every request, closing after
    /// `close_period`, validated against the real `subbit-tx` validator
    /// script. Consumers fetch the published catalog themselves (see
    /// `Consumer::connect`) rather than being handed one by this
    /// constructor.
    pub async fn spawn_demo(
        flat_cost: u64,
        provider_identity: Identity,
        close_period: subbit_core::Duration,
        finality_depth: u64,
    ) -> anyhow::Result<Self> {
        let mut config = subbit_gate::Config::default();
        config.terms.provider = provider_identity.hash28;
        config.terms.asset = subbit_l2_dto::Asset::Lovelace;
        config.terms.close_period = close_period;
        config.terms.script =
            subbit_core::Hash28::from(<[u8; 28]>::from(subbit_tx::VALIDATOR.hash));
        config.terms.catalog = demo_catalog(flat_cost);
        config.finality_depth = finality_depth;
        println!("canary: spawning provider");
        let provider = Self::spawn(config).await?;
        println!("canary: provider up at {}", provider.base_url());
        Ok(provider)
    }
}

/// This run's own demo cost catalog: a flat per-request cost for its one
/// resource, published at exactly the path consumers then spend against
/// (`/v1/spend/echo`) -- no separate hardcoded spend path for a consumer
/// to keep in sync with by hand.
pub(crate) fn demo_catalog(flat_cost: u64) -> subbit_l2_dto::Catalog {
    subbit_l2_dto::Catalog {
        default: subbit_l2_dto::ResourceInfo {
            cost: subbit_l2_dto::Cost::ByRequest(flat_cost),
            description: "Canary smoke-test resource".to_string(),
            mime_type: "text/plain".to_string(),
            max_timeout_seconds: 60,
            service_name: None,
            tags: None,
            icon_url: None,
        },
        resources: vec![(
            "/v1/spend/echo".to_string(),
            subbit_l2_dto::ResourceInfoOverride::default(),
        )],
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
