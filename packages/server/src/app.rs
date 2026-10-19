//! HTTP boundary: axum extractors in, `Ctx`'s dialect-agnostic
//! business logic (`Ctx::handle_spend`/`Ctx::handle_refund`) does
//! everything else, `into_axum_response` turns what it returns back
//! into a real HTTP response. Dialect selection, catalog/cost
//! lookups, and wire framing all live on `Ctx` and
//! `subbit_server::dialect`, not here.
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

use crate::{Backing, Ctx, Keytag, wire::Wire};

#[derive(Clone)]
pub struct AppState {
    pub ctx: Arc<Ctx>,
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
                .route("/ious", get(handle_ious))
                .route("/channels", get(handle_channels)),
        );

    Router::<AppState>::new()
        .nest("/v1", v1)
        // Same handler as `/v1/terms`, under the conventional
        // discovery path too, so a consumer can hit either without
        // a proxy translating between them.
        .route("/.well-known/subbit", get(handle_terms))
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
/// consistent ~40% throughput improvement (not the dominant factor in
/// any real-world ceiling measured so far, see `plan-fix-throughput.md`,
/// but worth taking for free).
async fn handle_spend(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> AxumResponse {
    into_axum_response(
        tokio::task::spawn_blocking(move || state.ctx.handle_spend(&headers, &path))
            .await
            .expect("handle_spend panicked"),
    )
}

async fn handle_refund(
    State(state): State<AppState>,
    Path(path): Path<String>,
    headers: HeaderMap,
) -> AxumResponse {
    into_axum_response(
        tokio::task::spawn_blocking(move || state.ctx.handle_refund(&headers, &path))
            .await
            .expect("handle_refund panicked"),
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
