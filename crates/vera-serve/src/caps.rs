//! Request capability enforcement (caps).
//!
//! Design limits (vera-serve-design.md §3):
//! - max_inputs: 64
//! - max_chars_per_input: 20_000
//! - max_chars_total: 200_000
//! - max_body_bytes: 4 MiB (enforced by tower-http RequestBodyLimitLayer)
//!
//! This module validates the *decoded* request payload against the caps.

use crate::config::ServeConfig;

/// Validation error returned when a request exceeds caps.
#[derive(Debug, thiserror::Error)]
pub enum CapsError {
    #[error("too many inputs: got {got}, max {max}")]
    TooManyInputs { got: usize, max: usize },

    #[error("input {idx} too long: {chars} chars, max {max}")]
    InputTooLong { idx: usize, chars: usize, max: usize },

    #[error("total input chars {total} exceeds limit {max}")]
    TotalTooLong { total: usize, max: usize },
}

/// Validate a slice of input strings against config caps.
pub fn validate_inputs(inputs: &[String], cfg: &ServeConfig) -> Result<(), CapsError> {
    if inputs.len() > cfg.max_inputs {
        return Err(CapsError::TooManyInputs {
            got: inputs.len(),
            max: cfg.max_inputs,
        });
    }

    let mut total_chars = 0usize;
    for (idx, s) in inputs.iter().enumerate() {
        let chars = s.chars().count();
        if chars > cfg.max_chars_per_input {
            return Err(CapsError::InputTooLong {
                idx,
                chars,
                max: cfg.max_chars_per_input,
            });
        }
        total_chars += chars;
    }

    if total_chars > cfg.max_chars_total {
        return Err(CapsError::TotalTooLong {
            total: total_chars,
            max: cfg.max_chars_total,
        });
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn default_cfg() -> ServeConfig {
        ServeConfig::default()
    }

    #[test]
    fn accepts_valid_inputs() {
        let inputs = vec!["hello world".to_string(); 5];
        assert!(validate_inputs(&inputs, &default_cfg()).is_ok());
    }

    #[test]
    fn rejects_too_many_inputs() {
        let inputs = vec!["x".to_string(); 65];
        assert!(matches!(
            validate_inputs(&inputs, &default_cfg()),
            Err(CapsError::TooManyInputs { .. })
        ));
    }

    #[test]
    fn rejects_oversized_input() {
        let long = "a".repeat(20_001);
        let inputs = vec![long];
        assert!(matches!(
            validate_inputs(&inputs, &default_cfg()),
            Err(CapsError::InputTooLong { .. })
        ));
    }
}
