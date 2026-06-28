//! vera serve — warm inference server for embeddings and reranking.
//!
//! Exposes an OpenAI-compatible HTTP surface:
//! - `POST /v1/embeddings`
//! - `POST /v1/rerank`
//! - `GET  /v1/health`
//! - `GET  /v1/version`
//!
//! See `docs/plans/vera-serve-design.md` for the full design (v5, all reviews passed).
//! Key design decisions baked into this scaffold:
//! - `ModelSlot<Mutex<SlotState>>` with explicit `in_flight` atomic + RAII lease guard
//! - `in_flight` incremented INSIDE the mutex before state becomes visible to eviction
//! - Release: `last_used` updated BEFORE `in_flight` decrement — both under mutex
//! - Default idle-timeout: 300s (warm). 0 = cold, -1 = forever.
//! - Auth: `VERA_SERVE_API_KEY` env var; unset = loopback-only (refuse LAN bind without key).
//! - Request caps: max-inputs 64, max-chars-per-input 20_000, max-chars-total 200_000, body 4MiB.
//! - Cancellation: per-request AbortToken via drop guard; in-flight chunk completes (≤32 inputs).

pub mod auth;
pub mod caps;
pub mod config;
pub mod health;
pub mod lease;
pub mod model_slot;
pub mod routes;
pub mod serve;
pub mod version;

pub use config::ServeConfig;
pub use serve::run_server;
