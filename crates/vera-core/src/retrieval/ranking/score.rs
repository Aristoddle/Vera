//! Prior scoring and result-shaping heuristics.

use crate::chunk_text::file_name;
use crate::corpus::{ContentClass, classify_content};
use crate::retrieval::query_classifier::QueryType;
use crate::retrieval::query_utils::{
    content_declares_public_symbol, content_starts_with_impl, file_stem, path_depth,
    trim_query_token,
};
use crate::types::{SearchFilters, SearchResult, SymbolType};
use std::collections::HashMap;

use super::query::*;
use super::*;

pub(super) fn score_prior(
    features: &QueryFeatures,
    result: &SearchResult,
    stage: RankingStage,
    filters: &SearchFilters,
    same_file_hits: usize,
) -> f64 {
    let stage_weight = match stage {
        RankingStage::Initial => 1.0,
        RankingStage::PostRerank => 0.55,
    };
    let depth = path_depth(&result.file_path) as f64;
    let role = classify_content(&result.file_path, result.language, &result.content);
    let mut bonus = 0.0;
    let file_path = result.file_path.to_ascii_lowercase();
    let result_filename = file_name(&result.file_path).to_ascii_lowercase();
    let allow_filename_semantic_bonus = matches!(
        role,
        ContentClass::Source | ContentClass::Config | ContentClass::Unknown
    );
    let path_fragment_match = features
        .path_fragment
        .as_deref()
        .is_some_and(|fragment| path_matches_fragment(&file_path, fragment));
    let filename_boost_allowed = features.path_fragment.is_none() || path_fragment_match;

    if path_fragment_match {
        bonus += stage_weight * 1.2;
    }

    if let Some(filename) = features.exact_filename.as_deref() {
        if filename_boost_allowed && result_filename == filename {
            let filename_bonus = if features.wants_config_paths {
                if depth == 0.0 {
                    1.15
                } else {
                    (0.45 - depth.min(5.0) * 0.08).max(0.08)
                }
            } else if depth == 0.0 {
                0.9
            } else {
                (0.6 - depth.min(5.0) * 0.06).max(0.12)
            };
            bonus += stage_weight * filename_bonus;
        } else if filename_boost_allowed && file_path.ends_with(filename) {
            bonus += stage_weight * 0.15;
        }
    }

    if features.wants_config_paths && matches!(role, ContentClass::Config) {
        bonus += stage_weight
            * if depth == 0.0 {
                0.35
            } else {
                (0.2 - depth.min(5.0) * 0.03).max(0.05)
            };
    }

    if let Some(identifier) = features.exact_identifier.as_deref() {
        if result
            .symbol_name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(identifier))
        {
            // Symbol name matches the query identifier. This is the strongest
            // signal: developers searching "Axios" want the Axios class definition.
            let is_definition_chunk = is_definition_symbol(result.symbol_type);
            let base_symbol_bonus = if features.query_word_count <= 2 {
                if is_definition_chunk { 1.6 } else { 0.7 }
            } else if is_definition_chunk {
                1.2
            } else {
                0.55
            };
            bonus += stage_weight * base_symbol_bonus;
            bonus += stage_weight * if depth <= 2.0 { 0.18 } else { 0.05 };
            if features.requested_symbol_types.contains(&SymbolType::Class)
                && is_internal_definition_path(&file_path)
            {
                bonus -= stage_weight * 0.35;
            }
            if features
                .exact_identifier_case
                .as_deref()
                .is_some_and(|name| result.symbol_name.as_deref() == Some(name))
            {
                bonus += stage_weight * 0.28;
            }
            // Extra boost when the file stem also matches (e.g., Axios in Axios.js).
            if file_stem(&result_filename).eq_ignore_ascii_case(identifier) {
                bonus += stage_weight * 0.45;
            }
        } else if file_stem(&result_filename).eq_ignore_ascii_case(identifier) {
            bonus += stage_weight * 0.35;
        } else if file_stem_prefix_matches_identifier(file_stem(&result_filename), identifier) {
            bonus += stage_weight * 0.28;
        } else if identifier_matches_parent_dir(identifier, &file_path) {
            bonus += stage_weight * 0.22;
        }
    }

    let stem_overlap = file_stem(&result_filename);
    if features.query_type == QueryType::NaturalLanguage
        && !features.keywords.is_empty()
        && !features.wants_config_paths
        && allow_filename_semantic_bonus
    {
        let normalized_stem = normalize_token(stem_overlap);
        if features.keywords.contains(&normalized_stem)
            || features
                .keywords
                .iter()
                .any(|keyword| shares_keyword_stem(&normalized_stem, keyword))
        {
            bonus += stage_weight * 0.6;
        } else {
            // Proportional stem matching: split the file stem into sub-tokens
            // and count how many query keywords match. Scale bonus by match ratio.
            // "BeanDeserializer" → ["bean", "deserializer"], query "bean deserialization"
            // matches 2/2 → full bonus. Require 4+ char parts to avoid noise
            // from short stems like "mod", "run".
            let stem_parts = identifier_stems(stem_overlap);
            let long_parts: Vec<_> = stem_parts.iter().filter(|p| p.len() >= 4).collect();
            if long_parts.len() >= 2 {
                let matched = features
                    .keywords
                    .iter()
                    .filter(|kw| {
                        kw.len() >= 4
                            && long_parts
                                .iter()
                                .any(|part| *part == kw.as_str() || shares_keyword_stem(part, kw))
                    })
                    .count();
                if matched >= 2 {
                    let ratio = matched as f64 / features.keywords.len().max(1) as f64;
                    bonus += stage_weight * 0.6 * ratio.min(1.0);
                }
            }
        }
    }

    if features.query_type == QueryType::NaturalLanguage
        && !features.keywords.is_empty()
        && !features.wants_config_paths
        && allow_filename_semantic_bonus
    {
        if let Some(symbol_name) = result.symbol_name.as_deref() {
            let symbol_bonus = symbol_keyword_bonus(symbol_name, &features.keywords);
            if symbol_bonus > 0.0 {
                bonus += stage_weight * symbol_bonus;
            }
        }
        let parent_bonus = parent_dir_keyword_bonus(&file_path, &features.keywords);
        if parent_bonus > 0.0 {
            bonus += stage_weight * parent_bonus;
        }
    }

    if !features.requested_symbol_types.is_empty()
        && result
            .symbol_type
            .is_some_and(|sym| features.requested_symbol_types.contains(&sym))
    {
        bonus += stage_weight * 0.62;
        if features
            .exact_identifier_case
            .as_deref()
            .is_some_and(|name| result.symbol_name.as_deref() == Some(name))
        {
            bonus += stage_weight * 0.2;
        }
    } else if !features.requested_symbol_types.is_empty() {
        bonus -= stage_weight
            * if features.exact_identifier_case.is_some() {
                0.9
            } else {
                0.55
            };
    }

    if features.mentions_definition && is_definition_symbol(result.symbol_type) {
        bonus += stage_weight
            * if result.symbol_name.is_some() {
                0.34
            } else {
                0.18
            };
    }

    // Boost definition chunks for NL queries when their symbol name overlaps
    // query keywords. Definitions are the canonical location for a concept;
    // they should strongly outrank incidental mentions. Use a weaker boost
    // for broad multi-keyword queries where the symbol match is partial.
    if features.query_type == QueryType::NaturalLanguage
        && is_definition_symbol(result.symbol_type)
        && result.symbol_name.is_some()
    {
        if let Some(symbol_name) = result.symbol_name.as_deref() {
            let sym_stems = identifier_stems(symbol_name);
            // Count keyword overlaps where the keyword is non-trivial (5+ chars)
            // to avoid short keywords like "file", "type", "list" causing false boosts.
            let overlap_count = features
                .keywords
                .iter()
                .filter(|kw| {
                    kw.len() >= 5
                        && sym_stems
                            .iter()
                            .any(|s| s == kw.as_str() || shares_keyword_stem(s, kw))
                })
                .count();
            if overlap_count > 0 {
                // Scale by overlap ratio: single keyword match in a 5-word query
                // gets a modest boost; full overlap gets the maximum.
                let long_keywords = features
                    .keywords
                    .iter()
                    .filter(|k| k.len() >= 5)
                    .count()
                    .max(1);
                let ratio = (overlap_count as f64 / long_keywords as f64).min(1.0);

                // Extra boost when the file stem also matches the symbol.
                let stem = file_stem(&result_filename);
                let stem_aligns = file_stem(&result_filename).eq_ignore_ascii_case(symbol_name)
                    || sym_stems.iter().any(|s| {
                        s == &normalize_token(stem)
                            || shares_keyword_stem(s, &normalize_token(stem))
                    });
                let base_boost = if stem_aligns { 1.5 } else { 1.0 };
                bonus += stage_weight * base_boost * ratio;
            }
        }
    }

    // Content-based definition detection: if the chunk's content defines
    // a symbol matching query keywords (via language-agnostic prefix matching),
    // boost it. This catches cases where symbol_type metadata is missing
    // or too coarse. Skip when the user wants non-source content.
    // Use a mild boost; the metadata-based definition boost above handles
    // strong signals.
    if features.query_type == QueryType::NaturalLanguage
        && !features.keywords.is_empty()
        && !features.wants_runtime_paths
        && !features.wants_config_paths
        && content_defines_query_keyword(&result.content, &features.keywords)
    {
        bonus += stage_weight * 0.3;
    }

    // Boost chunks whose symbol name matches an embedded CamelCase identifier
    // in the query. "How does StateManager handle transitions" should boost
    // chunks with symbol_name "StateManager" or file stem "state_manager".
    if !features.embedded_symbols.is_empty() {
        if let Some(symbol_name) = result.symbol_name.as_deref() {
            let sym_lower = symbol_name.to_ascii_lowercase();
            let matched = features
                .embedded_symbols
                .iter()
                .any(|es| sym_lower == *es || sym_lower.contains(es.as_str()));
            if matched {
                let def_bonus = if is_definition_symbol(result.symbol_type) {
                    1.0
                } else {
                    0.5
                };
                bonus += stage_weight * def_bonus;
            }
        }
        // Also check file stem against embedded symbols.
        let stem_lower = file_stem(&result_filename).to_ascii_lowercase();
        if stem_lower.len() >= 4 {
            let stem_matched = features.embedded_symbols.iter().any(|es| {
                stem_lower == *es
                    || es.starts_with(&stem_lower)
                    || stem_lower.starts_with(es.as_str())
            });
            if stem_matched {
                bonus += stage_weight * 0.4;
            }
        }
    }

    if same_file_hits >= 2 && features.query_type == QueryType::NaturalLanguage {
        let coherence = ((same_file_hits.min(5) - 1) as f64 * 0.15).min(0.60);
        bonus += stage_weight * coherence;
    }

    // --- Noise penalties ---
    // These are strong enough that noisy content classes rarely outrank source.
    // Penalties stack: a test file inside tests/ gets penalized twice.
    if !features.wants_test_paths && matches!(role, ContentClass::Test) {
        bonus -= stage_weight * 0.95;
    }
    if matches!(role, ContentClass::Archive) {
        bonus += if features.wants_archive_paths {
            stage_weight * 0.18
        } else {
            -stage_weight * 0.85
        };
    }
    if matches!(role, ContentClass::Runtime) {
        bonus += if features.wants_runtime_paths {
            stage_weight * 0.95
        } else {
            -stage_weight * 0.72
        };
    } else if features.wants_runtime_paths {
        bonus -= stage_weight * 0.24;
    }
    if !features.wants_docs_paths && matches!(role, ContentClass::Docs) {
        bonus -= stage_weight
            * if prefers_source_over_docs(features) {
                0.95
            } else {
                0.55
            };
    }
    if !features.wants_example_paths && matches!(role, ContentClass::Example | ContentClass::Bench)
    {
        bonus -= stage_weight * 0.55;
    }
    if !features.wants_compat_paths && is_compat_path(&file_path) {
        bonus -= stage_weight * 0.65;
    } else if features.wants_compat_paths && is_compat_path(&file_path) {
        bonus += stage_weight * 0.32;
    }
    if !features.wants_type_declarations && is_typescript_declaration(&file_path) {
        bonus -= stage_weight * 0.82;
    }
    if is_reexport_barrel(result) && !features.mentions_definition {
        bonus -= stage_weight * 0.95;
    }
    bonus += stage_weight * version_path_bonus(features, &file_path);
    if matches!(role, ContentClass::Generated) {
        bonus -= stage_weight
            * if features.wants_runtime_paths {
                0.18
            } else {
                0.95
            };
        if filters.include_generated == Some(false) {
            bonus -= stage_weight * 0.8;
        }
    }
    if matches!(role, ContentClass::Source | ContentClass::Config) {
        bonus += stage_weight
            * if features.query_type == QueryType::Identifier || features.path_fragment.is_some() {
                if depth <= 2.0 { 0.24 } else { 0.12 }
            } else if depth <= 2.0 {
                0.12
            } else {
                0.05
            };
    }
    if let Some(scope) = filters.scope {
        if crate::corpus::matches_scope(role, scope, filters.include_generated.unwrap_or(true)) {
            bonus += stage_weight * 0.18;
        } else {
            bonus -= stage_weight * 1.1;
        }
    }

    if features.mentions_implementation && looks_like_impl_block(result) {
        bonus += stage_weight * 0.18;
    }

    if features.query_type == QueryType::NaturalLanguage && is_public_symbol(result) {
        bonus += stage_weight * 0.05;
    }

    if prefers_structural_chunks(features) {
        bonus += stage_weight * structural_chunk_bias(result);
    }

    bonus
}

