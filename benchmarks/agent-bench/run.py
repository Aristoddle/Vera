#!/usr/bin/env python3
"""Run a small agent-level A/B benchmark for Vera."""

from __future__ import annotations

import argparse
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
QUESTIONS_FILE = Path(__file__).resolve().parent / "flask" / "questions.md"
VERA_BINARY = REPO_ROOT / "target" / "release" / "vera"
RUNS_ROOT = Path("/tmp/agent-bench")
ARMS = ("with-vera", "control")
PROMPT_HEADER = """Answer the question about the codebase in the current directory.

This is a read-only task: do not modify files and do not install anything.
Cite evidence as path:line. Answer every subquestion. Finish with a per-subquestion confidence table.

"""
QUESTION_START = re.compile(
    r"^#{1,6}\s+Question\s+(\d+)\s*$",
    re.IGNORECASE,
)
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


def parse_args() -> argparse.Namespace:
    parser = argparse.ArgumentParser(description=__doc__)
    mode = parser.add_mutually_exclusive_group()
    mode.add_argument(
        "--setup-only",
        action="store_true",
        help="Create both arms and index the Vera arm, without agent runs",
    )
    mode.add_argument("--run", type=Path, metavar="RUN_DIR", help="Run agents in an existing run")
    mode.add_argument(
        "--analyze",
        type=Path,
        metavar="RUN_DIR",
        help="Parse existing JSONL outputs and write results.json",
    )
    parser.add_argument(
        "--questions",
        type=int,
        metavar="N",
        help="Use only the first N questions (1-10)",
    )
    parser.add_argument("--model", default="claude-opus-5", help="droid model ID")
    parser.add_argument(
        "--effort", default="medium", help="droid reasoning effort level"
    )
    return parser.parse_args()


def fail(message: str) -> NoReturn:
    raise SystemExit(f"agent-bench: error: {message}")


def load_questions(limit: int | None) -> list[dict[str, Any]]:
    if not QUESTIONS_FILE.is_file():
        fail(
            f"questions file is missing: {QUESTIONS_FILE}. "
            "Create it with exactly 10 numbered questions."
        )
    sections: list[tuple[int, str, list[str]]] = []
    current: tuple[int, str, list[str]] | None = None
    for line in QUESTIONS_FILE.read_text(encoding="utf-8").splitlines():
        match = QUESTION_START.match(line)
        if match:
            if current is not None:
                sections.append(current)
            current = (int(match.group(1)), "", [])
        elif current is not None:
            current[2].append(line)
    if current is not None:
        sections.append(current)

    numbers = [number for number, _, _ in sections]
    if numbers != list(range(1, 11)):
        fail(
            f"{QUESTIONS_FILE} must contain exactly the numbered questions 1 through 10 "
            f"at top level; found {numbers or 'none'}"
        )
    questions = []
    for number, title, body in sections:
        text = "\n".join([title, *body]).strip()
        if not text:
            fail(f"question {number} in {QUESTIONS_FILE} is empty")
        questions.append({"number": number, "text": text})
    if limit is not None:
        if not 1 <= limit <= len(questions):
            fail(f"--questions must be between 1 and {len(questions)}")
        questions = questions[:limit]
    return questions


def ensure_binary() -> Path:
    if VERA_BINARY.is_file() and os.access(VERA_BINARY, os.X_OK):
        return VERA_BINARY
    print(f"Building {VERA_BINARY}...", file=sys.stderr)
    run_command(
        ["cargo", "build", "--release", "--bin", "vera"],
        cwd=REPO_ROOT,
        timeout=1800,
    )
    if not VERA_BINARY.is_file() or not os.access(VERA_BINARY, os.X_OK):
        fail(f"release binary was not produced: {VERA_BINARY}")
    return VERA_BINARY


