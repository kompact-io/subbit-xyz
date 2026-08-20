use anyhow::{Context, anyhow};
use clap::Subcommand;
use inquire::{Confirm, CustomType, Select, Text};
use serde::de::DeserializeOwned;
use serde_json::{Value, json};
use std::path::Path;

use cardano_sdk::{Hash, Input, SigningKey, VerificationKey};

use subbit_core::{Stage, Tag};
use subbit_tx::{Channel, tx::Tx};

use crate::{cache, ctx::Ctx, session};

#[derive(Subcommand)]
pub enum Cmd {
    /// Start a fresh staging tx from the session's current channels.
    Stage,
    /// Show what's currently staged.
    Status,
    /// Register an intent against an existing channel input. Without
    /// INPUT, choose from staged channels interactively. Without --want,
    /// walks through building one interactively.
    Propose {
        input: Option<String>,
        #[arg(long)]
        want: Option<String>,
    },
    /// Drop a staged intent for an input. Without INPUT, choose from
    /// pending wills interactively.
    DropIntent { input: Option<String> },
    /// Stage a new channel to open. Without --open, prompts for the JSON
    /// interactively.
    Open {
        #[arg(long)]
        open: Option<String>,
    },
    /// Drop a staged open by its channel tag. Without TAG, choose from
    /// pending opens interactively.
    DropOpen { tag: Option<String> },
    /// Drop everything staged (wills and opens).
    Clear,
    /// Build, sign, and submit the currently staged tx.
    Submit,
}

impl Cmd {
    pub async fn run(self, ctx: Ctx) -> anyhow::Result<()> {
        let cache_path = ctx.config.cache_path.clone();

        match self {
            Cmd::Stage => {
                let session = session::build(&ctx.config.session).await?;
                let tx = session.stage_tx()?;
                finish(
                    &cache_path,
                    &tx,
                    json!({"status": "staged", "channels": tx.channels().len()}),
                )
            }
            Cmd::Status => {
                let tx = load_tx(&cache_path)?;
                print_json(json!({
                    "channels_available": tx.channels().len(),
                    "pending_wills": tx.inputs().len(),
                    "pending_opens": tx.opens().len(),
                    "opens": tx.opens().keys().map(ToString::to_string).collect::<Vec<_>>(),
                }))
            }
            Cmd::Propose { input, want } => {
                let mut tx = load_tx(&cache_path)?;
                let parsed_input = match input {
                    Some(i) => parse_arg(&i, "input")?,
                    None => select_input(&tx)?,
                };
                let want = match want {
                    Some(w) => serde_json::from_str(&resolve_json_arg(&w)?)
                        .context("parsing --want JSON")?,
                    None => {
                        let channel = tx
                            .channels()
                            .get(&parsed_input)
                            .ok_or_else(|| anyhow!("no channel staged for input {parsed_input}"))?
                            .clone();
                        build_want_interactive(&channel)?
                    }
                };
                let input_display = parsed_input.to_string();
                tx.propose(parsed_input, want)?;
                finish(
                    &cache_path,
                    &tx,
                    json!({"status": "proposed", "input": input_display}),
                )
            }
            Cmd::DropIntent { input } => {
                let mut tx = load_tx(&cache_path)?;
                let parsed_input = match input {
                    Some(i) => parse_arg(&i, "input")?,
                    None => select_will_input(&tx)?,
                };
                let input_display = parsed_input.to_string();
                let dropped = tx.drop_intent(&parsed_input).is_some();
                finish(
                    &cache_path,
                    &tx,
                    json!({"status": drop_status(dropped), "input": input_display}),
                )
            }
            Cmd::Open { open } => {
                let mut tx = load_tx(&cache_path)?;
                let open = match open {
                    Some(o) => serde_json::from_str(&resolve_json_arg(&o)?)
                        .context("parsing --open JSON")?,
                    None => build_open_interactive(&ctx)?,
                };
                tx.add_open(open);
                finish(&cache_path, &tx, json!({"status": "open_staged"}))
            }
            Cmd::DropOpen { tag } => {
                let mut tx = load_tx(&cache_path)?;
                let parsed_tag = match tag {
                    Some(t) => parse_arg(&t, "tag")?,
                    None => select_tag(&tx)?,
                };
                let tag_display = parsed_tag.to_string();
                let dropped = tx.drop_open(&parsed_tag).is_some();
                finish(
                    &cache_path,
                    &tx,
                    json!({"status": drop_status(dropped), "tag": tag_display}),
                )
            }
            Cmd::Clear => {
                let mut tx = load_tx(&cache_path)?;
                tx.drop_all_intents();
                tx.drop_all_opens();
                finish(&cache_path, &tx, json!({"status": "cleared"}))
            }
            Cmd::Submit => {
                let mut session = session::build(&ctx.config.session).await?;
                let mut tx = load_tx(&cache_path)?;

                let mut built = session.build_tx(&mut tx)?;
                ctx.config
                    .keyring
                    .build()
                    .sign(&mut built)
                    .context("signing required signatories")?;

                let id = session.sign_and_submit(built).await?;
                print_json(json!({"status": "submitted", "id": id.to_string()}))?;
                session.wait_til(&id).await?;
                finish(
                    &cache_path,
                    &tx,
                    json!({"status": "confirmed", "id": id.to_string()}),
                )
            }
        }
    }
}

