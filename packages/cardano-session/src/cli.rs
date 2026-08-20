use clap::{Parser, Subcommand};

pub mod commands;

#[derive(Debug, Parser)]
#[command(
    name = "cardano-session",
    about = "Drive a CardanoSession from the command line"
)]
pub struct Cli {
    /// Path to a session config file.
    #[arg(long, default_value = "cardano-session-config.toml")]
    pub config: std::path::PathBuf,

    /// Skip refreshing the wallet/tip from the connector - use whatever
    /// was cached in the tip file (or nothing, if there wasn't one).
    #[arg(long)]
    pub skip_refresh: bool,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Write a default config to `--config`.
    Init {
        /// Overwrite the file if it already exists.
        #[arg(long)]
        force: bool,
    },
    /// Show the wallet's verification key, address, fuel, and ref scripts.
    Wallet,
    /// Upload a script as a reference script at the wallet address.
    Upload {
        /// Path to the script file. Format TBD - see `commands::upload`.
        script: std::path::PathBuf,
        /// Wait for the upload tx to land before returning.
        #[arg(long)]
        wait: bool,
    },
    /// Spend a reference script back into the wallet. Inverse of `upload`.
    Teardown {
        /// Script hash to tear down, as hex.
        hash: String,
        /// Wait for the teardown tx to land before returning.
        #[arg(long)]
        wait: bool,
    },
    /// Wait for a tx id to land in the wallet.
    WaitId { id: String },
    /// Fetch and show current UTXOs at an address; starts tracking it in
    /// `tip` as a side effect.
    UtxosAt { address: String },
}

// TODO: this always talks to `Session<Blockfrost>`. If the CLI should ever
// support a different connector, `connector::Config` needs an enum-dispatch
// story here rather than `Session::init` being hardwired to Blockfrost.
pub async fn run(cli: Cli) -> anyhow::Result<()> {
    // `init` doesn't need a config to already exist, let alone a live
    // session - handle it before either is built.
    if let Command::Init { force } = &cli.command {
        return commands::init(&cli.config, *force);
    }

    let config = commands::load_config(&cli.config)?;
    let mut session = crate::Session::init(config).await?;

    if let Some(tip) = commands::load_tip()? {
        session.load_tip(tip);
    }

    if !cli.skip_refresh {
        // `refresh_all` re-covers whatever `reload` just tracked, so the
        // wallet gets fetched twice here - harmless, not worth the extra
        // plumbing to avoid in a CLI that runs once and exits.
        session.reload().await?;
        session.refresh_all().await?;
    }

    let result = match cli.command {
        Command::Init { .. } => unreachable!("handled above"),
        Command::Wallet => commands::wallet(&session),
        Command::Upload { script, wait } => commands::upload(&mut session, &script, wait).await,
        Command::Teardown { hash, wait } => commands::teardown(&mut session, &hash, wait).await,
        Command::WaitId { id } => commands::wait_id(&mut session, &id).await,
        Command::UtxosAt { address } => commands::utxos_at(&mut session, &address).await,
    };

    // Best-effort: a caching failure shouldn't mask the command's own
    // result, so it's reported but doesn't override `result`.
    if let Err(e) = commands::save_tip(session.tip()) {
        eprintln!("warning: failed to cache tip: {e}");
    }

    result
}
