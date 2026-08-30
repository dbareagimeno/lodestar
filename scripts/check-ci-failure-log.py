#!/usr/bin/env python3
"""Valida el registro append-only de causas de fallos de CI."""

from __future__ import annotations

import argparse
import json
import os
import re
import subprocess
from pathlib import Path
from typing import Any


LOG_PATH = Path("docs/qa/ci-failures.jsonl")
META = {"schema": "lodestar-ci-failures", "version": 1}
REQUIRED_FIELDS = {
    "id",
    "occurred_at",
    "run_url",
    "commit",
    "branch",
    "job",
    "platform",
    "classification",
    "symptom",
    "root_cause",
    "repair",
    "prevention",
    "process_improvement",
}
CLASSIFICATIONS = {
    "product",
    "test",
    "portability",
    "dependency",
    "policy",
    "documentation",
    "infrastructure",
    "flaky",
}
UTC_TIMESTAMP = re.compile(r"^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$")
COMMIT = re.compile(r"^[0-9a-f]{7,40}$")


def repo_root() -> Path:
    result = subprocess.run(
        ["git", "rev-parse", "--show-toplevel"],
        check=True,
        capture_output=True,
        text=True,
    )
    return Path(result.stdout.strip())


def git(*args: str, check: bool = True) -> subprocess.CompletedProcess[bytes]:
    return subprocess.run(["git", *args], check=check, capture_output=True)


def resolve_base(explicit: str | None) -> str:
    requested = explicit or os.environ.get("CI_FAILURE_LOG_BASE")
    if requested:
        candidates = [requested]
    else:
        candidates = []
    event_path = os.environ.get("GITHUB_EVENT_PATH")
    if not candidates and event_path and Path(event_path).is_file():
        try:
            event = json.loads(Path(event_path).read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError) as error:
            raise SystemExit(f"ERROR: no se pudo leer GITHUB_EVENT_PATH: {error}") from error
        pull_request = event.get("pull_request")
        if isinstance(pull_request, dict):
            base = pull_request.get("base")
            if isinstance(base, dict) and isinstance(base.get("sha"), str):
                candidates.append(base["sha"])
        before = event.get("before")
        if isinstance(before, str) and before.strip("0"):
            candidates.append(before)
    if not candidates and os.environ.get("GITHUB_BASE_REF"):
        name = os.environ["GITHUB_BASE_REF"]
        candidates = [f"origin/{name}", name]
    if not candidates:
        candidates = ["develop"]
    for candidate in candidates:
        if git("rev-parse", "--verify", "--quiet", f"{candidate}^{{commit}}", check=False).returncode == 0:
            return candidate
    raise SystemExit(f"ERROR: no se pudo resolver la base append-only: {candidates}")


def base_bytes(base: str) -> bytes:
    result = git("show", f"{base}:{LOG_PATH.as_posix()}", check=False)
    if result.returncode == 0:
        return result.stdout
    missing = git("cat-file", "-e", f"{base}:{LOG_PATH.as_posix()}", check=False)
    if missing.returncode != 0:
        return b""
    raise SystemExit(f"ERROR: no se pudo leer {LOG_PATH} desde {base}")


def nonempty_string(record: dict[str, Any], field: str, line: int) -> str:
    value = record.get(field)
    if not isinstance(value, str) or not value.strip():
        raise SystemExit(f"ERROR: {LOG_PATH}:{line}: {field} debe ser string no vacío")
    return value


