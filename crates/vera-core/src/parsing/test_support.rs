//! Shared helpers for parsing tests (chunk extraction, metadata checks).

use crate::config::IndexingConfig;
use crate::types::{Chunk, Language};

pub(crate) fn default_config() -> IndexingConfig {
    IndexingConfig::default()
}

/// Parse `source` with the default config, panicking on failure.
pub(crate) fn parse(source: &str, path: &str, lang: Language) -> Vec<Chunk> {
    super::parse_and_chunk(source, path, lang, &default_config()).unwrap()
}

/// Find the chunk named `name`, panicking with a clear message if absent.
pub(crate) fn find_chunk<'a>(chunks: &'a [Chunk], name: &str) -> &'a Chunk {
    chunks
        .iter()
        .find(|c| c.symbol_name.as_deref() == Some(name))
        .unwrap_or_else(|| panic!("should find chunk named '{name}'"))
}