def run_command(
    command: list[str],
    *,
    cwd: Path,
    env: dict[str, str] | None = None,
    timeout: int,
    stdout: Any = subprocess.PIPE,
    stderr: Any = subprocess.PIPE,
) -> subprocess.CompletedProcess[str]:
    try:
        result = subprocess.run(
            command,
            cwd=cwd,
            env=env,
            text=True,
            stdout=stdout,
            stderr=stderr,
            timeout=timeout,
            check=False,
        )
    except FileNotFoundError as exc:
        fail(f"required executable is unavailable: {exc.filename}")
    except subprocess.TimeoutExpired:
        fail(f"command timed out after {timeout}s: {' '.join(command)}")
    if result.returncode != 0:
        stderr_text = result.stderr if isinstance(result.stderr, str) else ""
        fail(f"command failed ({result.returncode}): {' '.join(command)}\n{stderr_text[-1000:]}")
    return result


def make_run_dir() -> Path:
    RUNS_ROOT.mkdir(parents=True, exist_ok=True)
    stamp = datetime.now(timezone.utc).strftime("%Y%m%dT%H%M%S%fZ")
    run_dir = RUNS_ROOT / stamp
    suffix = 1
    while run_dir.exists():
        run_dir = RUNS_ROOT / f"{stamp}-{suffix}"
        suffix += 1
    for arm in ARMS:
        (run_dir / arm / "repo").mkdir(parents=True)
        (run_dir / arm / "prompts").mkdir()
    return run_dir


def copy_repo(destination: Path) -> None:
    if not SOURCE_REPO.is_dir():
        fail(f"source repository is missing: {SOURCE_REPO}")
    if shutil.which("rsync") is None:
        fail("rsync is required to create benchmark copies")
    destination.mkdir(parents=True, exist_ok=True)
    run_command(
        [
            "rsync",
            "-a",
            "--delete",
            "--exclude=.git",
            "--exclude=.vera",
            "--exclude=.factory",
            "--exclude=answer-key.md",
            f"{SOURCE_REPO}/",
            f"{destination}/",
        ],
        cwd=REPO_ROOT,
        timeout=300,
    )


def environment_for(arm: str, *, shim_dir: Path | None = None) -> dict[str, str]:
    env = os.environ.copy()
    binary_dir = str(VERA_BINARY.parent)
    path_parts = [binary_dir]
    if shim_dir is not None:
        path_parts.insert(0, str(shim_dir))
        env.pop("VERA_LOCAL", None)
    else:
        env["VERA_LOCAL"] = "1"
    env["PATH"] = os.pathsep.join(path_parts + [env.get("PATH", "")])
    return env


def write_control_shim(shim_dir: Path) -> None:
    shim_dir.mkdir(parents=True, exist_ok=True)
    shim = shim_dir / "vera"
    shim.write_text(
        "#!/bin/sh\nprintf '%s\\n' 'vera: not available in this environment' >&2\nexit 127\n",
        encoding="ascii",
    )
    shim.chmod(0o755)


