use std::collections::{BTreeMap, BTreeSet};

use anyhow::{anyhow, Result};
use inquire::Select;
use cardano_sdk::Input;
use subbit_core::{Hash28, Tag};
use subbit_tx::{tx::Tx, Channel};

fn hex_key(h: &Hash28) -> String {
    hex::encode(<[u8; 28]>::from(*h))
}

fn label_or_hex(labels: &BTreeMap<String, String>, h: &Hash28) -> String {
    let hex = hex_key(h);
    labels.get(&hex).cloned().unwrap_or(hex)
}

/// Render an `Input` for the picker using its channel's consumer/provider
/// labels, so staged channels are distinguishable by counterparty rather
/// than by opaque input hash.
fn describe_input(input: &Input, channel: &Channel, labels: &BTreeMap<String, String>) -> String {
    let constants = channel.constants();
    let consumer = label_or_hex(labels, constants.consumer());
    let provider = label_or_hex(labels, constants.provider());
    format!("{input} (consumer: {consumer}, provider: {provider})")
}

pub fn input(tx: &Tx, labels: &BTreeMap<String, String>) -> Result<Input> {
    pick(
        tx.channels().iter().map(|(i, c)| (i.clone(), c.clone())).collect(),
        labels,
        "no channels staged; run `tx new` first",
    )
}

/// Like `input`, but only offers inputs with a pending will — what
/// `DropIntent` actually operates on.
pub fn will_input(tx: &Tx, labels: &BTreeMap<String, String>) -> Result<Input> {
    let pending: BTreeSet<Input> = tx.inputs().into_iter().map(|(input, _)| input).collect();
    let entries = tx
        .channels()
        .iter()
        .filter(|(i, _)| pending.contains(i))
        .map(|(i, c)| (i.clone(), c.clone()))
        .collect();
    pick(entries, labels, "no pending wills")
}

fn pick(
    entries: Vec<(Input, Channel)>,
    labels: &BTreeMap<String, String>,
    empty_msg: &str,
) -> Result<Input> {
    if entries.is_empty() {
        return Err(anyhow!(empty_msg));
    }
    let display: Vec<String> = entries.iter().map(|(i, c)| describe_input(i, c, labels)).collect();
    let chosen = Select::new("Input:", display).prompt()?;
    entries
        .into_iter()
        .zip(display)
        .find(|(_, d)| *d == chosen)
        .map(|((i, _), _)| i)
        .ok_or_else(|| anyhow!("selected entry must be present"))
}

pub fn tag(tx: &Tx) -> Result<Tag> {
    let tags: Vec<Tag> = tx.opens().keys().cloned().collect();
    if tags.is_empty() {
        return Err(anyhow!("no opens staged"));
    }
    Ok(Select::new("Tag:", tags).prompt()?)
}
