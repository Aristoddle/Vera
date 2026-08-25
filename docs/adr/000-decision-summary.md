# Architecture Decisions

These are the main technical choices behind Vera's current architecture. Earlier decisions were made during the initial spike phase, and later retrieval work was folded into the same benchmark-driven process.

| Area | Choice | Details |
|------|--------|---------|
| Language | Rust | [001](001-implementation-language.md) |
| Storage | SQLite + sqlite-vec + Tantivy | [002](002-storage-backend.md) |
| Embedding | Potion Code local default; API and Jina ONNX opt in | [003](003-embedding-model.md) |
| Chunking | Symbol-aware via tree-sitter AST | [004](004-chunking-strategy.md) |
| Retrieval | BM25 + Vector + RRF + Query-aware ranking + optional reranking | [005](005-query-aware-retrieval.md) |

ADR 003 records the original spike evaluation. Its default-model conclusion was superseded in v1.1.0; the table above reflects the current default.

Early spike code was removed from the tree before v1.0; it remains in git history under `spikes/`.