pub(super) fn prefers_source_over_docs(features: &QueryFeatures) -> bool {
    features.query_type == QueryType::NaturalLanguage
        && features.query_word_count >= 4
        && !features.wants_config_paths
        && !features.wants_runtime_paths
        && !features.wants_archive_paths
}

pub(super) fn requested_symbol_types(query: &str) -> Vec<SymbolType> {
    let mut symbol_types = Vec::new();
    if query.contains("trait") {
        symbol_types.push(SymbolType::Trait);
    }
    if query.contains("class") {
        symbol_types.push(SymbolType::Class);
    }
    if query.contains("interface") {
        symbol_types.push(SymbolType::Interface);
    }
    if query.contains("struct") {
        symbol_types.push(SymbolType::Struct);
    }
    if query.contains("enum") {
        symbol_types.push(SymbolType::Enum);
    }
    if query.contains("function") {
        symbol_types.push(SymbolType::Function);
    }
    if query.contains("method") {
        symbol_types.push(SymbolType::Method);
    }
    symbol_types
}

pub(super) fn requested_versions(tokens: &[String]) -> Vec<String> {
    tokens
        .iter()
        .filter(|token| {
            token.len() >= 2
                && token.starts_with('v')
                && token[1..].chars().all(|ch| ch.is_ascii_digit())
        })
        .cloned()
        .collect()
}

