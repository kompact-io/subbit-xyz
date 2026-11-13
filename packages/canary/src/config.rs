//! Canary's own config, loaded through `subbit-config`.

use std::path::Path;

use anyhow::Context;
use cardano_sdk::{Address, address::kind};
use serde::{Deserialize, Serialize};
use subbit_core::Duration;

use crate::identity::Identity;
use crate::provider;

/// Field names intentionally mirror the existing unprefixed env vars
/// (`L2_SEED_HEX`, `WALLET_SKEY`) so the default empty env prefix keeps
/// reading them directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Hex-encoded 32-byte root seed.
    pub l2_seed_hex: String,
    /// Hex-encoded 32-byte signing key of an already-funded preprod wallet.
    pub wallet_skey: String,
    /// Floe gRPC endpoint.
    pub floe_endpoint: String,
    /// Base unit cost for the demo catalog.
    pub unit_cost: u64,
    /// Block depth the spawned server requires before treating a lineage item as backing.
    pub finality_depth: u64,
    /// Provider close period.
    pub close_period_secs: u64,
    /// One roster line, `<name>[:<modifier>]=<count>` -- e.g. `alice=5`, `richy:x402=3`.
    pub personas: Vec<String>,
    /// Fixed lovelace target each `:burst` persona commits to.
    pub burst_target_lovelace: Option<u64>,
    /// Resume an interrupted run from this round instead of round 0.
    pub resume: u8,
    /// Hard cap on active rounds before every consumer's agent is killed.
    pub max_rounds: u8,
    /// Upper bound on the slow-poll phase before forcing a round.
    pub round_poll_upper_bound_secs: u64,
    /// Seconds to wait past the earliest legal `Elapse` before the consumer asks for it.
    pub elapse_margin_secs: u64,
    /// Fail the run if any consumer ever stages `Elapse`.
    pub require_claim_win: bool,
    /// Upper bound on consumer *index* to scan for `--status`/`--tidy`.
    pub scan_consumers: usize,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            l2_seed_hex: "00".repeat(32),
            wallet_skey: "00".repeat(32),
            floe_endpoint: "https://grpc.floe.kompact.io".to_string(),
            unit_cost: 100,
            finality_depth: 1,
            close_period_secs: 60,
            personas: Vec::new(),
            burst_target_lovelace: None,
            resume: 0,
            max_rounds: 60,
            round_poll_upper_bound_secs: 5,
            elapse_margin_secs: 120,
            require_claim_win: false,
            scan_consumers: 256,
        }
    }
}

impl Config {
    pub fn load(sources: &subbit_config::Sources<'_>) -> anyhow::Result<Self> {
        sources.load().context("loading canary config")
    }

    pub fn write_default(path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("creating {}", parent.display()))?;
        }
        let text = toml::to_string_pretty(&Self::default())?;
        std::fs::write(path, text).with_context(|| format!("writing {}", path.display()))
    }

    pub fn l2_seed(&self) -> anyhow::Result<[u8; 32]> {
        decode_32(&self.l2_seed_hex, "L2_SEED_HEX")
    }

    pub fn wallet_skey(&self) -> anyhow::Result<[u8; 32]> {
        decode_32(&self.wallet_skey, "WALLET_SKEY")
    }

    /// Builds the provider's own config from this run's settings and identities.
    pub fn provider_config(
        &self,
        provider_identity: Identity,
        claim_wallet: Identity,
        ref_script_host: Address<kind::Shelley>,
    ) -> provider::Config {
        provider::Config {
            provider_identity,
            claim_wallet,
            ref_script_host,
            unit_cost: self.unit_cost,
            close_period: Duration::from_secs(self.close_period_secs),
            finality_depth: self.finality_depth,
            floe_endpoint: self.floe_endpoint.clone(),
        }
    }
}

fn decode_32(value: &str, name: &str) -> anyhow::Result<[u8; 32]> {
    hex::decode(value.trim())
        .with_context(|| format!("{name} is not valid hex"))?
        .try_into()
        .map_err(|_| anyhow::anyhow!("{name} must decode to 32 bytes"))
}
