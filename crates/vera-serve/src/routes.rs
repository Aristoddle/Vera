//! axum router — wires all /v1/* endpoints.
//!
//! Routes (vera-serve-design.md §4):
//! - POST /v1/embeddings  — OpenAI-compatible embedding request
//! - POST /v1/rerank      — reranking request
//! - GET  /v1/health      — liveness + model state (scrubbed for unauthenticated LAN)
//! - GET  /v1/version     — fork identity and version
//!
//! Phase 2: embedding and reranking handlers are stubs that return 501.
//! ModelSlot acquire + vera-core integration lands in the next pass.

use crate::{config::ServeConfig, health::HealthScrubbed, model_slot::ModelSlot, version::version_handler};
use axum::{
    extract::{Json, State},
    http::StatusCode,
    response::IntoResponse,
    routing::{get, post},
    Router,
};
use serde::Deserialize;
use std::sync::Arc;
use tower_http::limit::RequestBodyLimitLayer;

/// Shared application state threaded through all handlers.
#[derive(Clone)]
pub struct AppState {
    pub config: Arc<ServeConfig>,
    pub slot: Arc<ModelSlot>,
    pub started_at: std::time::Instant,
}

/// POST /v1/embeddings request body (OpenAI-compatible subset).
#[derive(Deserialize)]
pub struct EmbeddingsRequest {
    pub input: EmbeddingsInput,
    pub model: Option<String>,
}

#[derive(Deserialize)]
#[serde(untagged)]
pub enum EmbeddingsInput {
    Single(String),
    Batch(Vec<String>),
}

impl EmbeddingsInput {
    pub fn into_vec(self) -> Vec<String> {
        match self {
            Self::Single(s) => vec![s],
            Self::Batch(v) => v,
        }
    }
}

/// POST /v1/rerank request body.
#[derive(Deserialize)]
pub struct RerankRequest {
    pub query: String,
    pub documents: Vec<String>,
    pub model: Option<String>,
}

/// Build the axum Router with all routes and middleware.
pub fn build_router(state: AppState) -> Router {
    let body_limit = state.config.max_body_bytes;

    Router::new()
        .route("/v1/embeddings", post(embeddings_handler))
        .route("/v1/rerank", post(rerank_handler))
        .route("/v1/health", get(health_handler))
        .route("/v1/version", get(version_handler))
        .layer(RequestBodyLimitLayer::new(body_limit))
        .with_state(state)
}

/// POST /v1/embeddings — Phase 2 stub.
async fn embeddings_handler(
    State(state): State<AppState>,
    Json(req): Json<EmbeddingsRequest>,
) -> impl IntoResponse {
    let inputs = req.input.into_vec();
    if let Err(e) = crate::caps::validate_inputs(&inputs, &state.config) {
        return (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response();
    }
    // Phase 2: acquire lease, call vera-core, return embeddings array.
    (
        StatusCode::NOT_IMPLEMENTED,
        "vera-serve Phase 2: embedding generation not yet wired to vera-core",
    )
        .into_response()
}

/// POST /v1/rerank — Phase 2 stub.
async fn rerank_handler(
    State(state): State<AppState>,
    Json(req): Json<RerankRequest>,
) -> impl IntoResponse {
    if let Err(e) = crate::caps::validate_inputs(&req.documents, &state.config) {
        return (StatusCode::UNPROCESSABLE_ENTITY, e.to_string()).into_response();
    }
    // Phase 2: acquire lease, call vera-core reranker, return scored list.
    (
        StatusCode::NOT_IMPLEMENTED,
        "vera-serve Phase 2: reranking not yet wired to vera-core",
    )
        .into_response()
}

/// GET /v1/health — always responds; scrubs model info for unauthenticated LAN callers.
async fn health_handler(State(_state): State<AppState>) -> impl IntoResponse {
    // Phase 2: distinguish loopback vs LAN via ConnectInfo; return HealthFull when authed.
    // For now, always return scrubbed response (safe default).
    Json(HealthScrubbed { status: "ok" })
}
