//! HTTP boundary: axum extractors in, `Ctx`'s dialect-agnostic
//! business logic (`Ctx::handle_spend`/`Ctx::handle_refund`) does
//! everything else, `into_axum_response` turns what it returns back
//! into a real HTTP response. Dialect selection, catalog/cost
//! lookups, and wire framing all live on `Ctx` and
//! `subbit_gate::dialect`, not here.
//!
//! Lives in the library (not `main.rs`) so a bench can mount the
//! real, unmodified router in-process -- see
//! `benches/worker_threads.rs`.

use std::{collections::BTreeMap, sync::Arc};

use axum::{
    Router,
    body::Bytes,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response as AxumResponse},
    routing::{get, post},
};
use tokio::sync::Semaphore;

use crate::{Backing, Ctx, Keytag, wire::Wire};

#[derive(Clone)]
pub struct AppState {
    pub ctx: Arc<Ctx>,
    /// Caps how many `handle_spend`/`handle_refund` calls run on
    /// `spawn_blocking` OS threads at once. Profiling (back-of-envelope
    /// calc in `results.md`) found tokio's blocking pool otherwise
    /// grows to 200+ OS threads at concurrency=160 on a 12-core host --
    /// real oversubscription, measured (not theorized): CPU time
    /// spreads almost perfectly evenly across all of them (top 5
    /// busiest threads = ~7% of total CPU), and the same process
    /// burns the same ~630%/700% CPU budget for roughly half the
    /// throughput once a run's been going a while, vs. a fresh start --
    /// a growing-thread-population cost, not a fixed per-request one.
    /// Bounding concurrent blocking work to roughly core count is the
    /// direct fix for that specific mechanism.
    pub blocking_limit: Arc<Semaphore>,
}

impl AppState {
    pub fn new(ctx: Arc<Ctx>) -> Self {
        let cores = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        Self {
            ctx,
            blocking_limit: Arc::new(Semaphore::new(cores)),
        }
    }
}

/// One shared resource-guard path per verb, not a route tree per
/// dialect: `Ctx::handle_spend`/`Ctx::handle_refund` each select a
/// dialect from headers themselves. Admin routes are dialect-
/// independent and always mounted.
pub fn router(state: AppState) -> Router {
    let v1 = Router::<AppState>::new()
        .route("/spend/{*path}", get(handle_spend))
        .route("/refund/{*path}", get(handle_refund))
        .route("/terms", get(handle_terms))
        .nest(
            "/a",
            Router::<AppState>::new()
                .route("/backings", post(handle_backings))
                .route("/index", post(handle_index))
                .route("/ious", get(handle_ious))
                .route("/channels", get(handle_channels)),
        );

    Router::<AppState>::new()
        .nest("/v1", v1)
        // Same handler as `/v1/terms`, under the conventional
        // discovery path too, so a consumer can hit either without
        // a proxy translating between them.
        .route(subbit_l2_dto::WELL_KNOWN_PATH, get(handle_terms))
        .with_state(state)
}

