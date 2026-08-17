//! E34-H01: política dual-era, dependencias acotadas y catálogo cerrado.
//!
//! Este archivo es deliberadamente independiente de `main.rs`: C1 incluye el seam puro de
//! `protocol_policy` y C3 habla con el binario real por stdio.

use std::collections::{BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde_json::Value;

#[path = "../src/protocol_policy.rs"]
mod protocol_policy;

const MODERNA: &str = "2026-07-28";
const LEGACY: &str = "2025-11-25";
const INJECTED_INITIALIZE_ID: &str = "__lodestar_h01_initialize__";
const TOOLS: [&str; 10] = [
    "workspace_status",
    "knowledge_search",
    "knowledge_get",
    "metadata_inspect",
    "knowledge_check",
    "graph_query",
    "impact_analyze",
    "change_plan",
    "change_apply",
    "change_revert",
];

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("ruta de fixture con padre")).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn workspace_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    // Anti-vacuidad: tools/list se comprueba sobre un workspace que contiene Markdown real.
    write_file(root.path(), "note.md", "# nota\n\ncontenido no vacío\n");
    root
}

fn roundtrip(root: &Path, requests: &[Value]) -> Vec<Value> {
    let has_initialize = requests
        .iter()
        .any(|request| request["method"] == "initialize");
    let mut transcript = Vec::new();
    if !has_initialize {
        transcript.push(serde_json::json!({
            "jsonrpc": "2.0",
            "id": INJECTED_INITIALIZE_ID,
            "method": "initialize",
            "params": {
                "protocolVersion": LEGACY,
                "capabilities": {},
                "clientInfo": {"name": "e34-h01", "version": "1"}
            }
        }));
        transcript.push(serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));
    } else if !requests
        .iter()
        .any(|request| request["method"] == "notifications/initialized")
    {
        transcript.extend(requests.iter().cloned());
        if let Some(index) = transcript
            .iter()
            .position(|request| request["method"] == "initialize")
        {
            transcript.insert(
                index + 1,
                serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "notifications/initialized"
                }),
            );
        }
    }
    if !has_initialize
        || requests
            .iter()
            .any(|request| request["method"] == "notifications/initialized")
    {
        transcript.extend(requests.iter().cloned());
    }

    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("arranca lodestar-mcp");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    let mut responses = Vec::new();
    for request in &transcript {
        writeln!(stdin, "{}", serde_json::to_string(request).unwrap()).unwrap();
        if request["method"] == "notifications/initialized" {
            continue;
        }
        let mut line = String::new();
        if stdout.read_line(&mut line).unwrap_or(0) == 0 {
            break;
        }
        responses.push(serde_json::from_str(&line).expect("stdout contiene JSON-RPC válido"));
    }
    drop(stdin);
    let status = child.wait().expect("termina el proceso");
    assert!(status.success(), "lodestar-mcp termina con {status}");
    responses
        .into_iter()
        .filter(|response: &Value| {
            !(!has_initialize
                && response["id"] == INJECTED_INITIALIZE_ID
                && response["error"].is_null()
                && response["result"]["protocolVersion"] == LEGACY)
        })
        .collect()
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .parent()
        .unwrap()
        .to_path_buf()
}

fn cargo_metadata() -> Value {
    let output = Command::new("cargo")
        .args(["metadata", "--locked", "--no-deps", "--format-version", "1"])
        .current_dir(workspace_root())
        .output()
        .expect("cargo metadata arranca");
    assert!(
        output.status.success(),
        "cargo metadata debe terminar correctamente: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).expect("cargo metadata produce JSON estructurado")
}

fn metadata_packages(metadata: &Value) -> &Vec<Value> {
    metadata["packages"]
        .as_array()
        .expect("cargo metadata contiene packages")
}

fn package<'a>(packages: &'a [Value], name: &str) -> &'a Value {
    packages
        .iter()
        .find(|candidate| candidate["name"] == name)
        .unwrap_or_else(|| panic!("cargo metadata no contiene el paquete {name}"))
}

