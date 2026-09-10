use anyhow::anyhow;
use cardano_sdk::{Hash, SigningKey};
use inquire::{Confirm, CustomType, Select, Text};
use serde_json::{Value, json};
use subbit_core::{Duration, Hash28, VerifyingKey};

use crate::{ctx::Ctx, time::now};

pub fn prompt_hex_field(field: &str) -> anyhow::Result<Value> {
    Ok(json!(Text::new(&format!("{field} (hex):")).prompt()?))
}

pub fn prompt_json_field(field: &str, ty_hint: &str) -> anyhow::Result<Value> {
    let raw = Text::new(&format!("{field} ({ty_hint}, raw JSON):")).prompt()?;
    Ok(serde_json::from_str(&raw).unwrap_or(Value::String(raw)))
}

pub fn prompt_duration_ms(field: &str) -> anyhow::Result<u64> {
    Ok(CustomType::<u64>::new(&format!("{field} (ms):")).prompt()?)
}

/// Prompt for a field, retrying until `parse` succeeds. Use for any
/// interactive text prompt whose raw string needs to become a typed
/// value — CLI-flag parsing should use `ui::args::parse_arg` instead,
/// which hard-fails rather than loops, since there's no reprompt concept
/// for a one-shot `--flag`.
pub fn prompt_parsed<T, E: std::fmt::Display>(
    field: &str,
    hint: &str,
    parse: impl Fn(&str) -> Result<T, E>,
) -> anyhow::Result<T> {
    loop {
        let raw = Text::new(&format!("{field} ({hint}):")).prompt()?;
        match parse(&raw) {
            Ok(v) => return Ok(v),
            Err(e) => eprintln!("bad {field} {raw:?}: {e}"),
        }
    }
}

#[derive(Clone, Copy)]
pub enum Relative {
    /// Resolves to a deadline after now — e.g. `Close`'s `upper`.
    Future,
    /// Resolves to a point before now — e.g. `Elapse`'s `lower`.
    Past,
}

/// Prompt for a relative duration (e.g. "210s"), then resolve it against
/// `now()` in the given direction to an absolute time.
pub fn prompt_relative_duration(field: &str, direction: Relative) -> anyhow::Result<Duration> {
    let offset = prompt_parsed(field, "relative, e.g. 210s", |s: &str| {
        s.parse::<Duration>()
    })?;
    match direction {
        Relative::Future => Ok(now() + offset),
        Relative::Past => now()
            .checked_sub(*offset)
            .map(Duration)
            .ok_or_else(|| anyhow!("{field} offset is larger than the current time")),
    }
}

pub type VariantFn = fn() -> anyhow::Result<Option<Value>>;
pub type Variants<'a> = &'a [(&'a str, VariantFn)];

pub fn variant_prompt(label: &str, variants: Variants) -> anyhow::Result<Value> {
    let names: Vec<&str> = variants.iter().map(|(name, _)| *name).collect();
    let chosen = Select::new(label, names).prompt()?;
    let (name, build) = variants.iter().find(|(name, _)| *name == chosen).unwrap();
    Ok(match build()? {
        Some(fields) => json!({ *name: fields }),
        None => json!(name),
    })
}

/// Hex-encode to match the string form `Ctx::label_lookup()` keys are
/// stored under (labels are keyed by hex-encoded bytes). Shared with
/// `tx::select`.
pub fn hex_key(h: &Hash28) -> String {
    hex::encode(<[u8; 28]>::from(*h))
}

pub fn label_or_hex(labels: &std::collections::BTreeMap<String, String>, h: &Hash28) -> String {
    let hex = hex_key(h);
    labels.get(&hex).cloned().unwrap_or(hex)
}

/// Offer to pick a labelled key from the keyring; fall back to manual hex
/// entry if the keyring is empty, the user declines, or the value needs
/// to be parsed from raw input. `derive` maps a stored `[u8; 32]` signing
/// key to whatever `T` the caller needs; `describe` formats the picker
/// label.
pub fn pick_key<T>(
    ctx: &Ctx,
    field: &str,
    derive: impl Fn([u8; 32]) -> T,
    describe: impl Fn(&str, &T) -> String,
    parse: impl Fn(&str) -> anyhow::Result<T>,
) -> anyhow::Result<T> {
    let use_keyring = !ctx.config.keyring.keys.is_empty()
        && Confirm::new(&format!("select {field} from keyring?"))
            .with_default(true)
            .prompt()?;

    if !use_keyring {
        loop {
            let raw = Text::new(&format!("{field} (hex):")).prompt()?;
            match parse(&raw) {
                Ok(v) => return Ok(v),
                Err(e) => eprintln!("bad {field} {raw:?}: {e}"),
            }
        }
    }

    let entries: Vec<(String, T)> = ctx
        .config
        .keyring
        .keys
        .iter()
        .map(|(label, k)| {
            let derived = derive(k.into());
            (describe(label, &derived), derived)
        })
        .collect();

    let labels: Vec<String> = entries.iter().map(|(l, _)| l.clone()).collect();
    let chosen = Select::new(&format!("{field}:"), labels).prompt()?;
    entries
        .into_iter()
        .find(|(l, _)| *l == chosen)
        .map(|(_, k)| k)
        .ok_or_else(|| anyhow!("selected entry must be present"))
}
pub fn prompt_key_field(ctx: &Ctx, field: &str, want_hash: bool) -> anyhow::Result<Value> {
    if ctx.config.keyring.keys.is_empty()
        || !Confirm::new(&format!("select {field} from keyring?"))
            .with_default(true)
            .prompt()?
    {
        return prompt_hex_field(field);
    }

    let labels = ctx.label_lookup();
    let (vk, vkh) = pick_key_no_fallback(
        ctx,
        field,
        |sk: [u8; 32]| {
            let csk = SigningKey::from(sk);
            let cvk = csk.to_verification_key();
            let cvkh = Hash::<28>::new(cvk);
            let vk = VerifyingKey::from(<[u8; 32]>::from(cvk));
            let vkh = Hash28::from(<[u8; 28]>::from(cvkh));
            (vk, vkh)
        },
        |label, (_, vkh)| {
            let hex = hex_key(vkh);
            if labels.contains_key(&hex) {
                label.to_string()
            } else {
                format!("{label} ({hex})")
            }
        },
    )?;

    Ok(if want_hash {
        serde_json::to_value(vkh)?
    } else {
        serde_json::to_value(vk)?
    })
}

/// Like `pick_key`, but assumes the keyring is non-empty and the caller
/// has already confirmed the user wants to select from it — no manual
/// hex fallback, since the fallback would need to produce the same
/// paired type as `derive`, which manual entry can't supply.
fn pick_key_no_fallback<T>(
    ctx: &Ctx,
    field: &str,
    derive: impl Fn([u8; 32]) -> T,
    describe: impl Fn(&str, &T) -> String,
) -> anyhow::Result<T> {
    let entries: Vec<(String, T)> = ctx
        .config
        .keyring
        .keys
        .iter()
        .map(|(label, k)| {
            let derived = derive(k.into());
            (describe(label, &derived), derived)
        })
        .collect();

    let labels: Vec<String> = entries.iter().map(|(l, _)| l.clone()).collect();
    let chosen = Select::new(&format!("{field}:"), labels).prompt()?;
    entries
        .into_iter()
        .find(|(l, _)| *l == chosen)
        .map(|(_, k)| k)
        .ok_or_else(|| anyhow!("selected entry must be present"))
}
