//! /v1/health endpoint.
//!
//! Design (from vera-serve-design.md v5):
//! - Loopback/authenticated callers: full response including model_id, dim, embedding_profile_id.
//! - Non-loopback, unauthenticated: scrubbed response ({"status":"ok"} only).
//! - This prevents model fingerprinting by LAN scanners.

use serde::Serialize;

/// Full health response (for authenticated or loopback callers).
#[derive(Serialize)]
pub struct HealthFull {
    pub status: &'static str,
    pub model_id: String,
    pub dim: usize,
    pub embedding_profile_id: String,
    pub warm: bool,
    pub in_flight: u32,
    pub uptime_secs: u64,
    pub idle_timeout_secs: i64,
}

/// Scrubbed health response (for unauthenticated LAN callers).
#[derive(Serialize)]
pub struct HealthScrubbed {
    pub status: &'static str,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scrubbed_serializes_cleanly() {
        let s = HealthScrubbed { status: "ok" };
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, r#"{"status":"ok"}"#);
        // Must not leak any model info
        assert!(!json.contains("model"));
        assert!(!json.contains("dim"));
        assert!(!json.contains("embedding_profile"));
    }
}
