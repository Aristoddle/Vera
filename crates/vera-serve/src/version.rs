//! /v1/version endpoint.
//!
//! Returns the crate version and fork identity so callers can confirm they
//! are hitting the Aristoddle fork (not upstream flier268/Vera).

use axum::{response::IntoResponse, Json};
use serde::Serialize;

/// Version response body.
#[derive(Serialize)]
pub struct VersionResponse {
    pub version: &'static str,
    pub fork: &'static str,
    pub crate_name: &'static str,
}

/// GET /v1/version handler.
pub async fn version_handler() -> impl IntoResponse {
    Json(VersionResponse {
        version: env!("CARGO_PKG_VERSION"),
        fork: "Aristoddle/Vera",
        crate_name: env!("CARGO_PKG_NAME"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_fields_populated() {
        let v = VersionResponse {
            version: "0.1.0",
            fork: "Aristoddle/Vera",
            crate_name: "vera-serve",
        };
        let json = serde_json::to_string(&v).unwrap();
        assert!(json.contains("Aristoddle/Vera"));
        assert!(json.contains("vera-serve"));
    }
}
