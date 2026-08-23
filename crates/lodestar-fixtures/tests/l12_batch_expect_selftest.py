#!/usr/bin/env python3
"""Guarda ejecutable del esperado de L12-ROB-15."""

import json
import sys
from pathlib import Path


def main() -> int:
    repo = Path(__file__).resolve().parents[3]
    path = repo / "docs/qa/testbench/batches/gate_L12_robustez.json"
    lote = json.loads(path.read_text(encoding="utf-8"))
    caso = next(
        (case for case in lote.get("cases", []) if case.get("id") == "L12-ROB-15-PROTOCOLO"),
        None,
    )
    if caso is None:
        raise AssertionError("falta L12-ROB-15-PROTOCOLO")
    steps = caso.get("steps")
    if not isinstance(steps, list) or len(steps) < 5:
        raise AssertionError("L12-ROB-15 debe conservar -32601, dos silencios y sesión viva")

    method = steps[0]
    if method.get("expect", {}).get("equals", {}).get("response.error.code") != -32601:
        raise AssertionError("método desconocido con id debe conservar -32601")

    malformed, notification = steps[1], steps[2]
    if malformed.get("line") != "{json roto sin cerrar":
        raise AssertionError("la línea ilegible no es la reproducción ratificada")
    if notification.get("line") != '{"jsonrpc":"2.0","method":"metodo_que_no_existe"}':
        raise AssertionError("falta la notificación bien formada sin id")
    for label, step in (("JSON ilegible", malformed), ("notificación sin id", notification)):
        expect = step.get("expect", {}).get("equals", {})
        if expect.get("response", object()) is not None:
            raise AssertionError(f"{label} debe exigir response null")
        if "response.error.code" in expect:
            raise AssertionError(f"{label} no debe esperar un frame de error")

    status = next(
        (step for step in steps if step.get("kind") == "call" and step.get("tool") == "workspace_status"),
        None,
    )
    if status is None or status.get("expect", {}).get("is_error") is not False:
        raise AssertionError("workspace_status posterior debe probar la sesión viva")
    if "structured.workspaceRevision" not in status.get("expect", {}).get("matches", {}):
        raise AssertionError("workspace_status debe conservar la guarda de revisión")
    print("PASS: L12 exige dos silencios y conserva -32601/workspace_status")
    return 0


if __name__ == "__main__":
    try:
        raise SystemExit(main())
    except (AssertionError, OSError, json.JSONDecodeError) as error:
        print(f"FAIL: {error}", file=sys.stderr)
        raise SystemExit(1)
