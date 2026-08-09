#!/usr/bin/env python3
"""Arnés JSON-RPC/stdio para lodestar-mcp.

Modos:
  - Llamada suelta:   lodestar_harness.py --root DIR [--profile P] --call TOOL 'JSON'
  - Lote:             lodestar_harness.py --batch spec.json --out results.json
  - tools/list:       lodestar_harness.py --root DIR [--profile P] --list-tools

Un spec de lote es JSON:
{
  "batch": "L1",
  "root": "real" | "worktree",          # real = repo homelab, readonly obligatorio
  "profile": "readonly" | "standard",
  "fixtures": null | "mdi" | "chk_a" | "chk_b",
  "cases": [
    {"id": "TYP-01", "tool": "knowledge_search", "arguments": {...}},        # forma corta
    {"id": "APL-01", "fresh_root": true, "steps": [                          # forma larga
        {"kind": "call", "tool": "change_plan", "arguments": {...}},
        {"kind": "shell", "cmd": "git diff --stat"},
        {"kind": "call", "tool": "change_apply",
         "arguments": {"changeSetId": "@step0.structured.changeSetId"}},
        {"kind": "raw", "line": "{json roto"},
        {"kind": "spawn", "args": ["--root", "/no/existe"]}
    ]}
  ]
}

Placeholders: cualquier string "@stepN.ruta.de.campos" en arguments se sustituye
por el valor de ese paso previo del MISMO caso (p. ej. "@step0.structured.changeSetId").

Los errores de EJECUCIÓN de tool (result.isError + texto "CODIGO: mensaje") se
distinguen de los errores de PROTOCOLO JSON-RPC (-32602, -32700...).
"""
import argparse
import json
import os
import re
import subprocess
import sys
import tempfile
import time
import uuid

BINARY = "/Users/dbarea/repos/lodestar/target/release/lodestar-mcp"
HOMELAB = "/Users/dbarea/repos/homelab"
SCRATCH = os.path.dirname(os.path.abspath(__file__))
WT_BASE = os.path.join(SCRATCH, "wt")


class LodestarSession:
    def __init__(self, root, profile="standard", binary=BINARY):
        self.proc = subprocess.Popen(
            [binary, "--root", root, "--profile", profile],
            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
            text=True, bufsize=1,
        )
        self._next_id = 1
        self._initialize()

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


def make_worktree():
    os.makedirs(WT_BASE, exist_ok=True)
    path = os.path.join(WT_BASE, f"wt-{uuid.uuid4().hex[:8]}")
    subprocess.run(["git", "-C", HOMELAB, "worktree", "add", "--detach", path, "HEAD"],
                   check=True, capture_output=True, text=True)
    return path


def remove_worktree(path):
    subprocess.run(["git", "-C", HOMELAB, "worktree", "remove", "--force", path],
                   capture_output=True, text=True)


def inject_fixtures(fixture_set, root):
    script = os.path.join(SCRATCH, "make_fixtures.py")
    for part in fixture_set.split("+"):
        subprocess.run([sys.executable, script, part, root], check=True,
                       capture_output=True, text=True)


def resolve_placeholders(value, step_results):
    if isinstance(value, str):
        m = re.fullmatch(r"@step(\d+)\.(.+)", value)
        if m:
            obj = step_results[int(m.group(1))]
            for part in m.group(2).split("."):
                obj = obj[int(part)] if isinstance(obj, list) else obj[part]
            return obj
        return value
    if isinstance(value, dict):
        return {k: resolve_placeholders(v, step_results) for k, v in value.items()}
    if isinstance(value, list):
        return [resolve_placeholders(v, step_results) for v in value]
    return value


def run_step(step, session, root, step_results):
    kind = step.get("kind", "call")
    if kind == "call":
        args = resolve_placeholders(step.get("arguments", {}), step_results)
        return session.call(step["tool"], args)
    if kind == "shell":
        cmd = resolve_placeholders(step["cmd"], step_results)
        r = subprocess.run(cmd, shell=True, cwd=root, capture_output=True, text=True)
        return {"kind": "shell", "cmd": cmd, "rc": r.returncode,
                "stdout": r.stdout[-4000:], "stderr": r.stderr[-4000:]}
    if kind == "raw":
        return {"kind": "raw", "line": step["line"], "response": session.raw_line(step["line"])}
    if kind == "spawn":
        args = [root if a == "@root" else a for a in step["args"]]
        r = subprocess.run([BINARY] + args, capture_output=True, text=True,
                           timeout=15, input="")
        return {"kind": "spawn", "args": args, "rc": r.returncode,
                "stdout": r.stdout[:2000], "stderr": r.stderr[:2000]}
    if kind == "list_tools":
        return session.tools_list()
    raise ValueError(f"kind desconocido: {kind}")


