mod iou;
mod keyring;
mod session;
mod tx;

use std::path::PathBuf;

use anyhow::Result;
use clap::Subcommand;

use crate::config::Config;
use crate::ctx::Ctx;

#[derive(Subcommand)]
pub enum Cmd {
    /// Scaffold a new config file
    Init,
    /// Print the resolved config.
    Config,
    /// Manage the session (wallet, delegations, script host).
    #[command(subcommand)]
    Session(session::SessionCmd),
    /// Manage locally-held signing keys used for required-signer fields.
    #[command(subcommand)]
    Keyring(keyring::Cmd),
    /// Iou commands
    #[command(subcommand)]
    Iou(iou::Cmd),
    /// Iteratively stage and submit a subbit transaction.
    #[command(subcommand)]
    Tx(tx::Cmd),
}

impl Cmd {
    pub async fn run(self, path: PathBuf) -> Result<()> {
        if let Cmd::Init = self {
            Config::default().save(&path)?;
            println!("wrote a starter config to {}", path.display());
            return Ok(());
        }

        let ctx = Ctx::load(path)?;
        match self {
            Cmd::Init => unreachable!(),
            Cmd::Config => {
                println!("{}", serde_json::to_string_pretty(&ctx.config)?);
                Ok(())
            }
            Cmd::Session(cmd) => cmd.run(ctx).await,
            Cmd::Keyring(cmd) => cmd.run(ctx),
            Cmd::Iou(cmd) => cmd.run(&ctx),
            Cmd::Tx(cmd) => cmd.run(ctx).await,
        }
    }
}
