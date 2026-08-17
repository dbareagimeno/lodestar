//! E34-H04 — contrato Modern MCP 2026-07-28 (fase roja).
//!
//! El proceso bajo prueba es siempre el binario real. Las requests modernas llevan la metadata
//! en `params._meta`; las aserciones de wire se complementan con los tipos oficiales de rmcp
//! 3.1.2 para que un objeto vacío o un campo colocado en una clave ad hoc no pueda pasar.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};

use lodestar_app::{App, Profile};
use lodestar_mcp::LodestarMcpService;
use rmcp::model::{
    CacheScope, CallToolResult, DiscoverResult, ListToolsResult, ProtocolVersion,
    ServerJsonRpcMessage, ServerResult,
};
use serde_json::{json, Value};

const MODERN: &str = "2026-07-28";
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
    std::fs::create_dir_all(path.parent().expect("fixture parent")).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn workspace_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("fixture tempdir");
    write_file(
        root.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: índice\n---\n\n# Bundle\n\n[Nota](note.md)\n",
    );
    write_file(
        root.path(),
        "note.md",
        "---\ntype: Note\ntitle: Nota\nestado: inicial\n---\n\n# Nota\n\ncontenido no vacío para Modern\n",
    );
    root
}

fn modern_meta(version: &str) -> Value {
    json!({
        "io.modelcontextprotocol/protocolVersion": version,
        "io.modelcontextprotocol/clientCapabilities": {},
        "io.modelcontextprotocol/clientInfo": {"name": "lodestar-e34-h04", "version": "1"}
    })
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn discover(id: u64, version: &str) -> Value {
    request(
        id,
        "server/discover",
        json!({"_meta": modern_meta(version)}),
    )
}

fn list(id: u64) -> Value {
    list_version(id, MODERN)
}

fn list_version(id: u64, version: &str) -> Value {
    request(id, "tools/list", json!({"_meta": modern_meta(version)}))
}

fn call(id: u64, name: &str, arguments: Value) -> Value {
    call_version(id, MODERN, name, arguments)
}

fn call_version(id: u64, version: &str, name: &str, arguments: Value) -> Value {
    request(
        id,
        "tools/call",
        json!({"_meta": modern_meta(version), "name": name, "arguments": arguments}),
    )
}

fn child(root: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("lodestar-mcp debe arrancar")
}

fn raw(root: &Path, input: &[u8]) -> Output {
    let mut process = child(root);
    process
        .stdin
        .take()
        .expect("stdin del binario")
        .write_all(input)
        .expect("escribir transcript exacto");
    process.wait_with_output().expect("esperar EOF limpio")
}

fn frames(output: &Output) -> Vec<Value> {
    assert!(
        output.status.success(),
        "binario terminó mal: {:?}",
        output.status
    );
    assert!(!output.stdout.is_empty(), "stdout no puede ser vacío");
    assert!(output.stdout.ends_with(b"\n"), "cada frame termina en LF");
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).expect("cada frame stdout es JSON válido"))
        .collect()
}