def run_batch(spec_path, out_path):
    with open(spec_path) as f:
        spec = json.load(f)
    profile = spec.get("profile", "readonly")
    use_worktree = spec.get("root") == "worktree"
    if not use_worktree and profile != "readonly":
        raise SystemExit("REGLA DURA: contra el repo real solo se permite --profile readonly")

    results = {"batch": spec.get("batch"), "root_kind": spec.get("root"),
               "profile": profile, "cases": []}

    shared_root = None
    shared_session = None

    def open_shared():
        nonlocal shared_root, shared_session
        shared_root = make_worktree() if use_worktree else HOMELAB
        if use_worktree and spec.get("fixtures"):
            inject_fixtures(spec["fixtures"], shared_root)
        shared_session = LodestarSession(shared_root, profile)

    try:
        for case in spec["cases"]:
            fresh = bool(case.get("fresh_root"))
            no_server = bool(case.get("no_server"))
            session_error = None
            if no_server:
                root = make_worktree() if use_worktree else HOMELAB
                if use_worktree and spec.get("fixtures"):
                    inject_fixtures(spec["fixtures"], root)
                session = None
            elif fresh:
                root = make_worktree() if use_worktree else HOMELAB
                if use_worktree and spec.get("fixtures"):
                    inject_fixtures(spec["fixtures"], root)
                try:
                    session = LodestarSession(root, profile)
                except Exception as e:
                    session, session_error = None, f"{type(e).__name__}: {e}"
            else:
                if shared_session is None:
                    try:
                        open_shared()
                    except Exception as e:
                        results["cases"].append({"id": case["id"], "steps": [],
                                                 "session_error": f"{type(e).__name__}: {e}"})
                        if use_worktree and shared_root:
                            remove_worktree(shared_root)
                            shared_root = None
                        continue
                root, session = shared_root, shared_session

            steps = case.get("steps") or [{"kind": "call", "tool": case["tool"],
                                           "arguments": case.get("arguments", {})}]
            step_results = []
            case_out = {"id": case["id"], "steps": []}
            if session_error:
                case_out["session_error"] = session_error
            for step in steps:
                try:
                    res = run_step(step, session, root, step_results)
                except Exception as e:
                    res = {"harness_exception": f"{type(e).__name__}: {e}"}
                step_results.append(res)
                case_out["steps"].append(res)
            results["cases"].append(case_out)

            if fresh or no_server:
                if session is not None:
                    session.close()
                if use_worktree:
                    remove_worktree(root)
    finally:
        if shared_session is not None:
            shared_session.close()
        if use_worktree and shared_root:
            remove_worktree(shared_root)

    payload = json.dumps(results, ensure_ascii=False, indent=1)
    if out_path:
        with open(out_path, "w") as f:
            f.write(payload)
        print(f"resultados en {out_path} ({len(results['cases'])} casos)")
    else:
        print(payload)


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--root")
    ap.add_argument("--profile", default="readonly")
    ap.add_argument("--call", nargs=2, metavar=("TOOL", "JSON"))
    ap.add_argument("--list-tools", action="store_true")
    ap.add_argument("--batch")
    ap.add_argument("--out")
    args = ap.parse_args()

    if args.batch:
        run_batch(args.batch, args.out)
        return
    root = args.root or HOMELAB
    if os.path.realpath(root) == os.path.realpath(HOMELAB) and args.profile != "readonly":
        raise SystemExit("REGLA DURA: contra el repo real solo se permite --profile readonly")
    s = LodestarSession(root, args.profile)
    try:
        if args.list_tools:
            print(json.dumps(s.tools_list(), ensure_ascii=False, indent=1))
        elif args.call:
            print(json.dumps(s.call(args.call[0], json.loads(args.call[1])),
                             ensure_ascii=False, indent=1))
    finally:
        s.close()


if __name__ == "__main__":
    main()
