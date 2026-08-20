use clap::Parser;

mod cli;
mod config;
mod proxy;

#[cfg(feature = "subbit")]
mod subbit;

use cli::Cli;

fn init_tracing() {
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

fn main() {
    init_tracing();
    Cli::parse().run();
}
