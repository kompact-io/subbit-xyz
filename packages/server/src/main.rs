use std::net::SocketAddr;
use std::sync::Arc;

use clap::{Parser, Subcommand};

use subbit_server::{
    Config, Ctx,
    app::{self, AppState},
};

#[derive(Parser, Debug)]
#[command(
    name = "subbit-server",
    about = "Manage server side L2 ops. Runs as a 'sidecar'",
    version = concat!(env!("CARGO_PKG_VERSION"), " (", env!("GIT_HASH"), ")"),
)]
struct Cli {
    #[command(flatten)]
    config: subbit_config::Args,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Write a starter config.toml (default values) to --config and exit
    Init {
        /// Overwrite the file if it already exists
        #[arg(long)]
        force: bool,
    },
    /// Run the server
    Serve {
        /// Address to bind the HTTP server on
        #[arg(long, default_value = "127.0.0.1:7822")]
        listen: SocketAddr,
    },
}

impl Command {
    async fn run(self, sources: subbit_config::Sources<'_>) -> anyhow::Result<()> {
        if let Command::Init { force } = self {
            return Self::init(sources.base, force);
        }
        let config: Config = sources.load()?;
        match self {
            Command::Init { .. } => unreachable!(),
            Command::Serve { listen } => Self::serve(config, listen).await,
        }
    }

    fn init(base: &std::path::Path, force: bool) -> anyhow::Result<()> {
        if base.exists() && !force {
            anyhow::bail!(
                "{} already exists, pass --force to overwrite",
                base.display()
            );
        }
        write_config(base)?;
        println!("wrote starter config to {}", base.display());
        Ok(())
    }

    async fn serve(config: Config, listen: SocketAddr) -> anyhow::Result<()> {
        let ctx = Arc::new(Ctx::from_config(config)?);
        let app = app::router(AppState { ctx });
        let listener = tokio::net::TcpListener::bind(listen).await?;
        tracing::info!(%listen, "Starting");
        axum::serve(listener, app).await?;
        Ok(())
    }
}

fn init_tracing() {
    let filter =
        tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into());
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

/// Built by hand rather than `#[tokio::main]` so worker-thread count
/// is overridable at runtime (`TOKIO_WORKER_THREADS`), without a
/// rebuild -- needed to sweep it while diagnosing the throughput
/// ceiling described in `plan-fix-throughput.md`. Unset, this matches
/// `#[tokio::main]`'s own default (one worker per core).
fn build_runtime() -> std::io::Result<tokio::runtime::Runtime> {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    if let Some(n) = std::env::var("TOKIO_WORKER_THREADS")
        .ok()
        .and_then(|n| n.parse::<usize>().ok())
    {
        builder.worker_threads(n);
    }
    builder.enable_all().build()
}

fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let sources = cli
        .config
        .into_sources(std::path::Path::new("subbit-server-config.toml"));
    build_runtime()?.block_on(cli.command.run(sources))
}

// Config io
fn write_config(path: &std::path::Path) -> anyhow::Result<()> {
    let toml = toml::to_string_pretty(&Config::default())
        .map_err(|e| anyhow::anyhow!("serializing default config: {e}"))?;
    std::fs::write(path, toml)
        .map_err(|e| anyhow::anyhow!("writing config {}: {e}", path.display()))?;
    Ok(())
}