fn print_json(value: Value) -> anyhow::Result<()> {
    println!("{}", serde_json::to_string_pretty(&value)?);
    Ok(())
}

fn finish(cache_path: &Path, tx: &Tx, status: Value) -> anyhow::Result<()> {
    print_json(status)?;
    save_tx(cache_path, tx)
}

fn drop_status(dropped: bool) -> &'static str {
    if dropped { "dropped" } else { "not_found" }
}

fn parse_arg<T: std::str::FromStr>(raw: &str, what: &str) -> anyhow::Result<T> {
    raw.parse()
        .map_err(|_| anyhow!("couldn't parse {what} {raw:?}"))
}

/// ASSUMPTION: `cardano_sdk::Input` implements `Display` — required by
/// `inquire::Select`, not confirmed anywhere seen so far.
fn select_input(tx: &Tx) -> anyhow::Result<Input> {
    let inputs: Vec<Input> = tx.channels().keys().cloned().collect();
    if inputs.is_empty() {
        return Err(anyhow!("no channels staged; run `tx new` first"));
    }
    Ok(Select::new("Input:", inputs).prompt()?)
}

/// Like `select_input`, but only offers inputs with a pending will —
/// what `DropIntent` actually operates on.
fn select_will_input(tx: &Tx) -> anyhow::Result<Input> {
    let inputs: Vec<Input> = tx.inputs().into_iter().map(|(input, _)| input).collect();
    if inputs.is_empty() {
        return Err(anyhow!("no pending wills"));
    }
    Ok(Select::new("Input:", inputs).prompt()?)
}

fn select_tag(tx: &Tx) -> anyhow::Result<Tag> {
    let tags: Vec<Tag> = tx.opens().keys().cloned().collect();
    if tags.is_empty() {
        return Err(anyhow!("no opens staged"));
    }
    Ok(Select::new("Tag:", tags).prompt()?)
}

fn prompt_hex_field(field: &str) -> anyhow::Result<Value> {
    Ok(json!(Text::new(&format!("{field} (hex):")).prompt()?))
}

fn prompt_json_field(field: &str, ty_hint: &str) -> anyhow::Result<Value> {
    let raw = Text::new(&format!("{field} ({ty_hint}, raw JSON):")).prompt()?;
    Ok(serde_json::from_str(&raw).unwrap_or(Value::String(raw)))
}

fn prompt_duration_ms(field: &str) -> anyhow::Result<u64> {
    Ok(CustomType::<u64>::new(&format!("{field} (ms):")).prompt()?)
}

fn variant_prompt(
    label: &str,
    variants: &[(&str, fn() -> anyhow::Result<Option<Value>>)],
) -> anyhow::Result<Value> {
    let names: Vec<&str> = variants.iter().map(|(name, _)| *name).collect();
    let chosen = Select::new(label, names).prompt()?;
    let (name, build) = variants.iter().find(|(name, _)| *name == chosen).unwrap();
    Ok(match build()? {
        Some(fields) => json!({ *name: fields }),
        None => json!(name),
    })
}

fn build_want_interactive<W: DeserializeOwned>(channel: &Channel) -> anyhow::Result<W> {
    let variables = channel.variables();
    let amount = variables.amount();
    let value = match variables.stage() {
        Stage::Opened { .. } => {
            let mut variants: Vec<(&str, fn() -> anyhow::Result<Option<Value>>)> = vec![
                ("Add", || {
                    Ok(Some(
                        json!({"amount": CustomType::<u64>::new("amount:").prompt()?}),
                    ))
                }),
                ("Close", || {
                    Ok(Some(json!({"upper": prompt_duration_ms("upper")?})))
                }),
            ];
            // `Variables::sub` requires `amount > 0` (else `Error::NoFunds`).
            if amount > 0 {
                variants.insert(1, ("Sub", || Ok(Some(json!({"iou": prompt_iou()?})))));
            }
            variant_prompt(&format!("Want variant: (available: {amount})"), &variants)?
        }
        Stage::Closed { elapse_at, .. } => {
            let elapse_at_ms = u64::from(elapse_at);
            variant_prompt(
                &format!("Want variant: (available: {amount}, elapse_at: {elapse_at_ms}ms)"),
                &[
                    ("Settle", || Ok(Some(json!({"iou": prompt_iou()?})))),
                    ("Elapse", || {
                        Ok(Some(json!({"lower": prompt_duration_ms("lower")?})))
                    }),
                ],
            )?
        }
        Stage::Settled => json!("End"),
    };
    serde_json::from_value(value).context("building Want from interactive input")
}

