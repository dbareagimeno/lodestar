#!/usr/bin/env python3
"""Compare MCP tool registration, dispatch, profiles and contracts/mcp.yml."""

from __future__ import annotations

import re
import sys
from pathlib import Path


EXPECTED_TOOL_COUNT = 10


def repo_root() -> Path:
    current = Path.cwd().resolve()
    for candidate in (current, *current.parents):
        if (candidate / ".git").exists():
            return candidate
    raise SystemExit("error: ejecuta el script dentro de un checkout git")


def unique(items: list[str], label: str, failures: list[str]) -> None:
    duplicates = sorted({item for item in items if items.count(item) > 1})
    if duplicates:
        failures.append(f"{label} contiene duplicados: {', '.join(duplicates)}")


def parse_yaml_tools(text: str) -> tuple[list[str], list[str]]:
    tools: list[str] = []
    changes: list[str] = []
    current: str | None = None
    in_tools = False
    for line in text.splitlines():
        if line == "tools:":
            in_tools = True
            continue
        if in_tools and line and not line.startswith(" ") and not line.startswith("#"):
            break
        match = re.match(r"^  - nombre:\s*([a-z][a-z0-9_]*)\s*$", line)
        if match:
            current = match.group(1)
            tools.append(current)
        elif current and re.match(r"^    perfil:\s*standard\s*$", line):
            changes.append(current)
    return tools, changes


def main() -> None:
    root = repo_root()
    source_path = root / "crates/lodestar-mcp/src/tools.rs"
    contract_path = root / "contracts/mcp.yml"
    source = source_path.read_text(encoding="utf-8")
    contract = contract_path.read_text(encoding="utf-8")
    failures: list[str] = []

    try:
        list_block = source[source.index("pub fn list()") : source.index("pub const CHANGE_TOOLS")]
        call_start = source.index("pub fn call(")
        call_end = source.index("\nfn ", call_start)
        call_block = source[call_start:call_end]
    except ValueError as error:
        raise SystemExit(f"error: no se pudo localizar list/call/CHANGE_TOOLS en {source_path}") from error

    registered_matches = list(re.finditer(r'\{"name":\s*"([a-z][a-z0-9_]*)"', list_block))
    registered = [match.group(1) for match in registered_matches]
    dispatched = re.findall(r'^\s*"([a-z][a-z0-9_]*)"\s*=>\s*\{', call_block, re.MULTILINE)
    contracted, yaml_changes = parse_yaml_tools(contract)

    change_match = re.search(r"pub const CHANGE_TOOLS:[^=]+=\s*\[(.*?)\];", source, re.DOTALL)
    code_changes = re.findall(r'"([a-z][a-z0-9_]*)"', change_match.group(1)) if change_match else []
    if not change_match:
        failures.append("no se pudo extraer CHANGE_TOOLS")

    for index, match in enumerate(registered_matches):
        end = registered_matches[index + 1].start() if index + 1 < len(registered_matches) else len(list_block)
        definition = list_block[match.start() : end]
        name = match.group(1)
        if '"inputSchema"' not in definition:
            failures.append(f"{name} no declara inputSchema en tools::list()")
        if '"outputSchema"' not in definition:
            failures.append(f"{name} no declara outputSchema en tools::list()")

    for label, items in (
        ("registro", registered),
        ("despacho", dispatched),
        ("contrato", contracted),
        ("CHANGE_TOOLS", code_changes),
    ):
        unique(items, label, failures)

    if len(registered) != EXPECTED_TOOL_COUNT:
        failures.append(f"registro tiene {len(registered)} tools; se esperaban {EXPECTED_TOOL_COUNT}")
    if registered != dispatched:
        failures.append(f"registro != despacho: {registered!r} != {dispatched!r}")
    if registered != contracted:
        failures.append(f"registro != contrato: {registered!r} != {contracted!r}")
    if code_changes != yaml_changes:
        failures.append(f"CHANGE_TOOLS != perfiles standard: {code_changes!r} != {yaml_changes!r}")

    print(f"registro ({len(registered)}): {', '.join(registered)}")
    print(f"despacho ({len(dispatched)}): {', '.join(dispatched)}")
    print(f"contrato ({len(contracted)}): {', '.join(contracted)}")
    print(f"cambio ({len(code_changes)}): {', '.join(code_changes)}")
    if failures:
        for failure in failures:
            print(f"ERROR: {failure}", file=sys.stderr)
        raise SystemExit(1)
    print("superficie MCP: OK")


if __name__ == "__main__":
    main()
