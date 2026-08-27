# Graph Eval

Measures whether `vera references` returns the call sites that actually reach
one definition, rather than every call sharing the symbol's name. This is a
code-intelligence check, not a retrieval check: the Semble benchmark can look
healthy while caller lookups stay ambiguous, because they measure different
things.

## What a case is

Each case in `cases.json` names an ambiguous symbol, the definition under test,
and two regexes:

- `target_call` matches call sites that reach the target definition.
- `other_call` matches call sites that reach a different definition of the same
  name.

The runner asks Vera for callers, then reads each returned location in the
source and labels it. A returned site matching `other_call` is a confusion: the
answer names a call that belongs to another definition. Ground truth for recall
comes from scanning the repo with `target_call`, with docstrings and comments
blanked out first so documentation examples do not count as call sites.

Cases were established by reading the code: for every symbol, the definitions
were listed, the call sites inspected, and the receiver that distinguishes them
recorded in `verified_by`.

Matching is by file path and line number as returned by the CLI, checked
against the line range of the hit. No tolerance window is applied.

## Reproduce

```bash
python3 benchmarks/graph-eval/run.py --setup-only     # copies and indexes flask and axios
python3 benchmarks/graph-eval/run.py --run /home/you/.cache/graph-eval/<timestamp>
```

Setup copies the corpus repos out of `.bench/semble-repos/` and indexes the
copies, so the shared corpus is never modified. The release binary at
`target/release/vera` is used as-is and is never rebuilt by this harness.

## Baseline

Seven cases across Flask (Python) and axios (JavaScript), measured 2026-08-26.

| Lookup | Precision | Recall | Cases with a confusion |
|--------|-----------|--------|------------------------|
| Name only | 55% | 75% | 71% |
| With `--receiver` | 99% | 81% | 14% |

Name-only lookup is what callers get without extra information: asking for
callers of `get` in Flask returns 48 sites, of which 26 are dictionary lookups
and one is the route decorator being sought. Passing the receiver the call goes
through (`--receiver app`) resolves the ambiguity without type inference.

Recall is not 100% with receivers because a call inside the defining module is
often unqualified (`forEach(list, fn)` inside `utils.js`), and an unqualified
call records no receiver. Those sites are still returned by name-only lookup.
