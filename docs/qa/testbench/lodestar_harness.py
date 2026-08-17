#!/usr/bin/env python3
"""Arnés JSON-RPC/stdio para lodestar-mcp, con veredicto mecánico (E33-H02).

Modos:
  - Llamada suelta:   lodestar_harness.py --root DIR [--profile P] --call TOOL 'JSON'
  - tools/list:       lodestar_harness.py --root DIR [--profile P] --list-tools
  - Un lote:          lodestar_harness.py --batch spec.json [--root-corpus DIR] [--out F]
  - El banco entero:  lodestar_harness.py --run-all --root-corpus DIR [--out F]

Un spec de lote es JSON:
{
  "batch": "L1",
    "root": "corpus" | "real" | "worktree",   # corpus = copia efímera del canónico
  "profile": "readonly" | "standard",
  "fixtures": null | "mdi" | "chk_a" | ...,
  "gate": true,                             # opcional: pertenencia al gate de TODO el lote
  "cases": [
    {"id": "TYP-01", "tool": "knowledge_search", "arguments": {...},                 # corta
     "expect": {"is_error": false}},
    {"id": "APL-01", "fresh_root": true, "steps": [                                  # larga
        {"kind": "call", "tool": "change_plan", "arguments": {...},
         "expect": {"matches": {"structured.planHash": "^blake3:[0-9a-f]{64}$"}}},
        {"kind": "shell", "cmd": "grep -c foo doc.md", "expect": {"rc": 0}},
        {"kind": "call", "tool": "change_apply",
         "arguments": {"changeSetId": "@step0.structured.changeSetId"}},
        {"kind": "raw", "line": "{json roto"},
        {"kind": "spawn", "args": ["--root", "@root"]}
     ],
     "expect": [{"invariant": "same", "steps": [0, 2], "path": "structured.x"}]}
  ]
}

Placeholders: cualquier string "@stepN.ruta.de.campos" en `arguments` se sustituye por el
valor de ese paso previo del MISMO caso (p. ej. "@step0.structured.changeSetId"). En los
pasos `shell`/`spawn` se sustituyen además, por TEXTO, los tokens de entorno — así ningún
lote necesita rutas absolutas de una máquina (E33-H02, criterio estructural):

    @root      la raíz del workspace del caso        @repo     la raíz de este repo
    @bin.mcp   el binario lodestar-mcp               @bin.cli  el binario lodestar (CLI)
    @testbench el directorio de este arnés

El binario sale de `--binary`, de `LODESTAR_MCP_BIN` o, por defecto, de
`target/release/lodestar-mcp` **relativo a la raíz del repo**, derivada de la ubicación de
este fichero. Ni una ruta absoluta de máquina en el árbol del banco.

En un root `real`, el preflight exige `readonly` y rechaza cualquier `shell`/`spawn` antes de
abrir sesión. Los errores de EJECUCIÓN de tool (result.isError + texto "CODIGO: mensaje") se distinguen
de los errores de PROTOCOLO JSON-RPC (-32602, -32700...).

El formato `expect` y el veredicto están fijados en `FORMATO_EXPECT.md`; su evaluación
vive en `runner_expect.py`. Exit codes: 0 sin FAIL ejecutado · 1 con cualquier FAIL ejecutado · 2 uso
incorrecto · 3 error de ejecución del banco.
"""
import argparse
import json
import os
import re
import shlex
import shutil
import subprocess
import sys
import tempfile
import time
import uuid

import runner_expect as rx

SCRATCH = os.path.dirname(os.path.abspath(__file__))
REPO = os.path.abspath(os.path.join(SCRATCH, "..", "..", ".."))
WT_BASE = os.path.join(SCRATCH, "wt")

# Fallbacks RELATIVOS al repo (nunca rutas de una máquina concreta). El root real por
# defecto de la campaña exploratoria es el propio repo; para el homelab u otro workspace,
# `--root DIR`.
BIN_MCP_POR_DEFECTO = os.path.join(REPO, "target", "release", "lodestar-mcp")
BIN_CLI_POR_DEFECTO = os.path.join(REPO, "target", "release", "lodestar")

# Los lotes del GATE, en el orden en que corre `--run-all`. Es una lista literal y no un
# glob del directorio: qué entra al gate es una decisión, no un accidente del filesystem
# (y los lotes exploratorios heredados conviven en el mismo `batches/`).
LOTES_DEL_GATE = [
    "batches/gate_L1_consulta.json",
    "batches/gate_L2_proyeccion.json",
    "batches/gate_L3_metadata.json",
    "batches/gate_L5_grafo.json",
    "batches/gate_L6_plan.json",
    "batches/gate_L7_apply.json",
    "batches/gate_L8_readonly.json",
    "batches/gate_L9_check_a.json",
    "batches/gate_L10_check_b.json",
    "batches/gate_L11_scopes.json",
    "batches/gate_L12_robustez.json",
    "batches/gate_G_descubrimiento.json",
    "batches/gate_H_cli_recuperacion.json",
    "batches/gate_invariantes.json",
    "batches/gate_verify_g1.json",
    "batches/gate_verify_g2.json",
]


class ErrorDeUso(Exception):
    """Uso incorrecto del runner: exit 2 (flags incompatibles, lote inexistente…)."""