def validate_entry(record: Any, line: int, seen_ids: set[str]) -> None:
    if not isinstance(record, dict):
        raise SystemExit(f"ERROR: {LOG_PATH}:{line}: cada entrada debe ser un objeto JSON")
    missing = REQUIRED_FIELDS - record.keys()
    if missing:
        raise SystemExit(f"ERROR: {LOG_PATH}:{line}: faltan campos: {sorted(missing)}")
    entry_id = nonempty_string(record, "id", line)
    if entry_id in seen_ids:
        raise SystemExit(f"ERROR: {LOG_PATH}:{line}: id duplicado: {entry_id}")
    if "supersedes" in record:
        supersedes = record["supersedes"]
        if not isinstance(supersedes, str) or supersedes not in seen_ids:
            raise SystemExit(
                f"ERROR: {LOG_PATH}:{line}: supersedes debe referir a un id anterior"
            )
    seen_ids.add(entry_id)
    timestamp = nonempty_string(record, "occurred_at", line)
    if not UTC_TIMESTAMP.fullmatch(timestamp):
        raise SystemExit(f"ERROR: {LOG_PATH}:{line}: occurred_at debe usar YYYY-MM-DDTHH:MM:SSZ")
    run_url = nonempty_string(record, "run_url", line)
    if not run_url.startswith("https://github.com/") or "/actions/runs/" not in run_url:
        raise SystemExit(f"ERROR: {LOG_PATH}:{line}: run_url no es una ejecución de GitHub Actions")
    commit = nonempty_string(record, "commit", line)
    if not COMMIT.fullmatch(commit):
        raise SystemExit(f"ERROR: {LOG_PATH}:{line}: commit no parece un SHA git")
    for field in ("branch", "job", "platform", "symptom", "root_cause", "repair", "prevention"):
        nonempty_string(record, field, line)
    classification = nonempty_string(record, "classification", line)
    if classification not in CLASSIFICATIONS:
        raise SystemExit(
            f"ERROR: {LOG_PATH}:{line}: classification debe ser una de {sorted(CLASSIFICATIONS)}"
        )
    improvement = record.get("process_improvement")
    if not isinstance(improvement, dict):
        raise SystemExit(f"ERROR: {LOG_PATH}:{line}: process_improvement debe ser un objeto")
    for field in ("agents", "skills"):
        values = improvement.get(field)
        if not isinstance(values, list) or not all(isinstance(value, str) for value in values):
            raise SystemExit(
                f"ERROR: {LOG_PATH}:{line}: process_improvement.{field} debe ser una lista de strings"
            )
    action = improvement.get("action")
    if not isinstance(action, str):
        raise SystemExit(
            f"ERROR: {LOG_PATH}:{line}: process_improvement.action debe ser un string, vacío si no aplica"
        )


def validate_jsonl(contents: bytes) -> int:
    if not contents.endswith(b"\n"):
        raise SystemExit(f"ERROR: {LOG_PATH} debe terminar en newline")
    try:
        text = contents.decode("utf-8")
    except UnicodeDecodeError as error:
        raise SystemExit(f"ERROR: {LOG_PATH} no es UTF-8: {error}") from error
    lines = text.splitlines()
    if not lines:
        raise SystemExit(f"ERROR: {LOG_PATH} está vacío")
    try:
        metadata = json.loads(lines[0])
    except json.JSONDecodeError as error:
        raise SystemExit(f"ERROR: {LOG_PATH}:1: JSON inválido: {error}") from error
    if metadata != META:
        raise SystemExit(f"ERROR: {LOG_PATH}:1 debe ser exactamente {META}")
    seen_ids: set[str] = set()
    for number, raw in enumerate(lines[1:], start=2):
        if not raw.strip():
            raise SystemExit(f"ERROR: {LOG_PATH}:{number}: no se permiten líneas vacías")
        try:
            record = json.loads(raw)
        except json.JSONDecodeError as error:
            raise SystemExit(f"ERROR: {LOG_PATH}:{number}: JSON inválido: {error}") from error
        validate_entry(record, number, seen_ids)
    return len(lines) - 1


def validate_append_only(current: bytes, previous: bytes, base: str) -> None:
    if previous and not current.startswith(previous):
        raise SystemExit(
            f"ERROR: {LOG_PATH} no es append-only respecto de {base}; "
            "restaura el prefijo y añade una entrada correctiva con supersedes"
        )


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--base", help="ref git que contiene el prefijo inmutable")
    args = parser.parse_args()
    root = repo_root()
    os.chdir(root)
    if not LOG_PATH.is_file():
        raise SystemExit(f"ERROR: falta el registro obligatorio {LOG_PATH}")
    current = LOG_PATH.read_bytes()
    entries = validate_jsonl(current)
    base = resolve_base(args.base)
    previous = base_bytes(base)
    if previous:
        validate_jsonl(previous)
    validate_append_only(current, previous, base)
    print(f"registro CI append-only: OK ({entries} causas, base {base})")


if __name__ == "__main__":
    main()