pub(super) fn file_relevance_counts(
    features: &QueryFeatures,
    results: &[SearchResult],
) -> HashMap<String, usize> {
    let mut counts = HashMap::new();
    for result in results {
        if result_matches_query_features(features, result) {
            *counts.entry(result.file_path.clone()).or_insert(0) += 1;
        }
    }
    counts
}

pub(super) fn result_matches_query_features(
    features: &QueryFeatures,
    result: &SearchResult,
) -> bool {
    if let Some(identifier) = features.exact_identifier.as_deref() {
        if result
            .symbol_name
            .as_deref()
            .is_some_and(|name| name.eq_ignore_ascii_case(identifier))
            || file_stem(file_name(&result.file_path)).eq_ignore_ascii_case(identifier)
        {
            return true;
        }
    }

    if features.keywords.is_empty() {
        return false;
    }

    let path = result.file_path.to_ascii_lowercase();
    let content = result.content.to_ascii_lowercase();
    let symbol_stems = result
        .symbol_name
        .as_deref()
        .map(identifier_stems)
        .unwrap_or_default();
    let filename_stems = identifier_stems(file_stem(file_name(&path)));
    let parent_stems = parent_dir_stems(&path);

    features.keywords.iter().any(|keyword| {
        content.contains(keyword)
            || symbol_stems
                .iter()
                .chain(filename_stems.iter())
                .chain(parent_stems.iter())
                .any(|stem| stem == keyword || shares_keyword_stem(stem, keyword))
    })
}