class ErrorDeEjecucion(Exception):
    """El banco no pudo correr: exit 3 (binario ausente, corpus ilegible…)."""


class Entorno:
    """Lo que el runner necesita del mundo: binarios y corpus. Se pasa explícito para que
    ninguna función lo lea de una global (la lección de `BINARY`/`HOMELAB` hardcodeados)."""

    def __init__(self, binario=None, binario_cli=None, root_corpus=None, root_real=None):
        binario = binario or os.environ.get("LODESTAR_MCP_BIN") or BIN_MCP_POR_DEFECTO
        binario_cli = (binario_cli or os.environ.get("LODESTAR_CLI_BIN")
                       or BIN_CLI_POR_DEFECTO)
        # A relative override is relative to the repository, not to a temporary corpus
        # used as cwd by shell steps. This keeps `LODESTAR_MCP_BIN=target/release/...`
        # portable while still rendering an absolute executable token in those steps.
        self.binario = binario if os.path.isabs(binario) else os.path.abspath(
            os.path.join(REPO, binario))
        self.binario_cli = binario_cli if os.path.isabs(binario_cli) else os.path.abspath(
            os.path.join(REPO, binario_cli))
        self.root_corpus = root_corpus
        self.root_real = root_real
        self._corpus_temporal = None

    def exige_binario(self):
        if not (os.path.isfile(self.binario) and os.access(self.binario, os.X_OK)):
            raise ErrorDeEjecucion(
                "no existe o no es ejecutable el binario MCP «%s»: compila con `cargo build --release -p "
                "lodestar-mcp` o pasa --binary" % self.binario)

    def exige_binario_cli(self):
        if not (os.path.isfile(self.binario_cli) and os.access(self.binario_cli, os.X_OK)):
            raise ErrorDeEjecucion(
                "no existe o no es ejecutable el binario CLI «%s»: compila con `cargo build --release -p "
                "lodestar-cli` o pasa --binary-cli" % self.binario_cli)

    def corpus(self):
        """El corpus canónico: el de `--root-corpus` o uno generado en un tempdir (§7)."""
        if self.root_corpus:
            if not os.path.isdir(self.root_corpus):
                raise ErrorDeEjecucion("el corpus «%s» no es un directorio"
                                       % self.root_corpus)
            return self.root_corpus
        if self._corpus_temporal is None:
            base = tempfile.mkdtemp(prefix="banco-corpus-")
            destino = os.path.join(base, "corpus")
            generado = subprocess.run(
                [sys.executable, os.path.join(SCRATCH, "make_corpus.py"), destino],
                capture_output=True, text=True)
            if generado.returncode != 0:
                shutil.rmtree(base, ignore_errors=True)
                raise ErrorDeEjecucion("no se pudo generar el corpus: %s"
                                       % (generado.stderr or generado.stdout))
            self._corpus_temporal = base
            self.root_corpus = destino
        return self.root_corpus

    def limpia(self):
        if self._corpus_temporal:
            shutil.rmtree(self._corpus_temporal, ignore_errors=True)
            self._corpus_temporal = None


class LodestarSession:
    def __init__(self, root, profile="standard", binary=None):
        binary = binary or BIN_MCP_POR_DEFECTO
        self.proc = subprocess.Popen(
            [binary, "--root", root, "--profile", profile],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, bufsize=1,
        )
        self._next_id = 1
        try:
            self._initialize()
        except Exception:
            # El proceso ya existe aunque initialize falle; no dejarlo huérfano.
            self.close()
            raise

    def _send(self, obj):
        self.proc.stdin.write(json.dumps(obj) + "\n")
        self.proc.stdin.flush()

    def _read_response(self, req_id, timeout=30):
        deadline = time.time() + timeout
        while time.time() < deadline:
            line = self.proc.stdout.readline()
            if not line:
                err = self.proc.stderr.read() if self.proc.poll() is not None else ""
                raise RuntimeError(f"EOF del servidor esperando id={req_id}; stderr={err[:2000]}")
            line = line.strip()
            if not line:
                continue
            try:
                resp = json.loads(line)
            except json.JSONDecodeError:
                continue
            if resp.get("id") == req_id:
                return resp
            # respuestas a otros id o notificaciones: se ignoran
        raise RuntimeError(f"timeout esperando respuesta a id={req_id}")

    def rpc(self, method, params=None):
        req_id = self._next_id
        self._next_id += 1
        req = {"jsonrpc": "2.0", "id": req_id, "method": method}
        if params is not None:
            req["params"] = params
        self._send(req)
        return self._read_response(req_id)

    def _initialize(self):
        resp = self.rpc("initialize", {
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {"name": "lodestar-testbench", "version": "0.1"},
        })
        if "error" in resp:
            raise RuntimeError(f"initialize falló: {resp['error']}")
        self._send({"jsonrpc": "2.0", "method": "notifications/initialized"})

    def tools_list(self):
        resp = self.rpc("tools/list")
        if "error" in resp:
            return {"protocol_error": resp["error"]}
        return {"tools": [t["name"] for t in resp["result"]["tools"]]}

    def call(self, name, arguments):
        resp = self.rpc("tools/call", {"name": name, "arguments": arguments})
        out = {"kind": "call", "tool": name, "arguments": arguments}
        if "error" in resp:
            out["protocol_error"] = resp["error"]
            return out
        result = resp["result"]
        out["is_error"] = bool(result.get("isError", False))
        content = result.get("content") or []
        out["text"] = content[0].get("text") if content else None
        out["structured"] = result.get("structuredContent")
        if out["is_error"] and out["text"] and ":" in out["text"]:
            out["error_code"] = out["text"].split(":", 1)[0].strip()
        return out

    def raw_line(self, line, timeout=5):
        """Envía una línea cruda y devuelve la primera línea de respuesta (o None)."""
        self.proc.stdin.write(line + "\n")
        self.proc.stdin.flush()
        deadline = time.time() + timeout
        while time.time() < deadline:
            resp_line = self.proc.stdout.readline()
            if resp_line.strip():
                try:
                    return json.loads(resp_line)
                except json.JSONDecodeError:
                    return {"unparseable_response": resp_line.strip()}
            if self.proc.poll() is not None:
                return {"server_exited": self.proc.returncode}
        return None

    def close(self):
        try:
            self.proc.stdin.close()
            self.proc.wait(timeout=5)
        except Exception:
            self.proc.kill()


