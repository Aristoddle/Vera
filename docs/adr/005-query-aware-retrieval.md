# Retrieval: Hybrid Search With Query-Aware Ranking

> Status: historical decision record. The current default local path is Potion Code with deterministic query-aware ranking and reranking off. Cross-encoder reranking remains available as an opt-in stage.

Vera keeps the same core hybrid stack:

- BM25 for lexical matching
- dense embeddings for semantic matching
- Reciprocal Rank Fusion to merge both lists
- deterministic query-aware ranking and candidate shaping
- an optional cross-encoder reranker for final ordering

What changed by `v0.7.0` is that retrieval is no longer just "BM25 + vector + rerank". The pipeline added deterministic query-aware ranking and candidate shaping between fusion and final reranking.

## Decision

Keep the hybrid retrieval architecture, but add lightweight query-aware logic on top of it instead of switching to a new retrieval backend.

In practice this means:

- richer structured text for both BM25 and embeddings
- stronger path, filename, and symbol-name signals
- exact-match supplementation for identifiers and filenames
- structural priors for classes, traits, interfaces, and `impl` blocks
- file-aware diversification so one file cannot crowd out the candidate list
- selective same-file, cross-language, and helper-context expansion for hard queries
- reranker routing that skips unnecessary reranking for obvious path-dominant queries

## Why

This approach fixed the observed benchmark failures without changing the fundamental storage or model architecture.

The main failure modes were:

- config files losing to nested or incidental matches
- exact symbol lookups losing to semantically similar but wrong results
- cross-file queries returning only part of the answer set
- broad natural-language queries over-weighting docs, tests, and narrow helpers
- local reranker instability when ONNX CUDA batches were too large

The benchmark improvements from `v0.4.0` to `v0.7.0` show that these issues were better solved by candidate shaping and ranking than by replacing the entire retrieval stack.

## Trade-offs

- More heuristics in the retrieval layer means more code and more benchmark coverage is needed to keep behavior stable.
- The system is still model-sensitive. Better ranking logic does not remove the need for strong embeddings and reranking models.
- This does not provide the token-level matching behavior of a late-interaction system such as ColBERT.

## Rejected Alternative

Do not switch Vera to a late-interaction or multi-vector backend as part of this iteration.

That would introduce:

- a new index format
- different model requirements
- larger indexes
- more substantial implementation and maintenance cost

At the time, the hybrid stack reached the benchmark ceiling on the local 21-task suite, so a backend replacement was not justified in that pass. That suite is historical; current release quality is judged by the full Semble suite and independent contamination check in [../benchmarks.md](../benchmarks.md).
