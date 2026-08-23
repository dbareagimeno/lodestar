#!/usr/bin/env python3
"""Emit a stable semantic public API snapshot from rustdoc JSON.

This intentionally consumes rustdoc's model rather than source text.  Rustdoc IDs, spans,
documentation and other build-location details are removed while paths, signatures, fields,
variants, reexports and inherent methods remain part of the semantic contract.
"""

from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys
from typing import Any


DROP_KEYS = {"id", "crate_id", "span", "docs", "links", "attrs", "deprecation"}


def clean(value: Any) -> Any:
    if isinstance(value, dict):
        return {key: clean(item) for key, item in sorted(value.items()) if key not in DROP_KEYS}
    if isinstance(value, list):
        return [clean(item) for item in value]
    return value


def path_text(value: Any) -> str:
    if not isinstance(value, dict):
        return str(value)
    if "resolved_path" in value:
        return path_text(value["resolved_path"])
    if "path" in value:
        return str(value["path"])
    return type_text(value)


def generic_args(value: Any) -> str:
    if not isinstance(value, dict):
        return ""
    angle = value.get("angle_bracketed")
    if not isinstance(angle, dict):
        return ""
    args = angle.get("args", [])
    rendered = []
    for arg in args:
        if isinstance(arg, dict) and "type" in arg:
            rendered.append(type_text(arg["type"]))
        elif isinstance(arg, dict) and "const" in arg:
            rendered.append(str(arg["const"]))
        else:
            rendered.append(type_text(arg))
    return "<" + ", ".join(rendered) + ">" if rendered else ""


def type_text(value: Any) -> str:
    if value is None:
        return "()"
    if isinstance(value, str):
        return value
    if not isinstance(value, dict):
        return json.dumps(value, sort_keys=True, separators=(",", ":"))
    if "resolved_path" in value:
        resolved = value["resolved_path"]
        if isinstance(resolved, dict):
            return str(resolved.get("path", "?")) + generic_args(resolved.get("args"))
    if "generic" in value:
        return str(value["generic"])
    if "borrowed_ref" in value:
        ref = value["borrowed_ref"]
        prefix = "&mut " if ref.get("is_mutable") else "&"
        return prefix + type_text(ref.get("type"))
    if "raw_pointer" in value:
        pointer = value["raw_pointer"]
        return ("*mut " if pointer.get("is_mutable") else "*const ") + type_text(pointer.get("type"))
    if "slice" in value:
        return "[" + type_text(value["slice"]) + "]"
    if "tuple" in value:
        return "(" + ", ".join(type_text(item) for item in value["tuple"]) + ")"
    if "primitive" in value:
        return str(value["primitive"])
    if "infer" in value:
        return "_"
    if "qualified_path" in value:
        return type_text(value["qualified_path"])
    if "function_pointer" in value:
        return "fn " + json.dumps(clean(value["function_pointer"]), sort_keys=True, separators=(",", ":"))
    return json.dumps(clean(value), sort_keys=True, separators=(",", ":"))


def generics(value: Any) -> list[Any]:
    if not isinstance(value, dict):
        return []
    params = value.get("params", [])
    result = []
    for param in params:
        if not isinstance(param, dict):
            result.append(clean(param))
            continue
        # Keep the semantic parameter kind/name/bounds, but not rustdoc's IDs.
        result.append(clean(param))
    return result


def signature(inner: dict[str, Any]) -> dict[str, Any] | None:
    function = inner.get("function") or inner.get("method")
    if not isinstance(function, dict):
        return None
    sig = function.get("sig", {})
    inputs = []
    for item in sig.get("inputs", []):
        if isinstance(item, list) and len(item) == 2:
            inputs.append([item[0], type_text(item[1])])
    header = function.get("header", {})
    return {
        "inputs": inputs,
        "output": type_text(sig.get("output")),
        "generics": generics(function.get("generics")),
        "async": bool(header.get("is_async")),
        "unsafe": bool(header.get("is_unsafe")),
        "const": bool(header.get("is_const")),
    }


def item_kind(inner: dict[str, Any]) -> str:
    if not inner:
        return "unknown"
    return next(iter(inner))