# --------------------------------------------------------------- roots desechables

def make_worktree(root_real):
    """Worktree git efímero del root real. Solo para la campaña exploratoria: exige que
    el root sea un repo git (el corpus canónico no lo es, y usa `fresh_root_de_corpus`)."""
    os.makedirs(WT_BASE, exist_ok=True)
    path = os.path.join(WT_BASE, f"wt-{uuid.uuid4().hex[:8]}")
    subprocess.run(["git", "-C", root_real, "worktree", "add", "--detach", path, "HEAD"],
                   check=True, capture_output=True, text=True)
    return path


def remove_worktree(root_real, path):
    subprocess.run(["git", "-C", root_real, "worktree", "remove", "--force", path],
                   capture_output=True, text=True)


def copia_efimera(corpus):
    """Copia el corpus canónico a un directorio desechable (§7 de `FORMATO_EXPECT.md`).

    Es el sustituto **portable** del worktree git del homelab: los lotes que mutan corren
    contra una copia, así el corpus original nunca se contamina y el banco no necesita que
    el campo de pruebas sea un repo git.
    """
    base = tempfile.mkdtemp(prefix="banco-root-")
    destino = os.path.join(base, "root")
    # `symlinks=True`: el corpus lleva un symlink deliberado (SYMLINK-UNSUPPORTED) que
    # copiarlo por contenido convertiría en un fichero normal, borrando el diagnóstico.
    shutil.copytree(corpus, destino, symlinks=True)
    return base, destino


def inject_fixtures(fixture_set, root):
    script = os.path.join(SCRATCH, "make_fixtures.py")
    for part in fixture_set.split("+"):
        subprocess.run([sys.executable, script, part, root], check=True,
                       capture_output=True, text=True)


# ------------------------------------------------------------------ placeholders

def resolve_placeholders(value, step_results):
    if isinstance(value, str):
        m = re.fullmatch(r"@step(\d+)\.(.+)", value)
        if m:
            paso = step_results[int(m.group(1))]
            resolved = rx.resuelve_path(paso, m.group(2))
            if resolved is rx.NO_RESUELVE:
                raise ValueError("placeholder no resuelve: %s" % value)
            return resolved
        return value
    if isinstance(value, dict):
        return {k: resolve_placeholders(v, step_results) for k, v in value.items()}
    if isinstance(value, list):
        return [resolve_placeholders(v, step_results) for v in value]
    return value


def _tokens_entorno(root, entorno):
    return (("@bin.mcp", entorno.binario),
            ("@bin.cli", entorno.binario_cli),
            ("@testbench", SCRATCH),
            ("@repo", REPO),
            ("@root", root or ""))


def _sustituye_tokens_una_pasada(texto, root, entorno, render):
    """Reemplaza tokens originales una sola vez; los valores no se reescanean."""
    valores = dict(_tokens_entorno(root, entorno))
    patron = re.compile("|".join(re.escape(token) for token in valores))
    return patron.sub(lambda match: render(valores[match.group(0)]), texto)


def sustituye_entorno(texto, root, entorno):
    """Sustituye tokens para un comando shell, entrecomillando cada valor.

    `shlex.quote` hace que los valores sean datos incluso si contienen espacios o
    metacaracteres. No se vuelve a interpretar el texto sustituido como plantilla.
    """
    return _sustituye_tokens_una_pasada(texto, root, entorno,
                                       lambda valor: shlex.quote(str(valor)))


def sustituye_entorno_raw(texto, root, entorno):
    """Sustituye tokens para argv de `spawn`, sin quoting de shell ni reescaneo."""
    return _sustituye_tokens_una_pasada(texto, root, entorno, str)


