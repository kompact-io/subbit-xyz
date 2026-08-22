use axum::{Router, routing::post};
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
    let app = Router::new().route("/echo", post(echo));
    tracing::info!(listen=%bind, "Starting");
    axum::serve(listener, app).await.expect("server error");
}

async fn echo(body: axum::body::Bytes) -> axum::body::Bytes {
    tracing::info!(body_len = body.len(), "request in");
    tracing::info!(status = 200, body_len = body.len(), "response given");
    body
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
