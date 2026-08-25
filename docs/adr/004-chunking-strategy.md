# Chunking: Symbol-Aware via Tree-Sitter

The core rule is still symbol-aware chunking: functions, classes, structs, traits, interfaces, enums, and other structural containers become retrieval units instead of arbitrary line windows.

The current implementation also adds:

- whole-file chunks for config and document-like files
- gap chunks for module-level code between symbols
- preserved structural containers such as Rust `impl` blocks and Python class definitions
- large-symbol splitting when a symbol exceeds the configured chunk limit

Unsupported languages still fall back to sliding-window chunking (50 lines, 10-line overlap).

## Evaluation

Three strategies compared using the same embedding model (Qwen3-8B) and retrieval method across 21 tasks and 4 repos. Only the chunking changes.

| Metric | Sliding-Window | File-Level | Symbol-Aware |
|--------|----------------|------------|--------------|
| Recall@1 | 0.10 | 0.10 | **0.24** |
| Recall@5 | 0.49 | 0.49 | **0.59** |
| MRR | 0.28 | 0.26 | **0.38** |
| nDCG@10 | **0.71** | 0.52 | 0.70 |
| Token ratio | 1.00 | 0.80 | 0.86 |

### Where it matters

**Symbol lookup (6 tasks):** MRR jumps from 0.24 to 0.55. the target function ranks ~2nd instead of ~4th.

**Intent search (5 tasks):** Recall@5 goes from 0.50 to 0.90. Semantically coherent chunks produce cleaner embeddings.

**Config lookup (4 tasks):** No difference. Config files don't have symbol structure. All strategies get 0.75 Recall@5.

**Cross-file discovery (3 tasks):** Sliding-window slightly better (0.39 vs 0.28 Recall@10). Overlapping windows catch more cross-boundary context.

### Quality per token

| Strategy | MRR | Token ratio | MRR / token |
|----------|-----|-------------|-------------|
| Sliding-Window | 0.28 | 1.00 | 0.28 |
| File-Level | 0.26 | 0.80 | 0.33 |
| Symbol-Aware | 0.38 | 0.86 | **0.44** |

35% higher MRR with 14% fewer tokens.

## Trade-offs

- Requires a tree-sitter grammar per language (mitigated by sliding-window fallback)
- Slightly lower Recall@10 than sliding-window (0.61 vs 0.66). compensated by BM25 hybrid retrieval
- Cross-file discovery doesn't improve. needs graph-level metadata, not better chunking
- Large symbols beyond the configured chunk limit (default 200 lines) get split into sub-chunks to keep embedding quality stable
