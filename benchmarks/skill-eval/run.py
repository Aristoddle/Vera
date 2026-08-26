#!/usr/bin/env python3
"""Measure how reliably a Droid agent activates the Vera skill, and what each integration variant costs in tokens."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import re
import shutil
import subprocess
import sys
import time
from collections import Counter
from datetime import datetime, timezone
from pathlib import Path
from typing import Any, Iterable, NoReturn


REPO_ROOT = Path(__file__).resolve().parents[2]
SOURCE_REPO = REPO_ROOT / ".bench" / "semble-repos" / "flask"
SKILL_SOURCE = REPO_ROOT / "skills" / "vera"
VARIANTS_ROOT = Path(__file__).resolve().parent / "variants"
SCENARIOS_FILE = Path(__file__).resolve().parent / "scenarios.md"
VERA_BINARY = REPO_ROOT / "target" / "release" / "vera"
RUNS_ROOT = Path.home() / ".cache" / "skill-eval"
ARMS = ("bare", "snippet", "skill", "both", "skill-v2")
SNIPPET_ARMS = ("snippet", "both")
SKILL_ARMS = ("skill", "both", "skill-v2")
# Arms whose skill body comes from a candidate rewrite instead of the shipped
# skill. The variant directory holds only the files that differ; everything
# else is copied from the shipped skill so the arms differ by one file.
VARIANT_ARMS = {"skill-v2": "skill-v2"}

PROMPT_HEADER = """Answer the question about the codebase in the current directory.

This is a read-only task: do not modify files and do not install anything.
Cite evidence as path:line. Be concise.

"""

# Verbatim copy of AGENTS_MD_SNIPPET from crates/vera-cli/src/commands/agent.rs.
# `vera agent install` only writes this snippet through an interactive prompt,
# so the harness embeds the constant instead of invoking the installer for it.
AGENTS_MD_SNIPPET = r"""## Code Search

<!-- vera:begin -->

Use Vera before opening many files or running broad text search when you need to find where logic lives or how a feature works.

