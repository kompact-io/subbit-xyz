use std::collections::{BTreeMap, BTreeSet};

use anyhow::{Result, anyhow};
use cardano_sdk::Input;
use inquire::Select;
use subbit_core::Stage;
use subbit_tx::{Channel, tx::Tx};

use crate::{now, ui::prompt::label_or_hex};

fn compact(n: u64) -> String {
    if n == 0 {
        return "0".to_string();
    }
    format!("{:.0e}", n as f64)
}

/// Render an `Input` for the picker: short tx-hash prefix, output index,
/// counterparty labels, tag, amount, and stage — enough to disambiguate
/// staged channels at a glance.
fn describe_input(
    input: &Input,
    channel: &Channel,
    labels: &BTreeMap<String, String>,
    has_will: &BTreeSet<Input>,
) -> String {
    let constants = channel.constants();
    let variables = channel.variables();

    let hash = hex::encode(input.transaction_id().as_ref());
    let index = input.output_index();
    let provider = label_or_hex(labels, constants.provider());
    let consumer = label_or_hex(labels, constants.consumer());
    let tag = hex::encode(constants.tag().as_ref());
    let amount = compact(variables.amount());

    let stage = match variables.stage() {
        Stage::Opened { subbed } => format!("opened subbed:{}", compact(*subbed)),
        Stage::Closed { subbed, elapse_at } => {
            format!(
                "closed subbed:{} elapse:{}",
                compact(*subbed),
                human_time(elapse_at)
            )
        }
        Stage::Settled => "settled".to_string(),
    };
    let will_marker = if has_will.contains(input) {
        " [will pending]"
    } else {
        ""
    };
    format!(
        "{}#{index} | P:{provider} | C:{consumer} | T:{tag} | amt:{amount} | {stage}{will_marker}",
        &hash[..6.min(hash.len())]
    )
}

/// Good enough for human time
fn human_time(elapse_at: &subbit_core::Duration) -> String {
    const THRESHOLD_MS: i128 = 120_000; // 2 mins
    let delta = now().as_millis() as i128 - elapse_at.as_millis() as i128;
    let mins = |ms: i128| (ms.unsigned_abs() / 60_000).to_string();
    match delta {
        d if d > THRESHOLD_MS => format!("-{}m", mins(d)),
        d if d > 0 => "now".to_string(),
        d if d > -THRESHOLD_MS => "now".to_string(),
        d => format!("+{}m", mins(d)),
    }
}

pub fn input(tx: &Tx, labels: &BTreeMap<String, String>) -> Result<Input> {
    let has_will: BTreeSet<Input> = tx.inputs().into_iter().map(|(i, _)| i).collect();
    pick(
        tx.channels()
            .iter()
            .map(|(i, c)| (i.clone(), c.clone()))
            .collect(),
        labels,
        &has_will,
        "no channels staged; run `session refresh; tx init` first",
    )
}

pub fn will_input(tx: &Tx, labels: &BTreeMap<String, String>) -> Result<Input> {
    let pending: BTreeSet<Input> = tx.inputs().into_iter().map(|(input, _)| input).collect();
    let entries = tx
        .channels()
        .iter()
        .filter(|(i, _)| pending.contains(i))
        .map(|(i, c)| (i.clone(), c.clone()))
        .collect();
    pick(entries, labels, &pending, "no pending wills")
}

fn pick(
    entries: Vec<(Input, Channel)>,
    labels: &BTreeMap<String, String>,
    has_will: &BTreeSet<Input>,
    empty_msg: &'static str,
) -> Result<Input> {
    if entries.is_empty() {
        return Err(anyhow!(empty_msg));
    }
    let described: Vec<(Input, String)> = entries
        .into_iter()
        .map(|(i, c)| {
            let d = describe_input(&i, &c, labels, has_will);
            (i, d)
        })
        .collect();
    let display: Vec<String> = described.iter().map(|(_, d)| d.clone()).collect();
    let chosen = Select::new("Input:", display).prompt()?;
    described
        .into_iter()
        .find(|(_, d)| *d == chosen)
        .map(|(i, _)| i)
        .ok_or_else(|| anyhow!("selected entry must be present"))
}

pub fn keytag(tx: &Tx) -> Result<String> {
    let keys: Vec<String> = tx.opens().keys().cloned().collect();
    if keys.is_empty() {
        return Err(anyhow!("no opens staged"));
    }
    Ok(Select::new("Open:", keys).prompt()?)
}

pub fn resolve_input(
    tx: &Tx,
    input: Option<String>,
    iou_key: Option<String>,
    tag: Option<String>,
) -> Result<Input> {
    if let Some(i) = input {
        return crate::ui::args::parse_arg(&i, "input");
    }
    if let (Some(iou_key), Some(tag)) = (iou_key, tag) {
        return by_iou_key_tag(tx, &iou_key, &tag);
    }
    unreachable!("clap's requires= ensures iou_key/tag are only ever both-or-neither")
}

fn by_iou_key_tag(tx: &Tx, iou_key: &str, tag: &str) -> Result<Input> {
    tx.channels()
        .iter()
        .find(|(_, c)| {
            let constants = c.constants();
            hex::encode(<[u8; 32]>::from(*constants.iou_key())) == iou_key
                && hex::encode(constants.tag().as_ref()) == tag
        })
        .map(|(input, _)| input.clone())
        .ok_or_else(|| anyhow!("no staged channel matches iou_key={iou_key} tag={tag}"))
}
