//! Spawns the full echo demo stack (`subbit-gate`, `echo-server`,
//! `echo-proxy`) plus runs the real `echo-client` binary against it.

use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use tokio::process::{Child, Command};

/// Every workspace binary lands in the same `target/<profile>/`
/// directory regardless of which package defines it. This test
/// binary's own `current_exe()` isn't reliably a sibling of that
/// directory, though -- depending on cargo's own version it can land
/// directly there, under `deps/`, or (some newer cargo nightlies, see
/// `rust-toolchain.toml`) nested under a per-unit `build/<pkg>-<hash>/
/// out/` -- so instead locate the nearest `target/<profile>` ancestor
/// in its own path and look there, which every workspace binary
/// (including a plain `[[bin]]` like `subbit-gate`) is guaranteed to
/// also land in directly.
pub fn sibling_bin(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let components: Vec<_> = exe.components().collect();
    let target_idx = components
        .iter()
        .position(|c| c.as_os_str() == "target")
        .expect("current_exe path has no `target` component");
    // `components[..=target_idx + 1]` is `.../target/<profile>`.
    let dir: PathBuf = components
        .get(..=target_idx + 1)
        .expect("current_exe path has no `<profile>` component after `target`")
        .iter()
        .collect();
    let bin = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        bin.exists(),
        "{name} binary not found at {}; run `cargo build --workspace` \
         (or `cargo test --workspace`) before running these tests on their own",
        bin.display()
    );
    bin
}

fn free_local_addr() -> SocketAddr {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
    listener.local_addr().expect("local_addr")
}

async fn wait_listening(addr: SocketAddr) {
    for _ in 0..100 {
        if tokio::net::TcpStream::connect(addr).await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{addr} did not start listening in time");
}

/// Spawns the real `subbit-gate` binary against a temp db and an
/// ephemeral port, torn down on drop (`kill_on_drop`).
pub struct SubbitGate {
    _child: Child,
    addr: SocketAddr,
    _dir: tempfile::TempDir,
}

impl SubbitGate {
    pub async fn spawn(mut config: subbit_gate::Config) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        config.db.db_path = dir.path().join("db.redb").to_string_lossy().into_owned();

        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();

        let addr = free_local_addr();
        let child = Command::new(sibling_bin("subbit-gate"))
            .arg("--config")
            .arg(&config_path)
            .arg("serve")
            .arg("--listen")
            .arg(addr.to_string())
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn subbit-gate");

        wait_ready(addr).await;

        Self {
            _child: child,
            addr,
            _dir: dir,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

/// Polls the gate's admin route until it answers, or panics after ~5s.
async fn wait_ready(addr: SocketAddr) {
    let url = format!("http://{addr}/v1/a/ious");
    let client = reqwest::Client::new();
    for _ in 0..100 {
        if client.get(&url).send().await.is_ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("gate at {addr} did not become ready in time");
}

pub struct EchoServer {
    _child: Child,
    addr: SocketAddr,
}

impl EchoServer {
    pub async fn spawn() -> Self {
        let addr = free_local_addr();
        let child = Command::new(sibling_bin("echo-server"))
            .arg("--addr")
            .arg(addr.ip().to_string())
            .arg("--port")
            .arg(addr.port().to_string())
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn echo-server");
        wait_listening(addr).await;
        Self {
            _child: child,
            addr,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

pub struct EchoProxy {
    _child: Child,
    addr: SocketAddr,
    _dir: tempfile::TempDir,
}

impl EchoProxy {
    pub async fn spawn(upstream: SocketAddr, subbit_gate: SocketAddr) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let addr = free_local_addr();
        let config = subbit_examples_echo_proxy_config(addr, upstream, subbit_gate);
        let config_path = dir.path().join("echo-proxy-config.toml");
        std::fs::write(&config_path, config).expect("write config");

        let child = Command::new(sibling_bin("echo-proxy"))
            .arg("--config")
            .arg(&config_path)
            .arg("run")
            .kill_on_drop(true)
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .spawn()
            .expect("spawn echo-proxy");
        wait_listening(addr).await;
        Self {
            _child: child,
            addr,
            _dir: dir,
        }
    }

    pub fn addr(&self) -> SocketAddr {
        self.addr
    }
}

/// Hand-built rather than round-tripped through `echo_proxy::config::Config`:
/// that type isn't exposed as a library (the proxy crate is a binary-only
/// example), and its shape is small/stable enough to not be worth adding
/// a `lib.rs` just for this.
fn subbit_examples_echo_proxy_config(
    listen: SocketAddr,
    upstream: SocketAddr,
    subbit_gate: SocketAddr,
) -> String {
    format!(
        "listen = \"{listen}\"\nupstream = \"{upstream}\"\n\n[subbit]\ngate = \"{subbit_gate}\"\ntimeout_ms = 2000\n"
    )
}

/// Runs the real `echo-client send` binary once against `config_path`,
/// returning its stdout (the echoed body) on success.
pub async fn echo_client_send(
    config_path: &Path,
    path: &str,
    data: &str,
) -> anyhow::Result<String> {
    let output = Command::new(sibling_bin("echo-client"))
        .arg("--config")
        .arg(config_path)
        .arg("send")
        .arg("-p")
        .arg(path)
        .arg("-d")
        .arg(data)
        .output()
        .await?;
    if !output.status.success() {
        anyhow::bail!(
            "echo-client send -p {path} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    Ok(String::from_utf8(output.stdout)?.trim().to_string())
}