def setup_run(questions: list[dict[str, Any]]) -> Path:
    binary = ensure_binary()
    run_dir = make_run_dir()
    with_repo = run_dir / "with-vera" / "repo"
    control_repo = run_dir / "control" / "repo"
    copy_repo(with_repo)
    copy_repo(control_repo)
    shutil.rmtree(control_repo / ".vera", ignore_errors=True)
    shutil.rmtree(control_repo / ".factory", ignore_errors=True)

    shim_dir = run_dir / "control" / "bin"
    write_control_shim(shim_dir)
    with_env = environment_for("with-vera")
    index_log = run_dir / "with-vera" / "index.log"
    install_log = run_dir / "with-vera" / "agent-install.log"
    with index_log.open("w", encoding="utf-8") as output:
        result = subprocess.run(
            ["vera", "index", "."],
            cwd=with_repo,
            env=with_env,
            text=True,
            stdout=output,
            stderr=subprocess.STDOUT,
            timeout=1800,
            check=False,
        )
    if result.returncode != 0:
        fail(f"Vera indexing failed; see {index_log}")

    with install_log.open("w", encoding="utf-8") as output:
        result = subprocess.run(
            ["vera", "agent", "install", "--client", "droid", "--scope", "project"],
            cwd=with_repo,
            env=with_env,
            text=True,
            stdout=output,
            stderr=subprocess.STDOUT,
            timeout=300,
            check=False,
        )
    # `vera agent install` exits non-zero in this sandbox after a successful
    # install (a later connectivity check fails without a configured endpoint),
    # so judge success by the installed skill files, not the exit code.
    skill_dir = with_repo / ".factory" / "skills" / "vera"
    if not skill_dir.is_dir():
        fail(f"Vera agent installation failed; see {install_log}")
    if (control_repo / ".vera").exists() or (control_repo / ".factory").exists():
        fail("control arm contains Vera artifacts")

    metadata = {
        "created_at": datetime.now(timezone.utc).isoformat(),
        "source_repo": str(SOURCE_REPO),
        "vera_binary": str(binary),
        "questions_file": str(QUESTIONS_FILE),
        "questions": questions,
        "arms": {
            "with-vera": {"repo": str(with_repo), "path_prefix": str(binary.parent)},
            "control": {"repo": str(control_repo), "shim": str(shim_dir)},
        },
    }
    (run_dir / "setup.json").write_text(json.dumps(metadata, indent=2) + "\n", encoding="utf-8")
    print(f"Setup complete: {run_dir}")
    return run_dir


def write_prompts(run_dir: Path, questions: list[dict[str, Any]]) -> None:
    for arm in ARMS:
        prompt_dir = run_dir / arm / "prompts"
        prompt_dir.mkdir(parents=True, exist_ok=True)
        for question in questions:
            path = prompt_dir / f"q{question['number']:02d}.md"
            path.write_text(PROMPT_HEADER + question["text"] + "\n", encoding="utf-8")


