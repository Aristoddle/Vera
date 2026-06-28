//! Server startup — `run_server()` entry point.
//!
//! Wires config → ModelSlot → axum listener.
//! Phase 2: add graceful shutdown (SIGTERM/SIGINT → drain in-flight → exit).

use crate::{
    config::ServeConfig,
    lease::spawn_eviction_task,
    model_slot::ModelSlot,
    routes::{build_router, AppState},
};
use anyhow::Result;
use std::sync::Arc;
use tokio::net::TcpListener;

/// Start the vera serve HTTP server.
///
/// Validates config, binds the TCP listener, spawns the eviction task,
/// and serves until the process exits.
pub async fn run_server(config: ServeConfig) -> Result<()> {
    // Validate before binding — refuse LAN bind without API key.
    config.validate().map_err(|e| anyhow::anyhow!(e))?;

    let bind_addr = config.bind;
    let idle_timeout = config.idle_timeout_secs;
    let config = Arc::new(config);

    // Create a fresh model slot (empty, will load on first request).
    let slot = ModelSlot::new();
    spawn_eviction_task(Arc::clone(&slot), idle_timeout);

    let state = AppState {
        config: Arc::clone(&config),
        slot,
        started_at: std::time::Instant::now(),
    };

    let router = build_router(state);
    let listener = TcpListener::bind(bind_addr).await?;

    tracing::info!("vera serve listening on {bind_addr}");
    axum::serve(listener, router).await?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_is_loopback() {
        let cfg = ServeConfig::default();
        assert!(cfg.bind.ip().is_loopback());
        assert!(!cfg.allows_lan());
    }
}