/// Maximum chunks from the same file before saturation decay kicks in.
pub(super) const FILE_SATURATION_THRESHOLD: usize = 1;

/// Multiplicative penalty per extra chunk from the same file beyond the threshold.
/// 0.35 means each successive same-file chunk keeps 35% of its score, pushing
/// it below results from other files in most cases.
pub(super) const FILE_SATURATION_DECAY: f64 = 0.35;

pub(super) fn diversify_by_file(results: Vec<SearchResult>) -> Vec<SearchResult> {
    if results.len() <= 1 {
        return results;
    }

    use std::collections::HashMap;

    let mut file_counts: HashMap<String, usize> = HashMap::new();
    let mut scored: Vec<(f64, usize, SearchResult)> = results
        .into_iter()
        .enumerate()
        .map(|(idx, result)| {
            let count = file_counts.entry(result.file_path.clone()).or_insert(0);
            *count += 1;
            let effective_score = if *count > FILE_SATURATION_THRESHOLD {
                let excess = (*count - FILE_SATURATION_THRESHOLD) as f64;
                result.score * FILE_SATURATION_DECAY.powf(excess)
            } else {
                result.score
            };
            (effective_score, idx, result)
        })
        .collect();

    scored.sort_by(|a, b| {
        b.0.partial_cmp(&a.0)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(a.1.cmp(&b.1))
    });

    scored.into_iter().map(|(_, _, result)| result).collect()
}

