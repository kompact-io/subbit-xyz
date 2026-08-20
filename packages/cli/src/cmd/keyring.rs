use anyhow::Result;
use cardano_sdk::{Hash, SigningKey, VerificationKey};
use clap::Subcommand;
use serde::Serialize;

use crate::ctx::Ctx;

pub fn hash(bytes: &[u8]) -> [u8; 32] {
    Hash::<32>::new(bytes).into()
}

/// A signing key together with its derived verification key, vk hash, and label.
#[derive(Debug, Clone, Serialize)]
pub struct Info {
    pub label: String,
    #[serde(with = "hex::serde")]
    pub sk: [u8; 32],
    pub vk: VerificationKey,
    pub vkh: Hash<28>,
}

impl Info {
    fn new(sk: [u8; 32], label: String) -> Self {
        let vk = SigningKey::from(sk).to_verification_key();
        let vkh = Hash::<28>::new(vk);
        Self { label, sk, vk, vkh }
    }
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Add a signing key (bech32-encoded), optionally under a label.
    Add {
        #[arg(long, value_parser = hex32)]
        key: [u8; 32],
        #[arg(long, default_value = "NONE")]
        label: String,
    },
    /// Remove a signing key by its verification key hash.
    Remove {
        #[arg(long, value_parser = hex32)]
        key: [u8; 32],
    },
    /// Generate a signing key from a string seed; the seed becomes its label.
    Generate { seed: String },
    /// List key hashes, verification keys, and labels.
    List,
}

fn hex32(s: &str) -> Result<[u8; 32], String> {
    hex::decode(s)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| "bad hex".into())
}

/// Insert `key` (erroring if already present), tag it with `label`, persist,
/// and return its record.
fn insert(ctx: &mut Ctx, key: [u8; 32], label: String) -> Result<Info> {
    anyhow::ensure!(
        ctx.config.keyring.insert(key, label.clone()),
        "key already exists"
    );
    ctx.save()?;
    Ok(Info::new(key, label))
}

impl Cmd {
    pub fn run(self, mut ctx: Ctx) -> Result<()> {
        let output = match self {
            Cmd::Add { key, label } => serde_json::to_string(&insert(&mut ctx, key, label)?)?,
            Cmd::Remove { key } => {
                anyhow::ensure!(ctx.config.keyring.remove(key), "key does not exist");
                ctx.save()?;
                "true".to_string()
            }
            Cmd::Generate { seed } => {
                serde_json::to_string(&insert(&mut ctx, hash(seed.as_bytes()), seed)?)?
            }
            Cmd::List => {
                let entries: Vec<_> = ctx
                    .config
                    .keyring
                    .keys
                    .iter()
                    .map(|(key, label)| Info::new(key.into(), label.clone()))
                    .collect();
                serde_json::to_string_pretty(&entries)?
            }
        };
        println!("{output}");
        Ok(())
    }
}
