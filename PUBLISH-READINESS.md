# Crates.io publish readiness

**Packet:** TP-1011

**Fork baseline:** `Aristoddle/Vera` `master` at `8d881ea`

**Prepared:** 2026-08-24

**Boundary:** No real `cargo publish` command was run. Publication and the final public name are operator-only decisions.

## Decision required: public package family

Live checks used `GET https://crates.io/api/v1/crates/<name>` with a descriptive User-Agent. HTTP 200 means occupied; HTTP 404 means no exact crate existed at check time. Names are first-come-first-served, so 404 is a snapshot, not a reservation. Recheck immediately before publishing.

| Candidate CLI package | Core | MCP | Serve | Snapshot | Note |
|---|---|---|---|---|---|
| `vera` | `vera-core` | `vera-mcp` | `vera-serve` | **blocked** | `vera` and `vera-core` resolve to the unrelated [Coddeus/vera](https://github.com/Coddeus/vera) Vulkan project, latest `0.3.0`; the other two names returned 404. |
| `vera2` | `vera2-core` | `vera2-mcp` | `vera2-serve` | all 404 | Shortest available family checked. |
| `vera-code` | `vera-code-core` | `vera-code-mcp` | `vera-code-serve` | all 404 | Descriptive, not chosen. Hyphen and underscore spellings both returned 404. |
| `vera-search` | `vera-search-core` | `vera-search-mcp` | `vera-search-serve` | all 404 | Descriptive, not chosen. Hyphen and underscore spellings both returned 404. |
| `aristoddle-vera` | `aristoddle-vera-core` | `aristoddle-vera-mcp` | `aristoddle-vera-serve` | all 404 | Explicit fork provenance, not chosen. Hyphen and underscore spellings both returned 404. |

The CLI package can use the selected base name while retaining `[[bin]] name = "vera"`, so `cargo install <selected-base>` can still install a `vera` executable. If the operator prefers a `-cli` package suffix, every checked `<base>-cli` variant also returned 404.

## Changes already prepared

- Added shared `repository` and `readme` metadata to all four publish targets.
- Added registry versions alongside every internal path dependency.
- Changed the local `tree-sitter-hcl` fallback requirement from `1.2.0` to `1.1.0`; crates.io has `1.1.0`, while the local git checkout at `1.2.0` remains compatible with the caret requirement.
- Made `vera-core` package-self-contained by tracking the six C grammar build inputs used for SQL, Protobuf, Dockerfile, Astro, SCSS, and Vue parsing, plus each upstream MIT license.
- Made `vera-cli` package-self-contained by tracking the skill files consumed by `include_str!`.

## Dry-run evidence

### Before fixes

| Package | Raw `cargo publish --dry-run -p …` result |
|---|---|
| `vera-core` | FAIL: metadata warning; crates.io lacked `tree-sitter-hcl ^1.2.0`. |
| `vera-mcp` | FAIL: path dependency `vera-core` had no registry version. |
| `vera-serve` | FAIL: path dependency `vera-core` had no registry version. |
| `vera-cli` | FAIL: path dependency `vera-core` had no registry version; `vera-mcp` was also path-only. |

### After fixes

| Package | Verification result | Files | Raw size | `.crate` size |
|---|---:|---:|---:|---:|
| `vera-core` | **PASS**, ordinary dry-run | 108 | 36.8 MiB | 2,471,838 bytes |
| `vera-mcp` | **PASS** with a local `[patch.crates-io]` simulating prior core publication | 11 | 195.6 KiB | 49,111 bytes |
| `vera-serve` | **PASS** with the same simulation | 8 | 138.6 KiB | 38,094 bytes |
| `vera-cli` | **PASS** with local patches for core/MCP/serve | 37 | 424.8 KiB | 101,024 bytes |

The official crates.io limit is 10 MiB for the compressed `.crate` archive. The largest archive here is `vera-core` at about 2.36 MiB, roughly 24% of that limit. Its 34.1 MiB vendored SQL parser compresses heavily; the archive size, not the unpacked source size, is the enforced limit.

The unpatched dependent dry-runs still fail today because crates.io has no matching internal version. That is expected publish-order behavior, not a remaining manifest defect. The local patch commands prove the extracted packages compile; the operator must rerun ordinary dry-runs after each prerequisite package becomes visible in the registry index.

Focused tests also pass:

- `cargo test -p vera-core --lib` — 752 passed, 0 failed.
- `cargo test -p vera-cli` — 92 passed, 0 failed.
- Existing unused-code/import warnings remain; TP-1011 did not refactor unrelated code.

## Remaining blocker before any real publish

The current package names cannot be published as one coherent family because `vera-core` is occupied. The operator must select a base family. After that decision, make one follow-up manifest commit:

1. Rename the CLI `[package].name` to the selected base.
2. Rename the internal package names to `<base>-core`, `<base>-mcp`, and `<base>-serve`.
3. Preserve Rust import names with dependency aliases, for example:

   ```toml
   vera-core = { package = "<base>-core", path = "crates/vera-core", version = "0.12.15-aristoddle.2" }
   ```

4. Update the analogous MCP/serve aliases and regenerate `Cargo.lock`.
5. Recheck every exact crate name and its hyphen/underscore spelling.
6. Rerun all ordinary dry-runs without local patches.

## Operator-only publish sequence

After the naming commit and clean ordinary dry-runs:

```bash
# 1. Core first
cargo publish -p <base>-core

# Verify 0.12.15-aristoddle.2 is visible on crates.io before continuing.

# 2. Independent core consumers
cargo publish --dry-run -p <base>-mcp
cargo publish -p <base>-mcp
cargo publish --dry-run -p <base>-serve
cargo publish -p <base>-serve

# Verify both versions are visible on crates.io.

# 3. CLI last
cargo publish --dry-run -p <base>
cargo publish -p <base>
```

Those real publish commands are recorded for the operator; none were executed during TP-1011.

## Primary references

- Cargo Book, [Publishing on crates.io](https://doc.rust-lang.org/cargo/reference/publishing.html) — dry-run flow, metadata guidance, first-come names, 10 MiB archive limit.
- Cargo Book, [Specifying dependencies](https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#local-paths-in-published-crates) — published path dependencies require registry versions.
- Cargo Book, [Workspaces / package inheritance](https://doc.rust-lang.org/cargo/reference/workspaces.html#the-package-table) — inherited repository/readme fields and path rules.
