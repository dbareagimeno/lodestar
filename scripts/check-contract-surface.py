#!/usr/bin/env python3
"""Compare MCP tool registration, dispatch, profiles and contracts/mcp.yml."""

from __future__ import annotations

import re
import sys
from pathlib import Path


EXPECTED_TOOL_COUNT = 10
PROTOCOL_POLICY_SOURCE = Path("crates/lodestar-mcp/src/protocol_policy.rs")
PROTOCOL_DATES = {"2026-07-28", "2025-11-25"}


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


def parse_inline_yaml_list(raw: str) -> list[str]:
    """Parse the scalar-only inline lists used by the protocol policy without PyYAML."""
    return [item.strip().strip('"').strip("'") for item in raw.split(",") if item.strip()]


def era_methods(block: str, era: str, indent: int, key: str = "methods") -> list[str]:
    prefix = " " * indent
    match = re.search(
        rf"(?m)^{prefix}{re.escape(era)}:\s*(?:\n|$)(?:^(?:{prefix}  ).*\n)*?"
        rf"^{prefix}  {re.escape(key)}:\s*\[(.*?)\]\s*$",
        block,
    )
    if not match:
        return []
    return parse_inline_yaml_list(match.group(1))


def main() -> None:
    root = repo_root()
    source_path = root / "crates/lodestar-mcp/src/tools.rs"
    service_path = root / "crates/lodestar-mcp/src/lib.rs"
    facade_path = root / "crates/lodestar-mcp/src/main.rs"
    contract_path = root / "contracts/mcp.yml"
    source = source_path.read_text(encoding="utf-8")
    service = service_path.read_text(encoding="utf-8") if service_path.is_file() else ""
    facade = facade_path.read_text(encoding="utf-8")
    contract = contract_path.read_text(encoding="utf-8")
    failures: list[str] = []

    # E34-H02: el transporte no posee un segundo catálogo ni dispatcher. El servicio neutral
    # filtra y llama siempre al registro histórico único de tools.rs.
    if service.count("pub struct LodestarMcpService") != 1:
        failures.append("debe existir un único LodestarMcpService en src/lib.rs")
    for seam in ("tools::available_tools", "tools::call"):
        if seam not in service:
            failures.append(f"LodestarMcpService no reutiliza {seam}")
    if "tools::" in facade:
        failures.append("main.rs no debe saltarse LodestarMcpService para acceder a tools")
    if "propietario_catalogo_y_semantica: LodestarMcpService" not in contract:
        failures.append("el contrato no declara LodestarMcpService como propietario")

    # E34-H03: rmcp posee stdio/framing y el mismo executor servido posee todas las llamadas. Un
    # lector JSON manual o un SerialExecutor decorativo volverían a abrir dos caminos de wire.
    if "impl ServerHandler for SerialExecutor<LodestarMcpService>" not in service:
        failures.append("SerialExecutor<LodestarMcpService> debe ser el ServerHandler real")
    if "self.run(" not in service:
        failures.append("el adaptador rmcp no cruza el executor serial")
    if "rmcp::transport::stdio()" not in facade or ".serve(" not in facade:
        failures.append("main.rs debe servir rmcp::transport::stdio()")
    for manual_fragment in (
        "BufRead",
        "read_line",
        "stdin.lock()",
        "serde_json::from_str",
        "serde_json::from_slice",
    ):
        if manual_fragment in facade:
            failures.append(f"framing manual prohibido en main.rs: {manual_fragment}")
    for contract_fragment in (
        "transporte: rmcp::transport::stdio",
        "adaptador_rmcp: SerialExecutor<LodestarMcpService>",
        "stdout: exclusivamente mensajes MCP",
        "stderr: logs y diagnósticos de proceso",
    ):
        if contract_fragment not in contract:
            failures.append(f"contrato H03 incompleto: falta {contract_fragment}")

    # E34-H04: el adaptador de lifecycle no puede convertirse en un segundo handler. El handler
    # único sirve Modern tipado y el wrapper sólo acota initialize a Legacy.
    for service_fragment in (
        "pub struct LodestarMcpServer",
        "impl Service<RoleServer> for LodestarMcpServer",
        "DiscoverResult::from_server_info",
        "protocol_policy::modern_protocol_version()",
        "protocol_policy::legacy_protocol_version()",
        ".with_ttl_ms(0)",
        ".with_cache_scope(CacheScope::Private)",
        "result.result_type = Some(ResultType::COMPLETE)",
        "context.meta.client_info()",
        "client_info.name.trim().is_empty()",
        "client_info.version.trim().is_empty()",
        "ClientRequest::DiscoverRequest",
        "ClientRequest::CustomRequest",
    ):
        if service_fragment not in service:
            failures.append(f"wire Modern incompleto: falta {service_fragment}")
    for contract_fragment in (
        'methods: ["server/discover", "tools/list", "tools/call"]',
        "request_metadata_location: params._meta",
        "io.modelcontextprotocol/clientInfo",
        "non_empty_strings: true",
        "serverInfo_metadata_key: io.modelcontextprotocol/serverInfo",
        "version_no_soportada: -32022",
        "cacheScope: private",
    ):
        if contract_fragment not in contract:
            failures.append(f"contrato H04 incompleto: falta {contract_fragment}")

    for contract_fragment in (
        'methods: [initialize, "notifications/initialized", ping, "tools/list", "tools/call"]',
        "negotiation: always_legacy_baseline",
        "forbidden_result_fields: [resultType, ttlMs, cacheScope]",
        'forbidden_methods: ["server/discover", "resources/list", "prompts/list"]',
    ):
        if contract_fragment not in contract:
            failures.append(f"contrato H05 incompleto: falta {contract_fragment}")

    # E34-H06: la cancelación cooperativa sólo puede cortar la espera anterior al turno. Tras
    # admitir la request, el handler delega sin seleccionar sobre el token para conservar la
    # atomicidad del App. Estos fragmentos impiden volver a una espera ciega o documentar una
    # cancelación que producción no observa.
    for service_fragment in (
        "turn = fifo.lock()",
        "context.ct.cancelled()",
        "context.ct.is_cancelled()",
        "request cancelled before serialized execution",
    ):
        if service_fragment not in service:
            failures.append(f"cancelación H06 incompleta: falta {service_fragment}")
    for contract_fragment in (
        "notificacion: notifications/cancelled de rmcp",
        "antes_del_turno:",
        "despues_de_admitir:",
        "sin_request_id: no cancela otra request",
    ):
        if contract_fragment not in contract:
            failures.append(f"contrato H06 incompleto: falta {contract_fragment}")

    # E34: las revisiones de protocolo son datos de policy, nunca condiciones repartidas por la
    # fachada. Esta guarda incluye comentarios y literales para impedir que una futura rama se
    # cuele disfrazada de documentación junto al handler.
    policy_path = root / PROTOCOL_POLICY_SOURCE
    if not policy_path.is_file():
        failures.append(f"falta la fuente única de protocolo: {PROTOCOL_POLICY_SOURCE}")
    else:
        policy_dates = set(re.findall(r'\b20\d{2}-\d{2}-\d{2}\b', policy_path.read_text(encoding="utf-8")))
        if policy_dates != PROTOCOL_DATES:
            failures.append(
                f"protocol_policy debe contener exactamente {sorted(PROTOCOL_DATES)!r}; "
                f"contiene {sorted(policy_dates)!r}"
            )

    for rust_path in sorted((root / "crates/lodestar-mcp/src").glob("**/*.rs")):
        rust_source = rust_path.read_text(encoding="utf-8")
        relative = rust_path.relative_to(root)
        if relative != PROTOCOL_POLICY_SOURCE:
            dates = sorted(set(re.findall(r'\b20\d{2}-\d{2}-\d{2}\b', rust_source)))
            if dates:
                failures.append(f"fechas de protocolo fuera de protocol_policy en {relative}: {dates!r}")
        if "ProtocolVersion::KNOWN_VERSIONS" in rust_source:
            failures.append(f"uso prohibido de ProtocolVersion::KNOWN_VERSIONS en {relative}")

    if contract.count("\nprotocol_policy:\n") != 1:
        failures.append("contracts/mcp.yml debe declarar un único bloque protocol_policy")
    else:
        policy_start = contract.index("\nprotocol_policy:\n")
        meta_start = contract.index("\nmeta:\n", policy_start)
        policy_block = contract[policy_start:meta_start]
        meta_block = contract[meta_start:]
        announced_match = re.search(
            r"(?ms)^    metodos_por_era:\s*\n"
            r"      Modern:\s*\[(.*?)\]\s*\n"
            r"      Legacy:\s*\[(.*?)\]\s*$",
            meta_block,
        )
        if not announced_match:
            failures.append("meta.protocolo.metodos_por_era no es extraíble para las dos eras")
        else:
            announced = {
                "Modern": parse_inline_yaml_list(announced_match.group(1)),
                "Legacy": parse_inline_yaml_list(announced_match.group(2)),
            }
            declared = {
                "Modern": era_methods(policy_block, "Modern", 4),
                "Legacy": era_methods(policy_block, "Legacy", 4),
            }
            for era in ("Modern", "Legacy"):
                if not declared[era]:
                    failures.append(f"protocol_policy.{era}.methods no es extraíble")
                elif announced[era] != declared[era]:
                    failures.append(
                        f"métodos anunciados != policy para {era}: "
                        f"{announced[era]!r} != {declared[era]!r}"
                    )

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
