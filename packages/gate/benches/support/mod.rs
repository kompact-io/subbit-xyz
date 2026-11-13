//! Shared orchestration for benches that drive the real `subbit-gate`
//! binary as a separate OS process over real HTTP (`firehose.rs`,
//! `stamp_overhead.rs`) rather than in-process (`db_throughput.rs`,
//! `l2_throughput.rs`, `worker_threads.rs` don't need this module).
//! Pulled out once both benches had copy-pasted the same `Server`
//! verbatim and started drifting out of sync with each other.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    process::Stdio,
    time::Duration,
};

use subbit_gate::Config;
use tokio::process::{Child, Command};

/// Locates a workspace binary by walking up from this process's own
/// `current_exe()` to the nearest `target` path component, then
/// looking directly under `target/<profile>/`. Every workspace binary
/// lands there regardless of which package defines it -- but *this*
/// process's own path isn't reliably a fixed number of parents above
/// that: a plain `[[bin]]` lands directly in `target/<profile>/`, a
/// `[[bench]]`/`[[test]]` usually lands under `.../deps/`, and on some
/// cargo versions a package's own bench/test units land nested under
/// `.../build/<package>/<hash>/out/` (collides with this very crate's
/// name, `subbit-gate`, since it also has a `build.rs`) -- so anchor
/// on the one path component that's always present instead of
/// counting parent hops. Same technique as `examples/echo/tests`'
/// `support::sibling_bin`.
pub fn sibling_bin(name: &str) -> PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let components: Vec<_> = exe.components().collect();
    let target_idx = components
        .iter()
        .position(|c| c.as_os_str() == "target")
        .expect("current_exe path has no `target` component");
    let dir: PathBuf = components
        .get(..=target_idx + 1)
        .expect("current_exe path has no `<profile>` component after `target`")
        .iter()
        .collect();
    let bin = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    assert!(
        bin.exists(),
        "{name} binary not found at {}; run `cargo build --workspace` first",
        bin.display()
    );
    bin
}

/// Spawns `name`. Falls back to spawning a copy placed in
/// `scratch_dir` (the bench's own tempdir, already proven traversable
/// since the caller just wrote the config file there) when the
/// sibling path itself fails with `PermissionDenied` -- e.g. a build
/// tree sitting under a locked-down ancestor directory on a shared
/// host, where the binary's own `+x` bit is irrelevant because the OS
/// never gets far enough to check it.
fn spawn_sibling(name: &str, scratch_dir: &Path, config_path: &Path, addr: &SocketAddr) -> Child {
    let bin = sibling_bin(name);
    // Diagnostic escape hatch, not the default path: set
    // `FIREHOSE_GATE_LOG=1` to inherit the spawned server's stdout (its
    // `tracing` output, normally discarded) for ad hoc profiling runs --
    // see `results.md`'s profiling section. Off by default so normal
    // bench runs stay exactly as before.
    let inherit_log = std::env::var("FIREHOSE_GATE_LOG").is_ok_and(|v| v == "1");
    let command = |bin: &Path| {
        let mut cmd = Command::new(bin);
        cmd.arg("--config")
            .arg(config_path)
            .arg("serve")
            .arg("--listen")
            .arg(addr.to_string())
            .kill_on_drop(true)
            .stdout(Stdio::null());
        if inherit_log {
            // `main.rs::init_tracing` writes to stderr, not stdout.
            cmd.env("RUST_LOG", "subbit_gate::db=info,spend_diag=info")
                .stderr(Stdio::inherit());
        } else {
            cmd.stderr(Stdio::null());
        }
        cmd.spawn()
    };
    match command(&bin) {
        Ok(child) => child,
        Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
            let fallback = scratch_dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
            std::fs::copy(&bin, &fallback).expect("copy binary into scratch dir");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&fallback, std::fs::Permissions::from_mode(0o755))
                    .expect("chmod binary copy");
            }
            command(&fallback).expect("spawn binary (scratch-dir copy)")
        }
        Err(e) => panic!("spawn {name}: {e}"),
    }
}

/// A real `subbit-gate serve` process, listening on a real loopback
/// port, backed by a fresh on-disk db in its own tempdir.
pub struct Server {
    _child: Child,
    _dir: tempfile::TempDir,
    addr: SocketAddr,
}

impl Server {
    pub fn base_url(&self) -> String {
        format!("http://{}", self.addr)
    }

    pub async fn spawn(mut config: Config) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        config.db.db_path = dir.path().join("db.redb").to_string_lossy().into_owned();
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, toml::to_string_pretty(&config).unwrap()).unwrap();

        let addr: SocketAddr = {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.local_addr().unwrap()
        };
        let child = spawn_sibling("subbit-gate", dir.path(), &config_path, &addr);

        let client = reqwest::Client::new();
        let ready_url = format!("http://{addr}/v1/a/ious");
        for _ in 0..200 {
            if client.get(&ready_url).send().await.is_ok() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        Self {
            _child: child,
            _dir: dir,
            addr,
        }
    }
}