def run_question(
    run_dir: Path, arm: str, question: dict[str, Any], model: str, effort: str
) -> None:
    arm_dir = run_dir / arm
    repo_dir = arm_dir / "repo"
    prompt = arm_dir / "prompts" / f"q{question['number']:02d}.md"
    suffix = f"q{question['number']:02d}.{model}-{effort}.jsonl"
    output_path = arm_dir / suffix
    stderr_path = arm_dir / f"q{question['number']:02d}.{model}-{effort}.stderr.log"
    shim_dir = run_dir / "control" / "bin" if arm == "control" else None
    env = environment_for(arm, shim_dir=shim_dir)
    start = time.monotonic()
    with output_path.open("w", encoding="utf-8") as output, stderr_path.open(
        "w", encoding="utf-8"
    ) as errors:
        result = subprocess.run(
            [
                "droid",
                "exec",
                "--cwd",
                str(repo_dir),
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
            env=env,
            text=True,
            stdout=output,
            stderr=errors,
            timeout=7200,
            check=False,
        )
    wall_s = time.monotonic() - start
    (arm_dir / f"q{question['number']:02d}.{model}-{effort}.run.json").write_text(
        json.dumps({"returncode": result.returncode, "wall_s": wall_s}, indent=2) + "\n",
        encoding="utf-8",
    )
    status = "ok" if result.returncode == 0 else f"failed ({result.returncode})"
    print(f"  {arm} q{question['number']:02d}: {status}, {wall_s:.1f}s")


def run_agents(
    run_dir: Path, questions: list[dict[str, Any]], model: str, effort: str
) -> None:
    for arm in ARMS:
        if not (run_dir / arm / "repo").is_dir():
            fail(f"run directory is missing {arm}/repo: {run_dir}")
    write_prompts(run_dir, questions)
    arms_by_question = [
        ("with-vera", "control") if question["number"] % 2 else ("control", "with-vera")
        for question in questions
    ]
    print(f"Running {len(questions)} questions in {run_dir} with {model} ({effort})")
    for question, arm_order in zip(questions, arms_by_question):
        for arm in arm_order:
            run_question(run_dir, arm, question, model, effort)


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


def parse_jsonl(path: Path, run_meta: Path | None = None) -> dict[str, Any]:
    calls: Counter[str] = Counter()
    totals: Counter[str] = Counter()
    answer = ""
    duration_ms: float | None = None
    event_count = 0
    for event in json_objects(path):
        event_count += 1
        name = tool_name(event)
        if name is not None:
            calls[name] += 1
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
    if run_meta is not None and run_meta.is_file():
        metadata = json.loads(run_meta.read_text(encoding="utf-8"))
        wall_s = metadata.get("wall_s")
        returncode = metadata.get("returncode")
    return {
        "tool_calls": dict(sorted(calls.items())),
        "tokens_in": totals["tokens_in"],
        "tokens_out": totals["tokens_out"],
        "cache_read": totals["cache_read"],
        "cache_creation": totals["cache_creation"],
        "wall_s": wall_s,
        "duration_ms": duration_ms,
        "answer": answer,
        "event_count": event_count,
        **({"returncode": returncode} if returncode is not None else {}),
    }


def analyze_run(
    run_dir: Path, questions: list[dict[str, Any]], model: str, effort: str
) -> dict[str, Any]:
    if not run_dir.is_dir():
        fail(f"run directory does not exist: {run_dir}")
    result: dict[str, Any] = {
        "run_dir": str(run_dir),
        "model": model,
        "effort": effort,
        "questions": {},
        "summary": {},
    }
    for question in questions:
        number = question["number"]
        result["questions"][f"q{number:02d}"] = {}
        for arm in ARMS:
            arm_dir = run_dir / arm
            result["questions"][f"q{number:02d}"][arm] = parse_jsonl(
                arm_dir / f"q{number:02d}.{model}-{effort}.jsonl",
                arm_dir / f"q{number:02d}.{model}-{effort}.run.json",
            )

    for arm in ARMS:
        rows = [result["questions"][f"q{q['number']:02d}"][arm] for q in questions]
        result["summary"][arm] = {
            "questions": len(rows),
            "tool_calls": sum((Counter(row["tool_calls"]) for row in rows), Counter()),
            "tokens_in": sum(row["tokens_in"] for row in rows),
            "tokens_out": sum(row["tokens_out"] for row in rows),
            "cache_read": sum(row["cache_read"] for row in rows),
            "cache_creation": sum(row["cache_creation"] for row in rows),
            "wall_s_total": sum(row["wall_s"] or 0.0 for row in rows),
            "duration_ms_total": sum(row["duration_ms"] or 0.0 for row in rows),
        }
    result_path = run_dir / f"results.{model}-{effort}.json"
    result_path.write_text(json.dumps(result, indent=2) + "\n", encoding="utf-8")
    print_comparison(result)
    print(f"Wrote {result_path}")
    return result


def print_comparison(result: dict[str, Any]) -> None:
    print("\nArm comparison")
    print("arm         questions  tool calls  tokens in  tokens out  wall s  duration ms")
    for arm in ARMS:
        summary = result["summary"][arm]
        tool_count = sum(summary["tool_calls"].values())
        print(
            f"{arm:<11} {summary['questions']:>9}  {tool_count:>10}  "
            f"{summary['tokens_in']:>9}  {summary['tokens_out']:>10}  "
            f"{summary['wall_s_total']:>6.1f}  {summary['duration_ms_total']:>12.0f}"
        )


def main() -> None:
    args = parse_args()
    questions = load_questions(args.questions)
    if args.analyze is not None:
        analyze_run(args.analyze.resolve(), questions, args.model, args.effort)
        return
    if args.run is not None:
        run_agents(args.run.resolve(), questions, args.model, args.effort)
        analyze_run(args.run.resolve(), questions, args.model, args.effort)
        return
    run_dir = setup_run(questions)
    if args.setup_only:
        return
    run_agents(run_dir, questions, args.model, args.effort)
    analyze_run(run_dir, questions, args.model, args.effort)


if __name__ == "__main__":
    main()
