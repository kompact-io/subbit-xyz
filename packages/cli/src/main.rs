use std::path::PathBuf;

use clap::Parser;

mod cache;
mod cmd;
mod config;
mod ctx;
mod session;

use cmd::Cmd;

#[derive(Parser)]
#[command(
    name = "subbit",
    about = "Manage subbit sessions, keyrings, and staged transactions"
)]
struct Cli {
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
        self.cmd.run(self.config).await
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    Cli::parse().run().await
}
