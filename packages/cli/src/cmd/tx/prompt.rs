use anyhow::Context;
use cardano_sdk::VerificationKey;
use inquire::{CustomType, Select, Text};
use serde_json::{json, Value};
use subbit_core::Hash28;

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
fn hex_key(h: &Hash28) -> String {
    hex::encode(<[u8; 28]>::from(*h))
}

fn label_or_hex(labels: &std::collections::BTreeMap<String, String>, h: &Hash28) -> String {
    let hex = hex_key(h);
    labels.get(&hex).cloned().unwrap_or(hex)
}

/// Offer to resolve a key field from the keyring, falling back to manual
/// hex entry if the keyring is empty or the user declines. `want_hash`
/// picks vk (for `iou_key`) vs vkh (for `consumer`/`provider`).
///
/// STILL OPEN: this needs to derive a `subbit_core::Hash28` /
/// `subbit_core::VerifyingKey` from the stored `[u8; 32]` signing key —
/// not a `cardano_sdk::Hash<28>` — pending the conversion path you
/// mentioned (`<[u8;N]>::from(...).into()`). Left as a stub below.
pub fn prompt_key_field(ctx: &Ctx, field: &str, want_hash: bool) -> anyhow::Result<Value> {
    let entries: Vec<_> = ctx.config.keyring.keys.iter().collect();
    if entries.is_empty() {
        return prompt_hex_field(field);
    }
    if !inquire::Confirm::new(&format!("select {field} from keyring?"))
        .with_default(true)
        .prompt()?
    {
        return prompt_hex_field(field);
    }

    let labels = ctx.label_lookup();
    // candidates: (display, vk, vkh) — vkh derivation TBD, see note above
    todo!("derive subbit_core::VerifyingKey/Hash28 correctly, then format display via label_or_hex")
}