def main() -> int:
    parser = argparse.ArgumentParser()
    parser.add_argument("--manifest-path", required=True)
    parser.add_argument("--output", required=True)
    parser.add_argument("--target-dir", required=True)
    args = parser.parse_args()
    manifest = pathlib.Path(args.manifest_path).resolve()
    output = pathlib.Path(args.output)
    target = pathlib.Path(args.target_dir).resolve()
    target.mkdir(parents=True, exist_ok=True)

    env = os.environ.copy()
    env.setdefault("RUSTC_BOOTSTRAP", "1")
    env["CARGO_NET_OFFLINE"] = "true"
    command = [
        "cargo",
        "rustdoc",
        "--offline",
        "--manifest-path",
        str(manifest),
        "--lib",
        "--target-dir",
        str(target),
        "--",
        "-Z",
        "unstable-options",
        "--output-format",
        "json",
    ]
    run = subprocess.run(command, cwd=manifest.parent.parent, env=env, text=True, capture_output=True)
    if run.returncode:
        sys.stderr.write(run.stdout)
        sys.stderr.write(run.stderr)
        return run.returncode
    manifest_text = manifest.read_text(encoding="utf-8")
    package_match = re.search(r"(?m)^\s*name\s*=\s*[\"']([^\"']+)[\"']", manifest_text)
    package_name = package_match.group(1) if package_match else manifest.parent.name
    crate_name = package_name.replace("-", "_")
    rustdoc = target / "doc" / f"{crate_name}.json"
    if not rustdoc.is_file():
        print(f"rustdoc JSON not found: {rustdoc}", file=sys.stderr)
        return 1
    data = json.loads(rustdoc.read_text(encoding="utf-8"))
    index = data.get("index", {})
    paths = data.get("paths", {})
    by_id = {str(key): value for key, value in index.items()}

    def path_for(item_id: Any, item: dict[str, Any]) -> str:
        entry = paths.get(str(item_id), {})
        if isinstance(entry, dict) and entry.get("path"):
            return "::".join(entry["path"])
        return f"{crate_name}::{item.get('name') or ''}".rstrip(":")

    def field_shape(field_id: Any) -> dict[str, Any]:
        field = by_id.get(str(field_id), {})
        field_inner = field.get("inner", {}).get("struct_field", {})
        return {"name": field.get("name"), "type": type_text(field_inner)}

    def stable_variant_kind(value: Any) -> Any:
        if value == "plain":
            return value
        if not isinstance(value, dict):
            return clean(value)
        if isinstance(value.get("tuple"), list):
            return {"tuple": [field_shape(field_id)["type"] for field_id in value["tuple"]]}
        struct = value.get("struct")
        if isinstance(struct, dict):
            fields = [field_shape(field_id) for field_id in struct.get("fields", [])]
            fields.sort(key=lambda field: str(field.get("name")))
            return {
                "struct": {
                    "fields": fields,
                    "has_stripped_fields": bool(struct.get("has_stripped_fields")),
                }
            }
        return clean(value)

    semantic: list[dict[str, Any]] = []
    owner_by_id: dict[str, dict[str, Any]] = {}
    for item_id, item in by_id.items():
        if item.get("visibility") != "public" or not item.get("name"):
            continue
        inner = item.get("inner", {})
        kind = item_kind(inner)
        record: dict[str, Any] = {"kind": kind, "name": item["name"], "path": path_for(item_id, item)}
        body = inner.get(kind, {}) if isinstance(inner, dict) else {}
        if kind in {"function", "method"}:
            record["signature"] = signature(inner)
        elif kind in {"struct", "union"}:
            shape = body.get("kind", {})
            plain = shape.get("plain", {}) if isinstance(shape, dict) else {}
            record["fields"] = []
            for field_id in plain.get("fields", []):
                field = by_id.get(str(field_id), {})
                field_inner = field.get("inner", {}).get("struct_field", {})
                record["fields"].append({"name": field.get("name"), "type": type_text(field_inner)})
            record["fields"].sort(key=lambda field: str(field.get("name")))
            record["generics"] = generics(body.get("generics"))
            owner_by_id[item_id] = record
        elif kind == "enum":
            record["variants"] = []
            for variant_id in body.get("variants", []):
                variant = by_id.get(str(variant_id), {})
                variant_body = variant.get("inner", {}).get("variant", {})
                record["variants"].append(
                    {
                        "name": variant.get("name"),
                        "kind": stable_variant_kind(variant_body.get("kind")),
                    }
                )
            record["variants"].sort(key=lambda variant: str(variant.get("name")))
            record["generics"] = generics(body.get("generics"))
            owner_by_id[item_id] = record
        elif kind == "trait":
            record["generics"] = generics(body.get("generics"))
            owner_by_id[item_id] = record
        elif kind in {"type_alias", "constant", "static", "use"}:
            record["details"] = clean(body)
        semantic.append(record)

    # Inherent public methods are represented by impl blocks in rustdoc, not as top-level paths.
    for item_id, item in by_id.items():
        impl = item.get("inner", {}).get("impl") if isinstance(item.get("inner"), dict) else None
        if not isinstance(impl, dict) or impl.get("trait") is not None or impl.get("is_synthetic"):
            continue
        owner = impl.get("for", {}).get("id") if isinstance(impl.get("for"), dict) else None
        record = owner_by_id.get(str(owner))
        if record is None:
            continue
        methods = record.setdefault("methods", [])
        for method_id in impl.get("items", []):
            method = by_id.get(str(method_id), {})
            if method.get("visibility") != "public" or not method.get("name"):
                continue
            methods.append({"name": method["name"], "signature": signature(method.get("inner", {}))})
        methods.sort(key=lambda method: (method["name"], json.dumps(method["signature"], sort_keys=True)))

    for record in semantic:
        record.setdefault("methods", []) if record["kind"] in {"struct", "enum", "union", "trait"} else None
    semantic.sort(key=lambda record: (record["path"], record["kind"], record["name"]))
    result = {
        "metadata": {
            "package": package_name,
            "default_features": [],
            "manifest_path": str(manifest),
            "manifest_processed": True,
        },
        "semantic": semantic,
    }
    output.parent.mkdir(parents=True, exist_ok=True)
    output.write_text(
        json.dumps(result, indent=2, sort_keys=True) + "\n",
        encoding="utf-8",
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