pub(super) fn stamp_rank_scores(mut results: Vec<SearchResult>) -> Vec<SearchResult> {
    let len = results.len().max(1) as f64;
    for (idx, result) in results.iter_mut().enumerate() {
        result.score = 1.0 - (idx as f64 / len);
    }
    results
}

pub(super) fn looks_like_path_fragment(token: &str) -> bool {
    token.contains('/') || token.contains('\\')
}

pub(super) fn clean_query_token(token: &str) -> String {
    trim_query_token(token).to_ascii_lowercase()
}

pub(super) fn mentions_any(query: &str, needles: &[&str]) -> bool {
    needles.iter().any(|needle| query.contains(needle))
}

pub(super) fn is_query_stopword(token: &str) -> bool {
    matches!(
        token,
        "and"
            | "or"
            | "the"
            | "a"
            | "an"
            | "of"
            | "in"
            | "to"
            | "for"
            | "with"
            | "across"
            | "where"
            | "definition"
            | "definitions"
            | "configured"
            | "configuration"
    )
}

pub(super) fn normalize_token(token: &str) -> String {
    let token = token.to_ascii_lowercase();
    let trimmed = token.trim_end_matches('s');
    if trimmed.len() >= 3 {
        trimmed.to_string()
    } else {
        token
    }
}

pub(super) fn tokenize_path(path: &str) -> Vec<&str> {
    path.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect()
}

pub(super) fn contains_token(tokens: &[&str], expected: &[&str]) -> bool {
    tokens.iter().any(|token| expected.contains(token))
}

