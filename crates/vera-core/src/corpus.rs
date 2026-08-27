//! Corpus-aware file classification used by ranking and filtering.
//!
//! The index stores chunks, not repository-level metadata. This module keeps
//! the higher-level file classification lightweight and deterministic so search
//! can favor source code by default while still supporting docs, runtime
//! extracts, and generated artifacts when explicitly requested.

use crate::types::{Language, SearchScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContentClass {
    Source,
    Test,
    Docs,
    Archive,
    Example,
    Bench,
    Config,
    Runtime,
    Generated,
    Unknown,
}

pub fn classify_path(file_path: &str, language: Language) -> ContentClass {
    let path = normalize_path(file_path);
    let tokens = tokenize_path(&path);

    if is_runtime_like_path(&path, &tokens) {
        return ContentClass::Runtime;
    }
    if is_archive_like_path(&path) {
        return ContentClass::Archive;
    }
    if contains_token(
        &tokens,
        &[
            "test", "tests", "testing", "spec", "specs", "fixture", "fixtures",
        ],
    ) {
        return ContentClass::Test;
    }
    if contains_token(&tokens, &["example", "examples", "demo", "demos", "sample"]) {
        return ContentClass::Example;
    }
    if contains_token(&tokens, &["bench", "benches", "benchmark", "benchmarks"]) {
        return ContentClass::Bench;
    }
    if language == Language::Markdown
        || language == Language::Rst
        || contains_token(
            &tokens,
            &[
                "docs",
                "doc",
                "readme",
                "guide",
                "guides",
                "changelog",
                "history",
                "adr",
            ],
        )
    {
        return ContentClass::Docs;
    }
    if language.prefers_file_chunking() {
        return ContentClass::Config;
    }
    if contains_token(
        &tokens,
        &[
            "generated",
            "dist",
            "build",
            "coverage",
            "vendor",
            "binding",
            "bindings",
            "node_modules",
            "target",
            "out",
        ],
    ) {
        return ContentClass::Generated;
    }
    if contains_token(
        &tokens,
        &["src", "lib", "app", "apps", "crates", "packages"],
    ) {
        return ContentClass::Source;
    }
    if language != Language::Unknown && !language.is_document_like() {
        return ContentClass::Source;
    }

    ContentClass::Unknown
}

pub fn classify_content(file_path: &str, language: Language, content: &str) -> ContentClass {
    let class = classify_path(file_path, language);
    if matches!(
        class,
        ContentClass::Source | ContentClass::Unknown | ContentClass::Runtime
    ) && is_generated_like(file_path, content)
    {
        return ContentClass::Generated;
    }
    class
}

pub fn matches_scope(class: ContentClass, scope: SearchScope, include_generated: bool) -> bool {
    match scope {
        SearchScope::Source => {
            matches!(
                class,
                ContentClass::Source
                    | ContentClass::Test
                    | ContentClass::Example
                    | ContentClass::Bench
                    | ContentClass::Config
                    | ContentClass::Unknown
            ) || (include_generated && matches!(class, ContentClass::Generated))
        }
        SearchScope::Docs => matches!(class, ContentClass::Docs | ContentClass::Archive),
        SearchScope::Runtime => {
            matches!(class, ContentClass::Runtime)
                || (include_generated && matches!(class, ContentClass::Generated))
        }
        SearchScope::All => !matches!(class, ContentClass::Generated) || include_generated,
    }
}

pub fn content_class_label(class: ContentClass) -> &'static str {
    match class {
        ContentClass::Source => "source",
        ContentClass::Test => "test",
        ContentClass::Docs => "docs",
        ContentClass::Archive => "archive",
        ContentClass::Example => "example",
        ContentClass::Bench => "benchmark",
        ContentClass::Config => "config",
        ContentClass::Runtime => "runtime",
        ContentClass::Generated => "generated",
        ContentClass::Unknown => "unknown",
    }
}

pub fn is_generated_like(file_path: &str, content: &str) -> bool {
    classify_path(file_path, Language::Unknown) == ContentClass::Generated
        || is_minified_content(content)
}