/// Both handlers below hand the whole `HeaderMap` to `Ctx` as-is, not
/// a fixed set of fields read out of it first: which headers matter
/// (`Content-Length` for `ByEgress`, `SUBBIT_COST` for `ByClaim`, ...)
/// is a cost-model concern, decided by `Catalog` (see
/// `Catalog::actual_cost`), not something this boundary pins down.
/// That's what keeps this signature stable as cost models change.
///
/// Run on `spawn_blocking` rather than called inline: `Ctx::handle_spend`/
/// `Ctx::handle_refund` are synchronous all the way down (crypto
/// verify, the `Db` write), and tokio's async worker pool is small and
/// fixed (roughly core count) -- an inline call there blocks that
/// worker from servicing any other task for the call's duration.
/// `benches/worker_threads.rs`'s Test B measured this as a real,
/// consistent ~40% throughput improvement -- not the dominant factor
/// in any real-world ceiling measured so far, but worth taking for
/// free.
async fn handle_spend(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> AxumResponse {
    run_spend_blocking(
        state,
        path,
        "handle_spend",
        move |ctx, headers, path| ctx.handle_spend(headers, path),
        headers,
    )
    .await
}

async fn handle_refund(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> AxumResponse {
    run_spend_blocking(
        state,
        path,
        "handle_refund",
        move |ctx, headers, path| ctx.handle_refund(headers, path),
        headers,
    )
    .await
}

/// Shared body of `handle_spend`/`handle_refund`: acquire a
/// `blocking_limit` permit, root `path` (axum's `{*path}` wildcard
/// strips the leading slash -- matching matchit's own catch-all
/// semantics, see `subbit_cost::Catalog`'s own doc comment -- but
/// every resource path `Ctx`/`Catalog` deal in is leading-slash-
/// rooted; this is the one boundary responsible for that conversion),
/// then run `f` on `spawn_blocking`. See `handle_spend`'s own doc
/// comment on why this runs off the async worker pool at all.
async fn run_spend_blocking(
    state: AppState,
    path: String,
    panic_label: &'static str,
    f: impl FnOnce(&Ctx, &HeaderMap, &str) -> Wire + Send + 'static,
    headers: HeaderMap,
) -> AxumResponse {
    let permit = state
        .blocking_limit
        .clone()
        .acquire_owned()
        .await
        .expect("blocking_limit semaphore never closes");
    let ctx = state.ctx;
    let path = format!("/{path}");
    into_axum_response(
        tokio::task::spawn_blocking(move || {
            let _permit = permit;
            f(&ctx, &headers, &path)
        })
        .await
        .unwrap_or_else(|_| panic!("{panic_label} panicked")),
    )
}

/// Plain JSON `Terms`, no headers required, no envelope: a consumer
/// needs this before it can even address a resource.
async fn handle_terms(State(state): State<AppState>) -> AxumResponse {
    axum::Json(state.ctx.terms().clone()).into_response()
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

/// Distinguishes a malformed request (400) from a real server-side
/// failure (500) while still letting `#[instrument(err)]` log either
/// uniformly -- the two `handle_*` bodies below just match on variant.
#[derive(Debug, thiserror::Error)]
enum AdminError {
    #[error("decode: {0}")]
    Decode(#[from] minicbor::decode::Error),
    #[error("apply: {0}")]
    Apply(#[from] crate::db::Error),
}

impl AdminError {
    fn status(&self) -> StatusCode {
        match self {
            Self::Decode(_) => StatusCode::BAD_REQUEST,
            Self::Apply(_) => StatusCode::INTERNAL_SERVER_ERROR,
        }
    }
}

#[tracing::instrument(skip_all, fields(count), err(Debug))]
fn decode_and_apply_backings(state: &AppState, body: &Bytes) -> Result<usize, AdminError> {
    let backings = minicbor::decode::<BTreeMap<Keytag, Option<Backing>>>(body)?;
    let count = backings.len();
    tracing::Span::current().record("count", count);
    state.ctx.apply_backings(backings)?;
    Ok(count)
}

async fn handle_backings(State(state): State<AppState>, body: Bytes) -> AxumResponse {
    match decode_and_apply_backings(&state, &body) {
        Err(e) => (e.status(), format!("err: {}", e)).into_response(),
        Ok(_) => StatusCode::OK.into_response(),
    }
}

/// The real index pipeline's target: a full `subbit_index_dto::Output`
/// lineage report, from which this server does its own "Downstream"
/// choosing (see `index_output`'s module doc). `/backings` stays
/// mounted alongside this, unchanged, for the simpler naive-index
/// path (no lineage, no server-side choosing needed).
#[tracing::instrument(skip_all, fields(channels), err(Debug))]
fn decode_and_apply_index(state: &AppState, body: &Bytes) -> Result<usize, AdminError> {
    let output = minicbor::decode::<subbit_index_dto::Output>(body)?;
    let channels = output.channels.len();
    tracing::Span::current().record("channels", channels);
    state.ctx.apply_output(output)?;
    Ok(channels)
}

async fn handle_index(State(state): State<AppState>, body: Bytes) -> AxumResponse {
    match decode_and_apply_index(&state, &body) {
        Err(e) => (e.status(), format!("err: {}", e)).into_response(),
        Ok(_) => StatusCode::OK.into_response(),
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

async fn handle_channels(State(state): State<AppState>) -> AxumResponse {
    let Ok(summaries) = state.ctx.channel_summaries() else {
        return (StatusCode::INTERNAL_SERVER_ERROR, "db error".to_string()).into_response();
    };
    let Ok(body) = minicbor::to_vec(summaries) else {
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
