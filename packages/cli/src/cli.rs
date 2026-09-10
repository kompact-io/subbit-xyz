use std::path::PathBuf;

use clap::{Parser, Subcommand};

use crate::{Config, Ctx, iou, keyring, session, tx};

#[derive(Parser)]
#[command(
    name = "subbit-cli",
    about = "Manage subbit sessions, keyrings, and staged transactions",
    version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")"),
)]
pub struct Cli {
    /// Path to the CLI's config file (connector/wallet settings, script
    /// host, delegations, keyring, and where the tx cache lives).
    #[arg(
        long,
        global = true,
        env = "SUBBIT_CONFIG",
        default_value = "subbit-cli-config.toml"
    )]
    config: PathBuf,

    #[command(subcommand)]
    cmd: Cmd,
}

impl Cli {
    pub async fn run(self) -> anyhow::Result<()> {
        if let Cmd::Init = self.cmd {
            Config::default().save(&self.config)?;
            println!("wrote a starter config to {}", self.config.display());
            return Ok(());
        }

        let ctx = Ctx::load(self.config)?;
        match self.cmd {
            Cmd::Init => unreachable!("handled above, before Ctx is loaded"),
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

#[derive(Subcommand)]
enum Cmd {
    /// Scaffold a new config file
    Init,
    /// Print the resolved config.
    Config,
    /// Manage the session (wallet, delegations, script host).
    #[command(subcommand)]
    Session(session::Cmd),
    /// Manage locally-held signing keys used for required-signer fields.
    #[command(subcommand)]
    Keyring(keyring::Cmd),
    /// Iou commands
    #[command(subcommand)]
    Iou(iou::Cmd),
    // Iteratively stage and submit a subbit transaction.
    #[command(subcommand)]
    Tx(tx::Cmd),
}