/// C1 — el seam ejecutable resuelve únicamente las dos eras y rechaza cualquier otra fecha.
///
/// `policy_for` es la consulta ejecutable que H01 exige en el seam puro; no se sustituye por una
/// inspección textual de producción.
#[test]
fn mcp_policy_matrix() {
    assert_eq!(protocol_policy::LATEST, MODERNA);
    assert_eq!(
        protocol_policy::supported_versions(),
        &[MODERNA, LEGACY],
        "la lista pública de eras debe ser cerrada y ordenada Modern, Legacy"
    );
    assert_eq!(
        protocol_policy::resolve(MODERNA),
        Ok(protocol_policy::Era::Modern),
        "Modern debe mapearse a Era::Modern"
    );
    assert_eq!(
        protocol_policy::resolve(LEGACY),
        Ok(protocol_policy::Era::Legacy),
        "Legacy debe mapearse a Era::Legacy"
    );

    // Negativos ratificados: antigua, futura, draft e intermedia no pueden caer en un fallback.
    for unsupported in [
        "2024-11-05",         // antigua
        "2099-12-31",         // futura
        "draft",              // draft
        "2026-01-01",         // intermedia
        "2025-11-25-preview", // alias
    ] {
        assert_eq!(
            protocol_policy::resolve(unsupported),
            Err(protocol_policy::UnsupportedVersion),
            "la versión no ratificada {unsupported:?} debe rechazarse"
        );
    }

    // Lifecycle Legacy ya expuesto: initialize nunca convierte una oferta en una tercera era.
    for offered in [None, Some(MODERNA), Some(LEGACY), Some("2099-12-31")] {
        assert_eq!(
            protocol_policy::initialize_version(offered),
            LEGACY,
            "initialize debe seleccionar siempre la baseline Legacy para {offered:?}"
        );
    }

    // Seam exigido por H01. Los campos son deliberadamente tipados por la API (str/Option/bool),
    // no strings extraídos de `protocol_policy.rs`: Modern es stateless con metadata obligatoria,
    // resultType y hints de caché; Legacy negocia initialize, no exige metadata y no lleva esos
    // campos Modern. Ping forma parte únicamente del lifecycle Legacy.
    let modern = protocol_policy::policy_for(protocol_policy::Era::Modern);
    assert_eq!(modern.lifecycle, "stateless");
    assert_eq!(modern.request_metadata, "required");
    assert_eq!(modern.result_type.as_deref(), Some("complete"));
    assert!(modern.cache_ttl_ms.is_some());
    assert!(modern.cache_scope.is_some());
    assert!(!modern.initialize);
    assert!(!modern.ping);

    let legacy = protocol_policy::policy_for(protocol_policy::Era::Legacy);
    assert_eq!(legacy.lifecycle, "initialize");
    assert_eq!(legacy.request_metadata, "absent");
    assert!(legacy.result_type.is_none());
    assert!(legacy.cache_ttl_ms.is_none());
    assert!(legacy.cache_scope.is_none());
    assert!(legacy.initialize);
    assert!(legacy.ping);

    // Anti-dispersión: solo el seam incluido puede contener las fechas congeladas.
    let src_root = workspace_root().join("crates/lodestar-mcp/src");
    for entry in std::fs::read_dir(src_root).expect("existe src de lodestar-mcp") {
        let entry = entry.unwrap();
        if entry.path().extension().is_some_and(|ext| ext == "rs")
            && entry.file_name() != "protocol_policy.rs"
        {
            let source = std::fs::read_to_string(entry.path()).unwrap();
            let dates: BTreeSet<&str> = source
                .split(|c: char| !c.is_ascii_digit() && c != '-')
                .filter(|candidate| {
                    candidate.len() == 10
                        && candidate.as_bytes()[4] == b'-'
                        && candidate.as_bytes()[7] == b'-'
                })
                .collect();
            assert!(
                dates.is_empty(),
                "las fechas de protocolo se comparan fuera de protocol_policy: {}",
                entry.path().display()
            );
        }
    }
}

/// C2 — Cargo metadata estructurado fija dependencias y MSRV por paquete, sin parsear TOML a mano.
#[test]
fn mcp_dependency_scope_msrv() {
    let metadata = cargo_metadata();
    let packages = metadata_packages(&metadata);
    let mcp = package(packages, "lodestar-mcp");

    assert_eq!(mcp["rust_version"], "1.88");
    let dependencies = mcp["dependencies"]
        .as_array()
        .expect("metadata del mcp contiene dependencies");
    let rmcp = dependencies
        .iter()
        .find(|dependency| dependency["name"] == "rmcp")
        .expect("lodestar-mcp declara rmcp");
    assert_eq!(rmcp["req"], "=3.1.2");
    assert!(
        dependencies
            .iter()
            .any(|dependency| dependency["name"] == "tokio"),
        "lodestar-mcp declara Tokio"
    );

    let mut declarations: HashMap<&str, Vec<&str>> = HashMap::new();
    for candidate in packages {
        for dependency in candidate["dependencies"]
            .as_array()
            .expect("cada paquete tiene dependencies")
        {
            for name in ["rmcp", "tokio"] {
                if dependency["name"] == name {
                    declarations
                        .entry(name)
                        .or_default()
                        .push(candidate["name"].as_str().unwrap());
                }
            }
        }
    }
    assert_eq!(declarations["rmcp"], vec!["lodestar-mcp"]);
    assert_eq!(declarations["tokio"], vec!["lodestar-mcp"]);

    for candidate in packages {
        if candidate["name"] == "lodestar-mcp" {
            continue;
        }
        assert_eq!(
            candidate["rust_version"], "1.80",
            "{} debe conservar MSRV 1.80",
            candidate["name"]
        );
    }
}

