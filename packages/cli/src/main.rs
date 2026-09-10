use clap::Parser;

mod config;
pub use config::Config;

mod cli;
use cli::Cli;

mod ctx;
pub use ctx::Ctx;

mod iou;
mod keyring;
mod session;
mod tx;

mod cache;
mod json_label;
mod time;
mod ui;
pub use time::now;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();

    Cli::parse().run().await
}
