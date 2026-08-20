//! Echo client. `init` writes a starter config; `send` posts data to
//! `{base_url}/echo`. Requests go through `subbit_issuer`'s metering
//! middleware whenever the config has an `[issuer]` section, and are sent
//! plain otherwise — same binary, no separate flag to remember.

mod client;
mod config;

use anyhow::Context;
use clap::{Parser, Subcommand};

use client::Client;
use config::Config;

static DEFAULT_CONFIG_PATH: &str = "echo-client-config.toml";

#[derive(Parser)]
struct Cli {
    #[command(flatten)]
    sources: subbit_config::Args,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Write a starter config to file
    Init,
    /// Send data to the echo server
    Send {
        /// data to send; use @file to read from a file
        #[arg(short = 'd', long = "data", default_value_t = default_data())]
        data: String,
    },
}

impl Cli {
    async fn run(self) -> anyhow::Result<()> {
        let sources = self
            .sources
            .into_sources(std::path::Path::new(DEFAULT_CONFIG_PATH));

        if let Command::Init = self.command {
            Config::write_default(sources.base)?;
            println!("wrote starter config to {}", sources.base.display());
            return Ok(());
        }

        let config: Config = sources.load()?;

        match self.command {
            Command::Init => unreachable!(),
            Command::Send { data } => send(config, data).await,
        }
    }
}

async fn send(config: Config, data: String) -> anyhow::Result<()> {
    let body: Vec<u8> = match data.strip_prefix('@') {
        Some(path) => std::fs::read(path)?,
        None => data.into_bytes(),
    };
    serde_json::from_slice::<serde_json::Value>(&body).context("--data is not valid JSON")?;
    let base_url = reqwest::Url::parse(&config.base_url).context("config base_url is invalid")?;
    let resp = Client::build(&config)?.echo(&base_url, body).await?;
    println!("{resp}");
    Ok(())
}

fn init_tracing() {
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    Cli::parse().run().await
}

fn default_data() -> String {
    let user = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "world".into());
    format!("\"hello, {user}\"")
}
