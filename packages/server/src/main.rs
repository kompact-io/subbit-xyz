use std::{collections::BTreeMap, net::SocketAddr, sync::Arc};

use axum::{
    Router,
    body::Bytes,
    extract::{Extension, Path, Request as HttpRequest, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response as AxumResponse},
    routing::{get, post},
};
use clap::{Parser, Subcommand};
use subbit_core::envelope::{Request, Response};
use tracing_subscriber::EnvFilter;

use subbit_server::{Backing, Config, Ctx, Keytag};

#[derive(Parser, Debug)]
#[command(name = "subbit", about = "Subbit", version)]
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
                "{} already exists — pass --force to overwrite",
                base.display()
            );
        }
        write_config(base)?;
        println!("wrote starter config to {}", base.display());
        Ok(())
    }

    async fn serve(config: Config, listen: SocketAddr) -> anyhow::Result<()> {
        let ctx = Arc::new(Ctx::from_config(config)?);
        let app = router(ctx);
        tracing::info!(%listen, "starting subbit");
        let listener = tokio::net::TcpListener::bind(listen).await?;
        axum::serve(listener, app).await?;
        Ok(())
    }
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env())
        .init();

    let cli = Cli::parse();
    let sources = cli
        .config
        .into_sources(std::path::Path::new("subbit-server-config.toml"));
    cli.command.run(sources).await
}

fn router(ctx: Arc<Ctx>) -> Router {
    // Executive
    let x = Router::<Arc<Ctx>>::new()
        .route("/spend/{*path}", get(handle_spend))
        .route("/refund/{*path}", get(handle_refund))
        .layer(middleware::from_fn(require_request));

    // Open
    let o = Router::<Arc<Ctx>>::new();

    // Admin
    let a = Router::<Arc<Ctx>>::new()
        .route("/backings", post(handle_backings))
        .route("/ious", get(handle_ious));

    let v1 = Router::<Arc<Ctx>>::new()
        .nest("/x", x)
        .nest("/o", o)
        .nest("/a", a);

    Router::<Arc<Ctx>>::new().nest("/v1", v1).with_state(ctx)
}

async fn handle_backings(State(ctx): State<Arc<Ctx>>, body: Bytes) -> AxumResponse {
    match minicbor::decode::<BTreeMap<Keytag, Option<Backing>>>(&body) {
        Err(e) => (StatusCode::BAD_REQUEST, format!("err: {}", e)).into_response(),
        Ok(backings) => match ctx.apply_backings(backings) {
            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("err: {}", e)).into_response(),
            Ok(_) => StatusCode::OK.into_response(),
        },
    }
}

async fn handle_ious(State(ctx): State<Arc<Ctx>>) -> AxumResponse {
    let Ok(ious) = ctx.ious() else {
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

#[axum::debug_handler]
async fn handle_spend(
    State(ctx): State<Arc<Ctx>>,
    Path(path): Path<String>,
    Extension(req): Extension<Request>,
) -> AxumResponse {
    tracing::info!(method = "spend", path, "request in");
    respond("spend", &path, ctx.spend(req, &path))
}

#[axum::debug_handler]
async fn handle_refund(
    State(ctx): State<Arc<Ctx>>,
    Path(path): Path<String>,
    Extension(req): Extension<Request>,
) -> AxumResponse {
    tracing::info!(method = "refund", path, "request in");
    respond("refund", &path, ctx.refund(req, &path))
}

fn respond(method: &str, path: &str, result: Response) -> AxumResponse {
    let Ok(header_value) = HeaderValue::from_str(&subbit_core::base64::to_base64(&result)) else {
        tracing::error!(method, path, "failed to encode response envelope");
        return StatusCode::INTERNAL_SERVER_ERROR.into_response();
    };
    let status_code = match result {
        Ok(status) => status.status_code(),
        Err(err) => {
            tracing::warn!(method, path, error = ?err, "request failed");
            err.status_code()
        }
    };
    tracing::info!(
        method,
        path,
        status = status_code.as_u16(),
        "response given"
    );
    let mut response = status_code.into_response();
    response
        .headers_mut()
        .insert(HeaderName::from_static(RESPONSE_HEADER_NAME), header_value);
    response
}

const REQUEST_HEADER_NAME: &str = "subbit";

/// Middleware: decodes the `Request` out of the header once, before either
/// handler runs, and stashes it as a request extension. Missing/invalid
/// header short-circuits with 400 and neither handler is called.
async fn require_request(
    headers: HeaderMap,
    mut req: HttpRequest,
    next: Next,
) -> Result<AxumResponse, AxumResponse> {
    let raw = headers
        .get(REQUEST_HEADER_NAME)
        .and_then(|v| v.to_str().ok())
        .filter(|v| !v.is_empty())
        .ok_or_else(|| {
            tracing::warn!(
                method = "require_request",
                "missing {REQUEST_HEADER_NAME} header"
            );
            (
                StatusCode::PAYMENT_REQUIRED,
                format!("missing {REQUEST_HEADER_NAME} header"),
            )
                .into_response()
        })?;
    let Ok(request) = subbit_core::base64::from_base64::<Request>(raw) else {
        tracing::warn!(
            method = "require_request",
            "invalid {REQUEST_HEADER_NAME} header"
        );
        return Err((
            StatusCode::BAD_REQUEST,
            format!("invalid {REQUEST_HEADER_NAME} header"),
        )
            .into_response());
    };
    req.extensions_mut().insert(request);
    Ok(next.run(req).await)
}

const RESPONSE_HEADER_NAME: &str = "subbit";

// Config io
fn write_config(path: &std::path::Path) -> anyhow::Result<()> {
    let toml = toml::to_string_pretty(&Config::default())
        .map_err(|e| anyhow::anyhow!("serializing default config: {e}"))?;
    std::fs::write(path, toml)
        .map_err(|e| anyhow::anyhow!("writing config {}: {e}", path.display()))?;
    Ok(())
}