fn receipt_files(root: &Path) -> Vec<String> {
    let receipts = root.join(".lodestar/runtime/receipts");
    let Ok(entries) = std::fs::read_dir(receipts) else {
        return Vec::new();
    };
    let mut names = entries
        .map(|entry| entry.expect("entrada de recibo").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect::<Vec<_>>();
    names.sort();
    names
}

fn result(response: &Value) -> &Value {
    assert!(
        response["error"].is_null(),
        "respuesta moderna falló: {response}"
    );
    response["result"].as_object().expect("result es objeto");
    &response["result"]
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Session {
    fn new(root: &Path) -> Self {
        let mut child = child(root);
        let stdin = child.stdin.take().expect("stdin disponible");
        let stdout = BufReader::new(child.stdout.take().expect("stdout disponible"));
        Self {
            child,
            stdin,
            stdout,
        }
    }

    fn send(&mut self, value: Value) -> Value {
        assert_eq!(value["jsonrpc"], "2.0", "request debe ser JSON-RPC 2.0");
        let expected_id = value["id"].clone();
        assert!(!expected_id.is_null(), "request debe tener id");
        writeln!(self.stdin, "{value}").expect("request moderna escrita");
        self.stdin.flush().expect("request moderna flush");
        let mut line = String::new();
        self.stdout
            .read_line(&mut line)
            .expect("leer respuesta moderna");
        assert!(!line.is_empty(), "la request debe producir un frame");
        let response: Value =
            serde_json::from_str(line.trim_end()).expect("respuesta moderna JSON");
        assert_eq!(
            response["jsonrpc"], "2.0",
            "respuesta debe ser JSON-RPC 2.0"
        );
        assert_eq!(
            response["id"], expected_id,
            "respuesta conserva el id exacto"
        );
        response
    }

    fn finish(mut self) {
        drop(self.stdin);
        let status = self.child.wait().expect("EOF termina el proceso");
        assert!(
            status.success(),
            "EOF moderno termina limpiamente: {status}"
        );
    }
}

/// Catálogo canónico independiente del wire: lo produce directamente el servicio neutral sobre
/// `App`, no una segunda sesión de transporte (Legacy) que podría compartir el mismo defecto.
fn neutral_catalog(root: &Path) -> Value {
    let app = App::open(root).expect("fixture abre mediante App");
    let service = LodestarMcpService::new(app, Profile::Standard);
    service.list()["tools"].clone()
}

fn assert_modern_result_type(result: &Value) {
    assert_eq!(
        result["resultType"], "complete",
        "resultType moderno exacto"
    );
}

fn assert_cache_hints(result: &Value) {
    assert!(
        result["ttlMs"].is_u64(),
        "ttlMs moderno es entero no negativo"
    );
    assert_eq!(
        result["cacheScope"], "private",
        "cacheScope moderno conservador"
    );
}

#[test]
fn modern_discover() {
    let root = workspace_fixture();
    let mut session = Session::new(root.path());
    let response = session.send(discover(1, MODERN));
    let discovery = result(&response);
    assert_modern_result_type(discovery);
    assert_cache_hints(discovery);
    assert_eq!(discovery["supportedVersions"], json!([MODERN]));
    assert_eq!(
        discovery["capabilities"]
            .as_object()
            .unwrap()
            .keys()
            .collect::<Vec<_>>(),
        vec!["tools"]
    );
    assert!(discovery["capabilities"]["tools"].is_object());
    assert!(discovery["instructions"]
        .as_str()
        .is_some_and(|s| !s.trim().is_empty()));
    assert!(
        discovery["serverInfo"].is_null(),
        "serverInfo no puede ser campo ad hoc"
    );
    assert_eq!(
        discovery["_meta"]["io.modelcontextprotocol/serverInfo"]["name"],
        "lodestar-mcp"
    );
    let server_info = &discovery["_meta"]["io.modelcontextprotocol/serverInfo"];
    assert!(
        server_info["name"]
            .as_str()
            .is_some_and(|name| !name.trim().is_empty()),
        "serverInfo.name debe ser string no vacío"
    );
    assert!(
        server_info["version"]
            .as_str()
            .is_some_and(|version| !version.trim().is_empty()),
        "serverInfo.version debe ser string no vacío"
    );

    // Guard anti-vacuidad: discovery debe encadenar un listado real de las diez tools.
    let listed = session.send(list(2));
    let names = listed["result"]["tools"]
        .as_array()
        .expect("discover se valida con tools/list no vacío")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, TOOLS);
    session.finish();
}

#[test]
fn modern_metadata_faltante_es_32602_y_version_es_32022() {
    let root = workspace_fixture();
    let mut session = Session::new(root.path());

    // rmcp sólo puede observar estos rechazos por petición después de abrir stateless con discover.
    let opened = session.send(discover(1, MODERN));
    assert!(
        opened["error"].is_null(),
        "discover válido abre la era moderna: {opened}"
    );

    let missing_version = request(
        2,
        "tools/list",
        json!({"_meta": {
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {"name": "x", "version": "1"}
        }}),
    );
    let missing_caps = request(
        3,
        "tools/list",
        json!({"_meta": {
            "io.modelcontextprotocol/protocolVersion": MODERN,
            "io.modelcontextprotocol/clientInfo": {"name": "x", "version": "1"}
        }}),
    );
    let missing_client_info = request(
        4,
        "tools/list",
        json!({"_meta": {
            "io.modelcontextprotocol/protocolVersion": MODERN,
            "io.modelcontextprotocol/clientCapabilities": {}
        }}),
    );
    let null_values = request(
        5,
        "tools/list",
        json!({"_meta": {
            "io.modelcontextprotocol/protocolVersion": null,
            "io.modelcontextprotocol/clientCapabilities": null,
            "io.modelcontextprotocol/clientInfo": null
        }}),
    );
    let incomplete_client_info = request(
        6,
        "tools/list",
        json!({"_meta": {
            "io.modelcontextprotocol/protocolVersion": MODERN,
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {"name": "only-name"}
        }}),
    );
    let empty_client_info = request(
        7,
        "tools/list",
        json!({"_meta": {
            "io.modelcontextprotocol/protocolVersion": MODERN,
            "io.modelcontextprotocol/clientCapabilities": {},
            "io.modelcontextprotocol/clientInfo": {"name": "", "version": ""}
        }}),
    );
    let wrong_types = request(
        8,
        "tools/list",
        json!({"_meta": {
            "io.modelcontextprotocol/protocolVersion": 42,
            "io.modelcontextprotocol/clientCapabilities": [],
            "io.modelcontextprotocol/clientInfo": {"name": 7, "version": false}
        }}),
    );
    let outside_params = json!({
        "jsonrpc": "2.0", "id": 9, "method": "tools/list",
        "_meta": modern_meta(MODERN), "params": {}
    });
    for malformed in [
        missing_version,
        missing_caps,
        missing_client_info,
        null_values,
        incomplete_client_info,
        empty_client_info,
        wrong_types,
        outside_params,
    ] {
        let response = session.send(malformed);
        assert_eq!(
            response["error"]["code"], -32602,
            "metadata inválida: {response}"
        );
        assert!(response["result"].is_null());
    }

    // La misma identidad inválida también debe rechazarse en `server/discover`, no solo en
    // list/call. Cada request conserva su metadata en params._meta para evitar una validación
    // accidental contra estado de una petición anterior.
    let invalid_discovers = [
        request(
            10,
            "server/discover",
            json!({"_meta": {
                "io.modelcontextprotocol/protocolVersion": MODERN,
                "io.modelcontextprotocol/clientCapabilities": {}
            }}),
        ),
        request(
            11,
            "server/discover",
            json!({"_meta": {
                "io.modelcontextprotocol/protocolVersion": MODERN,
                "io.modelcontextprotocol/clientInfo": {"name": "x", "version": "1"}
            }}),
        ),
        request(
            12,
            "server/discover",
            json!({"_meta": {
                "io.modelcontextprotocol/protocolVersion": MODERN,
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {"name": "", "version": ""}
            }}),
        ),
        request(
            13,
            "server/discover",
            json!({"_meta": {
                "io.modelcontextprotocol/protocolVersion": MODERN,
                "io.modelcontextprotocol/clientCapabilities": [],
                "io.modelcontextprotocol/clientInfo": {"name": 7, "version": false}
            }}),
        ),
        json!({
            "jsonrpc": "2.0", "id": 14, "method": "server/discover",
            "_meta": modern_meta(MODERN), "params": {}
        }),
    ];
    for malformed in invalid_discovers {
        let response = session.send(malformed);
        assert_eq!(
            response["error"]["code"], -32602,
            "server/discover con clientInfo inválida: {response}"
        );
        assert!(response["result"].is_null());
    }

    for (id, version) in [(15, "2026-01-01"), (16, "2026-08-17")] {
        let unsupported = session.send(discover(id, version));
        assert_eq!(
            unsupported["error"]["code"], -32022,
            "versión no soportada en discover: {unsupported}"
        );
        assert_eq!(unsupported["error"]["data"]["requested"], version);
        assert_eq!(unsupported["error"]["data"]["supported"], json!([MODERN]));
        assert!(unsupported["result"].is_null());
    }

    // El rechazo por versión es por request: ni `tools/list` ni `tools/call` pueden reutilizar el
    // discover anterior ni caer a Legacy. Se cubren una fecha intermedia y una futura, incluyendo
    // sus datos exactos para que un -32602 genérico no pueda pasar.
    for (version, method) in [("2026-01-01", "intermedia"), ("2026-08-17", "futura")] {
        let unsupported_list = session.send(list_version(20, version));
        assert_eq!(
            unsupported_list["error"]["code"], -32022,
            "versión {method} rechazada en tools/list: {unsupported_list}"
        );
        assert_eq!(unsupported_list["error"]["data"]["requested"], version);
        assert_eq!(
            unsupported_list["error"]["data"]["supported"],
            json!([MODERN])
        );
        assert!(unsupported_list["result"].is_null());

        let unsupported_call = session.send(call_version(
            21,
            version,
            "knowledge_search",
            json!({"text": "contenido no vacío"}),
        ));
        assert_eq!(
            unsupported_call["error"]["code"], -32022,
            "versión {method} rechazada en tools/call: {unsupported_call}"
        );
        assert_eq!(unsupported_call["error"]["data"]["requested"], version);
        assert_eq!(
            unsupported_call["error"]["data"]["supported"],
            json!([MODERN])
        );
        assert!(unsupported_call["result"].is_null());
    }
    session.finish();
}

#[test]
fn modern_tools_list_y_call() {
    let root = workspace_fixture();
    let mut session = Session::new(root.path());

    // Apertura stateless directa: la primera request válida no es discover, y aun así debe poder
    // listar y llamar tools. Esto evita que el servidor dependa de estado latente del discovery.
    let listed = session.send(list(2));
    let listed_result = result(&listed);
    assert_modern_result_type(listed_result);
    assert_cache_hints(listed_result);
    let names = listed_result["tools"].as_array().unwrap();
    assert_eq!(names.len(), TOOLS.len());
    for (tool, expected) in names.iter().zip(TOOLS) {
        assert_eq!(tool["name"], expected);
        assert!(tool["inputSchema"].is_object());
        assert!(tool["outputSchema"].is_object());
    }

    let read = session.send(call(
        3,
        "knowledge_search",
        json!({"text": "contenido no vacío"}),
    ));
    let read_result = result(&read);
    assert_modern_result_type(read_result);
    assert!(read_result["structuredContent"]["results"]
        .as_array()
        .is_some_and(|r| !r.is_empty()));

    let before_apply = std::fs::read(root.path().join("note.md")).unwrap();
    let created_path = root.path().join("modern-created.md");
    let created_before = std::fs::read(&created_path).ok();
    let plan = session.send(call(
        4,
        "change_plan",
        json!({
            "operations": [{"op": "create", "path": "modern-created.md", "body": "# Modern\n"}],
            "policy": {"requireValidResult": false, "allowWarnings": true}
        }),
    ));
    let change_set_id = result(&plan)["structuredContent"]["changeSetId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .expect("change_plan devuelve id real")
        .to_owned();

    // Un plan real no autoriza a saltarse la validación de metadata en `change_apply`: los
    // rechazos deben ocurrir antes del escritor, conservar los bytes y no producir receipts.
    let receipt_files_before_rejections = receipt_files(root.path());
    let missing_meta = request(
        5,
        "tools/call",
        json!({"name": "change_apply", "arguments": {"changeSetId": change_set_id.clone()}}),
    );
    let malformed_meta = request(
        6,
        "tools/call",
        json!({
            "_meta": {
                "io.modelcontextprotocol/protocolVersion": MODERN,
                "io.modelcontextprotocol/clientCapabilities": {},
                "io.modelcontextprotocol/clientInfo": {"name": "", "version": 7}
            },
            "name": "change_apply",
            "arguments": {"changeSetId": change_set_id.clone()}
        }),
    );
    let unsupported_meta = call_version(
        7,
        "2026-08-17",
        "change_apply",
        json!({"changeSetId": change_set_id.clone()}),
    );
    for (invalid, code) in [
        (missing_meta, -32602),
        (malformed_meta, -32602),
        (unsupported_meta, -32022),
    ] {
        let response = session.send(invalid);
        assert_eq!(
            response["error"]["code"], code,
            "change_apply inválido: {response}"
        );
        assert!(response["result"].is_null());
        assert_eq!(
            std::fs::read(root.path().join("note.md")).unwrap(),
            before_apply,
            "change_apply rechazado no puede escribir documentos"
        );
        assert_eq!(
            std::fs::read(&created_path).ok(),
            created_before,
            "change_apply rechazado no puede crear ni modificar el .md planificado"
        );
        assert_eq!(
            receipt_files(root.path()),
            receipt_files_before_rejections,
            "change_apply rechazado no puede crear receipts"
        );
    }

    let applied = session.send(call(
        8,
        "change_apply",
        json!({"changeSetId": change_set_id}),
    ));
    let applied_result = result(&applied);
    assert_modern_result_type(applied_result);
    assert!(applied_result["structuredContent"].is_object());
    session.finish();
    assert_eq!(
        std::fs::read_to_string(created_path).unwrap(),
        "# Modern\n",
        "la llamada de cambio debe tener efecto real en disco"
    );
}

#[test]
fn modern_result_type_y_cache_hints() {
    let root = workspace_fixture();
    let mut session = Session::new(root.path());
    let discovery = session.send(discover(1, MODERN));
    let list_response = session.send(list(2));
    let call_response = session.send(call(
        3,
        "knowledge_get",
        json!({"ref": {"path": "note.md"}}),
    ));

    let typed_discovery: DiscoverResult = serde_json::from_value(discovery["result"].clone())
        .expect("discover deserializa con tipo oficial rmcp");
    assert!(typed_discovery.result_type.is_complete());
    assert_eq!(
        typed_discovery.supported_versions,
        vec![ProtocolVersion::V_2026_07_28]
    );
    assert_eq!(typed_discovery.ttl_ms, 0);
    assert_eq!(typed_discovery.cache_scope, CacheScope::Private);
    assert!(
        typed_discovery.server_info().is_some(),
        "serverInfo vive en metadata reservada"
    );

    let typed_list: ListToolsResult = serde_json::from_value(list_response["result"].clone())
        .expect("tools/list deserializa con tipo oficial rmcp");
    assert_eq!(
        typed_list.result_type.as_ref().unwrap().as_str(),
        "complete"
    );
    assert_eq!(typed_list.ttl_ms, Some(0));
    assert_eq!(typed_list.cache_scope, Some(CacheScope::Private));
    assert!(
        !typed_list.tools.is_empty(),
        "la lista tipada no puede ser placeholder vacío"
    );
    assert_modern_result_type(&list_response["result"]);
    assert_cache_hints(&list_response["result"]);

    let typed_call: CallToolResult = serde_json::from_value(call_response["result"].clone())
        .expect("tools/call deserializa con tipo oficial rmcp");
    assert_eq!(
        typed_call.result_type.as_ref().unwrap().as_str(),
        "complete"
    );
    assert!(typed_call.structured_content.is_some());
    // Anti-vacuidad: las guardas de wire se ejecutan sobre cada resultado moderno; quitar uno de
    // estos campos hace fallar directamente la aserción correspondiente, aunque rmcp tolere el
    // frame como legado al deserializarlo.
    assert_modern_result_type(&call_response["result"]);
    session.finish();
}

/// H04-C4 — llamadas Modern equivalentes son estables y ninguna respuesta de tool queda servida
/// desde una cache mutable del workspace: se repite el mismo listado byte-semántico y luego se
/// fuerza una publicación real antes de repetir una búsqueda cuyo resultado debe cambiar.
#[test]
fn modern_llamadas_equivalentes_estables_sin_cache_mutable_del_workspace() {
    let root = workspace_fixture();
    let mut session = Session::new(root.path());

    let first_list = session.send(list(1));
    let second_list = session.send(list(2));
    let first_list_result = result(&first_list).clone();
    let second_list_result = result(&second_list).clone();
    assert_eq!(first_list_result, second_list_result);
    assert_modern_result_type(&first_list_result);
    assert_cache_hints(&first_list_result);
    assert_eq!(first_list_result["ttlMs"], 0);
    assert_eq!(first_list_result["cacheScope"], "private");
    assert_eq!(
        first_list_result["tools"].as_array().map(Vec::len),
        Some(TOOLS.len()),
        "la estabilidad no puede aprobar con un catálogo vacío"
    );

    let before = session.send(call(
        3,
        "knowledge_search",
        json!({"text": "modern-cache-marker"}),
    ));
    let before_result = result(&before);
    assert_modern_result_type(before_result);
    assert!(before_result["structuredContent"]["results"]
        .as_array()
        .is_some_and(|results| results.is_empty()));

    let plan = session.send(call(
        4,
        "change_plan",
        json!({
            "operations": [{
                "op": "patch_frontmatter",
                "ref": {"path": "note.md"},
                "patch": {"estado": "modern-cache-marker"}
            }],
            "policy": {"requireValidResult": false, "allowWarnings": true}
        }),
    ));
    let change_set_id = result(&plan)["structuredContent"]["changeSetId"]
        .as_str()
        .filter(|id| !id.is_empty())
        .expect("el plan de control debe ser real")
        .to_owned();
    let applied = session.send(call(
        5,
        "change_apply",
        json!({"changeSetId": change_set_id}),
    ));
    assert_eq!(result(&applied)["structuredContent"]["applied"], true);

    // La primera búsqueda fue deliberadamente vacía y la publicación introduce el término. Si
    // existiera una cache mutable del workspace, la llamada equivalente devolvería ese vacío viejo.
    let after = session.send(call(
        6,
        "knowledge_search",
        json!({"text": "modern-cache-marker"}),
    ));
    let after_result = result(&after).clone();
    assert_modern_result_type(&after_result);
    let after_results = after_result["structuredContent"]["results"]
        .as_array()
        .expect("knowledge_search debe devolver results");
    assert!(
        !after_results.is_empty(),
        "la búsqueda debe observar la publicación real"
    );
    assert!(after_results.iter().any(|item| item["path"] == "note.md"));

    let equivalent_after = session.send(call(
        7,
        "knowledge_search",
        json!({"text": "modern-cache-marker"}),
    ));
    assert_eq!(
        after_result,
        result(&equivalent_after).clone(),
        "llamadas equivalentes sobre el mismo estado deben ser estables"
    );
    session.finish();
}

#[test]
fn modern_ping_method_not_found() {
    let root = workspace_fixture();
    let mut session = Session::new(root.path());
    assert!(session.send(discover(1, MODERN))["error"].is_null());
    let response = session.send(request(2, "ping", json!({"_meta": modern_meta(MODERN)})));
    assert_eq!(response["error"]["code"], -32601);
    assert!(
        response["result"].is_null(),
        "ping moderno no devuelve result: {{}} "
    );
    session.finish();
}

#[test]
fn modern_schema_oficial() {
    let root = workspace_fixture();
    let canonical_tools = neutral_catalog(root.path());

    // La sesión Modern se compara contra el servicio neutral canónico, no contra otra sesión de
    // wire Legacy que podría compartir el mismo defecto de catálogo o serialización.
    let mut session = Session::new(root.path());
    let _ = session.send(discover(1, MODERN));
    let list_response = session.send(list(2));
    let typed: ServerJsonRpcMessage = serde_json::from_value(list_response.clone())
        .expect("el frame tools/list debe usar el envelope oficial rmcp");
    let ServerJsonRpcMessage::Response(response) = typed else {
        panic!("tools/list no es response")
    };
    let ServerResult::ListToolsResult(typed_list) = response.result else {
        panic!("tools/list no deserializa a ListToolsResult oficial");
    };
    assert_eq!(typed_list.tools.len(), 10);
    assert_eq!(
        typed_list
            .tools
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect::<Vec<_>>(),
        TOOLS
    );
    let modern_tools = list_response["result"]["tools"]
        .as_array()
        .expect("Modern tools/list debe devolver array")
        .clone();
    let canonical_tools_array = canonical_tools
        .as_array()
        .expect("catálogo neutral canónico es un array");
    assert_eq!(
        modern_tools.as_slice(),
        canonical_tools_array.as_slice(),
        "Modern debe coincidir completamente con el catálogo neutral canónico"
    );
    for (tool, canonical) in typed_list
        .tools
        .into_iter()
        .zip(canonical_tools_array.iter())
    {
        let input = serde_json::to_value(&tool.input_schema).unwrap();
        let output =
            serde_json::to_value(tool.output_schema.expect("outputSchema oficial")).unwrap();
        assert_eq!(tool.name, canonical["name"].as_str().unwrap());
        assert_eq!(
            tool.description.as_deref(),
            canonical["description"].as_str(),
            "description exacta para {}",
            tool.name
        );
        assert_eq!(input, canonical["inputSchema"]);
        assert_eq!(output, canonical["outputSchema"]);
        assert_eq!(
            input["type"], "object",
            "input schema objeto para {}",
            tool.name
        );
        assert_eq!(
            output["type"], "object",
            "output schema objeto para {}",
            tool.name
        );
        assert!(
            input.get("properties").is_some(),
            "input schema no vacío para {}",
            tool.name
        );
    }
    session.finish();
}

#[test]
fn modern_metadata_inspect_schema_esquema_app_exacto() {
    let root = workspace_fixture();
    let mut session = Session::new(root.path());
    let _ = session.send(discover(1, MODERN));
    let listed = session.send(list(2));
    let served = listed["result"]["tools"]
        .as_array()
        .expect("Modern tools/list debe devolver tools")
        .iter()
        .find(|tool| tool["name"] == "metadata_inspect")
        .and_then(|tool| tool.get("outputSchema"))
        .cloned()
        .expect("metadata_inspect debe anunciar outputSchema");

    // El schema público se deriva en lodestar-app. Esta comparación independiente evita que la
    // fachada fabrique un `properties: {}` artificial y luego se auto-valide contra su catálogo.
    assert_eq!(
        served,
        lodestar_app::schemas::metadata_inspect_schema(),
        "metadata_inspect debe servir exactamente el outputSchema vigente de App"
    );
    session.finish();
}

#[test]
fn harness_raw_modern_exacto() {
    let root = workspace_fixture();
    let transcript = format!(
        "{}\n{}\n{}\n{}\n{}\n{}\n",
        serde_json::to_string(&discover(1, MODERN)).unwrap(),
        serde_json::to_string(&list(2)).unwrap(),
        r#"{"jsonrpc":"2.0","method":"notifications/initialized","params":{"_meta":{"io.modelcontextprotocol/protocolVersion":"2026-07-28","io.modelcontextprotocol/clientCapabilities":{}}}}"#,
        serde_json::to_string(&request(
            3,
            "initialize",
            json!({
                "protocolVersion": MODERN,
                "capabilities": {},
                "clientInfo": {"name": "legacy-shaped-modern", "version": "1"},
                "_meta": modern_meta(MODERN)
            })
        ))
        .unwrap(),
        serde_json::to_string(&request(
            4,
            "resources/list",
            json!({"_meta": modern_meta(MODERN)})
        ))
        .unwrap(),
        serde_json::to_string(&call(5, "tool-that-does-not-exist", json!({}))).unwrap(),
    );
    let output = raw(root.path(), transcript.as_bytes());
    let frames = frames(&output);
    assert_eq!(frames.len(), 5, "la notificación moderna no genera frame");
    let ids = frames
        .iter()
        .map(|frame| frame["id"].as_u64().expect("id numérico"))
        .collect::<Vec<_>>();
    assert_eq!(
        ids.len(),
        5,
        "el harness raw conserva exactamente cinco respuestas con id"
    );
    let unique_ids = ids.iter().copied().collect::<BTreeSet<_>>();
    assert_eq!(
        unique_ids,
        BTreeSet::from([1, 2, 3, 4, 5]),
        "el harness raw conserva el conjunto exacto de ids sin duplicados"
    );
    let frame_for = |id| {
        frames
            .iter()
            .find(|frame| frame["id"].as_u64() == Some(id))
            .unwrap_or_else(|| panic!("falta frame con id {id}"))
    };
    let discover_frame = frame_for(1);
    let list_frame = frame_for(2);
    let initialize_frame = frame_for(3);
    let resources_frame = frame_for(4);
    let unknown_tool_frame = frame_for(5);
    assert!(
        discover_frame["result"].is_object(),
        "discover raw exacto devuelve result"
    );
    assert!(list_frame["result"]["tools"]
        .as_array()
        .is_some_and(|tools| tools.len() == 10));
    assert_eq!(
        initialize_frame["error"]["code"], -32601,
        "initialize moderno prohibido"
    );
    assert_eq!(
        resources_frame["error"]["code"], -32601,
        "resources/list no soportado"
    );
    assert_eq!(
        unknown_tool_frame["error"]["code"], -32602,
        "tool desconocida conserva código"
    );
    for frame in frames {
        assert_eq!(frame["jsonrpc"], "2.0");
        assert!(frame.get("result").is_some() ^ frame.get("error").is_some());
    }
}