pub(super) fn is_internal_definition_path(path: &str) -> bool {
    let tokens = tokenize_path(path);
    contains_token(&tokens, &["sansio", "internal", "bindings"])
}

pub(super) fn path_matches_fragment(path: &str, fragment: &str) -> bool {
    path == fragment || path.ends_with(fragment) || path.contains(fragment)
}

/// Check if a file stem shares a 6+ char prefix with an identifier.
/// Strips namespace prefixes (e.g. "sinatra::showexceptions" → "showexceptions")
/// so that "format" matches "formatter" but "sinatra" doesn't match "sinatra::ShowExceptions".
pub(super) fn file_stem_prefix_matches_identifier(stem: &str, identifier: &str) -> bool {
    let stem_lower = stem.to_ascii_lowercase();
    let ident_lower = identifier.to_ascii_lowercase();
    let bare_ident = ident_lower
        .rsplit_once("::")
        .map(|(_, name)| name)
        .unwrap_or(&ident_lower);
    common_prefix_len(&stem_lower, bare_ident) >= 6
}

pub(super) fn identifier_matches_parent_dir(identifier: &str, path: &str) -> bool {
    parent_dir_stems(path)
        .iter()
        .any(|stem| stem.eq_ignore_ascii_case(identifier))
}

pub(super) fn parent_dir_keyword_bonus(path: &str, keywords: &[String]) -> f64 {
    let stems = parent_dir_stems(path);
    if stems.is_empty() || keywords.is_empty() {
        return 0.0;
    }

    let matched = keywords
        .iter()
        .filter(|keyword| {
            stems
                .iter()
                .any(|stem| stem == keyword.as_str() || shares_keyword_stem(stem, keyword))
        })
        .count();

    if matched == 0 {
        return 0.0;
    }

    // Scale bonus by how many query keywords match the directory hierarchy.
    let ratio = matched as f64 / keywords.len() as f64;
    0.15 + ratio * 0.35
}

pub(super) fn parent_dir_stems(path: &str) -> Vec<String> {
    let Some((dirs, _)) = path.rsplit_once('/') else {
        return Vec::new();
    };
    dirs.split('/')
        .rev()
        .take(3)
        .flat_map(identifier_stems)
        .collect()
}

pub(super) fn is_public_symbol(result: &SearchResult) -> bool {
    content_declares_public_symbol(&result.content)
}

pub(super) fn looks_like_impl_block(result: &SearchResult) -> bool {
    content_starts_with_impl(&result.content)
}

pub(super) fn shares_keyword_stem(left: &str, right: &str) -> bool {
    // Use minimum 4-char prefix overlap so short stems like "route" match
    // "routing" and "depend" matches "dependency". Longer words use longer
    // thresholds to avoid false positives.
    let shorter = left.len().min(right.len());
    let threshold = if shorter <= 5 { 4 } else { 5 };
    common_prefix_len(left, right) >= threshold
}

pub(super) fn common_prefix_len(left: &str, right: &str) -> usize {
    left.chars()
        .zip(right.chars())
        .take_while(|(l, r)| l == r)
        .count()
}

pub(super) fn symbol_keyword_bonus(symbol_name: &str, keywords: &[String]) -> f64 {
    let tokens = identifier_stems(symbol_name);

    if tokens.is_empty() {
        return 0.0;
    }

    if tokens
        .iter()
        .any(|token| keywords.iter().any(|keyword| keyword == token))
    {
        return 0.5;
    }

    if tokens.iter().any(|token| {
        keywords
            .iter()
            .any(|keyword| shares_keyword_stem(token, keyword))
    }) {
        return 0.32;
    }

    0.0
}

pub(super) fn identifier_stems(value: &str) -> Vec<String> {
    value
        .split(|ch: char| !ch.is_ascii_alphanumeric())
        .flat_map(split_camel_identifier)
        .map(|part| normalize_token(&part))
        .filter(|part| !part.is_empty() && !is_query_stopword(part))
        .collect()
}

