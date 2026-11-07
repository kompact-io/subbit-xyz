//! The two generic sibling-binary spawns every real process (gate, lineage-index, subbit-claim)
//! goes through -- see `provider::Provider`.

use std::process::Stdio;

use anyhow::Context;
use tokio::process::{Child, Command};

pub(crate) fn spawn_process(name: &str, args: &[&str]) -> anyhow::Result<Child> {
    let mut dir = std::env::current_exe().context("resolving current_exe")?;
    dir.pop();
    let bin = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    anyhow::ensure!(
        bin.exists(),
        "{name} binary not found at {}; run `cargo build --workspace` first",
        bin.display()
    );
    Command::new(&bin)
        .args(args)
        .env(
            "RUST_LOG",
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()),
        )
        .kill_on_drop(true)
        .stdout(Stdio::null())
        .stderr(Stdio::inherit())
        .spawn()
        .with_context(|| format!("spawn {}", bin.display()))
}

/// Writes `config` to `dir`'s own `config.toml`, then `spawn_process`es `name` with
/// `--config <path> <extra_args>`. `dir` is the caller's own (it must outlive the child, so
/// the caller holds it anyway).
pub(crate) fn spawn_with_config(
    dir: &tempfile::TempDir,
    name: &str,
    config: &impl serde::Serialize,
    extra_args: &[&str],
) -> anyhow::Result<Child> {
    let config_path = dir.path().join("config.toml");
    std::fs::write(&config_path, toml::to_string_pretty(config)?)?;
    let config_path = config_path.to_string_lossy();

    let mut args = vec!["--config", &config_path];
    args.extend_from_slice(extra_args);
    spawn_process(name, &args)
}
