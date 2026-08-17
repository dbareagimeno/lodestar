#!/usr/bin/env bash
# Smoke del guion de la demo (E27-H04): ejecuta contra examples/demo lo que
# examples/demo/README.md documenta y aserta las salidas clave. Si la demo o
# el motor cambian y el guion deja de ser verdad, este script falla — es lo
# que impide que README y demo se pudran en silencio.
#
# Uso: scripts/demo-smoke.sh   (desde cualquier cwd del repo; compila si hace
# falta con `cargo build -p lodestar-cli -p lodestar-mcp` antes de llamarlo,
# o exporta LODESTAR_CLI/LODESTAR_MCP apuntando a los binarios.)
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"
CLI="${LODESTAR_CLI:-target/debug/lodestar}"
MCP="${LODESTAR_MCP:-target/debug/lodestar-mcp}"
DEMO=examples/demo

fallo() {
    echo "SMOKE FAIL: $1" >&2
    exit 1
}

[ -x "$CLI" ] || fallo "no existe $CLI — compila con: cargo build --locked -p lodestar-cli"
[ -x "$MCP" ] || fallo "no existe $MCP — compila con: cargo build --locked -p lodestar-mcp"
command -v jq >/dev/null || fallo "falta jq"
command -v python3 >/dev/null || fallo "falta python3"

CLI_ABS="$(pwd)/$CLI"

# ---------------------------------------------------------------------------
# Paso 1 — `lodestar check --json`: los DOS defectos deliberados. El enlace
# roto es el único diagnóstico (la orfandad no es diagnóstico desde E16-H02),
# pero el JSON expone `isolated`, así que el huérfano también se aserta aquí
# (y de nuevo en el paso 2 vía graph_query, por el wire MCP).
# ---------------------------------------------------------------------------
set +e
salida_check="$(cd "$DEMO" && "$CLI_ABS" check --json 2>/dev/null)"
code=$?
set -e
[ "$code" -eq 1 ] || fallo "check exit=$code, esperado 1 (hard-fail por el enlace roto)"

echo "$salida_check" | jq -e '
    ([.diagnostics[][] | select(.level == "err")] | length == 1)
    and (.diagnostics["runbooks/incident-response.md"][0].code == "LINK-TARGET-MISSING")
    and ([.diagnostics[][] | select(.level == "warn")] | length == 0)
    and (.diagnostics | length == 10)
    and (.isolated == ["notes/scratchpad.md"])
' >/dev/null || fallo "check_reporta_los_defectos_deliberados: se esperaba exactamente 1 err (LINK-TARGET-MISSING en runbooks/incident-response.md), 0 warns, 10 documentos y el huérfano deliberado en .isolated"
echo "ok: check_reporta_los_defectos_deliberados (exit 1, 1 err, 0 warn, 10 docs, isolated=[notes/scratchpad.md])"

# ---------------------------------------------------------------------------
# Paso 2 — la sesión MCP del guion: search → isolated → impact → plan →
# apply → revert, asertando el resultado clave de cada respuesta.
# ---------------------------------------------------------------------------
MCP="$MCP" DEMO="$DEMO" python3 - <<'PY' || fallo "el_ciclo_del_guion_responde_lo_documentado"
import json, os, subprocess, sys

proc = subprocess.Popen(
    [os.environ["MCP"], "--root", os.environ["DEMO"]],
    stdin=subprocess.PIPE, stdout=subprocess.PIPE, text=True,
)
_id = 0

def call(method, params):
    global _id
    _id += 1
    proc.stdin.write(json.dumps({"jsonrpc": "2.0", "id": _id, "method": method, "params": params}) + "\n")
    proc.stdin.flush()
    resp = json.loads(proc.stdout.readline())
    if "error" in resp or resp.get("result", {}).get("isError"):
        sys.exit(f"respuesta de error en {method}: {resp}")
    return resp["result"]

def notify(method, params):
    proc.stdin.write(json.dumps({"jsonrpc": "2.0", "method": method, "params": params}) + "\n")
    proc.stdin.flush()

def tool(name, args):
    return call("tools/call", {"name": name, "arguments": args})["structuredContent"]

def aserta(cond, msg):
    if not cond:
        sys.exit(f"assert fallido: {msg}")

call("initialize", {
    "protocolVersion": "2025-11-25",
    "capabilities": {},
    "clientInfo": {"name": "lodestar-demo-smoke", "version": "1.0.0"},
})
notify("notifications/initialized", {})

r = tool("knowledge_search", {"where": "has(service) and service.tier = 1",
                              "include": ["frontmatter.oncall"]})
paths = sorted(x["path"] for x in r["results"])
aserta(paths == ["runbooks/deploy.md", "runbooks/incident-response.md"],
       f"knowledge_search where tier=1: {paths}")
aserta(all(x["frontmatter"].get("oncall") == "platform" for x in r["results"]),
       "include frontmatter.oncall no proyectó 'platform'")

r = tool("graph_query", {"operation": "isolated"})
aserta([n["id"] for n in r["nodes"]] == ["notes/scratchpad.md"],
       f"isolated debía ser exactamente el huérfano deliberado: {r['nodes']}")

r = tool("impact_analyze", {"ref": {"path": "architecture.md"},
                            "proposedOperation": {"kind": "move"}})
aserta(r["summary"]["directlyAffected"] == 5 and r["summary"]["transitivelyAffected"] == 8,
       f"impacto de mover architecture.md: {r['summary']}")

r = tool("change_plan", {
    "operations": [{"op": "patch_frontmatter", "path": "adr/0002-event-bus.md",
                     "patch": {"status": "accepted"}}],
    "policy": {"requireValidResult": False, "allowWarnings": True},
})
aserta(r["canApply"] is True, f"canApply: {r['canApply']}")
aserta(r["semanticDiff"]["frontmatterChanges"] == ["adr/0002-event-bus.md"],
       f"semanticDiff: {r['semanticDiff']}")

r = tool("change_apply", {"changeSetId": r["changeSetId"]})
aserta(r["applied"] is True and r["changedPaths"] == ["adr/0002-event-bus.md"],
       f"apply: {r}")

r = tool("change_revert", {"receiptId": r["receiptId"]})
aserta(r["reverted"] is True, f"revert: {r}")

proc.stdin.close()
proc.wait(timeout=10)
print("ok: el_ciclo_del_guion_responde_lo_documentado (search/isolated/impact/plan/apply/revert)")
PY

# ---------------------------------------------------------------------------
# Paso 3 — tras el revert, el árbol de la demo queda byte a byte como estaba.
# ---------------------------------------------------------------------------
sucio="$(git status --porcelain -- "$DEMO")"
[ -z "$sucio" ] || fallo "el_revert_deja_el_arbol_intacto: git status no está vacío:
$sucio"
echo "ok: el_revert_deja_el_arbol_intacto (git status --porcelain vacío)"

echo "SMOKE OK: el guion de examples/demo sigue siendo verdad."
