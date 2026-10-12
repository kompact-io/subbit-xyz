use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response as AxumResponse},
    routing::{get, post},
};
use clap::{Parser, Subcommand};

use subbit_server::{Backing, Config, Ctx, Keytag, wire::Wire};

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
        let app = router(AppState { ctx });
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

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    init_tracing();
    let cli = Cli::parse();
    let sources = cli
        .config
        .into_sources(std::path::Path::new("subbit-server-config.toml"));
    cli.command.run(sources).await
}

/// This boundary's whole job: axum extractors in, `Ctx`'s dialect-
/// agnostic business logic (`Ctx::handle_spend`/`Ctx::handle_refund`)
/// does everything else, and `into_axum_response` turns what it
/// returns back into a real HTTP response. Dialect selection, catalog/
/// cost lookups, and wire framing all live on `Ctx` and
/// `subbit_server::dialect`, not here.
#[derive(Clone)]
struct AppState {
    ctx: Arc<Ctx>,
}

/// One shared resource-guard path per verb, not a route tree per
/// dialect: `Ctx::handle_spend`/`Ctx::handle_refund` each select a
/// dialect from headers themselves. Admin routes are dialect-
/// independent and always mounted.
fn router(state: AppState) -> Router {
    let v1 = Router::<AppState>::new()
        .route("/spend/{*path}", get(handle_spend))
        .route("/refund/{*path}", get(handle_refund))
        .nest(
            "/a",
            Router::<AppState>::new()
                .route("/backings", post(handle_backings))
                .route("/ious", get(handle_ious)),
        );

    Router::<AppState>::new().nest("/v1", v1).with_state(state)
}

/// Both handlers below hand the whole `HeaderMap` to `Ctx` as-is, not
/// a fixed set of fields read out of it first: which headers matter
/// (`Content-Length` for `ByEgress`, `SUBBIT_COST` for `ByClaim`, ...)
/// is a cost-model concern, decided by `Catalog` (see
/// `Catalog::actual_cost`), not something this boundary pins down.
/// That's what keeps this signature stable as cost models change.
async fn handle_spend(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> AxumResponse {
    into_axum_response(state.ctx.handle_spend(&headers, &path))
}

async fn handle_refund(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> AxumResponse {
    into_axum_response(state.ctx.handle_refund(&headers, &path))
}

/// The one place a dialect-agnostic `Wire` becomes a real axum
/// response. Mechanical: nothing here decides *what* the response is.
fn into_axum_response(wire: Wire) -> AxumResponse {
    let mut response = (wire.status, wire.body).into_response();
    for (name, value) in wire.headers {
        response.headers_mut().insert(name, value);
    }
    response
}

async fn handle_backings(State(state): State<AppState>, body: Bytes) -> AxumResponse {
    match minicbor::decode::<BTreeMap<Keytag, Option<Backing>>>(&body) {
        Err(e) => (StatusCode::BAD_REQUEST, format!("err: {}", e)).into_response(),
        Ok(backings) => match state.ctx.apply_backings(backings) {
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("err: {}", e)).into_response(),
            Ok(_) => StatusCode::OK.into_response(),
        },
    }
}

async fn handle_ious(State(state): State<AppState>) -> AxumResponse {
    let Ok(ious) = state.ctx.ious() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "db error".to_string()).into_response();
    };
    let Ok(body) = minicbor::to_vec(ious) else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "encode failed".to_string(),
        )
            .into_response();
    };
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/cbor")],
        body,
    )
        .into_response()
}

// Config io
fn write_config(path: &std::path::Path) -> anyhow::Result<()> {
    let toml = toml::to_string_pretty(&Config::default())
        .map_err(|e| anyhow::anyhow!("serializing default config: {e}"))?;
    std::fs::write(path, toml)
        .map_err(|e| anyhow::anyhow!("writing config {}: {e}", path.display()))?;
    Ok(())
}
