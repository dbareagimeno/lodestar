//! E34-H02 — servicio neutral MCP.
//!
//! Estos tests no arrancan un transporte para ejercitar el servicio: construyen el servicio con
//! el `App` real y comparan su resultado estructurado con la fachada stdio. El fixture es neutral
//! (sin configuración de perfil), de modo que las diferencias entre standard y readonly se
//! limitan al gating explícito de cambios y a las capacidades que ese gating anuncia.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use lodestar_app::{App, Profile};
use lodestar_mcp::LodestarMcpService;
use serde_json::{json, Value};

const STANDARD_TOOLS: [&str; 10] = [
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

const READONLY_TOOLS: [&str; 7] = [
    "workspace_status",
    "knowledge_search",
    "knowledge_get",
    "metadata_inspect",
    "knowledge_check",
    "graph_query",
    "impact_analyze",
];
const INJECTED_INITIALIZE_ID: &str = "__lodestar_h02_initialize__";

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("fixture path has a parent")).unwrap();
    std::fs::write(path, contents).unwrap();
}

/// Fixture no vacío: una lectura debe producir un hit y el apply debe materializar un fichero.
fn workspace_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().unwrap();
    write_file(
        root.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: índice\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n[Nota](note.md)\n",
    );
    write_file(
        root.path(),
        "note.md",
        "---\ntype: Note\ntitle: Nota\ndescription: documento de prueba\n---\n\n# Nota\n\ncontenido no vacío para paridad\n",
    );
    root
}

fn service(profile: Profile) -> (tempfile::TempDir, LodestarMcpService) {
    let root = workspace_fixture();
    let app = App::open(root.path()).expect("fixture must open through App");
    (root, LodestarMcpService::new(app, profile))
}

fn facade_roundtrip(root: &Path, profile: &str, requests: &[Value]) -> Vec<Value> {
    let has_initialize = requests
        .iter()
        .any(|request| request["method"] == "initialize");
    let mut transcript = Vec::new();
    if !has_initialize {
        transcript.push(json!({
            "jsonrpc": "2.0",
            "id": INJECTED_INITIALIZE_ID,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "e34-h02", "version": "1"}
            }
        }));
        transcript.push(json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));
    }
    transcript.extend(requests.iter().cloned());

    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(root)
        .arg("--profile")
        .arg(profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("lodestar-mcp facade must start");
    let mut stdin = child.stdin.take().unwrap();
    for request in &transcript {
        writeln!(stdin, "{}", serde_json::to_string(request).unwrap()).unwrap();
    }
    drop(stdin);
    let stdout = BufReader::new(child.stdout.take().unwrap());
    let responses: Vec<Value> = stdout
        .lines()
        .map(|line| serde_json::from_str(&line.expect("stdout is readable JSON")))
        .collect::<Result<_, _>>()
        .expect("facade emits JSON-RPC values");
    let status = child.wait().expect("facade terminates");
    assert!(status.success(), "facade exits successfully: {status}");
    responses
        .into_iter()
        .filter(|response: &Value| {
            !(!has_initialize
                && response["id"] == INJECTED_INITIALIZE_ID
                && response["error"].is_null()
                && response["result"]["protocolVersion"] == "2025-11-25")
        })
        .collect()
}

fn request_call(name: &str, arguments: Value, id: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": name, "arguments": arguments }
    })
}

fn names(list_result: &Value) -> Vec<String> {
    list_result["tools"]
        .as_array()
        .expect("service list returns {tools: [...]} (not an empty placeholder)")
        .iter()
        .map(|tool| {
            tool["name"]
                .as_str()
                .expect("every catalog entry has a name")
                .to_owned()
        })
        .collect()
}

fn assert_unique_and_exact(list_result: &Value, expected: &[&str]) {
    let actual = names(list_result);
    assert_eq!(
        actual.len(),
        expected.len(),
        "catalogue must be non-empty and exact"
    );
    let unique: BTreeSet<&str> = actual.iter().map(String::as_str).collect();
    assert_eq!(
        unique.len(),
        actual.len(),
        "catalogue contains duplicate names: {actual:?}"
    );
    assert_eq!(unique, expected.iter().copied().collect());
}

