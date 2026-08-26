# Skill Activation Eval

Measures how reliably a Droid agent activates the Vera skill and what each integration variant costs in tokens. Five sandbox copies of the Flask repo are identical except for agent-facing configuration, and every arm resolves the same release `vera` binary first on `PATH`. Each arm answers eight short read-only probes; rule-based scoring checks which tools the agent actually invoked.

Runs live under `~/.cache/skill-eval/<timestamp>/`. One indexed copy of Flask is built once, then copied into the five arms so every cell searches the same index.

## Arms

| Arm     | AGENTS.md snippet | Skill at `.factory/skills/vera` |
| ------- | ----------------- | ------------------------------- |
| bare    | no                | no                              |
| snippet | yes               | no                              |
| skill   | no                | yes                             |
| both    | yes               | yes                             |
| skill-v2| no                | yes (rewritten)                 |

The snippet is the exact `AGENTS_MD_SNIPPET` constant from `crates/vera-cli/src/commands/agent.rs`; the installer only writes it through an interactive prompt. The skill directory is copied verbatim from `skills/vera/`. The `skill-v2` variant uses a rewritten skill focused on activation triggers ("before reading files to answer X, search first").

## Results (muse-spark-1.2-contributor, xhigh, 3 reps)

| Arm      | Appropriateness | Mean tokens | Notes |
|----------|-----------------|-------------|-------|
| bare     | 38%             | 21.5k       | No activation on conceptual questions |
| snippet  | 50%             | 22.0k       | Helps symbol lookup, not conceptual |
| skill    | 50%             | 20.3k       | Old skill, moderate activation |
| both     | 50%             | 18.2k       | Redundant with skill alone |
| skill-v2 | 75%             | 16.5k       | Trigger-first description wins |

Per-scenario: skill-v2 achieves 100% on structural (S05) where others score 0-33%, and 100% on restraint scenarios (S06/S07). Conceptual questions (S01/S02) remain challenging at 33-67%.

## Scenarios

The eight probes live in `scenarios.md`, each with an expected behavior class:

- VERA: matched when the agent invokes the `vera` CLI.
- EXACT: matched when the agent runs `vera grep` or `rg`/`grep`.
- NONE: matched when the agent uses neither.

Two probes expect no search at all (git log, license file), which measures restraint rather than activation.

## Reproduce

From the repository root:

```bash
python3 benchmarks/skill-eval/run.py --setup-only
python3 benchmarks/skill-eval/run.py --run ~/.cache/skill-eval/<timestamp> --reps 3
python3 benchmarks/skill-eval/run.py --analyze ~/.cache/skill-eval/<timestamp>
```

Running with no mode performs setup, the sweep, and analysis sequentially. Useful flags:

- `--scenarios N`: first N probes only.
- `--arm NAME` / `--scenario N`: restrict to one arm or one scenario (smoke tests).
- `--reps N`: repetitions per (arm, scenario) cell (default 2).
- `--model` / `--effort`: droid lane (defaults `kimi-k3`, `medium`). Transcripts are named `sNN.<model>-<effort>.r<k>.jsonl`, so lanes can share a run directory.
- `--force`: re-run completed cells (default: skip cells with a successful transcript).

## Scoring

Each repetition is classified from its stream transcript: `vera` (any shell command starting with `vera`), `exact` (an `rg`/`grep` command), or `none`. A repetition matches its scenario's expected class accordingly; a cell matches when at least half of its successful repetitions match. The report shows per-cell match rate, mean tokens in/out, and mean wall seconds, plus per-arm activation appropriateness (fraction of cells matching) and mean total tokens per run.

## Limitations

One model, one repository, eight probes, three repetitions. Scoring is rule-based: it observes tool invocations, not answer quality, and command classification depends on the stream schema. Provider timing and model variance affect token counts and wall time. Do not generalize beyond this workload.
