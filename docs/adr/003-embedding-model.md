# Embedding Model

> Status: historical spike evaluation. Since v1.1.0, the default local embedding model is `minishlab/potion-code-16M-v2`; Jina ONNX backends and API endpoints are opt-in.

**Original API mode:** Qwen3-Embedding-8B (4096-dim) via any OpenAI-compatible endpoint.
**Original local mode:** Jina v5 text-nano (768-dim ONNX) for offline use.

We originally planned Qwen3-0.6B as the local fallback but switched to Jina for simpler ONNX inference.

## Evaluation

Three models tested on 21 tasks (symbol lookup, intent search, cross-file discovery, config lookup, disambiguation) across 4 repos. Same chunking, same cosine similarity retrieval. only the model changes.

| Metric | Qwen3-8B | bge-en-icl | Qwen3-0.6B |
|--------|----------|------------|------------|
| Recall@5 | **0.49** | 0.28 | 0.40 |
| Recall@10 | **0.66** | 0.33 | 0.40 |
| MRR | **0.28** | 0.14 | 0.26 |
| nDCG@10 | **0.71** | 0.33 | 0.50 |
| Dimensions | 4096 | 4096 | 1024 |

### Per-category notes

- **Symbol lookup:** Both Qwen3 variants find 4/6 symbols in top-5. The 0.6B model actually has higher MRR here (0.39 vs 0.24). lower dimensionality reduces noise for exact matches.
- **Intent search:** Qwen3-8B dominates at Recall@10 (0.90 vs 0.40 for 0.6B). Larger model captures semantic nuance better.
- **Config lookup:** All models similar (~0.75 Recall@5). Config files are hard for embedding-only retrieval regardless.

### Against competitor baselines

| Tool | Recall@5 | Recall@10 | MRR | nDCG@10 |
|------|----------|-----------|-----|---------|
| ripgrep (lexical) | 0.28 | 0.37 | 0.26 | 0.29 |
| cocoindex-code (MiniLM) | 0.37 | 0.50 | 0.35 | 0.52 |
| Qwen3-8B | **0.49** | **0.66** | 0.28 | **0.71** |

Qwen3-8B has the highest recall and nDCG but its MRR (0.28) trails cocoindex-code (0.35). confirming that reranking is needed to push the best result to the top.

## Trade-offs

- 4096-dim vectors: 16KB per chunk, ~48MB for 3K chunks. acceptable
- API dependency when API mode is selected. The current default local path uses Potion Code
- Provider abstraction via env vars makes switching trivial
