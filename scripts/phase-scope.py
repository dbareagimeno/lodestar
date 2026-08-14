#!/usr/bin/env python3
"""Snapshot a checkout and enforce a tests/fixtures-only red phase."""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import stat
import sys
from pathlib import Path, PurePosixPath


SNAPSHOT_VERSION = 1


def repo_root() -> Path:
    current = Path.cwd().resolve()
    for candidate in (current, *current.parents):
        if (candidate / ".git").exists():
            return candidate
    raise SystemExit("error: ejecuta el script dentro de un checkout git")


def ignored(rel: PurePosixPath) -> bool:
    parts = rel.parts
    if not parts:
        return False
    if parts[0] in {".git", "target", "node_modules"}:
        return True
    if parts[0].startswith("mutants.out"):
        return True
    if "target" in parts or "__pycache__" in parts:
        return True
    if parts[:2] == (".lodestar", "runtime"):
        return True
    if parts[:4] == ("docs", "qa", "testbench", "wt"):
        return True
    return len(parts) >= 4 and parts[:3] == ("docs", "qa", "testbench") and parts[3].startswith(
        "corpus"
    )


def allowed_red_path(rel: PurePosixPath) -> bool:
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


def fingerprint(path: Path) -> str:
    info = path.lstat()
    mode = stat.S_IMODE(info.st_mode)
    if path.is_symlink():
        return f"symlink:{mode:o}:{os.readlink(path)}"
    digest = hashlib.sha256()
    with path.open("rb") as handle:
        for chunk in iter(lambda: handle.read(1024 * 1024), b""):
            digest.update(chunk)
    return f"file:{mode:o}:{digest.hexdigest()}"


def inventory(root: Path) -> dict[str, str]:
    files: dict[str, str] = {}
    for directory, dirnames, filenames in os.walk(root, followlinks=False):
        directory_path = Path(directory)
        relative_dir = directory_path.relative_to(root)
        dirnames[:] = sorted(
            name
            for name in dirnames
            if not ignored(PurePosixPath(*(relative_dir.parts + (name,))))
        )
        for filename in sorted(filenames):
            path = directory_path / filename
            rel = PurePosixPath(*path.relative_to(root).parts)
            if not ignored(rel):
                files[str(rel)] = fingerprint(path)
    return files


def write_snapshot(state_path: Path) -> None:
    root = repo_root()
    payload = {
        "version": SNAPSHOT_VERSION,
        "root": str(root),
        "files": inventory(root),
    }
    state_path = state_path.resolve()
    state_path.parent.mkdir(parents=True, exist_ok=True)
    temporary = state_path.with_suffix(state_path.suffix + ".tmp")
    temporary.write_text(json.dumps(payload, indent=2, sort_keys=True) + "\n", encoding="utf-8")
    temporary.replace(state_path)
    print(f"snapshot: {len(payload['files'])} ficheros -> {state_path}")


def load_snapshot(state_path: Path, root: Path) -> dict[str, str]:
    try:
        payload = json.loads(state_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as error:
        raise SystemExit(f"error: snapshot ilegible {state_path}: {error}") from error
    if payload.get("version") != SNAPSHOT_VERSION or not isinstance(payload.get("files"), dict):
        raise SystemExit(f"error: formato de snapshot no soportado: {state_path}")
    if Path(payload.get("root", "")).resolve() != root:
        raise SystemExit("error: el snapshot pertenece a otro checkout")
    return {str(key): str(value) for key, value in payload["files"].items()}


def verify_tests_only(state_path: Path) -> None:
    root = repo_root()
    before = load_snapshot(state_path.resolve(), root)
    after = inventory(root)
    changed = sorted(path for path in before.keys() | after.keys() if before.get(path) != after.get(path))
    forbidden = [path for path in changed if not allowed_red_path(PurePosixPath(path))]

    for path in changed:
        state = "creado" if path not in before else "borrado" if path not in after else "modificado"
        marker = "OK" if path not in forbidden else "FUERA-DE-ALCANCE"
        print(f"{marker}: {state}: {path}")

    if forbidden:
        print("error: la fase roja modificó ficheros fuera de tests de integración/fixtures", file=sys.stderr)
        raise SystemExit(1)
    print(f"scope rojo: OK ({len(changed)} cambios)")


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    commands = parser.add_subparsers(dest="command", required=True)
    snapshot = commands.add_parser("snapshot", help="guardar el estado previo a la fase roja")
    snapshot.add_argument("state", type=Path)
    verify = commands.add_parser("verify-tests-only", help="permitir solo tests/fixtures")
    verify.add_argument("state", type=Path)
    args = parser.parse_args()

    if args.command == "snapshot":
        write_snapshot(args.state)
    else:
        verify_tests_only(args.state)


if __name__ == "__main__":
    main()
