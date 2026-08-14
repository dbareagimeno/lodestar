#!/usr/bin/env python3
"""Lock exact TDD tests and fixtures by content hash."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import sys
from pathlib import Path, PurePosixPath


LOCK_VERSION = 1


def repo_root() -> Path:
    current = Path.cwd().resolve()
    for candidate in (current, *current.parents):
        if (candidate / ".git").exists():
            return candidate
    raise SystemExit("error: ejecuta el script dentro de un checkout git")


def allowed_lock_path(rel: PurePosixPath) -> bool:
    parts = rel.parts
    if len(parts) >= 4 and parts[0] == "crates" and parts[2] == "tests":
        return True
    if len(parts) >= 4 and parts[:2] == ("crates", "lodestar-fixtures") and parts[2] in {
        "src",
        "tests",
        "fixtures",
        "testdata",
    }:
        return True
    if len(parts) >= 4 and parts[0] == "crates" and parts[2] in {"fixtures", "testdata"}:
        return True
    return bool(parts) and parts[0] in {"tests", "fixtures", "testdata"}


def relative_inside(root: Path, candidate: Path) -> PurePosixPath:
    resolved = candidate.resolve()
    try:
        relative = resolved.relative_to(root)
    except ValueError as error:
        raise SystemExit(f"error: path fuera del checkout: {candidate}") from error
    return PurePosixPath(*relative.parts)


def digest(path: Path) -> str:
    if path.is_symlink():
        return "symlink:" + os.readlink(path)
    hasher = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            hasher.update(chunk)
    return hasher.hexdigest()


def expand_inputs(root: Path, inputs: list[Path]) -> list[Path]:
    expanded: list[Path] = []
    for raw in inputs:
        path = raw if raw.is_absolute() else root / raw
        if path.is_dir():
            expanded.extend(sorted(item for item in path.rglob("*") if item.is_file() or item.is_symlink()))
        else:
            expanded.append(path)
    return expanded


def snapshot(state_path: Path, inputs: list[Path]) -> None:
    root = repo_root()
    files: dict[str, str] = {}
    for path in expand_inputs(root, inputs):
        rel = relative_inside(root, path)
        if not allowed_lock_path(rel):
            raise SystemExit(f"error: solo se pueden bloquear tests/fixtures: {rel}")
        if not path.exists() and not path.is_symlink():
            raise SystemExit(f"error: no existe el fichero a bloquear: {rel}")
        files[str(rel)] = digest(path)
    if not files:
        raise SystemExit("error: indica al menos un test o fixture")

    payload = {"version": LOCK_VERSION, "root": str(root), "files": dict(sorted(files.items()))}
    state_path = state_path.resolve()
    state_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = state_path.with_suffix(state_path.suffix + ".tmp")
    temporary.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(state_path)
    for rel in payload["files"]:
        print(f"LOCK: {rel}")
    print(f"test lock: {len(files)} ficheros -> {state_path}")


def verify(state_path: Path) -> None:
    root = repo_root()
    try:
        payload = json.loads(state_path.resolve().read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"error: lock ilegible {state_path}: {error}") from error
    if payload.get("version") != LOCK_VERSION or not isinstance(payload.get("files"), dict):
        raise SystemExit(f"error: formato de lock no soportado: {state_path}")
    if Path(payload.get("root", "")).resolve() != root:
        raise SystemExit("error: el lock pertenece a otro checkout")

    failures: list[str] = []
    for rel, expected in sorted(payload["files"].items()):
        path = root / rel
        if not path.exists() and not path.is_symlink():
            failures.append(f"BORRADO: {rel}")
        elif digest(path) != expected:
            failures.append(f"MODIFICADO: {rel}")
        else:
            print(f"OK: {rel}")
    if failures:
        for failure in failures:
            print(failure, file=sys.stderr)
        print("error: los tests/fixtures bloqueados cambiaron durante la fase verde", file=sys.stderr)
        raise SystemExit(1)
    print(f"test lock: OK ({len(payload['files'])} ficheros)")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    create = commands.add_parser("snapshot", help="bloquear tests/fixtures exactos")
    create.add_argument("state", type=Path)
    create.add_argument("paths", type=Path, nargs="+")
    check = commands.add_parser("verify", help="comprobar que el lock no cambió")
    check.add_argument("state", type=Path)
    args = parser.parse_args()

    if args.command == "snapshot":
        snapshot(args.state, args.paths)
    else:
        verify(args.state)


if __name__ == "__main__":
    main()
