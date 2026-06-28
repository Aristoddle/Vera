//! Server configuration parsed from CLI args and environment.

use std::net::SocketAddr;

/// Parsed configuration for `vera serve`.
#[derive(Debug, Clone)]
pub struct ServeConfig {
    /// Bind address. Default: 127.0.0.1:8765.
    pub bind: SocketAddr,

    /// Idle timeout in seconds.
    /// 0  = cold load per request (no warm hold).
    /// N  = evict after N seconds idle (default: 300).
    /// -1 = keep-forever.
    pub idle_timeout_secs: i64,

    /// Maximum concurrent in-flight requests. Default: 2.
    pub max_concurrent: u32,

    /// Maximum inputs per /v1/embeddings or /v1/rerank request.
    pub max_inputs: usize,

    /// Maximum characters per individual input text.
    pub max_chars_per_input: usize,

    /// Maximum total characters across all inputs in one request.
    pub max_chars_total: usize,

    /// Maximum request body size in bytes (default: 4 MiB).
    pub max_body_bytes: usize,

    /// Optional API key from `VERA_SERVE_API_KEY` env var.
    /// None = loopback-only binding, no auth required on loopback.
    /// Some(key) = enforce Bearer auth on all connections.
    pub api_key: Option<String>,
}

impl Default for ServeConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8765".parse().expect("default bind is valid"),
            idle_timeout_secs: 300,
            max_concurrent: 2,
            max_inputs: 64,
            max_chars_per_input: 20_000,
            max_chars_total: 200_000,
            max_body_bytes: 4 * 1024 * 1024, // 4 MiB
            api_key: std::env::var("VERA_SERVE_API_KEY").ok(),
        }
    }
}

impl ServeConfig {
    /// Returns true if this config allows LAN (non-loopback) binding.
    pub fn allows_lan(&self) -> bool {
        !self.bind.ip().is_loopback()
    }

    /// Validate: refuse to bind LAN without an API key set.
    pub fn validate(&self) -> Result<(), String> {
        if self.allows_lan() && self.api_key.is_none() {
            return Err(format!(
                "refusing to bind to {} without VERA_SERVE_API_KEY: \
                 exposing an unauthenticated inference server on the LAN is unsafe. \
                 Either set VERA_SERVE_API_KEY or use --bind 127.0.0.1:{}",
                self.bind,
                self.bind.port()
            ));
        }
        Ok(())
    }
}