def run_step(step, session, root, step_results, entorno):
    kind = step.get("kind", "call")
    if kind == "call":
        args = resolve_placeholders(step.get("arguments", {}), step_results)
        if session is None:
            return {"kind": "call", "tool": step.get("tool"),
                    "harness_exception": "el caso no tiene sesión abierta"}
        return session.call(step["tool"], args)
    if kind == "shell":
        cmd = sustituye_entorno(resolve_placeholders(step["cmd"], step_results),
                                root, entorno)
        r = subprocess.run(cmd, shell=True, cwd=root, capture_output=True, text=True)
        return {"kind": "shell", "cmd": cmd, "rc": r.returncode,
                "stdout": r.stdout[-4000:], "stderr": r.stderr[-4000:]}
    if kind == "raw":
        if session is None:
            return {"kind": "raw", "line": step["line"],
                    "harness_exception": "el caso no tiene sesión abierta"}
        return {"kind": "raw", "line": step["line"], "response": session.raw_line(step["line"])}
    if kind == "spawn":
        valores = resolve_placeholders(step.get("args", []), step_results)
        args = [sustituye_entorno_raw(a, root, entorno) for a in valores]
        r = subprocess.run([entorno.binario] + args, capture_output=True, text=True,
                           timeout=15, input="")
        return {"kind": "spawn", "args": args, "rc": r.returncode,
                "stdout": r.stdout[:2000], "stderr": r.stderr[:2000]}
    if kind == "list_tools":
        if session is None:
            return {"harness_exception": "el caso no tiene sesión abierta"}
        return session.tools_list()
    raise ValueError(f"kind desconocido: {kind}")


# ----------------------------------------------------------------- ejecución de lotes

def _sin_claves_repetidas(pares):
    """Hook de `json.load` que RECHAZA un objeto con la misma clave dos veces.

    El parser de JSON, por defecto, se queda con la última y descarta la anterior EN
    SILENCIO. En un lote de este banco eso significa perder una aserción sin que nada lo
    diga: escribir `length` dos veces en el mismo `expect` deja viva solo una y el caso
    sigue dando PASS. Un esperado que desaparece sin ruido es exactamente el modo de
    fallo que el banco existe para no tener.
    """
    vistas = set()
    for clave, _ in pares:
        if clave in vistas:
            raise ErrorDeUso("la clave «%s» aparece dos veces en el mismo objeto: el "
                             "JSON descartaría la primera en silencio y con ella su "
                             "aserción" % clave)
        vistas.add(clave)
    return dict(pares)


def _clase_de_root(spec):
    """`corpus` (desechable, por defecto para lotes nuevos), `worktree` o `real`."""
    return spec.get("root", "corpus")


def _es_desechable(clase):
    return clase in ("corpus", "worktree")


def _pasos_del_caso(case):
    """Devuelve los pasos efectivos sin ejecutar ni abrir una sesión."""
    if "steps" in case:
        steps = case["steps"]
        if not isinstance(steps, list):
            raise ErrorDeUso("el caso «%s»: «steps» debe ser una lista" % case.get("id"))
        return steps
    return [{"kind": "call", "tool": case.get("tool"),
             "arguments": case.get("arguments", {})}]


def _es_entero_json(valor):
    return isinstance(valor, int) and not isinstance(valor, bool)


def _valida_mapa_paths(valor, nombre, caso_id, paso):
    if not isinstance(valor, dict):
        raise ErrorDeUso("el caso «%s», paso %s: «%s» debe ser un mapa path → valor"
                         % (caso_id, paso, nombre))
    if any(not isinstance(path, str) or not path for path in valor):
        raise ErrorDeUso("el caso «%s», paso %s: las claves de «%s» deben ser paths"
                         % (caso_id, paso, nombre))


def _valida_expect(expect, caso_id, paso):
    """Comprueba la forma de las aserciones antes de abrir o ejecutar el caso.

    Las claves desconocidas se dejan pasar deliberadamente: el evaluador las convierte en
    un FAIL nombrado, para que un typo no sea un uso-2 indistinguible de una aserción falsa.
    """
    if not isinstance(expect, dict):
        raise ErrorDeUso("el caso «%s», paso %s: «expect» debe ser un objeto"
                         % (caso_id, paso))
    def err(mensaje):
        raise ErrorDeUso("el caso «%s», paso %s: %s" % (caso_id, paso, mensaje))

    if "is_error" in expect and not isinstance(expect["is_error"], bool):
        err("«is_error» debe ser booleano")
    if "error_code" in expect and not isinstance(expect["error_code"], str):
        err("«error_code» debe ser cadena")
    if "protocol_error_code" in expect and not _es_entero_json(expect["protocol_error_code"]):
        err("«protocol_error_code» debe ser entero no booleano")
    for clave in ("equals", "contains", "not_contains"):
        if clave in expect:
            _valida_mapa_paths(expect[clave], clave, caso_id, paso)
    for clave in ("matches",):
        if clave in expect:
            _valida_mapa_paths(expect[clave], clave, caso_id, paso)
            for patron in expect[clave].values():
                if not isinstance(patron, str):
                    err("los valores de «matches» deben ser regex cadena")
                try:
                    re.compile(patron)
                except re.error as exc:
                    err("regex inválida en «matches»: %s" % exc)
    for clave in ("present", "absent"):
        if clave in expect:
            paths = expect[clave]
            if not isinstance(paths, list) or any(not isinstance(path, str) or not path
                                                  for path in paths):
                err("«%s» debe ser lista de paths cadena" % clave)
    for clave in ("length", "min_length"):
        if clave in expect:
            _valida_mapa_paths(expect[clave], clave, caso_id, paso)
            if any(not _es_entero_json(n) for n in expect[clave].values()):
                err("los valores de «%s» deben ser enteros no booleanos" % clave)
    if "type" in expect:
        _valida_mapa_paths(expect["type"], "type", caso_id, paso)
        if any(not isinstance(nombre, str) or nombre not in rx.TIPOS_JSON
               for nombre in expect["type"].values()):
            err("«type» usa nombres JSON cerrados (object/array/string/number/boolean/null)")
    if "rc" in expect and not _es_entero_json(expect["rc"]):
        err("«rc» debe ser entero no booleano")
    if "describe" in expect and not isinstance(expect["describe"], str):
        err("«describe» debe ser cadena")