- `vera search "query"` for semantic code search. Describe behavior: "JWT validation", not "auth". If one phrasing misses, try 2-3 varied queries or add `--intent "goal"`.
- `vera search ... --changed`, `--since <rev>`, or `--base <rev>` when the task is limited to modified files or a PR diff
- `vera grep "pattern"` for exact text or regex in indexed files
- `vera structural definitions <symbol>`, `vera structural env <NAME>`, `vera structural routes`, or `vera structural impls <symbol>` for common structural tasks and explicit type relationships
- `vera explain-path path/to/file` to explain why a file is or is not indexed
- `vera references <symbol>` for callers and `vera references <symbol> --callees` for callees
- `vera overview` for a project summary (languages, entry points, hotspots). Add `--changed`, `--since <rev>`, or `--base <rev>` to scope it to modified files.
- `vera stats --json` for index health, including tree-sitter error, parse-failure, and Tier 0 fallback counts
- `vera search --deep "query"` for RAG-fusion query expansion + merged ranking
- Narrow `vera search` or `vera grep` with `--lang`, `--path`, `--type`, or `--scope docs`
- `vera watch .` to auto-update the index, or `vera update .` after edits (`vera index .` if `.vera/` is missing)
- For detailed usage, query patterns, and troubleshooting, read the Vera skill file installed by `vera agent install`
<!-- vera:end -->
"""

EXPECTED_CLASSES = ("VERA", "EXACT", "NONE")
SCENARIO_HEADING = re.compile(r"^##\s+S(\d+)\s+([a-z0-9-]+)\s*$")
EXPECTED_LINE = re.compile(r"^[-*]\s*Expected:\s*(VERA|EXACT|NONE)\s*$", re.IGNORECASE)
EXACT_SEARCH_PATTERN = re.compile(r"\b(?:rg|grep)\b")
TOKEN_KEYS = {
    "input": "tokens_in",
    "input_tokens": "tokens_in",
    "prompt_tokens": "tokens_in",
    "output": "tokens_out",
    "output_tokens": "tokens_out",
    "completion_tokens": "tokens_out",
    "cache_read": "cache_read",
    "cache_read_input_tokens": "cache_read",
    "cache_creation": "cache_creation",
    "cache_creation_input_tokens": "cache_creation",
}
COMMAND_KEYS = {"command", "cmd", "raw_input"}
WRAPPER_KEYS = {"arguments", "input", "parameters", "params", "args"}


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--setup-only",
        action="store_true",
        help="Create the four arms and the shared index, without agent runs",
    )
    mode.add_argument("--run", type=Path, metavar="RUN_DIR", help="Run agents in an existing run")
    mode.add_argument(
        "--analyze",
        type=Path,
        metavar="RUN_DIR",
        help="Parse existing JSONL outputs and write results.json",
    )
    parser.add_argument(
        "--scenarios", type=int, metavar="N", help="Use only the first N scenarios (1-8)"
    )
    parser.add_argument("--scenario", type=int, metavar="N", help="Use only scenario N")
    parser.add_argument("--arm", choices=ARMS, help="Use only one arm")
    parser.add_argument("--model", default="kimi-k3", help="droid model ID")
    parser.add_argument("--effort", default="medium", help="droid reasoning effort level")
    parser.add_argument(
        "--reps", type=int, default=2, metavar="N", help="Repetitions per (arm, scenario) cell"
    )
    parser.add_argument(
        "--force",
        action="store_true",
        help="Re-run cells that already completed successfully (default: skip them)",
    )
    return parser.parse_args()


def fail(message: str) -> NoReturn:
    raise SystemExit(f"skill-eval: error: {message}")


def lane_slug(model: str, effort: str) -> str:
    """Filesystem-safe tag for a model/effort lane (model IDs may contain '/')."""
    readable = re.sub(r"[^A-Za-z0-9._-]+", "-", f"{model}-{effort}")
    digest = hashlib.sha256(f"{model}\0{effort}".encode("utf-8")).hexdigest()[:12]
    return f"{readable}-{digest}"


def run_command(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str] | None = None,
    timeout: int,
) -> subprocess.CompletedProcess[str]:
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            env=env,
            text=True,
            stdout=subprocess.PIPE,
            stderr=subprocess.PIPE,
            timeout=timeout,
            check=False,
        )
    except FileNotFoundError as exc:
        fail(f"required executable is unavailable: {exc.filename}")
    except subprocess.TimeoutExpired:
        fail(f"command timed out after {timeout}s: {' '.join(command)}")
    if result.returncode != 0:
        fail(f"command failed ({result.returncode}): {' '.join(command)}\n{result.stderr[-1000:]}")
    return result


def ensure_binary() -> Path:
    if VERA_BINARY.is_file() and os.access(VERA_BINARY, os.X_OK):
        return VERA_BINARY
    print(f"Building {VERA_BINARY}...", file=sys.stderr)
    run_command(["cargo", "build", "--release", "--bin", "vera"], cwd=REPO_ROOT, timeout=1800)
    if not VERA_BINARY.is_file() or not os.access(VERA_BINARY, os.X_OK):
        fail(f"release binary was not produced: {VERA_BINARY}")
    return VERA_BINARY


FACTORY_HOME_FILES = (
    "settings.json",
    "auth.v2.file",
    "auth.v2.key",
    "host.json",
    "last-startup-version",
)


def build_factory_home(run_dir: Path) -> Path:
    """Copy the minimum droid config into a run-local home.

    The real `~/.factory` carries global AGENTS.md instructions, personal
    skills, and custom droids, all of which would change how the agent
    behaves regardless of the skill configuration under test.
    """
    home = run_dir / "factory-home"
    config = home / ".factory"
    config.mkdir(parents=True, exist_ok=True)
    source = Path.home() / ".factory"
    for name in FACTORY_HOME_FILES:
        candidate = source / name
        if candidate.is_file():
            shutil.copy2(candidate, config / name)
    return home


def vera_env(run_dir: Path | None = None) -> dict[str, str]:
    """Environment shared by indexing and agent cells: release binary first on PATH."""
    env = os.environ.copy()
    env["PATH"] = os.pathsep.join([str(VERA_BINARY.parent), env.get("PATH", "")])
    env["VERA_LOCAL"] = "1"
    if run_dir is not None:
        env["FACTORY_HOME_OVERRIDE"] = str(build_factory_home(run_dir))
    return env


def load_scenarios(limit: int | None) -> list[dict[str, Any]]:
    if not SCENARIOS_FILE.is_file():
        fail(f"scenarios file is missing: {SCENARIOS_FILE}")
    scenarios: list[dict[str, Any]] = []
    current: dict[str, Any] | None = None
    for line in SCENARIOS_FILE.read_text(encoding="utf-8").splitlines():
        heading = SCENARIO_HEADING.match(line)
        if heading:
            if current is not None:
                scenarios.append(current)
            current = {
                "id": int(heading.group(1)),
                "name": heading.group(2),
                "expected": None,
                "body": [],
            }
        elif current is not None:
            expected = EXPECTED_LINE.match(line)
            if expected:
                current["expected"] = expected.group(1).upper()
            elif line.strip():
                current["body"].append(line.strip())
    if current is not None:
        scenarios.append(current)

    ids = [scenario["id"] for scenario in scenarios]
    if not ids or ids != list(range(1, len(ids) + 1)):
        fail(
            f"{SCENARIOS_FILE} must contain consecutively numbered '## S<n> <name>' "
            f"sections; found {ids or 'none'}"
        )
    for scenario in scenarios:
        if scenario["expected"] not in EXPECTED_CLASSES:
            fail(
                f"scenario S{scenario['id']} needs an '- Expected: <CLASS>' line "
                f"({'/'.join(EXPECTED_CLASSES)})"
            )
        scenario["prompt"] = "\n".join(scenario.pop("body")).strip()
        if not scenario["prompt"]:
            fail(f"scenario S{scenario['id']} has no prompt text")
    if limit is not None:
        if not 1 <= limit <= len(scenarios):
            fail(f"--scenarios must be between 1 and {len(scenarios)}")
        scenarios = scenarios[:limit]
    return scenarios


def copy_repo(destination: Path) -> None:
    if not SOURCE_REPO.is_dir():
        fail(f"source repository is missing: {SOURCE_REPO}")
    destination.mkdir(parents=True, exist_ok=True)
    # .git stays: the git-only probe needs commit history to answer from.
    run_command(
        [
            "rsync",
            "-a",
            "--delete",
            "--exclude=.vera",
            "--exclude=.factory",
            "--exclude=answer-key.md",
            f"{SOURCE_REPO}/",
            f"{destination}/",
        ],
        cwd=REPO_ROOT,
        timeout=300,
    )


def apply_arm_config(arm: str, repo: Path) -> None:
    """Apply the arm's agent-facing configuration; everything else stays identical."""
    if arm in SNIPPET_ARMS:
        # Mirror the installer's fresh-file output: trimmed snippet plus newline.
        (repo / "AGENTS.md").write_text(AGENTS_MD_SNIPPET.rstrip() + "\n", encoding="utf-8")
    if arm in SKILL_ARMS:
        installed = repo / ".factory" / "skills" / "vera"
        shutil.copytree(SKILL_SOURCE, installed)
        variant = VARIANT_ARMS.get(arm)
        if variant:
            variant_dir = VARIANTS_ROOT / variant
            if not variant_dir.is_dir():
                fail(f"variant directory is missing: {variant_dir}")
            for candidate in sorted(variant_dir.rglob("*")):
                if candidate.is_file():
                    target = installed / candidate.relative_to(variant_dir)
                    target.parent.mkdir(parents=True, exist_ok=True)
                    shutil.copy2(candidate, target)
    if arm not in SNIPPET_ARMS and (repo / "AGENTS.md").exists():
        fail(f"{arm} arm must not contain AGENTS.md: {repo}")
    if arm not in SKILL_ARMS and (repo / ".factory").exists():
        fail(f"{arm} arm must not contain .factory artifacts: {repo}")


