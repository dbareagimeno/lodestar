#!/usr/bin/env python3
"""Construye la entrada wire validada para la corrida full de lodestar-bench.

Cada muestra ejecuta una invocación nueva de este arnés; el arnés abre y cierra un proceso MCP
nuevo con --profile readonly. La salida stdout se conserva en memoria únicamente para derivar
los tamaños y result_check; el fichero de salida es el único resultado de esta herramienta.
"""

import argparse
import json
import os
from pathlib import Path
import subprocess
import sys
import time


SAMPLES = 5
SEARCH_ARGUMENTS = {"text": "marker-search-h04", "where": 'service = "bench"'}


def fail(message):
    raise SystemExit(f"ERROR: {message}")


def compact_bytes(value):
    return len(
        json.dumps(value, ensure_ascii=False, separators=(",", ":")).encode("utf-8")
    )


def percentile(values, index):
    return sorted(values)[index]


def call_once(repo, harness, binary, root, tool, arguments):
    command = [
        sys.executable,
        str(harness),
        "--root",
        str(root),
        "--profile",
        "readonly",
        "--binary",
        str(binary),
        "--call",
        tool,
        json.dumps(arguments, ensure_ascii=False, separators=(",", ":")),
    ]
    started = time.perf_counter()
    completed = subprocess.run(
        command,
        cwd=repo,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        check=False,
    )
    elapsed = time.perf_counter() - started
    if completed.returncode != 0:
        stderr = completed.stderr.decode("utf-8", errors="replace").strip()
        fail(f"{tool}: lodestar_harness.py exitó {completed.returncode}: {stderr}")
    if not completed.stdout.strip():
        fail(f"{tool}: lodestar_harness.py no produjo stdout")
    try:
        envelope = json.loads(completed.stdout.decode("utf-8"))
    except json.JSONDecodeError as error:
        fail(f"{tool}: stdout no es JSON: {error}")
    if (
        envelope.get("kind") != "call"
        or envelope.get("tool") != tool
        or envelope.get("arguments") != arguments
        or envelope.get("is_error") is not False
    ):
        fail(f"{tool}: respuesta MCP inesperada: {json.dumps(envelope, ensure_ascii=False)}")
    structured = envelope.get("structured")
    if not isinstance(structured, dict):
        fail(f"{tool}: falta structuredContent")
    if elapsed <= 0:
        fail(f"{tool}: reloj wall inválido ({elapsed})")
    return elapsed, len(completed.stdout), compact_bytes(structured), envelope, structured


def result_check(tool, envelope, structured):
    if tool == "workspace_status":
        counts = structured.get("counts")
        documents = counts.get("documents") if isinstance(counts, dict) else None
        if not isinstance(documents, int) or documents < 10_000:
            fail(f"{tool}: documents no alcanza 10000: {documents!r}")
        return {"documents": documents, "is_error": envelope["is_error"]}

    results = structured.get("results")
    total = structured.get("totalApproximate")
    if (
        not isinstance(results, list)
        or not results
        or results[0].get("path") != "control.md"
        or not isinstance(total, int)
        or total < 1
    ):
        fail(f"{tool}: no encontró control.md: {json.dumps(structured, ensure_ascii=False)}")
    return {
        "is_error": envelope["is_error"],
        "path": results[0]["path"],
        "total_approximate": total,
    }


def measure_tool(repo, harness, binary, root, tool, arguments):
    observations = [
        call_once(repo, harness, binary, root, tool, arguments)
        for _ in range(SAMPLES)
    ]
    real_seconds = [observation[0] for observation in observations]
    payload_stdout = [observation[1] for observation in observations]
    payload_structured = [observation[2] for observation in observations]
    checks = [
        result_check(tool, observation[3], observation[4]) for observation in observations
    ]
    if len(set(payload_stdout)) != 1 or len(set(payload_structured)) != 1:
        fail(f"{tool}: payload no determinista entre los cinco procesos frescos")
    if len(set(json.dumps(check, sort_keys=True) for check in checks)) != 1:
        fail(f"{tool}: result_check no determinista entre los cinco procesos frescos")
    return {
        "tool": tool,
        "arguments": arguments,
        "sample_count": SAMPLES,
        "real_seconds": real_seconds,
        "p50_seconds": percentile(real_seconds, 2),
        "p95_seconds": percentile(real_seconds, 4),
        "payload_bytes": payload_stdout[0],
        "payload_bytes_stdout": payload_stdout[0],
        "payload_bytes_structured_content": payload_structured[0],
        "result_check": checks[0],
    }


def main():
    parser = argparse.ArgumentParser(
        description="mide cinco procesos MCP readonly frescos y crea wire-calibration JSON"
    )
    parser.add_argument("--root", required=True, type=Path)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()

    repo = Path(__file__).resolve().parents[3]
    harness = repo / "docs/qa/testbench/lodestar_harness.py"
    root = (args.root if args.root.is_absolute() else Path.cwd() / args.root).resolve()
    binary = (
        args.binary if args.binary.is_absolute() else repo / args.binary
    ).resolve()
    if not root.is_dir():
        fail(f"no existe el root {root}")
    if not binary.is_file() or not os.access(binary, os.X_OK):
        fail(f"no existe o no es ejecutable el binario {binary}")
    if binary.parent.name != "release":
        fail(f"el binario wire debe proceder de target/release: {binary}")
    document_count = sum(1 for path in root.rglob("*.md") if path.is_file())
    if document_count < 10_000:
        fail(f"el root no es Realista/10000: {document_count} documentos Markdown")

    results = [
        measure_tool(repo, harness, binary, root, "workspace_status", {}),
        measure_tool(repo, harness, binary, root, "knowledge_search", SEARCH_ARGUMENTS),
    ]
    report = {
        "status": "complete",
        "profile": "realista",
        "corpus_profile": "realista",
        "scale": 10_000,
        "runtime_profile": "readonly",
        "harness": "docs/qa/testbench/lodestar_harness.py",
        "transport": "JSON-RPC/stdio",
        "binary": "lodestar-mcp",
        "build_profile": "release",
        "corpus_documents": document_count,
        "process_protocol": (
            "Cinco procesos MCP frescos por tool; cada invocacion ejecuta "
            "lodestar_harness.py --profile readonly, con initialize, tools/call y cierre."
        ),
        "clock": "time.perf_counter wall-clock alrededor de cada proceso del arnes",
        "results": results,
    }
    output = (args.out if args.out.is_absolute() else Path.cwd() / args.out).resolve()
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(json.dumps(report, ensure_ascii=False, indent=2) + "\n", encoding="utf-8")
    print(
        json.dumps(
            {
                "status": report["status"],
                "profile": report["profile"],
                "scale": report["scale"],
                "sample_count": SAMPLES,
                "output": str(output),
            },
            ensure_ascii=False,
        )
    )


if __name__ == "__main__":
    main()
