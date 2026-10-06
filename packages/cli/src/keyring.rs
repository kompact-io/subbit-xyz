use cardano_sdk::{Hash, SigningKey, VerificationKey};
use clap::Subcommand;
use serde::Serialize;

use crate::{ctx::Ctx, ui::args::hex32};

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
    fn new(label: String, sk: [u8; 32]) -> Self {
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
    /// Print each key as SHELL-safe env vars: LABEL_VK and LABEL_VKH,
    /// suitable for `source`-ing or piping into envsubst.
    Env,
}

fn shell_safe(label: &str) -> String {
    label
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect()
}

/// Insert `key` (erroring if already present), tag it with `label`, persist,
/// and return its record.
fn insert(ctx: &mut Ctx, label: String, key: [u8; 32]) -> anyhow::Result<Info> {
    anyhow::ensure!(
        ctx.config.keyring.insert(label.clone(), key),
        "key already exists"
    );
    ctx.save()?;
    Ok(Info::new(label, key))
}

impl Cmd {
    pub fn run(self, mut ctx: Ctx) -> anyhow::Result<()> {
        let output = match self {
            Cmd::Add { key, label } => serde_json::to_string(&insert(&mut ctx, label, key)?)?,
            Cmd::Remove { key } => {
                anyhow::ensure!(ctx.config.keyring.remove_by_key(key), "key does not exist");
                ctx.save()?;
                "true".to_string()
            }
            Cmd::Generate { seed } => {
                serde_json::to_string(&insert(&mut ctx, seed.clone(), hash(seed.as_bytes()))?)?
            }
            Cmd::List => {
                let entries: Vec<_> = ctx
                    .config
                    .keyring
                    .keys
                    .iter()
                    .map(|(label, key)| Info::new(label.clone(), key.into()))
                    .collect();
                serde_json::to_string_pretty(&entries)?
            }
            Cmd::Env => {
                for (label, key) in ctx.config.keyring.keys.iter() {
                    let info = Info::new(label.clone(), key.into());
                    let name = shell_safe(label);
                    println!("{name}_VK={}", hex::encode(<[u8; 32]>::from(info.vk)));
                    println!("{name}_VKH={}", hex::encode(<[u8; 28]>::from(info.vkh)));
                }
                return Ok(());
            }
        };
        println!("{output}");
        Ok(())
    }
}