pub(super) fn split_camel_identifier(value: &str) -> Vec<String> {
    if value.is_empty() {
        return Vec::new();
    }

    let mut parts = Vec::new();
    let mut start = 0;
    let chars: Vec<(usize, char)> = value.char_indices().collect();
    for idx in 1..chars.len() {
        let (_, prev) = chars[idx - 1];
        let (byte_idx, current) = chars[idx];
        let boundary = (prev.is_ascii_lowercase() && current.is_ascii_uppercase())
            || (prev.is_ascii_alphabetic() && current.is_ascii_digit())
            || (prev.is_ascii_digit() && current.is_ascii_alphabetic());
        if boundary {
            parts.push(value[start..byte_idx].to_ascii_lowercase());
            start = byte_idx;
        }
    }
    parts.push(value[start..].to_ascii_lowercase());
    parts
}

pub(super) fn is_definition_symbol(symbol_type: Option<SymbolType>) -> bool {
    matches!(
        symbol_type,
        Some(
            SymbolType::Class
                | SymbolType::Struct
                | SymbolType::Trait
                | SymbolType::Interface
                | SymbolType::Enum
                | SymbolType::Function
                | SymbolType::Method
                | SymbolType::Module
        )
    )
}

pub(super) fn is_compat_path(path: &str) -> bool {
    let tokens = tokenize_path(path);
    contains_token(
        &tokens,
        &[
            "compat",
            "compatibility",
            "legacy",
            "shim",
            "shims",
            "polyfill",
            "polyfills",
        ],
    )
}

pub(super) fn is_typescript_declaration(path: &str) -> bool {
    path.ends_with(".d.ts") || path.ends_with(".d.mts") || path.ends_with(".d.cts")
}

pub(super) fn is_reexport_barrel(result: &SearchResult) -> bool {
    let filename = file_name(&result.file_path).to_ascii_lowercase();
    if !matches!(
        filename.as_str(),
        "index.ts" | "index.tsx" | "index.js" | "index.jsx" | "mod.rs"
    ) {
        return false;
    }

    let non_empty: Vec<&str> = result
        .content
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with("//"))
        .collect();
    if non_empty.is_empty() || non_empty.len() > 24 {
        return false;
    }

    let reexports = non_empty
        .iter()
        .filter(|line| {
            line.starts_with("export ")
                || line.starts_with("pub use ")
                || line.starts_with("pub mod ")
                || line.starts_with("module.exports")
        })
        .count();
    reexports > 0 && reexports * 4 >= non_empty.len() * 3
}

pub(super) fn version_path_bonus(features: &QueryFeatures, path: &str) -> f64 {
    if features.requested_versions.is_empty() {
        return 0.0;
    }

    let tokens = tokenize_path(path);
    if tokens.iter().any(|token| {
        features
            .requested_versions
            .iter()
            .any(|version| version == token)
    }) {
        return 0.55;
    }

    if tokens.iter().any(|token| {
        token.len() >= 2
            && token.starts_with('v')
            && token[1..].chars().all(|ch| ch.is_ascii_digit())
    }) {
        return -0.34;
    }

    -0.08
}

pub(super) fn prefers_structural_chunks(features: &QueryFeatures) -> bool {
    features.query_type == QueryType::NaturalLanguage
        && features.exact_identifier.is_none()
        && features.query_word_count >= 4
        && !features.wants_config_paths
}

pub(super) fn structural_chunk_bias(result: &SearchResult) -> f64 {
    let lines = chunk_line_span(result);
    let mut bonus = 0.0;

    match result.symbol_type {
        Some(
            SymbolType::Struct | SymbolType::Class | SymbolType::Trait | SymbolType::Interface,
        ) => {
            bonus += 0.38;
        }
        Some(SymbolType::Enum | SymbolType::Module) => {
            bonus += 0.28;
        }
        Some(SymbolType::Block) if looks_like_impl_block(result) || lines >= 24 => {
            bonus += 0.24;
        }
        Some(SymbolType::Variable) => {
            bonus -= 0.45;
        }
        Some(SymbolType::Method | SymbolType::Function) if lines <= 8 => {
            bonus -= 0.32;
        }
        _ => {}
    }

    if lines <= 4 {
        bonus -= 0.2;
    } else if (12..=120).contains(&lines) {
        bonus += 0.12;
    }

    bonus
}