fn normalize_workspace_status_capabilities(result: &mut Value) {
    for field in ["writes", "transactions", "revert"] {
        result["structuredContent"]["capabilities"][field] = Value::Null;
    }

    let text = result["content"][0]["text"]
        .as_str()
        .expect("workspace_status content contains JSON text");
    let mut text_payload: Value = serde_json::from_str(text)
        .expect("workspace_status content text is the structured JSON payload");
    for field in ["writes", "transactions", "revert"] {
        text_payload["capabilities"][field] = Value::Null;
    }
    result["content"][0]["text"] = Value::String(text_payload.to_string());
}

/// C1 — tools/list y tools/call del servicio neutral son semánticamente iguales a la fachada
/// estándar. Incluye tool desconocida, argumentos inválidos y un error de dominio reconocible.
#[test]
fn service_paridad_standard_readonly() {
    let (root, mut neutral) = service(Profile::Standard);
    let listed = neutral.list();
    let facade = facade_roundtrip(
        root.path(),
        "standard",
        &[json!({ "jsonrpc": "2.0", "id": 1, "method": "tools/list" })],
    );
    assert_eq!(
        listed, facade[0]["result"],
        "neutral list must match stdio facade"
    );
    assert_unique_and_exact(&listed, &STANDARD_TOOLS);

    let request = request_call(
        "knowledge_search",
        json!({ "text": "contenido no vacío" }),
        2,
    );
    let facade = facade_roundtrip(root.path(), "standard", std::slice::from_ref(&request));
    let actual = neutral
        .call("knowledge_search", &json!({ "text": "contenido no vacío" }))
        .expect("valid neutral call returns structured result");
    assert_eq!(
        actual, facade[0]["result"],
        "neutral call must preserve MCP envelope"
    );
    assert!(
        actual["structuredContent"].is_object(),
        "success is not an empty placeholder"
    );
    assert!(!actual["structuredContent"]["results"]
        .as_array()
        .expect("knowledge_search structuredContent contains results array")
        .is_empty());

    // Tool desconocida: el código de protocolo no puede convertirse en éxito vacío.
    let unknown_request = request_call("legacy_query", json!({}), 3);
    let facade = facade_roundtrip(
        root.path(),
        "standard",
        std::slice::from_ref(&unknown_request),
    );
    let unknown = neutral.call("legacy_query", &json!({}));
    assert!(
        unknown.is_err(),
        "unknown tool must be rejected before dispatch"
    );
    assert_eq!(facade[0]["error"]["code"], json!(-32602));
    assert!(
        facade[0]["result"].is_null(),
        "unknown tool has no success result"
    );

    // Argumentos inválidos: el error de validación conserva el resultado MCP de tool.
    let invalid_request = request_call(
        "knowledge_search",
        json!({ "text": "contenido", "typoNoPermitido": true }),
        4,
    );
    let facade = facade_roundtrip(
        root.path(),
        "standard",
        std::slice::from_ref(&invalid_request),
    );
    let invalid = neutral
        .call(
            "knowledge_search",
            &json!({ "text": "contenido", "typoNoPermitido": true }),
        )
        .expect("invalid tool arguments remain a structured tool error");
    assert_eq!(invalid, facade[0]["result"]);
    assert_eq!(invalid["isError"], true);
    assert!(invalid["content"][0]["text"]
        .as_str()
        .is_some_and(|s: &str| !s.trim().is_empty()));

    // Error de dominio (ruta fuera del workspace): no puede mutar a éxito vacío ni perder el mensaje.
    let domain_request = request_call(
        "knowledge_get",
        json!({ "ref": { "path": "../fuera.md" } }),
        5,
    );
    let facade = facade_roundtrip(
        root.path(),
        "standard",
        std::slice::from_ref(&domain_request),
    );
    let domain = neutral
        .call(
            "knowledge_get",
            &json!({ "ref": { "path": "../fuera.md" } }),
        )
        .expect("domain failures remain structured tool errors");
    assert_eq!(domain, facade[0]["result"]);
    assert_eq!(domain["isError"], true);
    assert!(domain["content"][0]["text"]
        .as_str()
        .is_some_and(|s: &str| !s.trim().is_empty()));
}

