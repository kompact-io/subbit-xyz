//! Spawns the real `subbit-index` and `subbit-claim` binaries as child
//! processes -- same technique `provider.rs`'s own `sibling_bin` uses for
//! `subbit-gate`. Only used by the `--mimic` scenario
//! (`orchestrator::Orchestrator::run_mimic_scenario`): the plain preprod
//! path still uses canary's in-process `l1::index_via_floe`/
//! `L1::claim_tick` shortcuts.

use std::path::{Path, PathBuf};
use std::process::Stdio;

use anyhow::Context;
use cardano_sdk::address::kind;
use tokio::process::{Child, Command};

/// Every workspace binary lands in the same `target/<profile>/`
/// directory regardless of which package defines it -- same lookup
/// `provider.rs` already does for `subbit-gate`.
fn sibling_bin(name: &str) -> anyhow::Result<PathBuf> {
    let mut dir = std::env::current_exe().context("resolving current_exe")?;
    dir.pop();
    let bin = dir.join(format!("{name}{}", std::env::consts::EXE_SUFFIX));
    anyhow::ensure!(
        bin.exists(),
        "{name} binary not found at {}; run `cargo build --workspace` first",
        bin.display()
    );
    Ok(bin)
}

fn spawn_run(bin: &Path, config_path: &Path) -> anyhow::Result<Child> {
    Command::new(bin)
        .arg("--config")
        .arg(config_path)
        .arg("run")
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

/// Mirrors `subbit_index::cardano::Config`-wrapping `Config` struct
/// defined inside `packages/index/src/bin/lineage.rs` (a bin-only type,
/// not part of the `subbit_index` lib, so it can't be imported directly
/// -- this reproduces its exact field shape instead). `endpoint` is a
/// plain `String` rather than `url::Url`: both serialize to the same
/// TOML string, and the real binary's own `Url` field deserializes it
/// either way, without canary needing `url` as a direct dependency.
#[derive(serde::Serialize)]
struct LineageIndexConfig {
    cardano: subbit_index::cardano::Config,
    endpoint: String,
    poll_interval_secs: u64,
}

pub struct LineageIndex {
    _child: Child,
    _dir: tempfile::TempDir,
}

impl LineageIndex {
    /// Spawns `lineage-index run` against `floe_endpoint`, posting to
    /// `{provider_base_url}/v1/a/index`. Lives for as long as the
    /// returned value does (`kill_on_drop`).
    pub async fn spawn(floe_endpoint: &str, provider_base_url: &str) -> anyhow::Result<Self> {
        let dir = tempfile::tempdir().context("tempdir")?;
        let config = LineageIndexConfig {
            cardano: subbit_index::cardano::Config {
                endpoint: floe_endpoint.to_string(),
            },
            endpoint: format!("{provider_base_url}/v1/a/index"),
            // Short poll so the scenario doesn't sit idle waiting on it.
            poll_interval_secs: 5,
        };
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, toml::to_string_pretty(&config)?)?;
        let child = spawn_run(&sibling_bin("lineage-index")?, &config_path)?;
        Ok(Self {
            _child: child,
            _dir: dir,
        })
    }
}

pub struct ClaimProcess {
    _child: Child,
    _dir: tempfile::TempDir,
}

impl ClaimProcess {
    /// Spawns `subbit-claim run` as a real floe-only process. `fee_key`
    /// is a *separately* funded wallet (never the main `WALLET_SKEY`
    /// funding wallet `L1` itself drives Open/Close through), so the two
    /// never race to spend the same UTxOs. `ref_script_host` is the main
    /// funding wallet's own address (`L1::change_address`), since that's
    /// where `upload_validator_if_missing` actually put the reference
    /// script -- claim's fee wallet never hosts it.
    pub async fn spawn(
        floe_endpoint: &str,
        provider_base_url: &str,
        network: cardano_sdk::Network,
        fee_key: [u8; 32],
        ref_script_host: cardano_sdk::Address<kind::Shelley>,
        provider_signing_key: [u8; 32],
    ) -> anyhow::Result<Self> {
        let mut keys = std::collections::BTreeMap::new();
        keys.insert(
            "provider".to_string(),
            subbit_claim::keyring::KeyHex::from(provider_signing_key),
        );
        let config = subbit_claim::config::Config {
            floe: subbit_claim::floe::Config {
                endpoint: floe_endpoint.to_string(),
            },
            network,
            fee_key: subbit_claim::keyring::KeyHex::from(fee_key),
            ref_script_host: Some(ref_script_host),
            keyring: subbit_claim::keyring::Config { keys },
            server_endpoint: format!("{provider_base_url}/v1/a/channels").parse()?,
            // Short poll + generous waits -- the scenario, not a
            // deployment economizing on request volume.
            poll_interval_secs: 5,
            min_total: 1,
            min_single: 1,
            panic_settle_ms: 3_600_000,
            step_chunk: 20,
            wait_attempts: 30,
            wait_interval_secs: 10,
        };
        let dir = tempfile::tempdir().context("tempdir")?;
        let config_path = dir.path().join("config.toml");
        std::fs::write(&config_path, toml::to_string_pretty(&config)?)?;
        let child = spawn_run(&sibling_bin("subbit-claim")?, &config_path)?;
        Ok(Self {
            _child: child,
            _dir: dir,
        })
    }
}
