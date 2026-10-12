use axum::{Router, http::HeaderValue, response::IntoResponse, routing::post};
use clap::Parser;

#[derive(Parser)]
struct Cli {
    #[arg(long, env = "ECHO_SERVER_ADDR", default_value = "0.0.0.0")]
    addr: String,
    #[arg(long, env = "ECHO_SERVER_PORT", default_value_t = 3246)]
    port: u16,
}

async fn run(addr: String, port: u16) {
    let bind = format!("{addr}:{port}");
    let listener = tokio::net::TcpListener::bind(&bind)
        .await
        .expect("bind failed");
    let app = Router::new()
        .route("/echo", post(echo))
        .route("/echo/egress", post(echo))
        .route("/echo/claim", post(echo_claim));
    tracing::info!(listen=%bind, "Starting");
    axum::serve(listener, app).await.expect("server error");
}

async fn echo(body: axum::body::Bytes) -> axum::body::Bytes {
    tracing::info!(body_len = body.len(), "request in");
    tracing::info!(status = 200, body_len = body.len(), "response given");
    body
}

/// As `echo`, but also reports a claimed cost: standing in for a real
/// per-request usage metric (e.g. AI billing on internal tokens),
/// which is exactly what `Cost::ByClaim` is for - the provider's own
/// number, not independently verifiable by the consumer.
async fn echo_claim(body: axum::body::Bytes) -> axum::response::Response {
    tracing::info!(body_len = body.len(), "request in");
    let claimed = 1 + body.len() as u64 / 4;
    tracing::info!(
        status = 200,
        body_len = body.len(),
        claimed,
        "response given"
    );
    let mut resp = body.into_response();
    resp.headers_mut().insert(
        subbit_l2_dto::CLAIM_COST_HEADER,
        HeaderValue::from_str(&claimed.to_string()).expect("digits are valid header bytes"),
    );
    resp
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
async fn main() {
    init_tracing();
    let cli = Cli::parse();
    run(cli.addr, cli.port).await;
}
