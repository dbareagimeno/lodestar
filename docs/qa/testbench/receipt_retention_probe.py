#!/usr/bin/env python3
"""Comprueba la retención y el recibo más reciente en una sesión nueva."""

from __future__ import annotations

import sys
from pathlib import Path


def main() -> int:
    if len(sys.argv) != 3:
        print("uso: receipt_retention_probe.py ROOT BINARIO", file=sys.stderr)
        return 2

    testbench = Path(__file__).resolve().parent
    root, binary = map(Path, sys.argv[1:])
    sys.path.insert(0, str(testbench))
    from lodestar_harness import LodestarSession

    session = None
    try:
        session = LodestarSession(str(root), "standard", binary=str(binary))
        plan = session.call(
            "change_plan",
            {
                "operations": [
                    {
                        "op": "patch_frontmatter",
                        "path": "guias/doc-00.md",
                        "patch": {"g": 3},
                    }
                ],
                "policy": {"requireValidResult": False, "allowWarnings": True},
            },
        )
        if plan.get("is_error") or not plan.get("structured", {}).get("changeSetId"):
            raise RuntimeError(f"change_plan falló: {plan}")
        apply = session.call(
            "change_apply",
            {"changeSetId": plan["structured"]["changeSetId"]},
        )
        if apply.get("is_error") or not apply.get("structured", {}).get("applied"):
            raise RuntimeError(f"change_apply falló: {apply}")
        status = session.call("workspace_status", {})
        receipts = status.get("structured", {}).get("receipts")
        if status.get("is_error") or not isinstance(receipts, list):
            raise RuntimeError(f"workspace_status falló: {status}")
        latest_match = bool(receipts) and receipts[0].get("receiptId") == apply["structured"].get("receiptId")
        print(f"COUNT={len(receipts)}")
        print("LATEST=1" if latest_match else "LATEST=0")
        print(f"LATEST_MATCH={latest_match}")
        return 0 if len(receipts) == 1 and latest_match else 1
    except Exception as error:
        print(f"receipt probe failed: {error}", file=sys.stderr)
        return 1
    finally:
        if session is not None:
            session.close()


if __name__ == "__main__":
    raise SystemExit(main())