def _valida_invariante(inv, caso_id, pasos_count):
    if not isinstance(inv, dict):
        raise ErrorDeUso("el caso «%s»: cada invariante debe ser un objeto" % caso_id)
    # Las claves no conocidas no se bloquean aquí; rx las reporta como FAIL nombrado.
    conocidos = {"invariant", "steps", "path", "describe"}
    if "invariant" in inv and inv["invariant"] not in ("same", "differs"):
        raise ErrorDeUso("el caso «%s»: «invariant» debe ser same o differs" % caso_id)
    if "steps" in inv:
        indices = inv["steps"]
        if (not isinstance(indices, list) or len(indices) < 2
                or any(not _es_entero_json(i) or i < 0 or i >= pasos_count for i in indices)):
            raise ErrorDeUso("el caso «%s»: «steps» debe contener al menos dos índices "
                             "enteros no booleanos existentes" % caso_id)
    if "path" in inv and (not isinstance(inv["path"], str) or not inv["path"]):
        raise ErrorDeUso("el caso «%s»: «path» debe ser cadena" % caso_id)
    if "describe" in inv and not isinstance(inv["describe"], str):
        raise ErrorDeUso("el caso «%s»: «describe» debe ser cadena" % caso_id)
    # Si aparece cualquier parte del contrato de un invariante, exigir sus campos
    # estructurales; un objeto compuesto solo por claves desconocidas sigue siendo un
    # FAIL del evaluador, no un uso-2.
    if set(inv) & conocidos and not {"invariant", "steps", "path"}.issubset(inv):
        raise ErrorDeUso("el caso «%s»: un invariante requiere invariant, steps y path" % caso_id)


def _valida_preflight(spec, spec_path, clase, entorno):
    """Valida la forma y las restricciones de ejecución antes de cualquier paso."""
    cases = spec.get("cases")
    if not isinstance(cases, list):
        raise ErrorDeUso("el lote «%s»: «cases» debe ser una lista" % spec_path)

    usa_cli = False
    for case in cases:
        if not isinstance(case, dict):
            raise ErrorDeUso("el lote «%s»: cada caso debe ser un objeto" % spec_path)
        if "expect" in case:
            case_expect = case["expect"]
            tiene_steps = "steps" in case
            if tiene_steps and not isinstance(case_expect, list):
                raise ErrorDeUso("el caso «%s»: en forma larga «expect» debe ser una "
                                 "lista de invariantes" % case.get("id"))
            if not tiene_steps and not isinstance(case_expect, (dict, list)):
                raise ErrorDeUso("el caso «%s»: «expect» debe ser objeto o lista"
                                 % case.get("id"))
            if isinstance(case_expect, list) and any(
                    not isinstance(invariante, dict) for invariante in case_expect):
                raise ErrorDeUso("el caso «%s»: cada invariante de «expect» debe ser un "
                                 "objeto" % case.get("id"))
            if isinstance(case_expect, dict) and not tiene_steps:
                _valida_expect(case_expect, case.get("id"), "0")
            elif isinstance(case_expect, list):
                for invariante in case_expect:
                    _valida_invariante(invariante, case.get("id"),
                                       len(_pasos_del_caso(case)))
        for indice, step in enumerate(_pasos_del_caso(case)):
            if not isinstance(step, dict):
                raise ErrorDeUso("el caso «%s», paso %d: debe ser un objeto"
                                 % (case.get("id"), indice))
            if "expect" in step and not isinstance(step["expect"], dict):
                raise ErrorDeUso("el caso «%s», paso %d: «expect» debe ser un objeto"
                                 % (case.get("id"), indice))
            if "expect" in step:
                _valida_expect(step["expect"], case.get("id"), indice)
            kind = step.get("kind", "call")
            if clase == "real" and kind in ("shell", "spawn"):
                raise ErrorDeUso("REGLA DURA: un lote root=real no puede contener pasos "
                                 "shell/spawn (caso «%s», paso %d)"
                                 % (case.get("id"), indice))
            if kind == "shell":
                cmd = step.get("cmd")
                if not isinstance(cmd, str):
                    raise ErrorDeUso("el caso «%s», paso %d: «cmd» debe ser cadena"
                                     % (case.get("id"), indice))
                usa_cli = usa_cli or "@bin.cli" in cmd
            elif kind == "spawn":
                args = step.get("args")
                if not isinstance(args, list) or not all(isinstance(arg, str) for arg in args):
                    raise ErrorDeUso("el caso «%s», paso %d: «args» debe ser lista de cadenas"
                                     % (case.get("id"), indice))
                usa_cli = usa_cli or any("@bin.cli" in arg for arg in args)

    if usa_cli:
        entorno.exige_binario_cli()


