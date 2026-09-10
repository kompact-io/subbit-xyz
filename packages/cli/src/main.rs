use clap::Parser;

mod cache;

mod config;
pub use config::Config;

mod ctx;
pub use ctx::Ctx;

mod json_label;
mod session;

mod ui;

mod cli;
use cli::Cli;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    Cli::parse().run().await
}
