//! Bearer token authentication middleware.
//!
//! Design (vera-serve-design.md §2):
//! - `VERA_SERVE_API_KEY` env var → enforce Bearer on all connections.
//! - No key set → loopback-only; requests from non-loopback are rejected.
//! - Constant-time comparison to resist timing attacks.

use axum::{
    extract::Request,
    http::{header, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
};
use std::net::SocketAddr;

/// Axum middleware layer for Bearer auth.
///
/// If `api_key` is `None`, the config validator should have refused non-loopback
/// binding; we still enforce loopback here as a defence-in-depth check.
pub async fn auth_middleware(
    api_key: Option<String>,
    peer_addr: Option<axum::extract::ConnectInfo<SocketAddr>>,
    req: Request,
    next: Next,
) -> Response {
    // If no API key: only allow loopback.
    if api_key.is_none() {
        if let Some(axum::extract::ConnectInfo(addr)) = peer_addr {
            if !addr.ip().is_loopback() {
                return (StatusCode::FORBIDDEN, "loopback only without VERA_SERVE_API_KEY")
                    .into_response();
            }
        }
        return next.run(req).await;
    }

    // API key present: require matching Bearer token.
    let expected = api_key.unwrap();
    match req.headers().get(header::AUTHORIZATION) {
        Some(val) => {
            let val = val.to_str().unwrap_or("");
            let token = val.strip_prefix("Bearer ").unwrap_or("");
            if constant_time_eq(token.as_bytes(), expected.as_bytes()) {
                next.run(req).await
            } else {
                (StatusCode::UNAUTHORIZED, "invalid Bearer token").into_response()
            }
        }
        None => (StatusCode::UNAUTHORIZED, "missing Authorization header").into_response(),
    }
}

/// Constant-time byte comparison (resist timing attacks on the key).
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff: u8 = 0;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constant_time_eq_basics() {
        assert!(constant_time_eq(b"hello", b"hello"));
        assert!(!constant_time_eq(b"hello", b"world"));
        assert!(!constant_time_eq(b"hi", b"hello"));
    }
}