def run_batch(spec_path, entorno, incluir_demos=False):
    """Corre un lote y devuelve su diccionario de resultados con veredicto por caso.

    La REGLA DURA generalizada (`FORMATO_EXPECT.md §7`): contra un root **declarado real**
    solo se admite `readonly`; contra un root desechable (`corpus`/`worktree`), cualquier
    perfil. Se comprueba aquí, antes de abrir una sola sesión.
    """
    if not os.path.exists(spec_path):
        raise ErrorDeUso("no existe el lote «%s»" % spec_path)
    try:
        with open(spec_path, encoding="utf-8") as f:
            spec = json.load(f, object_pairs_hook=_sin_claves_repetidas)
    except (OSError, json.JSONDecodeError, ErrorDeUso) as e:
        if isinstance(e, ErrorDeUso):
            raise ErrorDeUso("el lote «%s»: %s" % (spec_path, e))
        raise ErrorDeUso("el lote «%s» no es JSON legible: %s" % (spec_path, e))

    profile = spec.get("profile", "readonly")
    clase = _clase_de_root(spec)
    if clase not in ("corpus", "worktree", "real"):
        raise ErrorDeUso("«root» debe ser corpus, worktree o real; recibido «%s»" % clase)
    if not _es_desechable(clase) and profile != "readonly":
        raise ErrorDeUso("REGLA DURA: contra un root real solo se permite el perfil "
                         "readonly (lote «%s», perfil «%s»)" % (spec_path, profile))

    _valida_preflight(spec, spec_path, clase, entorno)
    entorno.exige_binario()

    if clase == "corpus":
        base_root = entorno.corpus()
    else:
        base_root = entorno.root_real or REPO
        if not os.path.isdir(base_root):
            raise ErrorDeEjecucion("el root «%s» no es un directorio" % base_root)

    results = {"batch": spec.get("batch"), "spec": os.path.relpath(spec_path, SCRATCH),
               "root_kind": clase, "profile": profile, "cases": []}

    shared_root = None
    shared_session = None
    shared_base = None      # el tempdir que envuelve una copia efímera compartida
    efimeros = []           # tempdirs por caso, a borrar al final
    worktrees = []          # worktrees por caso, incluido si un paso aborta antes del cierre
    sesiones_locales = []   # sesiones de casos fresh, incluido si un paso aborta

    def abre_root():
        """Un root nuevo de la clase del lote, con sus fixtures ya inyectadas."""
        base = root = None
        try:
            if clase == "corpus":
                base, root = copia_efimera(base_root)
                efimeros.append(base)
            elif clase == "worktree":
                base, root = None, make_worktree(base_root)
                worktrees.append(root)
            else:
                base, root = None, base_root
            if _es_desechable(clase) and spec.get("fixtures"):
                inject_fixtures(spec["fixtures"], root)
            return base, root
        except ErrorDeEjecucion:
            raise
        except (OSError, subprocess.CalledProcessError) as exc:
            if clase == "worktree" and root:
                remove_worktree(base_root, root)
                if root in worktrees:
                    worktrees.remove(root)
            elif base:
                shutil.rmtree(base, ignore_errors=True)
            raise ErrorDeEjecucion("no se pudo preparar el root efímero: %s" % exc) from exc

    def cierra_root(base, root):
        if clase == "worktree":
            remove_worktree(base_root, root)
            if root in worktrees:
                worktrees.remove(root)
        # las copias efímeras del corpus se borran al final, en el `finally`

    try:
        for case in spec["cases"]:
            if not rx.entra_al_gate(case, spec) and not incluir_demos:
                # Demo (§5): sin `--incluir-demos` ni se ejecuta.
                results["cases"].append({"id": case["id"], "steps": [], "verdict": rx.SKIP,
                                         "failures": [], "gate": False,
                                         "skip_reason": "caso de demostración (gate: "
                                                        "false); usa --incluir-demos"})
                continue

            fresh = bool(case.get("fresh_root"))
            no_server = bool(case.get("no_server"))
            session_error = None
            base_caso = None
            if no_server:
                base_caso, root = abre_root()
                session = None
            elif fresh:
                base_caso, root = abre_root()
                try:
                    session = LodestarSession(root, profile, binary=entorno.binario)
                    sesiones_locales.append(session)
                except Exception as e:
                    cierra_root(base_caso, root)
                    raise ErrorDeEjecucion("no se pudo abrir la sesión MCP: %s" % e) from e
            else:
                if shared_session is None:
                    try:
                        shared_base, shared_root = abre_root()
                        shared_session = LodestarSession(shared_root, profile,
                                                         binary=entorno.binario)
                    except Exception as e:
                        if shared_root is not None:
                            cierra_root(shared_base, shared_root)
                            shared_root, shared_base = None, None
                        raise ErrorDeEjecucion("no se pudo abrir la sesión MCP: %s" % e) from e
                root, session = shared_root, shared_session

            steps = _pasos_del_caso(case)
            step_results = []
            case_out = {"id": case["id"], "steps": []}
            if session_error:
                case_out["session_error"] = session_error
            for step_index, step in enumerate(steps):
                try:
                    res = run_step(step, session, root, step_results, entorno)
                except ErrorDeEjecucion:
                    raise
                except Exception as e:
                    raise ErrorDeEjecucion("falló el paso %d del caso «%s»: %s"
                                           % (step_index, case.get("id"), e)) from e
                step_results.append(res)
                case_out["steps"].append(res)

            veredicto, fallos = rx.evalua_caso(
                dict(case, session_error=session_error) if session_error else case,
                step_results)
            case_out["verdict"] = veredicto
            case_out["failures"] = fallos
            case_out["gate"] = rx.entra_al_gate(case, spec)
            if case.get("descripcion"):
                case_out["descripcion"] = case["descripcion"]
            results["cases"].append(case_out)

            if fresh or no_server:
                if session is not None:
                    session.close()
                    if session in sesiones_locales:
                        sesiones_locales.remove(session)
                cierra_root(base_caso, root)
    finally:
        for session in list(sesiones_locales):
            session.close()
        if shared_session is not None:
            shared_session.close()
        if shared_root is not None:
            cierra_root(shared_base, shared_root)
        for worktree in list(worktrees):
            remove_worktree(base_root, worktree)
        for base in efimeros:
            shutil.rmtree(base, ignore_errors=True)

    return results


