//! Output presentation helpers shared by the CLI and MCP frontends.

use serde::Serialize;
use std::borrow::Cow;

use crate::types::{SearchResult, SymbolType};

/// Compact JSON representation that drops low-signal fields (`score`, `language`)
/// and omits null optional fields. This is the default for AI agent consumption.
#[derive(Serialize)]
pub struct CompactResult<'a> {
    pub file_path: &'a str,
    pub line_start: u32,
    pub line_end: u32,
    pub content: Cow<'a, str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub symbol_type: Option<&'a SymbolType>,
}

impl<'a> CompactResult<'a> {
    pub fn from_search_result(r: &'a SearchResult) -> Self {
        Self {
            file_path: &r.file_path,
            line_start: r.line_start,
            line_end: r.line_end,
            content: Cow::Borrowed(r.content.as_str()),
            symbol_name: r.symbol_name.as_deref(),
            symbol_type: r.symbol_type.as_ref(),
        }
    }
}

/// Truncate `content` to fit within `allowed` bytes, breaking at a line boundary.
pub fn truncate_to_budget(content: &str, allowed: usize) -> Cow<'_, str> {
    if content.len() <= allowed {
        return Cow::Borrowed(content);
    }
    let end = content
        .char_indices()
        .take_while(|(i, _)| *i < allowed)
        .last()
        .map(|(i, c)| i + c.len_utf8())
        .unwrap_or(0);
    let break_at = content[..end].rfind('\n').unwrap_or(end);
    let mut truncated = content[..break_at].to_string();
    truncated.push_str("\n[...truncated]");
    Cow::Owned(truncated)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truncate_to_budget_short_passthrough() {
        let short = "short content";
        let result = truncate_to_budget(short, 1000);
        assert_eq!(result, short);
    }

    #[test]
    fn truncate_to_budget_long_truncates() {
        let long = "line1\nline2\nline3\n".repeat(100);
        let result = truncate_to_budget(&long, 100);
        assert!(result.len() < 120);
        assert!(result.ends_with("[...truncated]"));
    }
}
