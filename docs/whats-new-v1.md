# What's New in v1.0

Vera 1.0 is the first stable release. It brings the hybrid BM25 and vector search pipeline, cross-encoder reranking, agent integration, local inference backends, and code-intelligence commands together with a measured release benchmark and clearer operational diagnostics. The existing [feature guide](features.md) and [architecture overview](how-it-works.md) cover the system in detail; this page focuses on what changed for the v1 release.

## Search Quality

The v1.0.0 release candidate was measured on the full 1,251-task Semble suite across 63 repositories, using hybrid BM25 and vector retrieval, RRF fusion, and a local ONNX cross-encoder reranker on the `vera-cuda` lane.

| Metric | v1.0.0-rc |
|--------|-----------|
| nDCG@10 | `0.7327` |
| Recall@1 | `0.5476` |
| Recall@5 | `0.8144` |
| Mean search latency | `2421 ms` |

The mean latency is reranker-dominated. The BM25-only scoped-filter reference lane is about `54 ms` p95 (`54.94 ms` in the benchmark table), so the two numbers describe different pipeline costs. See [the full benchmark report](benchmarks.md) for the methodology, artifacts, and older comparison lanes.

The release ablations determined which ranking changes belong in the default pipeline:

| Change | Release decision |
|--------|------------------|
| C1: rerank no-surplus skip | Shipped as a latency guard. Vera skips reranking when the fused pool has no surplus over the requested result limit, with a `-0.0002` nDCG delta and `-2.4 ms` mean latency effect. |
| C2: rerank path-glob searches | Shipped. Path-scoped searches remain eligible for reranking, improving full-suite nDCG by `+0.0113`. |
| Structural graph augmentation | Merged as an experimental opt-in under `VERA_GRAPH_AUGMENT=1`. It adds bounded caller and implementation chunks to the rerank pool, gaining `+0.0047` nDCG at roughly `+83%` mean latency. It is off by default because the latency cost outweighs the gain and Recall@5 did not move. |

## Agent-Level Benchmark

The benchmark used 10 cross-file Flask questions, fresh agents, and two A/B arms: `with-vera` had a local index and project skill installed, while `control` had the Vera CLI blocked by a shim that exits 127; a judge graded answers blind against a verified answer key.

| Tested model | With Vera | Control | Observed result |
|--------------|-----------|---------|-----------------|
| `claude-opus-5` | `10.0/10` | `10.0/10` | Quality parity and efficiency parity on this run |
| `kimi-k3` | `10.0/10` | `9.9/10` | With Vera used 17% fewer input tokens: `230.6k` versus `278.0k` |

This is a small workload signal, not a general performance claim. The question set, reproduction commands, raw measurements, and limitations are in the [agent benchmark README](../benchmarks/agent-bench/README.md).

## New Features

### Search And Agent Workflows

- Repeat `--path` to search several path patterns with OR semantics. Other filters still combine with AND semantics, so `--lang`, `--type`, and `--scope` continue to narrow the combined path match.
- Function and method symbol types are aliases, so `--type function` and `--type method` can be used interchangeably when selecting callable symbols.
- `vera agent install` writes Vera skill files for supported agent clients and can add a short usage snippet to a project agent configuration file.
- `vera serve` starts the local HTTP inference server. It exposes OpenAI-compatible embeddings, Cohere/Jina-compatible reranking, and a health endpoint for clients that need a local model service.
- `vera search --deep` uses RAG-fusion query expansion when a completion endpoint is configured: it generates targeted subqueries, searches them in parallel, and merges the results. Without a completion endpoint, deep search falls back to iterative symbol-following.

See [Features](features.md) for the complete command and integration surface.

### Models And Local Backends

- Voyage AI rerank endpoints are supported, including the `rerank-2` API format.
- `VERA_EMBEDDING_MODEL_ALIASES` lets compatible deployment names share an index after the normal dimension check. Alias groups are separated with semicolons and names within a group with commas.
- Local mode now checks ONNX model integrity and gives `vera doctor` and `vera repair` enough information to recover damaged or incomplete assets.
- CoreML embedding batches scale with available Apple Silicon unified memory, and the CoreML reranker uses the supported CPU fallback instead of the problematic fp16 path.
- OpenAI-compatible embedding providers can report a token-limit error and have the oversized input truncated and retried automatically.

### Indexing And Updates

- Index and update commands show phase progress for discovery, classification, parsing, and embedding. Use `--no-progress` or `--json` when a machine-readable or quiet interface is needed.
- `vera update . --max-files <N>` bounds the added or modified files processed in one run and reports deferred files for a later update.

## Fixes

- Bare directory patterns such as `--path src/app` now match files below that directory instead of returning no results.
- `--intent` no longer leaks its intent prefix into BM25 field queries, fixing the BM25 error path for intent searches.
- CoreML reranking no longer takes the slower fp16 CPU fallback path. The supported quantized CPU path is used, and `vera doctor` reports that the reranker is CPU-only on CoreML.
- Files with invalid UTF-8 are read lossily during indexing and retrieval, so they are no longer silently skipped.
- Unicode file paths are handled correctly by path glob matching and grep byte-to-character offsets.
- Embedding cache keys include the model namespace, preventing cached vectors from one model from being reused for another model.
- `crossbeam-epoch` was updated to address the tracked RustSec advisory.

## Upgrade Notes

Source builds now need the vendored tree-sitter grammar bootstrap step after cloning the repository:

```bash
bash scripts/bootstrap-vendored-grammars.sh
cargo build --release
```

The script downloads the four grammar sources that are not tracked in git. If they are missing, the build script fails early and prints the bootstrap command instead of allowing a later linker failure. Prebuilt package installs do not need this source-build step. See the [installation guide](installation.md) for the complete source-build instructions.

There are no breaking CLI changes in v1.0. Existing search, indexing, update, agent, and MCP workflows keep their command names and flags.

Existing indexes keep working: v1.0 opens them and searches as before. Reindex with `vera index .` to pick up the new stemmed and identifier-tokenized BM25 fields, caller and type-relation data, and index-health metrics. Until then, search quality on old indexes stays at pre-v1 levels.

## Evidence

For the full Semble tables and benchmark artifacts, read [docs/benchmarks.md](benchmarks.md). For the agent experiment's question set, harness, grading process, and limitations, read [benchmarks/agent-bench/README.md](../benchmarks/agent-bench/README.md).
