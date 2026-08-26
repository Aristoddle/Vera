---
name: vera
description: Code search over the current repository. ALWAYS run `vera search "<behavior>"` first when the question is conceptual (how does X work, where is Y handled, tracing a flow, unfamiliar code) instead of opening or grepping files. Use `vera grep` for exact strings and regex, `vera references <symbol>` for callers and callees, `vera structural` for definitions, routes, env reads, and implementations. Do not use rg for conceptual questions; use rg only for bulk edits or files the index does not cover.
---

# Vera

Ranked code search over an indexed repository. Results are markdown codeblocks: `path:line_start-line_end symbol_type:symbol_name`, then the code.

## Pick the tool

| Need | Command |
|------|---------|
| How something works, where behavior lives, unfamiliar code | `vera search "request retry backoff"` |
| Exact string, identifier, import, TODO | `vera grep "EMBEDDING_MODEL_BASE_URL"` |
| Who calls this, what does it call | `vera references parse_config` / `--callees` |
| Definitions, routes, env reads, SQL, implementations | `vera structural definitions parse_config` |
| Project orientation before a first change | `vera overview` |
| Bulk find-and-replace, filenames, unindexed files | `rg` |

## Do not use Vera when

- You already know the exact path and line: open the file.
- You are editing across many files mechanically: use `rg`.
- The answer is a literal string you can match: `vera grep` beats `vera search`.
- You have already run two searches that returned the same region: stop searching and read the code.

## Search well

- Search behavior, not nouns: `"JWT expiry handling"`, not `"auth"` or `"utils"`.
- Pass several angles in one call: `vera search "OAuth token refresh" "JWT expiry" "auth middleware"`.
- Start broad with `--compact` (signatures only, fewer tokens), then narrow with `--lang`, `--path`, `--type`, `--limit`.
- Add `--intent "<goal>"` when the query is vague but the goal is clear.
- Scope to a change with `--changed`, `--since <rev>`, or `--base <rev>` when reviewing a diff.
- `--deep` rewrites the query through an LLM; use it only after normal search misses.

## Treat hits as leads

- A search hit is a lead, not evidence. Before stating how something behaves, open the cited lines.
- Follow the call graph rather than re-searching: `vera references <symbol>` on a promising hit answers "who drives this" in one step.
- Cite `path:line` from code you actually read.
- After editing files, run `vera update .` before searching again.

## Recovery

| Symptom | Fix |
|---------|-----|
| `no index found` | `vera index .` |
| Stale results after edits | `vera update .` (or `vera watch .`) |
| A file is missing from results | `vera explain-path path/to/file` |
| Local model or ONNX error | `vera doctor --probe`, then `references/troubleshooting.md` |
| Missing local assets | `vera repair` |
| Install, API keys, backends | `references/install.md` |
| MCP server | `references/mcp.md` |

## References

- `references/install.md`: install, setup, API and local config, `.veraignore` rules
- `references/query-patterns.md`: more query examples and rg guidance
- `references/troubleshooting.md`: common errors and fixes
- `references/mcp.md`: optional MCP server usage