# --------------------------------------------------------------------- el veredicto

def imprime_veredicto(corridas):
    """Imprime el resumen que exige `FORMATO_EXPECT.md §7` y devuelve los conteos.

    Una línea por caso; por cada FAIL, una línea de detalle por aserción incumplida que
    nombra caso, paso, path, esperado y real; y una línea agregada `RESUMEN: …`.
    """
    conteos = {"gate": 0, "pass": 0, "fail": 0, "exploratorios": 0, "skip": 0,
               "fuera_de_gate": 0, "fuera_de_gate_fail": 0}
    for corrida in corridas:
        print("\n### lote %s (%s, perfil %s)"
              % (corrida.get("batch"), corrida.get("root_kind"), corrida.get("profile")))
        for caso in corrida["cases"]:
            veredicto = caso.get("verdict", rx.EXPLORATORY)
            en_gate = caso.get("gate", True)
            etiqueta = "EXPLOR" if veredicto == rx.EXPLORATORY else veredicto
            sufijo = "" if en_gate or veredicto == rx.SKIP else "  (fuera de gate)"
            print("%-6s %s%s" % (etiqueta, caso["id"], sufijo))
            if veredicto == rx.EXPLORATORY:
                conteos["exploratorios"] += 1
            elif veredicto == rx.SKIP:
                conteos["skip"] += 1
            elif en_gate:
                conteos["gate"] += 1
                conteos["pass" if veredicto == rx.PASS else "fail"] += 1
            else:
                conteos["fuera_de_gate"] += 1
                if veredicto == rx.FAIL:
                    conteos["fuera_de_gate_fail"] += 1

            if veredicto == rx.FAIL:
                for fallo in caso.get("failures", []):
                    paso = fallo.get("step")
                    print("       caso %s · paso %s · %s: esperado %s · real %s (%s)"
                          % (caso["id"],
                             "-" if paso is None else paso,
                             fallo.get("path") or "<caso>",
                             rx._breve(fallo.get("expected")),
                             rx._breve(fallo.get("actual")),
                             fallo.get("reason")))
                    if fallo.get("describe"):
                        print("         porqué: %s" % fallo["describe"])

    # Los conteos se rotulan `PASSes`/`FAILes` y no `PASS`/`FAIL` a secas por una razón
    # mecánica: la palabra suelta `FAIL` es la MARCA de veredicto de un caso (la que se
    # busca con `\bFAIL\b` para saber si algo falló). Si el agregado la usara también
    # para decir «FAIL 0», una corrida limpia contendría la marca de fallo sin que nada
    # haya fallado, y ningún grep podría distinguir «hubo un FAIL» de «hubo cero FAIL».
    print("\nRESUMEN: gate %d casos · PASSes %d · FAILes %d · exploratorios %d · "
          "omitidos %d · fuera de gate %d (FAILes %d)"
          % (conteos["gate"], conteos["pass"], conteos["fail"], conteos["exploratorios"],
             conteos["skip"], conteos["fuera_de_gate"], conteos["fuera_de_gate_fail"]))
    return conteos


def escribe_resultados(corridas, out_path):
    payload = {"runs": corridas} if len(corridas) != 1 else corridas[0]
    texto = json.dumps(payload, ensure_ascii=False, indent=1)
    if out_path:
        with open(out_path, "w", encoding="utf-8") as f:
            f.write(texto)
        print("resultados en %s" % out_path)
    return texto


# ---------------------------------------------------------------------------- CLI