/// C2 — las siete lecturas son la misma semántica en ambos perfiles; las tres de cambio sólo
/// existen en standard. La lectura devuelve un resultado real y change_apply materializa disco.
#[test]
fn service_golden_cross_fachada_no_vacia() {
    // Ambos perfiles apuntan al mismo workspace para que `workspace_status.root` y la revisión
    // sean comparables; cada servicio conserva su propio `App`, sin compartir estado mutable.
    let standard_root = workspace_fixture();
    let standard_app = App::open(standard_root.path()).expect("standard App opens fixture");
    let readonly_app = App::open(standard_root.path()).expect("readonly App opens fixture");
    let mut standard = LodestarMcpService::new(standard_app, Profile::Standard);
    let mut readonly = LodestarMcpService::new(readonly_app, Profile::Readonly);
    assert_unique_and_exact(&standard.list(), &STANDARD_TOOLS);
    assert_unique_and_exact(&readonly.list(), &READONLY_TOOLS);

    let reads = [
        ("workspace_status", json!({})),
        ("knowledge_search", json!({ "text": "contenido no vacío" })),
        ("knowledge_get", json!({ "ref": { "path": "note.md" } })),
        ("metadata_inspect", json!({ "mode": "catalog" })),
        (
            "knowledge_check",
            json!({ "scope": { "kind": "workspace" } }),
        ),
        ("graph_query", json!({ "operation": "isolated" })),
        (
            "impact_analyze",
            json!({ "ref": { "path": "note.md" }, "proposedOperation": { "kind": "move" } }),
        ),
    ];
    for (name, args) in reads {
        let standard_result = standard
            .call(name, &args)
            .unwrap_or_else(|e| panic!("standard {name} must execute: {e:?}"));
        let readonly_result = readonly
            .call(name, &args)
            .unwrap_or_else(|e| panic!("readonly {name} must execute: {e:?}"));
        assert!(standard_result["structuredContent"].is_object());
        assert!(readonly_result["structuredContent"].is_object());
        if name == "workspace_status" {
            // El estado conserva las tres capacidades por perfil; el resto del resultado debe ser
            // idéntico en structuredContent y en el JSON serializado de content[0].text.
            for field in ["writes", "transactions", "revert"] {
                assert_eq!(
                    standard_result["structuredContent"]["capabilities"][field], true,
                    "standard workspace_status must enable {field}"
                );
                assert_eq!(
                    readonly_result["structuredContent"]["capabilities"][field], false,
                    "readonly workspace_status must disable {field}"
                );
            }
            let mut standard_comparable = standard_result.clone();
            let mut readonly_comparable = readonly_result.clone();
            normalize_workspace_status_capabilities(&mut standard_comparable);
            normalize_workspace_status_capabilities(&mut readonly_comparable);
            assert_eq!(standard_comparable, readonly_comparable);
        } else if name == "knowledge_search" {
            let standard_results = standard_result["structuredContent"]["results"]
                .as_array()
                .expect("knowledge_search structuredContent contains results array");
            let readonly_results = readonly_result["structuredContent"]["results"]
                .as_array()
                .expect("knowledge_search structuredContent contains results array");
            assert!(
                !standard_results.is_empty(),
                "standard search must be non-empty"
            );
            assert!(
                !readonly_results.is_empty(),
                "readonly search must be non-empty"
            );
            assert_eq!(
                standard_result, readonly_result,
                "knowledge_search must be exactly equal across profiles on the neutral fixture"
            );
        } else {
            assert_eq!(standard_result, readonly_result, "read parity for {name}");
        }
    }

    // Ocultar una tool de `tools/list` no basta: una invocación directa debe alcanzar el mismo
    // rechazo de protocolo vigente (-32602) y nunca ejecutar una operación de cambio.
    let readonly_change_calls = [
        (
            "change_plan",
            json!({
                "operations": [{
                    "op": "create",
                    "path": "readonly-must-not-exist.md",
                    "body": "# no debe escribirse\n"
                }],
                "policy": { "requireValidResult": false, "allowWarnings": true }
            }),
        ),
        (
            "change_apply",
            json!({ "changeSetId": "changeset:readonly-never" }),
        ),
        (
            "change_revert",
            json!({ "receiptId": "receipt:readonly-never" }),
        ),
    ];
    for (offset, (name, args)) in readonly_change_calls.iter().enumerate() {
        let direct = readonly.call(name, args);
        let direct_error =
            direct.expect_err("readonly direct change invocation must be rejected before dispatch");
        assert!(
            direct_error.contains(name),
            "direct readonly rejection should identify the unavailable tool: {direct_error}"
        );

        let request = request_call(name, args.clone(), 20 + offset as u64);
        let response = facade_roundtrip(
            standard_root.path(),
            "readonly",
            std::slice::from_ref(&request),
        )
        .pop()
        .expect("readonly facade returns one rejection response");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(response["error"]["code"], -32602);
        assert!(
            response["result"].is_null(),
            "profile gating is a protocol error, not a successful or tool-error result: {response}"
        );
        assert!(
            response["error"]["message"]
                .as_str()
                .is_some_and(|message| message.contains(name)),
            "readonly rejection should identify the unavailable tool: {response}"
        );
    }
    assert!(
        !standard_root
            .path()
            .join("readonly-must-not-exist.md")
            .exists(),
        "readonly direct calls must not materialize a change"
    );

    // Guard anti-vacuidad de escritura real: planificar no es aplicar, y apply deja el fichero.
    let plan = standard
        .call(
            "change_plan",
            &json!({
                "operations": [{ "op": "create", "path": "created-by-service.md", "body": "# creado\n" }],
                "policy": { "requireValidResult": false, "allowWarnings": true }
            }),
        )
        .expect("standard can plan a real change");
    let change_set_id = plan["structuredContent"]["changeSetId"]
        .as_str()
        .filter(|id: &&str| !id.is_empty())
        .expect("plan is not an empty success");
    assert!(standard
        .call("change_apply", &json!({ "changeSetId": change_set_id }))
        .expect("standard can apply the planned change")["structuredContent"]
        .is_object());
    assert_eq!(
        std::fs::read_to_string(standard_root.path().join("created-by-service.md"))
            .expect("change_apply must materialize the requested file"),
        "# creado\n",
        "the write guard must observe a real canonical-disk effect"
    );
}

