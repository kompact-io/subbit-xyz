use anyhow::{anyhow, Context};
use inquire::{Confirm, CustomType, Select, Text};
use serde::de::DeserializeOwned;
use serde_json::{json, Value};
use subbit_core::Stage;
use subbit_tx::Channel;

use crate::{
    ctx::Ctx,
    prompt::{prompt_duration_ms, prompt_hex_field, prompt_json_field, prompt_key_field, variant_prompt, Variants, VariantFn},
};

pub fn want<W: DeserializeOwned>(channel: &Channel) -> anyhow::Result<W> {
    let variables = channel.variables();
    let amount = variables.amount();
    let value = match variables.stage() {
        Stage::Opened { .. } => {
            let mut variants: Vec<(&str, VariantFn)> = vec![
                ("Add", || Ok(Some(json!({"amount": CustomType::<u64>::new("amount:").prompt()?})))),
                ("Close", || Ok(Some(json!({"upper": prompt_duration_ms("upper")?})))),
            ];
            if amount > 0 {
                variants.insert(1, ("Sub", || Ok(Some(json!({"iou": iou()?})))));
            }
            variant_prompt(&format!("Want variant: (available: {amount})"), &variants)?
        }
        Stage::Closed { elapse_at, .. } => {
            let elapse_at_ms = u64::from(*elapse_at);
            variant_prompt(
                &format!("Want variant: (available: {amount}, elapse_at: {elapse_at_ms}ms)"),
                &[
                    ("Settle", || Ok(Some(json!({"iou": iou()?})))),
                    ("Elapse", || Ok(Some(json!({"lower": prompt_duration_ms("lower")?})))),
                ],
            )?
        }
        Stage::Settled => json!("End"),
    };
    serde_json::from_value(value).context("building Want from interactive input")
}

fn iou() -> anyhow::Result<Value> {
    Ok(json!({
        "amount": CustomType::<u64>::new("amount:").prompt()?,
        "signature": prompt_hex_field("signature")?,
    }))
}

pub fn open<O: DeserializeOwned>(ctx: &Ctx) -> anyhow::Result<O> {
    let channel = channel(ctx)?;
    let delegation = if Confirm::new("include a delegation credential?").with_default(false).prompt()? {
        prompt_json_field("delegation", "Credential, shape unconfirmed")?
    } else {
        Value::Null
    };
    serde_json::from_value(json!({"channel": channel, "delegation": delegation}))
        .context("building Open from interactive input")
}

fn channel(ctx: &Ctx) -> anyhow::Result<Value> {
    Ok(json!({
        "constants": constants(ctx)?,
        "variables": variables()?,
    }))
}

fn constants(ctx: &Ctx) -> anyhow::Result<Value> {
    Ok(json!({
        "tag": prompt_hex_field("tag")?,
        "currency": currency()?,
        "iou_key": prompt_key_field(ctx, "iou_key", false)?,
        "consumer": prompt_key_field(ctx, "consumer", true)?,
        "provider": prompt_key_field(ctx, "provider", true)?,
        "close_period": prompt_duration_ms("close_period")?,
    }))
}

fn variables() -> anyhow::Result<Value> {
    Ok(json!({
        "amount": CustomType::<u64>::new("amount:").prompt()?,
        "stage": stage()?,
    }))
}

fn currency() -> anyhow::Result<Value> {
    variant_prompt(
        "Currency:",
        &[
            ("Ada", || Ok(None)),
            ("Asset", || Ok(Some(json!({"hash": prompt_hex_field("hash")?, "name": prompt_hex_field("name")?})))),
        ],
    )
}

fn stage() -> anyhow::Result<Value> {
    variant_prompt(
        "Stage:",
        &[
            ("Opened", || Ok(Some(json!({"subbed": CustomType::<u64>::new("subbed:").prompt()?})))),
            ("Closed", || Ok(Some(json!({
                "subbed": CustomType::<u64>::new("subbed:").prompt()?,
                "elapse_at": prompt_duration_ms("elapse_at")?,
            })))),
            ("Settled", || Ok(None)),
        ],
    )
}