def main():
    ap = argparse.ArgumentParser(
        description="Arnés y runner asertable del banco de conformidad (E33-H02).")
    ap.add_argument("--root", help="raíz del workspace (campaña exploratoria: root REAL, "
                                   "solo readonly)")
    ap.add_argument("--profile", default="readonly")
    ap.add_argument("--call", nargs=2, metavar=("TOOL", "JSON"))
    ap.add_argument("--list-tools", action="store_true")
    ap.add_argument("--batch", help="un lote suelto")
    ap.add_argument("--run-all", action="store_true",
                    help="todos los lotes del gate (LOTES_DEL_GATE)")
    ap.add_argument("--root-corpus", help="corpus canónico ya generado; si falta, se "
                                          "genera uno efímero con make_corpus.py")
    ap.add_argument("--binary", help="binario lodestar-mcp (o LODESTAR_MCP_BIN)")
    ap.add_argument("--binary-cli", help="binario lodestar CLI (o LODESTAR_CLI_BIN)")
    ap.add_argument("--incluir-demos", action="store_true",
                    help="ejecuta también los casos gate:false (demostraciones)")
    ap.add_argument("--out", help="escribe el JSON de resultados")
    args = ap.parse_args()

    modos_primarios = []
    if args.batch:
        modos_primarios.append("--batch")
    if args.run_all:
        modos_primarios.append("--run-all")
    if args.call:
        modos_primarios.append("--call")
    if args.list_tools:
        modos_primarios.append("--list-tools")
    if len(modos_primarios) > 1:
        # This check intentionally happens before Entorno (and therefore before any
        # binary/corpus validation), but still uses the runner's controlled usage exit.
        print("USO: los modos primarios son mutuamente excluyentes: %s"
              % ", ".join(modos_primarios), file=sys.stderr)
        return 2

    entorno = Entorno(binario=args.binary, binario_cli=args.binary_cli,
                      root_corpus=args.root_corpus, root_real=args.root)

    try:
        call_arguments = None
        if args.call:
            try:
                call_arguments = json.loads(args.call[1])
            except (TypeError, json.JSONDecodeError) as exc:
                raise ErrorDeUso("JSON inválido en --call: %s" % exc) from exc
            if not isinstance(call_arguments, dict):
                raise ErrorDeUso("los argumentos de --call deben ser un objeto JSON")
        if args.batch or args.run_all:
            if args.run_all:
                lotes = [os.path.join(SCRATCH, rel) for rel in LOTES_DEL_GATE]
                faltan = [l for l in lotes if not os.path.exists(l)]
                if faltan:
                    raise ErrorDeUso("faltan lotes del gate: %s"
                                     % ", ".join(os.path.relpath(l, SCRATCH)
                                                 for l in faltan))
            else:
                lotes = [args.batch]

            corridas = [run_batch(lote, entorno, incluir_demos=args.incluir_demos)
                        for lote in lotes]
            conteos = imprime_veredicto(corridas)
            # La evidencia cruda (las respuestas del wire) va a `--out`, no a stdout: el
            # veredicto tiene que ser legible sin `| tail`, que es justo lo que §7 pide
            # imprimir. Sin `--out` no se pierde nada evaluable — los FAIL ya se detallan.
            try:
                escribe_resultados(corridas, args.out)
            except OSError as e:
                raise ErrorDeEjecucion("no se pudo escribir --out «%s»: %s" % (args.out, e))
            # Exit 1 = «hay FAIL». Los casos `gate: false` NO se ejecutan salvo que se
            # pidan con --incluir-demos, así que en la corrida por release (la que juzga
            # el gate) esto es exactamente «al menos un caso del gate es FAIL», el código
            # 1 del contrato. Con --incluir-demos —modo demostración, nunca release— un
            # FAIL de demo también sale ≠ 0: es lo que exige BDD-2 de la historia
            # («exit ≠ 0 si hay algún FAIL»). Ver la nota de desviación del README.
            return 1 if (conteos["fail"] or conteos["fuera_de_gate_fail"]) else 0

        # Modo suelto (llamada única / tools/list), la campaña exploratoria. `--root` es
        # por definición un root REAL declarado, así que la regla dura generalizada
        # (§7) se aplica sin excepción: solo readonly.
        root = args.root or REPO
        if args.profile != "readonly":
            raise ErrorDeUso("REGLA DURA: contra un root real (--root) solo se permite "
                             "--profile readonly; para mutar, usa un lote con "
                             "«root»: «corpus»")
        entorno.exige_binario()
        if not os.path.isdir(root):
            raise ErrorDeEjecucion("el root «%s» no es un directorio" % root)
        try:
            s = LodestarSession(root, args.profile, binary=entorno.binario)
        except Exception as exc:
            raise ErrorDeEjecucion("no se pudo abrir la sesión MCP: %s" % exc) from exc
        try:
            try:
                if args.list_tools:
                    print(json.dumps(s.tools_list(), ensure_ascii=False, indent=1))
                elif args.call:
                    print(json.dumps(s.call(args.call[0], call_arguments),
                                     ensure_ascii=False, indent=1))
                else:
                    raise ErrorDeUso("nada que hacer: usa --batch, --run-all, --call o "
                                     "--list-tools")
            except ErrorDeUso:
                raise
            except Exception as exc:
                raise ErrorDeEjecucion("falló la llamada MCP: %s" % exc) from exc
        finally:
            s.close()
        return 0
    except ErrorDeUso as e:
        print("USO: %s" % e, file=sys.stderr)
        return 2
    except ErrorDeEjecucion as e:
        print("ERROR DE EJECUCIÓN: %s" % e, file=sys.stderr)
        return 3
    finally:
        entorno.limpia()


if __name__ == "__main__":
    sys.exit(main())
