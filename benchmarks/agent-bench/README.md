# Agent-Level Vera Benchmark

This benchmark measures whether a coding agent answers cross-file questions about Flask with less tool use and lower latency when Vera is available. It records answer text, tool-call counts, token usage reported by the agent stream, and wall-clock time. Answer quality is graded separately by `judge.py`, which scores each answer blind (no arm label) against `flask/answer-key.md` with a 0-10 rubric.

The `with-vera` arm is a fresh copy of Flask with a local Vera index and the project-scoped Droid skill installed. Its `PATH` resolves the release Vera binary first. The `control` arm is an otherwise identical copy with Vera artifacts removed and a `vera` shim that exits 127 with `vera: not available in this environment`. Questions run sequentially, alternating which arm goes first.

## Questions

Put exactly 10 questions in `flask/questions.md` under `## Question 1` through `## Question 10` headings. The harness reads this file before setup or analysis and fails clearly if it is missing or malformed. Keep any private grading material outside both sandbox copies; the harness does not copy it into an arm and does not include it in prompts.

## Reproduce

From the Vera repository root, build the harness's release binary and create the two arms:

```bash
python3 benchmarks/agent-bench/run.py --setup-only
```

The setup phase copies `.bench/semble-repos/flask` into a timestamped directory under `/tmp/agent-bench/`, excludes `.git`, `.vera`, and `.factory`, indexes only the Vera arm with `VERA_LOCAL=1`, and installs the Droid skill there. It does not modify the source Flask checkout.

The command prints the run directory. Use that path for the question sweep:

```bash
python3 benchmarks/agent-bench/run.py --run /tmp/agent-bench/<timestamp>
```

For a smoke sweep, limit both arms to the first question:

```bash
python3 benchmarks/agent-bench/run.py --run /tmp/agent-bench/<timestamp> --questions 1
```

To parse or re-parse existing JSONL outputs without invoking an agent:

```bash
python3 benchmarks/agent-bench/run.py --analyze /tmp/agent-bench/<timestamp>
```

Pick the tested model and reasoning effort with `--model` and `--effort` (defaults: `claude-opus-5`, `medium`). Each model+effort pair writes its own transcripts (`qNN.<model>-<effort>.jsonl`) and `results.<model>-<effort>.json`, so several lanes can share one run directory. Grade a lane's answers with:

```bash
python3 benchmarks/agent-bench/judge.py /tmp/agent-bench/<timestamp> <model>-<effort>
```

Running the script with no mode performs setup, the full sweep, and analysis sequentially.

## Limitations

This is one model, one repository, and one set of 10 questions. The arms share provider, prompt, model, reasoning effort, and sequential execution, but provider timing and model variance still affect measurements. Tool-call count and token accounting depend on the stream schema and are not a measure of answer correctness. The experiment has no statistical power claims and should not be generalized beyond this workload.
