//! Persistent storage backends for Vera's index.
//!
//! This module provides three storage components:
//! - [`metadata::MetadataStore`] — SQLite-based chunk metadata storage
//! - [`vector::VectorStore`] — dual sqlite-vec and flat SIMD vector storage
//! - [`bm25::Bm25Index`] — Tantivy-based BM25 full-text search index
//!
//! These are composed by the indexing pipeline and retrieval engine.

pub mod bm25;
pub mod metadata;
pub mod vector;