/// C3 — el registro es cerrado y único: no hay alias legacy ni dispatcher alternativo por perfil.
#[test]
fn service_catalogo_dispatcher_sin_duplicados() {
    let (_standard_root, mut standard) = service(Profile::Standard);
    let (_readonly_root, mut readonly) = service(Profile::Readonly);
    let standard_names = names(&standard.list());
    let readonly_names = names(&readonly.list());
    assert_eq!(
        standard_names.iter().collect::<BTreeSet<_>>().len(),
        STANDARD_TOOLS.len(),
        "every standard name is registered once"
    );
    assert_eq!(
        readonly_names.iter().collect::<BTreeSet<_>>().len(),
        READONLY_TOOLS.len(),
        "every readonly name is registered once"
    );
    assert_eq!(
        standard_names
            .iter()
            .filter(|name| !readonly_names.contains(name))
            .cloned()
            .collect::<BTreeSet<_>>(),
        ["change_plan", "change_apply", "change_revert"]
            .into_iter()
            .map(String::from)
            .collect(),
        "profile filters exactly the three change tools"
    );

    // Lifecycle is owned by the same neutral service as catalog and dispatch. Keep these checks
    // structural: C2 already owns the exact profile catalog assertions.
    for (label, service) in [("standard", &standard), ("readonly", &readonly)] {
        let discovery = service.discover();
        assert!(discovery.is_object(), "{label} discovery is structured");
        assert!(
            discovery["capabilities"]["tools"].is_object(),
            "{label} discovery advertises the tools capability"
        );
        assert!(
            discovery["tools"]
                .as_array()
                .is_some_and(|tools| !tools.is_empty()),
            "{label} discovery must expose a non-empty tool surface"
        );
        assert_eq!(
            discovery["tools"],
            service.list()["tools"],
            "{label} discovery and list must expose exactly the same tool catalog"
        );
        assert!(
            discovery["instructions"]
                .as_str()
                .is_some_and(|instructions| !instructions.trim().is_empty()),
            "{label} discovery must carry actionable instructions"
        );
        assert_eq!(
            service.ping(),
            json!({}),
            "{label} ping is the neutral empty result"
        );
    }

    for alias in [
        "query",
        "legacy_query",
        "initialize",
        "transport/legacy/tools/list",
    ] {
        assert!(
            standard.call(alias, &json!({})).is_err(),
            "alias leaked into dispatcher: {alias}"
        );
        assert!(
            readonly.call(alias, &json!({})).is_err(),
            "alias leaked into readonly dispatcher: {alias}"
        );
    }
    // Shared read handler is profile-independent; only the explicit capability filter may differ.
    let standard_call = standard
        .call("knowledge_search", &json!({ "text": "contenido" }))
        .unwrap();
    let readonly_call = readonly
        .call("knowledge_search", &json!({ "text": "contenido" }))
        .unwrap();
    assert_eq!(
        standard_call, readonly_call,
        "shared read name must use one dispatcher path"
    );
}