def make_run_dir() -> Path:
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%SZ")
    run_dir = RUNS_ROOT / stamp
    suffix = 1
    while run_dir.exists():
        run_dir = RUNS_ROOT / f"{stamp}-{suffix}"
        suffix += 1
    return run_dir


def setup_run(scenarios: list[dict[str, Any]]) -> Path:
    binary = ensure_binary()
    run_dir = make_run_dir()
    base_repo = run_dir / "base" / "repo"
    copy_repo(base_repo)

    index_log = run_dir / "base" / "index.log"
    with index_log.open("w", encoding="utf-8") as output:
        result = subprocess.run(
            ["vera", "index", "."],
            cwd=base_repo,
            env=vera_env(),
            text=True,
            stdout=output,
            stderr=subprocess.STDOUT,
            timeout=1800,
            check=False,
        )
    if result.returncode != 0 or not (base_repo / ".vera").is_dir():
        fail(f"Vera indexing failed; see {index_log}")

    for arm in ARMS:
        repo = run_dir / arm / "repo"
        (run_dir / arm / "prompts").mkdir(parents=True)
        shutil.copytree(base_repo, repo, symlinks=True)
        apply_arm_config(arm, repo)
    write_prompts(run_dir, scenarios)

    metadata = {
        "created_at": datetime.now(timezone.utc).isoformat(),
        "source_repo": str(SOURCE_REPO),
        "vera_binary": str(binary),
        "scenarios_file": str(SCENARIOS_FILE),
        "scenarios": scenarios,
        "arms": {
            arm: {
                "repo": str(run_dir / arm / "repo"),
                "agents_md_snippet": arm in SNIPPET_ARMS,
                "skill": arm in SKILL_ARMS,
            }
            for arm in ARMS
        },
    }
    (run_dir / "setup.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    print(f"Setup complete: {run_dir}")
    return run_dir


def load_setup(run_dir: Path) -> dict[str, Any]:
    path = run_dir / "setup.json"
    if not path.is_file():
        fail(f"missing setup metadata: {path}; create the run with --setup-only first")
    return json.loads(path.read_text(encoding="utf-8"))


def select_scenarios(stored: list[dict[str, Any]], args: argparse.Namespace) -> list[dict[str, Any]]:
    scenarios = stored
    if args.scenarios is not None:
        if not 1 <= args.scenarios <= len(stored):
            fail(f"--scenarios must be between 1 and {len(stored)}")
        scenarios = stored[: args.scenarios]
    if args.scenario is not None:
        found = [scenario for scenario in scenarios if scenario["id"] == args.scenario]
        if not found:
            fail(f"--scenario {args.scenario} does not match any selected scenario")
        scenarios = found
    return scenarios


def selected_arms(args: argparse.Namespace) -> tuple[str, ...]:
    return (args.arm,) if args.arm else ARMS


def write_prompts(run_dir: Path, scenarios: list[dict[str, Any]]) -> None:
    for arm in ARMS:
        prompt_dir = run_dir / arm / "prompts"
        prompt_dir.mkdir(parents=True, exist_ok=True)
        for scenario in scenarios:
            path = prompt_dir / f"s{scenario['id']:02d}.md"
            path.write_text(PROMPT_HEADER + scenario["prompt"] + "\n", encoding="utf-8")


def cell_paths(arm_dir: Path, scenario_id: int, slug: str, rep: int) -> tuple[Path, Path, Path]:
    stem = f"s{scenario_id:02d}.{slug}.r{rep}"
    return (
        arm_dir / f"{stem}.jsonl",
        arm_dir / f"{stem}.stderr.log",
        arm_dir / f"{stem}.run.json",
    )


def rep_numbers(arm_dir: Path, scenario_id: int, slug: str) -> list[int]:
    numbers = set()
    for path in arm_dir.glob(f"s{scenario_id:02d}.{slug}.r*.jsonl"):
        match = re.search(r"\.r(\d+)\.jsonl$", path.name)
        if match:
            numbers.add(int(match.group(1)))
    return sorted(numbers)


def run_cell(
    run_dir: Path,
    arm: str,
    scenario: dict[str, Any],
    rep: int,
    model: str,
    effort: str,
    force: bool,
) -> None:
    arm_dir = run_dir / arm
    repo_dir = arm_dir / "repo"
    slug = lane_slug(model, effort)
    output_path, stderr_path, meta_path = cell_paths(arm_dir, scenario["id"], slug, rep)
    if not force and output_path.is_file() and meta_path.is_file():
        try:
            if json.loads(meta_path.read_text(encoding="utf-8")).get("returncode") == 0:
                print(f"  {arm} s{scenario['id']:02d} r{rep}: skipped (already done)")
                return
        except json.JSONDecodeError:
            pass
    prompt = arm_dir / "prompts" / f"s{scenario['id']:02d}.md"
    start = time.monotonic()
    timeout_s = 7200
    returncode: int | None = None
    timed_out = False
    try:
        with output_path.open("w", encoding="utf-8") as output, stderr_path.open(
            "w", encoding="utf-8"
        ) as errors:
            result = subprocess.run(
                [
                    "droid",
                    "exec",
                    "--cwd",
                    str(repo_dir),
                    "--disable-builtin-skills",
                    "--auto",
                    "medium",
                    "-o",
                    "stream-json",
                    "-m",
                    model,
                    "-r",
                    effort,
                    "-f",
                    str(prompt),
                ],
                cwd=repo_dir,
                env=vera_env(run_dir),
                text=True,
                stdout=output,
                stderr=errors,
                timeout=timeout_s,
                check=False,
            )
            returncode = result.returncode
    except subprocess.TimeoutExpired:
        timed_out = True
    wall_s = time.monotonic() - start
    metadata: dict[str, Any] = {"returncode": returncode, "wall_s": wall_s}
    if timed_out:
        metadata.update({"failed": True, "timed_out": True, "timeout_s": timeout_s})
    meta_path.write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    status = "timeout" if timed_out else ("ok" if returncode == 0 else f"failed ({returncode})")
    print(f"  {arm} s{scenario['id']:02d} r{rep}: {status}, {wall_s:.1f}s")


def run_agents(
    run_dir: Path,
    scenarios: list[dict[str, Any]],
    arms: tuple[str, ...],
    model: str,
    effort: str,
    reps: int,
    force: bool,
) -> None:
    for arm in arms:
        if not (run_dir / arm / "repo").is_dir():
            fail(f"run directory is missing {arm}/repo: {run_dir}")
    write_prompts(run_dir, scenarios)
    total_cells = len(arms) * len(scenarios) * reps
    print(f"Running {total_cells} cells in {run_dir} with {model} ({effort})")
    for scenario in scenarios:
        for arm in arms:
            for rep in range(1, reps + 1):
                run_cell(run_dir, arm, scenario, rep, model, effort, force)


def json_objects(path: Path) -> Iterable[dict[str, Any]]:
    if not path.is_file():
        return
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        try:
            value = json.loads(line)
        except json.JSONDecodeError:
            continue
        if isinstance(value, dict):
            yield value


def first_nested(value: Any, keys: set[str]) -> Any:
    if isinstance(value, dict):
        for key in keys:
            if key in value:
                return value[key]
        for nested_value in value.values():
            found = first_nested(nested_value, keys)
            if found is not None:
                return found
    elif isinstance(value, list):
        for nested_value in value:
            found = first_nested(nested_value, keys)
            if found is not None:
                return found
    return None


def event_type(event: dict[str, Any]) -> str:
    value = event.get("type")
    return value if isinstance(value, str) else ""


def tool_name(event: dict[str, Any]) -> str | None:
    event_kind = event_type(event).lower()
    if event_kind not in {"tool_call", "function_call", "tool_use", "function_use"}:
        return None
    direct = first_nested(event, {"tool_name", "toolName", "name"})
    return direct if isinstance(direct, str) and direct else event_kind


def add_usage(event: dict[str, Any], totals: Counter[str]) -> None:
    usage = first_nested(event, {"usage"})
    if not isinstance(usage, dict):
        return
    for key, total_key in TOKEN_KEYS.items():
        value = usage.get(key)
        if isinstance(value, (int, float)):
            totals[total_key] += value


def completion_text(event: dict[str, Any]) -> str:
    direct = event.get("finalText")
    if isinstance(direct, str):
        return direct
    direct = event.get("final_text")
    if isinstance(direct, str):
        return direct
    for key in ("text", "content", "output_text"):
        value = event.get(key)
        if isinstance(value, str):
            return value
    return ""


def iter_commands(value: Any) -> Iterable[str]:
    """Yield shell command strings embedded anywhere in a stream event.

    Handles both plain fields ("command": "...") and JSON-encoded argument
    payloads ("arguments"/"input": "{\"command\": \"...\"}").
    """
    if isinstance(value, dict):
        for key, item in value.items():
            if isinstance(item, str):
                if key in COMMAND_KEYS:
                    yield item
                elif key in WRAPPER_KEYS:
                    stripped = item.lstrip()
                    if stripped[:1] in "{[":
                        try:
                            yield from iter_commands(json.loads(item))
                            continue
                        except json.JSONDecodeError:
                            pass
                    yield item
            else:
                yield from iter_commands(item)
    elif isinstance(value, list):
        for item in value:
            yield from iter_commands(item)


def parse_jsonl(path: Path, run_meta: Path | None = None) -> dict[str, Any]:
    calls: Counter[str] = Counter()
    totals: Counter[str] = Counter()
    vera_calls = 0
    exact_calls = 0
    read_calls = 0
    answer = ""
    duration_ms: float | None = None
    event_count = 0
    for event in json_objects(path):
        event_count += 1
        name = tool_name(event)
        if name is not None:
            calls[name] += 1
            if name.lower() == "read":
                read_calls += 1
        commands = list(dict.fromkeys(c.strip() for c in iter_commands(event)))
        for command in commands:
            lowered = command.lower()
            if lowered.startswith("vera ") or lowered == "vera":
                vera_calls += 1
            elif EXACT_SEARCH_PATTERN.search(lowered):
                exact_calls += 1
        if event_type(event) == "completion":
            add_usage(event, totals)
            duration = event.get("durationMs")
            if isinstance(duration, (int, float)):
                duration_ms = float(duration)
            text = completion_text(event)
            if text:
                answer = text
        final_text = event.get("finalText")
        if isinstance(final_text, str) and final_text:
            answer = final_text

    wall_s: float | None = None
    returncode: int | None = None
    timed_out = False
    if run_meta is not None and run_meta.is_file():
        metadata = json.loads(run_meta.read_text(encoding="utf-8"))
        wall_s = metadata.get("wall_s")
        returncode = metadata.get("returncode")
        timed_out = metadata.get("timed_out") is True
    behavior = "vera" if vera_calls else ("exact" if exact_calls else "none")
    return {
        "tool_calls": dict(sorted(calls.items())),
        "vera_calls": vera_calls,
        "exact_search_calls": exact_calls,
        "read_calls": read_calls,
        "behavior": behavior,
        "tokens_in": totals["tokens_in"],
        "tokens_out": totals["tokens_out"],
        "cache_read": totals["cache_read"],
        "cache_creation": totals["cache_creation"],
        "wall_s": wall_s,
        "duration_ms": duration_ms,
        "answer": answer,
        "event_count": event_count,
        **({"timed_out": True} if timed_out else {}),
        **({"returncode": returncode} if returncode is not None else {}),
    }


def matches_expected(expected: str, behavior: str) -> bool:
    if expected == "VERA":
        return behavior == "vera"
    if expected == "EXACT":
        return behavior in ("vera", "exact")
    return behavior == "none"


def mean(values: list[float]) -> float | None:
    return sum(values) / len(values) if values else None


def analyze_run(
    run_dir: Path,
    scenarios: list[dict[str, Any]],
    arms: tuple[str, ...],
    model: str,
    effort: str,
) -> dict[str, Any]:
    if not run_dir.is_dir():
        fail(f"run directory does not exist: {run_dir}")
    slug = lane_slug(model, effort)
    result: dict[str, Any] = {
        "run_dir": str(run_dir),
        "model": model,
        "effort": effort,
        "scenarios": scenarios,
        "cells": {},
        "arms": {},
    }
    for arm in arms:
        arm_dir = run_dir / arm
        result["cells"][arm] = {}
        ok_reps: list[dict[str, Any]] = []
        cells_scored = 0
        cells_matched = 0
        for scenario in scenarios:
            sid = scenario["id"]
            runs = []
            for rep in rep_numbers(arm_dir, sid, slug):
                stem = f"s{sid:02d}.{slug}.r{rep}"
                parsed = parse_jsonl(arm_dir / f"{stem}.jsonl", arm_dir / f"{stem}.run.json")
                # A nonzero droid exit means the cell is not a valid measurement.
                if (
                    parsed["event_count"] == 0
                    or parsed.get("returncode") not in (None, 0)
                    or parsed.get("timed_out")
                ):
                    parsed["failed"] = True
                parsed["matched"] = (not parsed.get("failed")) and matches_expected(
                    scenario["expected"], parsed["behavior"]
                )
                runs.append({"rep": rep, **parsed})
            ok = [run for run in runs if not run.get("failed")]
            match_rate = (sum(1 for run in ok if run["matched"]) / len(ok)) if ok else None
            cell_matched = bool(ok) and match_rate >= 0.5
            cells_scored += 1 if ok else 0
            cells_matched += 1 if cell_matched else 0
            ok_reps.extend(ok)
            result["cells"][arm][f"S{sid:02d}"] = {
                "name": scenario["name"],
                "expected": scenario["expected"],
                "reps": len(runs),
                "reps_ok": len(ok),
                "reps_matched": sum(1 for run in ok if run["matched"]),
                "matched": cell_matched if ok else None,
                "match_rate": match_rate,
                "vera_calls_total": sum(run["vera_calls"] for run in ok),
                "exact_search_calls_total": sum(run["exact_search_calls"] for run in ok),
                "read_calls_total": sum(run["read_calls"] for run in ok),
                "tool_calls_total": dict(sum((Counter(r["tool_calls"]) for r in ok), Counter())),
                "tokens_in_mean": mean([float(r["tokens_in"]) for r in ok]),
                "tokens_out_mean": mean([float(r["tokens_out"]) for r in ok]),
                "wall_s_mean": mean([r["wall_s"] or 0.0 for r in ok]),
                "runs": runs,
            }
        total_tokens = [r["tokens_in"] + r["tokens_out"] for r in ok_reps]
        result["arms"][arm] = {
            "cells_scored": cells_scored,
            "cells_matched": cells_matched,
            "activation_appropriateness": (
                cells_matched / cells_scored if cells_scored else None
            ),
            "runs_ok": len(ok_reps),
            "mean_total_tokens": mean([float(value) for value in total_tokens]),
            "tokens_in_total": sum(r["tokens_in"] for r in ok_reps),
            "tokens_out_total": sum(r["tokens_out"] for r in ok_reps),
            "wall_s_total": sum(r["wall_s"] or 0.0 for r in ok_reps),
        }

    result_path = run_dir / f"results.{slug}.json"
    result_path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print_report(result)
    print(f"Wrote {result_path}")
    return result


def print_report(result: dict[str, Any]) -> None:
    print("\nCell results")
    print(
        f"{'arm':<8} {'scenario':<22} {'exp':<6} {'match':<5} "
        f"{'reps':>5} {'tok_in':>10} {'tok_out':>9} {'wall_s':>7}"
    )
    for arm, cells in result["cells"].items():
        for key, cell in cells.items():
            matched = "-" if cell["matched"] is None else ("yes" if cell["matched"] else "no")
            tok_in = "-" if cell["tokens_in_mean"] is None else f"{cell['tokens_in_mean']:.0f}"
            tok_out = "-" if cell["tokens_out_mean"] is None else f"{cell['tokens_out_mean']:.0f}"
            wall = "-" if cell["wall_s_mean"] is None else f"{cell['wall_s_mean']:.1f}"
            label = f"{key} {cell['name']}"
            print(
                f"{arm:<8} {label:<22} {cell['expected']:<6} {matched:<5} "
                f"{cell['reps_matched']}/{cell['reps_ok']:<3} "
                f"{tok_in:>10} {tok_out:>9} {wall:>7}"
            )
    print("\nArm summary")
    print(
        f"{'arm':<8} {'appropriateness':>15} {'mean_total_tok':>15} "
        f"{'runs_ok':>8} {'tok_in':>12} {'tok_out':>10} {'wall_s':>9}"
    )
    for arm, summary in result["arms"].items():
        appropriateness = summary["activation_appropriateness"]
        mean_total = summary["mean_total_tokens"]
        appr_text = "-" if appropriateness is None else f"{appropriateness:.0%}"
        total_text = "-" if mean_total is None else f"{mean_total:.0f}"
        print(
            f"{arm:<8} {appr_text:>15} {total_text:>15} {summary['runs_ok']:>8} "
            f"{summary['tokens_in_total']:>12} {summary['tokens_out_total']:>10} "
            f"{summary['wall_s_total']:>9.1f}"
        )


def main() -> None:
    args = parse_args()
    if args.reps < 1:
        fail("--reps must be at least 1")
    if args.analyze is not None:
        run_dir = args.analyze.resolve()
        scenarios = select_scenarios(load_setup(run_dir)["scenarios"], args)
        analyze_run(run_dir, scenarios, selected_arms(args), args.model, args.effort)
        return
    if args.run is not None:
        run_dir = args.run.resolve()
        stored = load_setup(run_dir)["scenarios"]
    else:
        stored = load_scenarios(None)
        run_dir = setup_run(stored)
        if args.setup_only:
            return
    scenarios = select_scenarios(stored, args)
    arms = selected_arms(args)
    run_agents(run_dir, scenarios, arms, args.model, args.effort, args.reps, args.force)
    analyze_run(run_dir, scenarios, arms, args.model, args.effort)


if __name__ == "__main__":
    main()
