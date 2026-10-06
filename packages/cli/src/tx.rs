use anyhow::Context;
use clap::Subcommand;
use serde_json::{Value, json};

use crate::{ctx::Ctx, session, ui};

mod forms;
mod select;

#[derive(Subcommand)]
pub enum Cmd {
    /// Start a fresh staging tx from the session's current channels.
    Init {
        /// Only stage channels where you hold a labelled key (default).
        #[arg(long, default_value_t = false)]
        no_filter: bool,
    },
    /// Show what's currently staged.
    Status,
    /// Register a want to step a channel input. Interactive
    /// or specify either input, or (key, tag) pair
    Step {
        input: Option<String>,
        #[arg(long, requires = "tag")]
        iou_key: Option<String>,
        #[arg(long, requires = "iou_key")]
        tag: Option<String>,
        #[arg(long)]
        want: Option<String>,
    },
    /// Drop a staged step for an input. Without INPUT (or --iou-key/--tag),
    /// choose from pending steps interactively.
    DropStep {
        input: Option<String>,
        #[arg(long, requires = "tag")]
        iou_key: Option<String>,
        #[arg(long, requires = "iou_key")]
        tag: Option<String>,
    },
    /// Stage a new channel to open. Without --open, prompts for the JSON
    /// interactively.
    Open {
        #[arg(long)]
        open: Option<String>,
    },
    /// Drop a staged open by its (iou_key, tag) pair.
    /// Without TAG, choose from
    /// pending opens interactively.
    DropOpen {
        #[arg(requires = "tag")]
        iou_key: Option<String>,
        #[arg(requires = "iou_key")]
        tag: Option<String>,
    },
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
                ui::output::print_json(json!({
                    "channels_available": tx.channels().len(),
                    "pending_wills": tx.inputs().len(),
                    "pending_opens": tx.opens().len(),
                    "opens": tx.opens().keys().map(ToString::to_string).collect::<Vec<_>>(),
                }))
            }
            Cmd::Init { no_filter } => {
                let session = session::build(&ctx.config.session, false).await?;
                let labels = ctx.label_lookup();

                let mut tx = session.stage_tx()?;
                if !no_filter {
                    tx.retain_channels(|channel| {
                        let c = channel.constants();
                        labels.contains_key(&ui::prompt::hex_key(c.consumer()))
                            || labels.contains_key(&ui::prompt::hex_key(c.provider()))
                    });
                }
                finish(
                    &ctx,
                    &tx,
                    json!({"status": "staged", "channels": tx.channels().len()}),
                )
            }
            Cmd::Step {
                input,
                iou_key,
                tag,
                want,
            } => {
                let mut tx = ctx.load_tx()?;
                let parsed_input = match (&input, &iou_key, &tag) {
                    (None, None, None) => select::input(&tx, &ctx.label_lookup())?,
                    _ => select::resolve_input(&tx, input, iou_key, tag)?,
                };
                let want = match want {
                    Some(w) => serde_json::from_str(&ui::args::resolve_json_arg(&w)?)
                        .context("parsing --want JSON")?,
                    None => {
                        let channel = tx
                            .channels()
                            .get(&parsed_input)
                            .ok_or_else(|| {
                                anyhow::anyhow!("no channel staged for input {parsed_input}")
                            })?
                            .clone();
                        forms::want(&channel)?
                    }
                };
                let input_display = parsed_input.to_string();
                tx.propose(parsed_input, want)?;
                finish(
                    &ctx,
                    &tx,
                    json!({"status": "proposed", "input": input_display}),
                )
            }
            Cmd::DropStep {
                input,
                iou_key,
                tag,
            } => {
                let mut tx = ctx.load_tx()?;
                let parsed_input = match (&input, &iou_key, &tag) {
                    (None, None, None) => select::will_input(&tx, &ctx.label_lookup())?,
                    _ => select::resolve_input(&tx, input, iou_key, tag)?,
                };
                let input_display = parsed_input.to_string();
                let dropped = tx.drop_intent(&parsed_input).is_some();
                finish(
                    &ctx,
                    &tx,
                    json!({"status": ui::output::drop_status(dropped), "input": input_display}),
                )
            }
            Cmd::Open { open } => {
                let mut tx = ctx.load_tx()?;
                let open = match open {
                    Some(o) => serde_json::from_str(&ui::args::resolve_json_arg(&o)?)
                        .context("parsing --open JSON")?,
                    None => forms::open(&ctx)?,
                };
                tx.add_open(open);
                finish(&ctx, &tx, json!({"status": "open_staged"}))
            }
            Cmd::DropOpen { iou_key, tag } => {
                let mut tx = ctx.load_tx()?;
                let parsed_tag: String = match (iou_key, tag) {
                    (Some(iou_key), Some(tag)) => ui::args::parse_arg(&(iou_key + &tag), "keytag")?,
                    (None, None) => select::keytag(&tx)?,
                    _ => unreachable!("clap's requires= ensures both-or-neither"),
                };
                let tag_display = parsed_tag.to_string();
                let dropped = tx.drop_open(&parsed_tag).is_some();
                finish(
                    &ctx,
                    &tx,
                    json!({"status": ui::output::drop_status(dropped), "tag": tag_display}),
                )
            }
            Cmd::Clear => {
                let mut tx = ctx.load_tx()?;
                tx.drop_all_intents();
                tx.drop_all_opens();
                finish(&ctx, &tx, json!({"status": "cleared"}))
            }
            Cmd::Submit => {
                let mut session = session::build(&ctx.config.session, false).await?;
                let mut tx = ctx.load_tx()?;

                let mut built = session.build_tx(&mut tx)?;
                ctx.config
                    .keyring
                    .clone()
                    .build()
                    .sign(&mut built)
                    .context("signing required signatories")?;

                let id = session.sign_and_submit(built).await?;
                ui::output::print_json(json!({"status": "submitted", "id": id.to_string()}))?;
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
    ui::output::print_json(status)?;
    ctx.save_tx(tx)
}
