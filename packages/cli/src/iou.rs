// iou.rs
use std::path::PathBuf;

use anyhow::{Context, anyhow};
use cardano_sdk::{Signature, SigningKey, VerificationKey};
use clap::Subcommand;
use inquire::CustomType;
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use subbit_core::{Iou, Tag, TagTbs};

use crate::{
    ctx::Ctx,
    ui::{
        args::hex32,
        prompt::{pick_key, prompt_parsed},
    },
};

fn hex_arg<T: DeserializeOwned>(s: &str) -> anyhow::Result<T> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|e| anyhow!(e))
}

fn prompt_tag() -> anyhow::Result<Tag> {
    prompt_parsed("tag", "hex", |s: &str| s.parse::<Tag>())
}

fn prompt_amount() -> anyhow::Result<u64> {
    Ok(CustomType::<u64>::new("amount:").prompt()?)
}

/// Resolve a signing key by keyring label, falling back to treating `key`
/// as raw hex if no such label exists.
fn resolve_signing_key(ctx: &Ctx, key: &str) -> anyhow::Result<[u8; 32]> {
    if let Some(sk) = ctx.config.keyring.keys.get(key) {
        return Ok(sk.into());
    }
    hex32(key).map_err(|e| anyhow!(e))
}

#[derive(Debug, Deserialize)]
struct SignRow {
    key: String,
    tag: String,
    amount: u64,
}

#[derive(Debug, Serialize)]
struct SignedRow {
    key: String, // verifying key, hex-encoded
    tag: String,
    amount: u64,
    signature: String,
}

#[derive(Subcommand)]
pub enum Cmd {
    /// Sign an IOU. All flags (--key/--tag also via env), or none for
    /// interactive, with --key selectable from the keyring.
    Sign {
        #[arg(long, env = "IOU_SIGNING_KEY", value_parser = hex32)]
        key: Option<[u8; 32]>,
        #[arg(long, env = "IOU_TAG")]
        tag: Option<Tag>,
        #[arg(long)]
        amount: Option<u64>,
    },
    /// Verify an IOU. All flags (--key/--tag also via env), or none for
    /// interactive, with --key selectable from the keyring.
    Verify {
        #[arg(long, env = "IOU_VERIFYING_KEY", value_parser = hex_arg::<VerificationKey>)]
        key: Option<VerificationKey>,
        #[arg(long, env = "IOU_TAG")]
        tag: Option<Tag>,
        #[arg(long)]
        amount: Option<u64>,
        #[arg(long, value_parser = hex_arg::<Signature>)]
        signature: Option<Signature>,
    },
    /// Batch-sign IOUs from a CSV of `key,tag,amount` rows (`key` is a
    /// keyring label, falling back to raw hex if the label isn't found).
    /// Writes `key,tag,amount,signature` — `key` in the output is the
    /// verifying key, hex-encoded, not the signing label.
    Batch {
        #[arg(long)]
        from: PathBuf,
        #[arg(long)]
        to: Option<PathBuf>,
    },
}

impl Cmd {
    pub fn run(self, ctx: &Ctx) -> anyhow::Result<()> {
        match self {
            Cmd::Sign { key, tag, amount } => {
                let (key, tag, amount) = match (key, tag, amount) {
                    (Some(k), Some(t), Some(a)) => (k, t, a),
                    _ => (
                        pick_key(
                            ctx,
                            "signing key",
                            |k| k,
                            |label, _| label.to_string(),
                            |s| hex32(s).map_err(|e| anyhow!(e)),
                        )?,
                        prompt_tag()?,
                        prompt_amount()?,
                    ),
                };
                let signature = SigningKey::from(key).sign(TagTbs::new(tag, amount).to_vec());
                let iou = Iou::new(amount, <[u8; 64]>::from(signature).into());
                println!("{}", serde_json::to_string(&iou)?);
            }
            Cmd::Verify {
                key,
                tag,
                amount,
                signature,
            } => {
                let (key, tag, amount, signature) = match (key, tag, amount, signature) {
                    (Some(k), Some(t), Some(a), Some(s)) => (k, t, a, s),
                    _ => (
                        pick_key(
                            ctx,
                            "key",
                            |k| SigningKey::from(k).to_verification_key(),
                            |label, _| label.to_string(),
                            hex_arg::<VerificationKey>,
                        )?,
                        prompt_tag()?,
                        prompt_amount()?,
                        prompt_parsed("signature", "hex", hex_arg::<Signature>)?,
                    ),
                };
                println!(
                    "{}",
                    key.verify(TagTbs::new(tag, amount).to_vec(), &signature)
                );
            }
            Cmd::Batch { from, to } => {
                let mut reader = csv::Reader::from_path(&from)
                    .with_context(|| format!("opening {}", from.display()))?;
                let to = to.unwrap_or_else(|| {
                    from.with_file_name(format!(
                        "{}-signed.csv",
                        from.file_stem().unwrap().to_string_lossy()
                    ))
                });

                let mut writer = csv::Writer::from_path(&to)
                    .with_context(|| format!("opening {}", to.display()))?;

                for (i, row) in reader.deserialize::<SignRow>().enumerate() {
                    let row = row.with_context(|| format!("reading row {}", i + 1))?;
                    let sk = resolve_signing_key(ctx, &row.key)
                        .with_context(|| format!("row {}: resolving key {:?}", i + 1, row.key))?;
                    let tag: Tag = row
                        .tag
                        .parse()
                        .map_err(|_| anyhow!("row {}: bad tag {:?}", i + 1, row.tag))?;

                    let signing_key = SigningKey::from(sk);
                    let vk = signing_key.to_verification_key();
                    let signature = signing_key.sign(TagTbs::new(tag, row.amount).to_vec());

                    writer
                        .serialize(SignedRow {
                            key: hex::encode(<[u8; 32]>::from(vk)),
                            tag: row.tag,
                            amount: row.amount,
                            signature: hex::encode(<[u8; 64]>::from(signature)),
                        })
                        .with_context(|| format!("writing row {}", i + 1))?;
                }
                writer.flush().context("flushing output file")?;
                println!("wrote {}", to.display());
            }
        }
        Ok(())
    }
}