pub(super) fn chunk_line_span(result: &SearchResult) -> u32 {
    result.line_end.saturating_sub(result.line_start) + 1
}

/// Extract CamelCase/camelCase identifiers embedded in NL queries.
///
/// "How does StateManager handle transitions" → ["statemanager"]
/// "Where is the parseConfig function" → ["parseconfig"]
///
/// These are compound identifiers that contain mixed case transitions,
/// indicating a specific code symbol the user is asking about.
pub(super) fn extract_embedded_symbols(
    raw_tokens: &[&str],
    exact_identifier: Option<&str>,
) -> Vec<String> {
    let exact_lower = exact_identifier.map(|s| s.to_ascii_lowercase());
    raw_tokens
        .iter()
        .filter_map(|token| {
            let trimmed = trim_query_token(token);
            if trimmed.len() < 4 {
                return None;
            }
            // Must have a case transition (CamelCase or camelCase).
            let has_case_transition = trimmed
                .as_bytes()
                .windows(2)
                .any(|pair| pair[0].is_ascii_lowercase() && pair[1].is_ascii_uppercase());
            if !has_case_transition {
                return None;
            }
            let lower = trimmed.to_ascii_lowercase();
            // Skip if this is already the exact_identifier (already boosted separately).
            if exact_lower.as_deref() == Some(&lower) {
                return None;
            }
            Some(lower)
        })
        .collect()
}

/// Check if chunk content defines a symbol using language-agnostic keyword matching.
///
/// Looks for definition keywords (class, struct, def, function, etc.) followed
/// by a symbol name that matches query keywords. This is stronger than just
/// checking symbol_type metadata because it confirms the chunk is the actual
/// definition site, not just a reference.
pub(super) fn content_defines_query_keyword(content: &str, keywords: &[String]) -> bool {
    static DEFINITION_PREFIXES: &[&str] = &[
        "class ",
        "struct ",
        "enum ",
        "trait ",
        "interface ",
        "type ",
        "module ",
        "def ",
        "fn ",
        "func ",
        "function ",
        "fun ",
        "pub fn ",
        "pub struct ",
        "pub enum ",
        "pub trait ",
        "pub type ",
        "pub mod ",
        "export class ",
        "export function ",
        "export interface ",
        "export type ",
        "export enum ",
        "export default class ",
        "export default function ",
        "abstract class ",
        "data class ",
        "object ",
        "protocol ",
        "record ",
        "namespace ",
        "package ",
        "defmodule ",
    ];

    // Only consider keywords with 5+ chars to avoid false positives
    // from common short words like "file", "type", "list".
    let long_keywords: Vec<&String> = keywords.iter().filter(|k| k.len() >= 5).collect();
    if long_keywords.is_empty() {
        return false;
    }

    for line in content.lines().take(5) {
        let trimmed = line.trim();
        for prefix in DEFINITION_PREFIXES {
            if let Some(rest) = trimmed.strip_prefix(prefix) {
                // Extract the symbol name after the keyword.
                let symbol: String = rest
                    .chars()
                    .take_while(|ch| ch.is_ascii_alphanumeric() || *ch == '_')
                    .collect();
                if symbol.len() >= 3 {
                    let sym_stems = identifier_stems(&symbol);
                    let matches_keyword = long_keywords.iter().any(|kw| {
                        sym_stems
                            .iter()
                            .any(|s| s == kw.as_str() || shares_keyword_stem(s, kw))
                    });
                    if matches_keyword {
                        return true;
                    }
                }
            }
        }
    }
    false
}