pub fn is_minified_content(content: &str) -> bool {
    let line_count = content.lines().count();
    if line_count <= 1 {
        return content.len() >= 1_200;
    }

    let long_lines = content.lines().filter(|line| line.len() >= 240).count();
    let avg_line_len = content.len() / line_count;

    long_lines * 3 >= line_count
        || (avg_line_len >= 180 && long_lines * 2 >= line_count)
        || content.contains("sourceMappingURL=")
        || content.contains("__webpack_require__")
}

fn is_runtime_like_path(path: &str, tokens: &[&str]) -> bool {
    path.contains(".asar")
        || path.contains("asar.unpacked")
        || path.contains("/tmp/")
        || path.contains("/temp/")
        || path.contains("/cache/")
        || contains_token(
            tokens,
            &[
                "runtime",
                "bundle",
                "bundles",
                "extracted",
                "extract",
                "installed",
                "unpacked",
                "prebuilt",
                "preload",
                "tmp",
                "temp",
            ],
        )
}

fn normalize_path(path: &str) -> String {
    path.replace('\\', "/").to_ascii_lowercase()
}

fn tokenize_path(path: &str) -> Vec<&str> {
    path.split(|ch: char| !ch.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
        .collect()
}

fn contains_token(tokens: &[&str], expected: &[&str]) -> bool {
    tokens.iter().any(|token| expected.contains(token))
}

fn is_archive_like_path(path: &str) -> bool {
    const DIRECTORY_ARCHIVE_MARKERS: &[&str] = &[
        "archive",
        "archived",
        "legacy",
        "deprecated",
        "backup",
        "backups",
        "old",
    ];
    const TEST_MARKERS: &[&str] = &[
        "test", "tests", "testing", "spec", "specs", "fixture", "fixtures",
    ];

    let components: Vec<&str> = path
        .split('/')
        .filter(|component| !component.is_empty())
        .collect();
    let directories = components
        .get(..components.len().saturating_sub(1))
        .unwrap_or(&[]);

    if directories
        .iter()
        .any(|component| DIRECTORY_ARCHIVE_MARKERS.contains(component))
    {
        return true;
    }

    directories
        .iter()
        .any(|component| matches!(*component, "snapshot" | "snapshots"))
        && directories
            .iter()
            .any(|component| TEST_MARKERS.contains(component))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_archive_docs_before_docs() {
        assert_eq!(
            classify_path("archive/docs/hotkeys.md", Language::Markdown),
            ContentClass::Archive
        );
    }

    #[test]
    fn archive_tokens_in_live_source_filenames_do_not_hide_source() {
        assert_eq!(
            classify_path("src/snapshot_writer.rs", Language::Rust),
            ContentClass::Source
        );
        assert_eq!(
            classify_path("src/legacy_parser.rs", Language::Rust),
            ContentClass::Source
        );
    }

    #[test]
    fn archive_directories_still_classify_as_archive() {
        for path in [
            "archive/docs/hotkeys.md",
            "backups/restore.rs",
            "tests/fixtures/snapshot/case.rs",
        ] {
            assert_eq!(
                classify_path(path, Language::Rust),
                ContentClass::Archive,
                "{path} should remain archive content"
            );
        }
    }

    #[test]
    fn active_snapshot_directories_and_archive_named_sources_stay_source() {
        assert_eq!(
            classify_path("src/snapshots/writer.rs", Language::Rust),
            ContentClass::Source
        );
        assert_eq!(
            classify_path("src/old.rs", Language::Rust),
            ContentClass::Source
        );
    }

    #[test]
    fn classifies_rst_as_docs() {
        assert_eq!(
            classify_path("guide.rst", Language::Rst),
            ContentClass::Docs
        );
    }

    #[test]
    fn classifies_runtime_extracts() {
        assert_eq!(
            classify_path("/tmp/game/runtime/Game.pretty.js", Language::JavaScript),
            ContentClass::Runtime
        );
    }

    #[test]
    fn minified_content_is_generated_like() {
        let content = format!("function x(){{{}}}", "a=1;".repeat(800));
        assert!(is_minified_content(&content));
        assert_eq!(
            classify_content("src/app.js", Language::JavaScript, &content),
            ContentClass::Generated
        );
    }
}
