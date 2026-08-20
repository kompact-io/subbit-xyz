#![allow(unused)]

use std::path::Path;

use anyhow::Context;
use cardano_connector_direct::Blockfrost;
use cardano_sdk::Hash;

use crate::{Config, Session, Tip};

const TIP_CACHE_PATH: &str = "/tmp/cardano-session-tip.json";

pub fn init(path: &Path, force: bool) -> anyhow::Result<()> {
    anyhow::ensure!(
        force || !path.exists(),
        "{} already exists - pass --force to overwrite",
        path.display()
    );
    let contents =
        toml::to_string_pretty(&Config::default()).context("serializing default config")?;
    std::fs::write(path, contents).with_context(|| format!("writing {}", path.display()))?;
    print_json(&serde_json::json!({ "wrote": path.display().to_string() }))
}

pub fn load_config(path: &Path) -> anyhow::Result<Config> {
    let contents = std::fs::read_to_string(path)
        .with_context(|| format!("reading config at {} - run `init` first?", path.display()))?;
    toml::from_str(&contents).with_context(|| format!("parsing config at {}", path.display()))
}

/// `None` if the cache file doesn't exist yet (first run, or it was never
/// written - `save_tip` failing is logged but not fatal).
pub fn load_tip() -> anyhow::Result<Option<Tip>> {
    match std::fs::read_to_string(TIP_CACHE_PATH) {
        Ok(contents) => Ok(Some(
            serde_json::from_str(&contents).with_context(|| format!("parsing {TIP_CACHE_PATH}"))?,
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e).with_context(|| format!("reading {TIP_CACHE_PATH}")),
    }
}

pub fn save_tip(tip: &Tip) -> anyhow::Result<()> {
    let contents = serde_json::to_string_pretty(tip).context("serializing tip")?;
    std::fs::write(TIP_CACHE_PATH, contents).with_context(|| format!("writing {TIP_CACHE_PATH}"))
}

pub fn wallet(session: &Session<Blockfrost>) -> anyhow::Result<()> {
    let fuel: Vec<_> = session.fuel().into_iter().collect();
    let ref_scripts: Vec<_> = session.ref_scripts().collect();
    print_json(&serde_json::json!({
        "verification_key": session.verification_key(),
        "address": session.change_address(),
        "fuel": fuel,
        "ref_scripts": ref_scripts,
    }))
}

pub async fn upload(
    session: &mut Session<Blockfrost>,
    script_path: &Path,
    wait: bool,
) -> anyhow::Result<()> {
    // TODO: decide how a script file on disk deserializes into
    // `cardano_sdk::PlutusScript` - raw CBOR, a JSON envelope (cardano-cli
    // style), a plutus.json import, etc.
    let script: cardano_sdk::PlutusScript = {
        let _ = script_path;
        todo!("parse script file into cardano_sdk::PlutusScript")
    };
    let id = session.upload(script).await?;
    let mut value = serde_json::json!({ "submitted": id });
    if wait {
        session.wait_wallet(&id).await?;
        value["confirmed"] = serde_json::json!(true);
    }
    print_json(&value)
}

pub async fn teardown(
    session: &mut Session<Blockfrost>,
    hash: &str,
    wait: bool,
) -> anyhow::Result<()> {
    let hash = Hash::<28>::try_from(hash)?;
    let id = session.teardown(&hash).await?;
    let mut value = serde_json::json!({ "submitted": id });
    if wait {
        session.wait_wallet(&id).await?;
        value["confirmed"] = serde_json::json!(true);
    }
    print_json(&value)
}

pub async fn wait_id(session: &mut Session<Blockfrost>, id: &str) -> anyhow::Result<()> {
    let id = Hash::<32>::try_from(id)?;
    session.wait_wallet(&id).await?;
    print_json(&serde_json::json!({ "confirmed": true }))
}

pub async fn utxos_at(session: &mut Session<Blockfrost>, address: &str) -> anyhow::Result<()> {
    let address: cardano_sdk::Address<cardano_sdk::address::kind::Shelley> = address
        .parse()
        .with_context(|| format!("parsing address {address:?}"))?;
    // Always a live fetch, not a cache read - querying for a specific
    // address implies wanting current data. Also starts tracking it, so
    // it's picked up by `refresh_all` (and cached to the tip file) from
    // here on.
    session.refresh_at(address.clone()).await?;
    let utxos: Vec<_> = session.utxos_at(&address).into_iter().flatten().collect();
    print_json(&utxos)
}

fn print_json(value: &impl serde::Serialize) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(value)?);
    Ok(())
}