/// C2 — compilación MSRV real, acotada para no ralentizar el subconjunto normal.
///
/// No se usa `rustc --version`, una variable de entorno ni una simulación: ambos comandos invocan
/// Cargo con el toolchain exacto y `--locked`, y fallan si la toolchain no está instalada.
#[test]
#[ignore = "gate explícito de toolchains: cargo +1.80.1 y cargo +1.88.0"]
fn mcp_msrv_real_toolchains_compile() {
    let base = Command::new("cargo")
        .args([
            "+1.80.1",
            "check",
            "--workspace",
            "--exclude",
            "lodestar-mcp",
            "--locked",
        ])
        .current_dir(workspace_root())
        .status()
        .expect("cargo +1.80.1 arranca");
    assert!(base.success(), "las crates no-mcp compilan con MSRV 1.80.1");

    let mcp = Command::new("cargo")
        .args(["+1.88.0", "check", "-p", "lodestar-mcp", "--locked"])
        .current_dir(workspace_root())
        .status()
        .expect("cargo +1.88.0 arranca");
    assert!(mcp.success(), "lodestar-mcp compila con MSRV 1.88.0");
}

/// C3 — wire real: catálogo exacto, capacidades únicamente `tools` y métodos fuera de alcance.
#[test]
fn mcp_catalogo_unico_y_capacidades() {
    let fixture = workspace_fixture();
    let forbidden_methods = [
        "resources/list",
        "resources/templates/list",
        "resources/subscribe",
        "resources/unsubscribe",
        "prompts/list",
        "sampling/createMessage",
        "elicitation/create",
        "tasks/list",
        "tasks/get",
        "tasks/result",
        "tasks/cancel",
        "completion/complete",
        "logging/setLevel",
        "subscriptions/list",
        "initializeExtra",
    ];
    let mut requests = vec![serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": LEGACY,
            "capabilities": {},
            "clientInfo": {"name": "e34-h01", "version": "1"}
        }
    })];
    requests.push(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list"
    }));
    // MRTR/input-response metadata is outside H01's closed tool surface. It must not be silently
    // ignored and must not execute the otherwise valid workspace_status call.
    requests.push(serde_json::json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "workspace_status",
            "arguments": {},
            "inputResponses": [],
            "_meta": {"io.modelcontextprotocol/requestState": "mrtr"}
        }
    }));
    for (offset, method) in forbidden_methods.iter().enumerate() {
        requests.push(serde_json::json!({
            "jsonrpc": "2.0",
            "id": offset + 4,
            "method": method,
            "params": {}
        }));
    }

    let responses = roundtrip(fixture.path(), &requests);
    assert_eq!(
        responses.len(),
        requests.len(),
        "cada request con id responde"
    );

    let initialize = &responses[0];
    assert_eq!(initialize["error"], Value::Null);
    assert_eq!(initialize["result"]["protocolVersion"], LEGACY);
    let capabilities = initialize["result"]["capabilities"]
        .as_object()
        .expect("initialize devuelve capabilities como objeto");
    assert_eq!(
        capabilities.keys().collect::<Vec<_>>(),
        vec!["tools"],
        "initialize solo anuncia la capacidad tools"
    );
    assert!(capabilities["tools"].is_object());
    for legacy_only in ["resultType", "ttlMs", "cacheScope"] {
        assert!(
            initialize["result"].get(legacy_only).is_none(),
            "Legacy no anuncia el campo Modern {legacy_only}"
        );
    }

    let listed = responses[1]["result"]["tools"]
        .as_array()
        .expect("tools/list devuelve un array");
    let names: Vec<&str> = listed
        .iter()
        .map(|tool| tool["name"].as_str().expect("cada tool tiene name"))
        .collect();
    assert_eq!(names, TOOLS, "catálogo exacto en orden ratificado");
    assert_eq!(
        names.iter().collect::<BTreeSet<_>>().len(),
        TOOLS.len(),
        "el catálogo no contiene duplicados"
    );
    for tool in listed {
        assert_eq!(tool["inputSchema"]["type"], "object");
    }

    assert_eq!(
        responses[2]["error"]["code"], -32602,
        "MRTR/inputResponses no debe ignorarse ni ejecutar workspace_status: {}",
        responses[2]
    );
    assert!(responses[2]["result"].is_null());

    for (response, method) in responses[3..].iter().zip(forbidden_methods) {
        assert_eq!(
            response["error"]["code"], -32601,
            "método fuera de alcance {method} no debe aceptarse: {response}"
        );
        assert!(
            response["result"].is_null(),
            "método fuera de alcance {method} no devuelve result"
        );
    }
}
