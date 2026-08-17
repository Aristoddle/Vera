//! Query-aware ranking heuristics layered on top of dense + lexical retrieval.
//!
//! These heuristics intentionally stay simple and deterministic. They target
//! recurring benchmark failures that single-vector retrieval struggles with:
//! config files at repo root, test/docs noise, symbol-type disambiguation, and
//! same-file crowding for multi-file questions.

use crate::corpus::{ContentClass, classify_path, content_class_label};
use crate::types::{Language, SearchFilters, SearchResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RankingStage {
    Initial,
    PostRerank,
}

pub(crate) mod query;
pub(crate) mod score;

#[cfg(test)]
mod tests;

use query::*;
use score::*;

#[cfg(test)]
pub(crate) fn apply_query_ranking(
    query: &str,
    results: Vec<SearchResult>,
    stage: RankingStage,
) -> Vec<SearchResult> {
    apply_query_ranking_with_filters(query, results, stage, &SearchFilters::default())
}

pub(crate) fn apply_query_ranking_with_filters(
    query: &str,
    results: Vec<SearchResult>,
    stage: RankingStage,
    filters: &SearchFilters,
) -> Vec<SearchResult> {
    if results.len() <= 1 {
        return results;
    }

    let features = QueryFeatures::from_query(query);
    let file_relevance = file_relevance_counts(&features, &results);
    let len = results.len() as f64;
    let mut scored: Vec<(f64, usize, SearchResult)> = results
        .into_iter()
        .enumerate()
        .map(|(idx, mut result)| {
            let base_rank = 1.0 - (idx as f64 / len);
            let same_file_hits = file_relevance
                .get(&result.file_path)
                .copied()
                .unwrap_or_default();
            let prior = score_prior(&features, &result, stage, filters, same_file_hits);
            let combined = base_rank + prior;
            result.score = combined;
            (combined, idx, result)
        })
        .collect();

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });

    let reranked = scored.into_iter().map(|(_, _, result)| result).collect();
    let reranked = if features.wants_multi_file_diversity {
        diversify_by_file(reranked)
    } else {
        reranked
    };
    stamp_rank_scores(reranked)
}

pub(crate) fn classify_file_role(file_path: &str, language: Language) -> ContentClass {
    classify_path(file_path, language)
}

pub(crate) fn file_role_label(file_path: &str, language: Language) -> &'static str {
    content_class_label(classify_file_role(file_path, language))
}

pub(crate) fn is_path_weighted_query(query: &str) -> bool {
    let lower = query.trim().to_ascii_lowercase();
    if lower.contains(".toml")
        || lower.contains(".json")
        || lower.contains(".yaml")
        || lower.contains(".yml")
        || lower.contains(".ini")
        || lower.contains(".conf")
        || lower.contains("dockerfile")
        || lower.contains("makefile")
        || lower.contains("cmakelists.txt")
    {
        return true;
    }

    // A slash alone does not make a path query: prose like "read/write
    // request handling" must stay semantic. Require a slash-bearing token
    // that is the whole query or has path shape (prefix or file extension).
    let tokens: Vec<&str> = lower
        .split_whitespace()
        .map(crate::retrieval::query_utils::trim_query_token)
        .filter(|token| !token.is_empty())
        .collect();
    tokens
        .iter()
        .any(|token| is_path_shaped_token(token, tokens.len() == 1))
}

fn is_path_shaped_token(token: &str, single_token_query: bool) -> bool {
    if !token.contains('/') && !token.contains('\\') {
        return false;
    }
    if single_token_query
        || token.starts_with("./")
        || token.starts_with("../")
        || token.starts_with('/')
        || token.starts_with('~')
        || token.as_bytes().get(1) == Some(&b':')
    // Windows drive prefix
    {
        return true;
    }
    // src/main.rs: the last path segment carries a file extension.
    let last_segment = token.rsplit(['/', '\\']).next().unwrap_or(token);
    last_segment.contains('.')
}
