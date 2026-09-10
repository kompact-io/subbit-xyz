use anyhow::{Context, anyhow};
use inquire::{Confirm, CustomType, Select, Text};
use serde_json::{Value, json};

use crate::ctx::Ctx;

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
/// stored under (labels are keyed by hex-encoded bytes).
fn hex_key(bytes: &[u8]) -> String {
    hex::encode(bytes)
}

fn label_or_hex(labels: &std::collections::BTreeMap<String, String>, hex: String) -> String {
    labels.get(&hex).cloned().unwrap_or(hex)
}

/// Offer to pick a labelled key from the keyring; fall back to manual hex
/// entry if the keyring is empty, the user declines, or the value needs
/// to be parsed from raw input. `derive` maps a stored `[u8; 32]` signing
/// key to whatever `T` the caller needs; `describe` formats the picker
/// label (e.g. `tx`'s consumer/provider picker wants to show the derived
/// vkh alongside the config label, not just the label alone).
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
        return parse(&Text::new(&format!("{field} (hex):")).prompt()?);
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

// STILL OPEN (blocking): `tx::forms`'s key-field prompt (consumer/
// provider/iou_key) needs to derive `subbit_session`'s actual
// `Hash28`/`VerifyingKey` types from a `[u8; 32]` signing key, via
// whatever `<[u8;N]>::from(...).into()` path that crate exposes — not
// yet pinned down. `pick_key` above is generic and ready to take
// whichever `derive`/`describe` closures make that concrete.