fn prompt_iou() -> anyhow::Result<Value> {
    Ok(json!({
        "amount": CustomType::<u64>::new("amount:").prompt()?,
        "signature": prompt_hex_field("signature")?,
    }))
}

fn build_constants_interactive(ctx: &Ctx) -> anyhow::Result<Value> {
    Ok(json!({
        "tag": prompt_hex_field("tag")?,
        "currency": build_currency_interactive()?,
        "iou_key": prompt_key_field(ctx, "iou_key", false)?,
        "consumer": prompt_key_field(ctx, "consumer", true)?,
        "provider": prompt_key_field(ctx, "provider", true)?,
        "close_period": prompt_duration_ms("close_period")?,
    }))
}

fn build_channel_interactive(ctx: &Ctx) -> anyhow::Result<Value> {
    Ok(json!({
        "constants": build_constants_interactive(ctx)?,
        "variables": build_variables_interactive()?,
    }))
}

fn build_open_interactive<O: DeserializeOwned>(ctx: &Ctx) -> anyhow::Result<O> {
    let channel = build_channel_interactive(ctx)?;
    let delegation = if Confirm::new("include a delegation credential?")
        .with_default(false)
        .prompt()?
    {
        prompt_json_field("delegation", "Credential, shape unconfirmed")?
    } else {
        Value::Null
    };
    serde_json::from_value(json!({"channel": channel, "delegation": delegation}))
        .context("building Open from interactive input")
}

fn build_variables_interactive() -> anyhow::Result<Value> {
    Ok(json!({
        "amount": CustomType::<u64>::new("amount:").prompt()?,
        "stage": build_stage_interactive()?,
    }))
}

fn build_currency_interactive() -> anyhow::Result<Value> {
    variant_prompt(
        "Currency:",
        &[
            ("Ada", || Ok(None)),
            ("Asset", || {
                Ok(Some(
                    json!({"hash": prompt_hex_field("hash")?, "name": prompt_hex_field("name")?}),
                ))
            }),
        ],
    )
}

fn build_stage_interactive() -> anyhow::Result<Value> {
    variant_prompt(
        "Stage:",
        &[
            ("Opened", || {
                Ok(Some(
                    json!({"subbed": CustomType::<u64>::new("subbed:").prompt()?}),
                ))
            }),
            ("Closed", || {
                Ok(Some(json!({
                    "subbed": CustomType::<u64>::new("subbed:").prompt()?,
                    "elapse_at": prompt_duration_ms("elapse_at")?,
                })))
            }),
            ("Settled", || Ok(None)),
        ],
    )
}

/// Offer to resolve a key field from the keyring, falling back to manual
/// hex entry if the keyring is empty or the user declines. `want_hash`
/// picks vk (for `iou_key`) vs vkh (for `consumer`/`provider`).
///
/// ASSUMPTION: `Constants`'s `iou_key`/`consumer`/`provider` fields
/// deserialize from whatever `VerificationKey`/`Hash<28>` serialize to
/// (confirmed to exist via `Info`'s `#[derive(Serialize)]`), matching
/// what the existing hex `Text` prompt already relied on implicitly.
fn prompt_key_field(ctx: &Ctx, field: &str, want_hash: bool) -> anyhow::Result<Value> {
    let entries: Vec<_> = ctx.config.keyring.keys.iter().collect();
    if entries.is_empty() {
        return prompt_hex_field(field);
    }

    if !Confirm::new(&format!("select {field} from keyring?"))
        .with_default(true)
        .prompt()?
    {
        return prompt_hex_field(field);
    }

    let candidates: Vec<(String, VerificationKey, Hash<28>)> = entries
        .into_iter()
        .map(|(key_hex, label)| {
            let vk = SigningKey::from(<[u8; 32]>::from(key_hex)).to_verification_key();
            let vkh = Hash::<28>::new(vk);
            (format!("{label} ({vkh:?})"), vk, vkh)
        })
        .collect();

    let display: Vec<String> = candidates.iter().map(|(d, ..)| d.clone()).collect();
    let chosen = Select::new(&format!("{field}:"), display).prompt()?;
    let (_, vk, vkh) = candidates
        .into_iter()
        .find(|(d, ..)| *d == chosen)
        .expect("selected entry must be present");

    Ok(if want_hash {
        serde_json::to_value(vkh)?
    } else {
        serde_json::to_value(vk)?
    })
}

fn load_tx(cache_path: &Path) -> anyhow::Result<Tx> {
    cache::try_load(cache_path)?.ok_or_else(|| {
        anyhow!(
            "no staged tx at {}; run `tx new` first",
            cache_path.display()
        )
    })
}

fn save_tx(cache_path: &Path, tx: &Tx) -> anyhow::Result<()> {
    cache::save(cache_path, tx)
}

fn resolve_json_arg(arg: &str) -> anyhow::Result<String> {
    match arg.strip_prefix('@') {
        Some(path) => Ok(std::fs::read_to_string(path)?),
        None => Ok(arg.to_string()),
    }
}
