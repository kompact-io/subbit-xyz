use clap::Parser;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt::init();
    let cli = cardano_session::cli::Cli::parse();
    cardano_session::cli::run(cli).await
}
