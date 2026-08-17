//! Query feature extraction for ranking heuristics.

use crate::chunk_text::file_name;
use crate::retrieval::query_classifier::{QueryType, classify_query};
use crate::retrieval::query_utils::{
    looks_like_compound_identifier, looks_like_filename, trim_query_token,
};
use crate::types::SymbolType;

use super::*;

#[derive(Debug, Clone)]
pub(super) struct QueryFeatures {
    pub(super) query_word_count: usize,
    pub(super) path_fragment: Option<String>,
    pub(super) exact_filename: Option<String>,
    pub(super) exact_identifier_case: Option<String>,
    pub(super) exact_identifier: Option<String>,
    pub(super) keywords: Vec<String>,
    /// CamelCase/snake_case identifiers embedded in NL queries.
    /// E.g., "How does StateManager handle transitions" yields ["StateManager"].
    pub(super) embedded_symbols: Vec<String>,
    pub(super) requested_symbol_types: Vec<SymbolType>,
    pub(super) query_type: QueryType,
    pub(super) wants_test_paths: bool,
    pub(super) wants_docs_paths: bool,
    pub(super) wants_example_paths: bool,
    pub(super) wants_config_paths: bool,
    pub(super) wants_runtime_paths: bool,
    pub(super) wants_archive_paths: bool,
    pub(super) wants_compat_paths: bool,
    pub(super) wants_type_declarations: bool,
    pub(super) requested_versions: Vec<String>,
    pub(super) wants_multi_file_diversity: bool,
    pub(super) mentions_implementation: bool,
    pub(super) mentions_definition: bool,
}

impl QueryFeatures {
    pub(super) fn from_query(query: &str) -> Self {
        let lower = query.trim().to_ascii_lowercase();
        let query_type = classify_query(query);
        let raw_tokens: Vec<&str> = query.split_whitespace().collect();
        let cleaned_tokens: Vec<String> = query
            .split_whitespace()
            .map(clean_query_token)
            .filter(|token| !token.is_empty())
            .collect();
        let path_fragment = cleaned_tokens
            .iter()
            .find(|token| looks_like_path_fragment(token))
            .cloned();
        let exact_filename = cleaned_tokens
            .iter()
            .find(|token| looks_like_filename(token) && !looks_like_qualified_identifier(token))
            .map(|token| file_name(token).to_string());
        let exact_identifier = raw_tokens
            .iter()
            .map(|token| trim_query_token(token))
            .find(|token| {
                !token.is_empty()
                    && (looks_like_qualified_identifier(token)
                        || (!looks_like_filename(&token.to_ascii_lowercase())
                            && looks_like_compound_identifier(token)))
            })
            .map(|token| token.to_ascii_lowercase())
            .or_else(|| {
                if query_type == QueryType::Identifier && cleaned_tokens.len() == 1 {
                    cleaned_tokens
                        .first()
                        .filter(|token| {
                            !looks_like_filename(token) || looks_like_qualified_identifier(token)
                        })
                        .cloned()
                } else {
                    None
                }
            });
        let exact_identifier_case = exact_identifier.as_ref().and_then(|_| {
            raw_tokens
                .iter()
                .map(|token| trim_query_token(token))
                .find(|token| {
                    !token.is_empty()
                        && (looks_like_qualified_identifier(token)
                            || (!looks_like_filename(&token.to_ascii_lowercase())
                                && looks_like_compound_identifier(token)))
                })
                .map(ToString::to_string)
        });
        let keywords = cleaned_tokens
            .iter()
            .filter(|token| {
                (!looks_like_filename(token) || looks_like_qualified_identifier(token))
                    && !is_query_stopword(token)
            })
            .map(|token| normalize_token(token))
            .filter(|token| !token.is_empty())
            .collect();
        let requested_symbol_types = requested_symbol_types(&lower);

        // Extract CamelCase/snake_case identifiers embedded in NL queries.
        // "How does StateManager handle transitions" → ["statemanager"]
        let embedded_symbols = if query_type == QueryType::NaturalLanguage {
            extract_embedded_symbols(&raw_tokens, exact_identifier.as_deref())
        } else {
            Vec::new()
        };

        Self {
            query_word_count: raw_tokens.len(),
            path_fragment,
            exact_identifier_case,
            wants_test_paths: mentions_any(&lower, &["test", "tests", "spec", "__tests__"]),
            wants_docs_paths: mentions_any(&lower, &["docs", "documentation", "readme"]),
            wants_example_paths: mentions_any(&lower, &["example", "examples", "demo", "sample"]),
            wants_config_paths: is_path_weighted_query(query)
                || mentions_any(
                    &lower,
                    &["configuration", "config", "workspace", "settings"],
                ),
            wants_runtime_paths: mentions_any(
                &lower,
                &[
                    "runtime",
                    "bundle",
                    "bundles",
                    "minified",
                    "minify",
                    "extract",
                    "extracted",
                    "asar",
                    "dist",
                ],
            ),
            wants_archive_paths: mentions_any(
                &lower,
                &["archive", "archived", "legacy", "snapshot", "deprecated"],
            ),
            wants_compat_paths: mentions_any(
                &lower,
                &[
                    "compat",
                    "compatibility",
                    "legacy",
                    "shim",
                    "polyfill",
                    "adapter",
                ],
            ),
            wants_type_declarations: mentions_any(
                &lower,
                &["declaration", "declarations", ".d.ts", "types", "typings"],
            ),
            requested_versions: requested_versions(&cleaned_tokens),
            wants_multi_file_diversity: !is_path_weighted_query(query)
                && (query_type == QueryType::NaturalLanguage
                    || (exact_identifier.is_some() && raw_tokens.len() <= 2)),
            mentions_implementation: mentions_any(
                &lower,
                &[
                    "implementation",
                    "implementations",
                    "impl",
                    "mounted",
                    "registration",
                ],
            ),
            mentions_definition: mentions_any(
                &lower,
                &[
                    "definition",
                    "definitions",
                    "define",
                    "declared",
                    "declaration",
                ],
            ),
            exact_filename,
            exact_identifier,
            keywords,
            embedded_symbols,
            requested_symbol_types,
            query_type,
        }
    }
}

/// Check whether a token is a scope-qualified identifier rather than a file
/// name whose extension happens to contain a dot.
pub(crate) fn looks_like_qualified_identifier(token: &str) -> bool {
    let valid_segment = |segment: &str| {
        !segment.is_empty()
            && segment
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '_')
    };

    if token.contains("::") {
        return token
            .split("::")
            .all(|scope| scope.split('.').all(&valid_segment));
    }

    let segments: Vec<_> = token.split('.').collect();
    segments.len() >= 3 && segments.iter().all(|segment| valid_segment(segment))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_scope_qualified_identifiers_as_exact_symbols() {
        let features = QueryFeatures::from_query("std::io::Error");

        assert_eq!(features.exact_filename, None);
        assert_eq!(features.exact_identifier.as_deref(), Some("std::io::error"));
        assert_eq!(
            features.exact_identifier_case.as_deref(),
            Some("std::io::Error")
        );
    }

    #[test]
    fn rejects_dotted_file_names_as_qualified_identifiers() {
        assert!(!looks_like_qualified_identifier("config.toml"));
        assert!(looks_like_qualified_identifier("config.retrieval.rrf_k"));
        assert!(looks_like_qualified_identifier("crate::retrieval::hybrid"));
    }
}
