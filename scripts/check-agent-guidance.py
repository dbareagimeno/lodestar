#!/usr/bin/env python3
"""Reject operational guidance known to have drifted from the repository."""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path


RULES = (
    ("base-main", re.compile(r"(?:rama|branch)[^\n]{0,60}(?:desde|from)\s+`?main\b", re.IGNORECASE)),
    ("diff-main", re.compile(r"\bgit\s+diff\s+(?:--merge-base\s+)?main(?:\.\.\.|\b)", re.IGNORECASE)),
    ("rama-claude", re.compile(r"\bclaude/<", re.IGNORECASE)),
    ("prototipo-autoridad", re.compile(r"(?:el\s+)?prototipo\s+es\s+la\s+spec|gana\s+el\s+prototipo", re.IGNORECASE)),
    ("diferencial-activo", re.compile(r"(?:ejecuta|correr|corre|escribe|añade|agrega|usar|usa)[^\n]{0,80}(?:arnés|sondas?)\s+diferencial", re.IGNORECASE)),
    ("skill-inexistente", re.compile(r"/simplify\b")),
    ("invariantes-retirados", re.compile(r"\b(?:7|siete)\s+invariantes\b", re.IGNORECASE)),
    ("vcs-retirado", re.compile(r"\bvcs\.rs\b")),
    ("frontera-retirada", re.compile(r"front\s*[↔-]\s*back", re.IGNORECASE)),
)


def repo_root() -> Path:
    current = Path.cwd().resolve()
    for candidate in (current, *current.parents):
        if (candidate / ".git").exists():
            return candidate
    raise SystemExit("error: ejecuta el script dentro de un checkout git")


def files_under(path: Path, suffixes: set[str]) -> list[Path]:
    if not path.exists():
        return []
    return sorted(item for item in path.rglob("*") if item.is_file() and item.suffix in suffixes)


def guidance_files(root: Path, include_legacy: bool) -> list[Path]:
    files = [root / "AGENTS.md", root / "docs/CODEX_WORKFLOW.md"]
    files.extend(files_under(root / ".agents", {".md", ".yaml", ".yml"}))
    files.extend(files_under(root / ".codex", {".toml"}))
    if include_legacy:
        files.extend([root / "CLAUDE.md", root / "docs/WORKFLOWS.md"])
        files.extend(files_under(root / ".claude", {".md"}))
    return sorted({path for path in files if path.exists()})


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--include-legacy", action="store_true", help="incluir CLAUDE.md y .claude/")
    args = parser.parse_args()
    root = repo_root()
    findings: list[tuple[Path, int, str, str]] = []

    for path in guidance_files(root, args.include_legacy):
        text = path.read_text(encoding="utf-8")
        for number, line in enumerate(text.splitlines(), start=1):
            if "guidance-lint: allow" in line:
                continue
            for name, pattern in RULES:
                if pattern.search(line):
                    findings.append((path.relative_to(root), number, name, line.strip()))
        if path.parent == root / ".codex/agents":
            for match in re.finditer(r"^\s*model\s*=", text, re.MULTILINE):
                number = text.count("\n", 0, match.start()) + 1
                findings.append((path.relative_to(root), number, "modelo-codex-fijado", text.splitlines()[number - 1].strip()))
        if args.include_legacy and path.parent == root / ".claude/agents":
            for match in re.finditer(r"^\s*model:\s*\S+", text, re.MULTILINE):
                number = text.count("\n", 0, match.start()) + 1
                findings.append((path.relative_to(root), number, "modelo-legacy-fijado", text.splitlines()[number - 1].strip()))

    if findings:
        for path, line, rule, content in findings:
            print(f"{path}:{line}: {rule}: {content}", file=sys.stderr)
        print(f"guidance: ERROR ({len(findings)} hallazgos)", file=sys.stderr)
        raise SystemExit(1)
    print(f"guidance: OK ({len(guidance_files(root, args.include_legacy))} ficheros)")


if __name__ == "__main__":
    main()
