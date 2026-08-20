use anyhow::anyhow;
use cardano_sdk::{Signature, SigningKey, VerificationKey};
use clap::Subcommand;
use inquire::{Confirm, CustomType, Select, Text};
use serde::de::DeserializeOwned;
use subbit_core::{Iou, Tag, TagTbs};

use crate::ctx::Ctx;

fn hex32(s: &str) -> Result<[u8; 32], String> {
    hex::decode(s)
        .ok()
        .and_then(|v| v.try_into().ok())
        .ok_or_else(|| "bad hex".into())
}

fn hex_arg<T: DeserializeOwned>(s: &str) -> anyhow::Result<T> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).map_err(|e| anyhow!(e))
}

fn prompt_tag() -> anyhow::Result<Tag> {
    Text::new("tag (hex):")
        .prompt()?
        .parse()
        .map_err(|_| anyhow!("bad tag"))
}

fn prompt_amount() -> anyhow::Result<u64> {
    Ok(CustomType::<u64>::new("amount:").prompt()?)
}

/// Pick a key from the keyring by label, or fall back to manual hex entry.
/// `derive` maps a stored signing key to whatever `T` is needed.
fn select_key<T>(
    ctx: &Ctx,
    field: &str,
    parse: impl Fn(&str) -> anyhow::Result<T>,
    derive: impl Fn([u8; 32]) -> T,
) -> anyhow::Result<T> {
    if ctx.config.keyring.keys.is_empty()
        || !Confirm::new(&format!("select {field} from keyring?"))
            .with_default(true)
            .prompt()?
    {
        return parse(&Text::new(&format!("{field} (hex):")).prompt()?);
    }
    let entries: Vec<(String, T)> = ctx
        .config
        .keyring
        .keys
        .iter()
        .map(|(k, label)| (label.clone(), derive(k.into())))
        .collect();
    let labels: Vec<String> = entries.iter().map(|(l, _)| l.clone()).collect();
    let chosen = Select::new(&format!("{field}:"), labels).prompt()?;
    entries
        .into_iter()
        .find(|(l, _)| *l == chosen)
        .map(|(_, k)| k)
        .ok_or_else(|| anyhow!("selected entry must be present"))
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
}

impl Cmd {
    pub fn run(self, ctx: &Ctx) -> anyhow::Result<()> {
        match self {
            Cmd::Sign { key, tag, amount } => {
                let (key, tag, amount) = match (key, tag, amount) {
                    (Some(k), Some(t), Some(a)) => (k, t, a),
                    _ => (
                        select_key(
                            ctx,
                            "signing key",
                            |s| hex32(s).map_err(|e| anyhow!(e)),
                            |k| k,
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
                        select_key(ctx, "key", hex_arg::<VerificationKey>, |k| {
                            SigningKey::from(k).to_verification_key()
                        })?,
                        prompt_tag()?,
                        prompt_amount()?,
                        hex_arg::<Signature>(&Text::new("signature (hex):").prompt()?)?,
                    ),
                };
                println!(
                    "{}",
                    key.verify(TagTbs::new(tag, amount).to_vec(), &signature)
                );
            }
        }
        Ok(())
    }
}
