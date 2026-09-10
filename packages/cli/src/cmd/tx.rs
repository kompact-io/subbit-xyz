use anyhow::{Context};
use clap::Subcommand;
use serde_json::{Value, json};

use crate::{ctx::Ctx, session};


mod forms;
mod select;
mod prompt;



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
        match self {
            Cmd::Status => {
                let tx = ctx.load_tx()?;
                print_json(json!({
                    "channels_available": tx.channels().len(),
                    "pending_wills": tx.inputs().len(),
                    "pending_opens": tx.opens().len(),
                    "opens": tx.opens().keys().map(ToString::to_string).collect::<Vec<_>>(),
                }))
            }

            Cmd::Stage => {
                let session = crate::session::build(&ctx.config.session).await?;
                let tx = session.stage_tx()?;
                finish(&ctx, &tx, json!({"status": "staged", "channels": tx.channels().len()}))
            }
            Cmd::Propose { input, want } => {
                let mut tx = ctx.load_tx()?;
                let parsed_input = match input {
                    Some(i) => parse_arg(&i, "input")?,
                    None => select::input(&tx)?,
                };
                let want = match want {
                    Some(w) => serde_json::from_str(&resolve_json_arg(&w)?)
                        .context("parsing --want JSON")?,
                    None => {
                        let channel = tx
                            .channels()
                            .get(&parsed_input)
                            .ok_or_else(|| anyhow::anyhow!("no channel staged for input {parsed_input}"))?
                            .clone();
                        forms::want(&channel)?
                    }
                };
                let input_display = parsed_input.to_string();
                tx.propose(parsed_input, want)?;
                finish(&ctx, &tx, json!({"status": "proposed", "input": input_display}))
            }
            Cmd::DropIntent { input } => {
                let mut tx = ctx.load_tx()?;
                let parsed_input = match input {
                    Some(i) => parse_arg(&i, "input")?,
                    None => select_will_input(&tx)?,
                };
                let input_display = parsed_input.to_string();
                let dropped = tx.drop_intent(&parsed_input).is_some();
                finish(
                    &ctx,
                    &tx,
                    json!({"status": drop_status(dropped), "input": input_display}),
                )
            }
            Cmd::Open { open } => {
                let mut tx = ctx.load_tx()?;
                let open = match open {
                    Some(o) => serde_json::from_str(&resolve_json_arg(&o)?)
                        .context("parsing --open JSON")?,
                    None => build_open_interactive(&ctx)?,
                };
                tx.add_open(open);
                finish(&ctx, &tx, json!({"status": "open_staged"}))
            }
            Cmd::DropOpen { tag } => {
                let mut tx = ctx.load_tx()?;
                let parsed_tag = match tag {
                    Some(t) => parse_arg(&t, "tag")?,
                    None => select_tag(&tx)?,
                };
                let tag_display = parsed_tag.to_string();
                let dropped = tx.drop_open(&parsed_tag).is_some();
                finish(
                    &ctx,
                    &tx,
                    json!({"status": drop_status(dropped), "tag": tag_display}),
                )
            }
            Cmd::Clear => {
                let mut tx = ctx.load_tx()?;
                tx.drop_all_intents();
                tx.drop_all_opens();
                finish(&ctx, &tx, json!({"status": "cleared"}))
            }
            Cmd::Submit => {
                let mut session = session::build(&ctx.config.session).await?;
                let mut tx = ctx.load_tx()?;

                let mut built = session.build_tx(&mut tx)?;
                ctx.config
                    .keyring
                    .clone()
                    .build()
                    .sign(&mut built)
                    .context("signing required signatories")?;

                let id = session.sign_and_submit(built).await?;
                print_json(json!({"status": "submitted", "id": id.to_string()}))?;
                session.wait_til(&id).await?;
                finish(
                    &ctx,
                    &tx,
                    json!({"status": "confirmed", "id": id.to_string()}),
                )
            }
        }
    }
}

fn finish(ctx: &Ctx, tx: &subbit_tx::tx::Tx, status: Value) -> anyhow::Result<()> {
    print_json(status)?;
    ctx.save_tx(tx)
}

