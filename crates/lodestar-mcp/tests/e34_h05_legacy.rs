//! E34-H05 — baseline Legacy MCP 2025-11-25 (fase roja).
//!
//! El único oráculo de catálogo y semántica es LodestarMcpService. El proceso real se arranca
//! siempre con stdout y stderr separados: un cliente Legacy no puede depender de logs ni de un
//! bucle privado de framing.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Output, Stdio};

use lodestar_app::{App, Profile};
use lodestar_mcp::LodestarMcpService;
use serde_json::{json, Value};

const LEGACY: &str = "2025-11-25";
const TOOLS_STANDARD: [&str; 10] = [
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
const TOOLS_READONLY: [&str; 7] = [
    "workspace_status",
    "knowledge_search",
    "knowledge_get",
    "metadata_inspect",
    "knowledge_check",
    "graph_query",
    "impact_analyze",
];

fn write_file(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    std::fs::create_dir_all(path.parent().expect("fixture path has a parent")).unwrap();
    std::fs::write(path, contents).unwrap();
}

fn workspace_fixture() -> tempfile::TempDir {
    let root = tempfile::tempdir().expect("tempdir");
    write_file(
        root.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\n---\n\n# Bundle\n\n[Nota](note.md)\n",
    );
    write_file(
        root.path(),
        "note.md",
        "---\ntype: Note\ntitle: Nota\nestado: inicial\n---\n\n# Nota\n\ncontenido Legacy no vacío\n",
    );
    root
}

fn initialize(id: u64, offered: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "initialize",
        "params": {
            "protocolVersion": offered,
            "capabilities": {},
            "clientInfo": {"name": "e34-h05-legacy", "version": "1"}
        }
    })
}

fn initialized() -> Value {
    json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
}

fn request(id: u64, method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn ping(id: u64) -> Value {
    request(id, "ping", json!({}))
}

fn discover(id: u64) -> Value {
    request(id, "server/discover", json!({}))
}

fn list(id: u64) -> Value {
    request(id, "tools/list", json!({}))
}

fn call(id: u64, name: &str, arguments: Value) -> Value {
    request(
        id,
        "tools/call",
        json!({"name": name, "arguments": arguments}),
    )
}

fn spawn(root: &Path, profile: &str) -> Child {
    Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(root)
        .arg("--profile")
        .arg(profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lodestar-mcp debe arrancar")
}

struct Session {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
}

impl Session {
    fn new(root: &Path, profile: &str) -> Self {
        let mut child = spawn(root, profile);
        let stdin = child.stdin.take().expect("stdin del servidor");
        let stdout = BufReader::new(child.stdout.take().expect("stdout del servidor"));
        let stderr = child.stderr.take().expect("stderr del servidor");
        Self {
            child,
            stdin,
            stdout,
            stderr,
        }
    }

    fn send(&mut self, value: &Value) -> Value {
        let id = value["id"].clone();
        assert!(!id.is_null(), "send sólo sirve requests con id");
        writeln!(self.stdin, "{value}").expect("escribir request Legacy");
        self.stdin.flush().expect("flush de request Legacy");
        let mut line = String::new();
        assert!(
            self.stdout.read_line(&mut line).expect("leer stdout") > 0,
            "la request {value} debe recibir un frame"
        );
        let response: Value = serde_json::from_str(line.trim_end()).expect("frame JSON válido");
        assert_eq!(response["jsonrpc"], "2.0");
        assert_eq!(
            response["id"], id,
            "respuesta correlacionada con su request"
        );
        response
    }

    fn notify(&mut self, value: &Value) {
        assert!(value.get("id").is_none(), "notify no puede llevar id");
        writeln!(self.stdin, "{value}").expect("escribir notificación");
        self.stdin.flush().expect("flush de notificación");
    }

    fn close(mut self) -> (bool, Vec<u8>) {
        drop(self.stdin);
        let status = self.child.wait().expect("EOF termina el proceso");
        let mut stderr = Vec::new();
        self.stderr
            .read_to_end(&mut stderr)
            .expect("leer stderr separado");
        (status.success(), stderr)
    }
}

fn raw_from_cwd(root: &Path, profile: &str, input: &[u8]) -> Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--profile")
        .arg(profile)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lodestar-mcp debe arrancar desde cwd");
    child
        .stdin
        .take()
        .expect("stdin del servidor")
        .write_all(input)
        .expect("escribir bytes raw");
    child
        .wait_with_output()
        .expect("esperar EOF del servidor desde cwd")
}

struct RawSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    stderr: ChildStderr,
}

impl RawSession {
    fn new(root: &Path, profile: &str) -> Self {
        let mut child = spawn(root, profile);
        let stdin = child.stdin.take().expect("stdin raw");
        let stdout = BufReader::new(child.stdout.take().expect("stdout raw"));
        let stderr = child.stderr.take().expect("stderr raw");
        Self {
            child,
            stdin,
            stdout,
            stderr,
        }
    }

    fn request(&mut self, bytes: &[u8]) -> Vec<u8> {
        assert!(bytes.ends_with(b"\n"), "cada request raw termina en LF");
        self.stdin.write_all(bytes).expect("escribir request raw");
        self.stdin.flush().expect("flush request raw");
        let mut frame = Vec::new();
        self.stdout
            .read_until(b'\n', &mut frame)
            .expect("leer respuesta raw");
        assert!(!frame.is_empty(), "request raw debe producir un frame");
        assert!(frame.ends_with(b"\n"), "respuesta raw termina en LF");
        frame
    }

    fn notification(&mut self, bytes: &[u8]) {
        assert!(bytes.ends_with(b"\n"), "notificación raw termina en LF");
        self.stdin
            .write_all(bytes)
            .expect("escribir notificación raw");
        self.stdin.flush().expect("flush notificación raw");
    }

    fn finish(mut self) -> (bool, Vec<u8>, Vec<u8>) {
        drop(self.stdin);
        let mut remaining_stdout = Vec::new();
        self.stdout
            .read_to_end(&mut remaining_stdout)
            .expect("leer stdout raw restante");
        let status = self.child.wait().expect("EOF raw termina el proceso");
        let mut stderr = Vec::new();
        self.stderr
            .read_to_end(&mut stderr)
            .expect("leer stderr raw");
        (status.success(), remaining_stdout, stderr)
    }
}

fn neutral_service(root: &Path, profile: Profile) -> LodestarMcpService {
    LodestarMcpService::new(App::open(root).expect("App abre fixture"), profile)
}

fn neutral_catalog(root: &Path, profile: Profile) -> Value {
    neutral_service(root, profile).list()["tools"].clone()
}

fn md_snapshot(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn visit(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for entry in std::fs::read_dir(dir).expect("leer fixture") {
            let entry = entry.expect("entrada de fixture");
            let path = entry.path();
            if path.file_name().is_some_and(|name| name == ".lodestar") {
                continue;
            }
            if path.is_dir() {
                visit(base, &path, out);
            } else if path.extension().is_some_and(|extension| extension == "md") {
                let relative = path
                    .strip_prefix(base)
                    .expect("path relativo")
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(relative, std::fs::read(path).expect("leer markdown"));
            }
        }
    }
    let mut result = BTreeMap::new();
    visit(root, root, &mut result);
    result
}

fn assert_legacy_response(response: &Value) {
    assert!(
        response["error"].is_null(),
        "respuesta Legacy inesperada: {response}"
    );
    let result = response["result"]
        .as_object()
        .expect("result Legacy objeto");
    for forbidden in ["resultType", "ttlMs", "cacheScope"] {
        assert!(
            !result.contains_key(forbidden),
            "respuesta Legacy no puede incluir {forbidden}: {response}"
        );
    }
}

fn assert_legacy_result_shape(response: &Value) {
    assert_legacy_response(response);
    let result = &response["result"];
    for forbidden in ["resultType", "ttlMs", "cacheScope"] {
        assert!(
            result.get(forbidden).is_none(),
            "campo Modern reservado: {response}"
        );
    }
}

fn extract_tool_result(response: &Value) -> &Value {
    assert_legacy_response(response);
    &response["result"]
}

#[test]
fn legacy_initialize_y_negociacion_fechas() {
    let offered_versions = [
        "2025-11-25",
        "2024-11-05",
        "2025-03-26",
        "2025-06-18",
        "2026-07-28",
        "2099-12-31",
        "1990-01-01",
    ];
    for (id, offered) in offered_versions.into_iter().enumerate() {
        let root = workspace_fixture();
        let before = md_snapshot(root.path());
        let mut session = Session::new(root.path(), "standard");
        let response = session.send(&initialize((id + 1) as u64, offered));
        assert_legacy_response(&response);
        let result = &response["result"];
        assert_eq!(result["protocolVersion"], LEGACY, "ofrecida={offered}");
        if offered != LEGACY {
            assert_ne!(
                result["protocolVersion"], offered,
                "no se debe ecoar la revisión"
            );
        }
        assert!(result["capabilities"]["tools"].is_object());
        assert!(!result["instructions"]
            .as_str()
            .unwrap_or_default()
            .is_empty());
        assert!(!result["serverInfo"]["name"]
            .as_str()
            .unwrap_or_default()
            .is_empty());
        assert!(!result["serverInfo"]["version"]
            .as_str()
            .unwrap_or_default()
            .is_empty());
        let after_handshake = session.send(&ping((id + 101) as u64));
        assert_eq!(after_handshake["result"], json!({}));
        assert_eq!(
            before,
            md_snapshot(root.path()),
            "initialize no escribe en workspace"
        );
        let (success, _stderr) = session.close();
        assert!(success, "EOF Legacy debe cerrar limpiamente");
    }

    // El handshake conserva exactamente las instrucciones del servicio neutral bajo cada perfil:
    // el wire no mantiene una descripción paralela ni filtra por una tabla propia.
    let mut instructions_by_profile = BTreeMap::new();
    for (profile_name, profile) in [
        ("standard", Profile::Standard),
        ("readonly", Profile::Readonly),
    ] {
        let root = workspace_fixture();
        let mut session = Session::new(root.path(), profile_name);
        let response = session.send(&initialize(50, LEGACY));
        assert_legacy_response(&response);
        let expected = neutral_service(root.path(), profile).instructions();
        assert_eq!(
            response["result"]["instructions"], expected,
            "initialize debe transportar exactamente las instrucciones del servicio neutral ({profile_name})"
        );
        instructions_by_profile.insert(profile_name, expected);
        let (success, _) = session.close();
        assert!(success);
    }
    assert_ne!(
        instructions_by_profile["standard"], instructions_by_profile["readonly"],
        "standard y readonly deben conservar sus filtros de instrucciones"
    );
}

#[test]
fn legacy_initialized_sin_respuesta() {
    let root = workspace_fixture();
    let mut session = Session::new(root.path(), "standard");
    let init = session.send(&initialize(1, LEGACY));
    assert_legacy_response(&init);
    session.notify(&initialized());
    let pong = session.send(&ping(2));
    assert_eq!(pong["id"], 2);
    assert_eq!(pong["result"], json!({}));
    let invalid_notification = json!({"jsonrpc":"2.0","id":3,"method":"notifications/initialized"});
    writeln!(session.stdin, "{invalid_notification}").expect("escribir initialized con id");
    session.stdin.flush().expect("flush initialized con id");
    let mut line = String::new();
    assert!(
        session
            .stdout
            .read_line(&mut line)
            .expect("leer error correlacionado")
            > 0
    );
    let error: Value = serde_json::from_str(line.trim_end()).expect("error JSON-RPC");
    assert_eq!(error["id"], 3);
    assert_eq!(error["error"]["code"], -32601);
    let (success, _) = session.close();
    assert!(success);
}

#[test]
fn legacy_ping() {
    let root = workspace_fixture();
    let before = md_snapshot(root.path());
    let mut session = Session::new(root.path(), "standard");
    assert_legacy_response(&session.send(&initialize(1, LEGACY)));
    let response = session.send(&ping(2));
    assert_eq!(response["id"], 2);
    assert_eq!(response["result"], json!({}));
    assert_legacy_result_shape(&response);
    assert_eq!(before, md_snapshot(root.path()), "ping no altera Markdown");
    let (success, _) = session.close();
    assert!(success);
}

#[test]
fn legacy_tools_list_call() {
    let root = workspace_fixture();
    let read_args = json!({"ref":{"path":"note.md"},"include":["body","revision"]});

    let mut standard = Session::new(root.path(), "standard");
    assert_legacy_response(&standard.send(&initialize(1, LEGACY)));
    standard.notify(&initialized());
    let discover_response = standard.send(&discover(2));
    assert_eq!(discover_response["error"]["code"], -32601);
    assert!(discover_response["result"].is_null());
    let listed_standard = standard.send(&list(3));
    let neutral_standard = neutral_catalog(root.path(), Profile::Standard);
    assert_eq!(listed_standard["result"]["tools"], neutral_standard);
    let names = listed_standard["result"]["tools"]
        .as_array()
        .expect("tools/list array")
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, TOOLS_STANDARD);
    let unknown = standard.send(&call(4, "tool_that_does_not_exist", json!({})));
    assert_eq!(unknown["error"]["code"], -32602);
    assert!(unknown["result"].is_null());
    let expected_read = neutral_service(root.path(), Profile::Standard)
        .call("knowledge_get", &read_args)
        .expect("neutral knowledge_get");
    let actual_read = standard.send(&call(5, "knowledge_get", read_args.clone()));
    assert_eq!(extract_tool_result(&actual_read), &expected_read);
    assert!(!actual_read["result"]["structuredContent"].is_null());

    let plan_args = json!({"operations":[{"op":"replace_text","path":"note.md","find":"contenido Legacy","replace":"contenido aplicado","expectedOccurrences":1}]});
    let planned = standard.send(&call(6, "change_plan", plan_args));
    let plan = extract_tool_result(&planned)["structuredContent"].clone();
    let change_set_id = plan["changeSetId"].as_str().expect("changeSetId");
    let applied = standard.send(&call(
        7,
        "change_apply",
        json!({"changeSetId": change_set_id}),
    ));
    assert!(!extract_tool_result(&applied)["isError"]
        .as_bool()
        .unwrap_or(false));
    assert!(
        std::fs::read_to_string(root.path().join("note.md"))
            .unwrap()
            .contains("contenido aplicado"),
        "change_apply debe publicar una escritura real"
    );
    let (success, _) = standard.close();
    assert!(success);

    let mut readonly = Session::new(root.path(), "readonly");
    assert_legacy_response(&readonly.send(&initialize(10, LEGACY)));
    readonly.notify(&initialized());
    let discover_response = readonly.send(&discover(11));
    assert_eq!(discover_response["error"]["code"], -32601);
    assert!(discover_response["result"].is_null());
    let listed_readonly = readonly.send(&list(12));
    assert_eq!(
        listed_readonly["result"]["tools"],
        neutral_catalog(root.path(), Profile::Readonly)
    );
    let names = listed_readonly["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert_eq!(names, TOOLS_READONLY);
    let unknown = readonly.send(&call(13, "tool_that_does_not_exist", json!({})));
    assert_eq!(unknown["error"]["code"], -32602);
    assert!(unknown["result"].is_null());
    let readonly_read = readonly.send(&call(14, "knowledge_get", read_args));
    assert!(!readonly_read["result"]["structuredContent"].is_null());
    for (id, name, args) in [
        (15, "change_plan", json!({})),
        (
            16,
            "change_apply",
            json!({"changeSetId":"changeset:missing"}),
        ),
        (17, "change_revert", json!({"receiptId":"receipt:missing"})),
    ] {
        let forbidden = readonly.send(&call(id, name, args));
        assert_eq!(forbidden["error"]["code"], -32602, "{name} readonly");
        assert!(forbidden["result"].is_null(), "{name} no devuelve result");
    }
    let (success, _) = readonly.close();
    assert!(success);
}

#[test]
fn legacy_sin_result_type_ni_cache_hints() {
    for profile in ["standard", "readonly"] {
        let root = workspace_fixture();
        let mut session = Session::new(root.path(), profile);
        assert_legacy_result_shape(&session.send(&initialize(1, LEGACY)));
        session.notify(&initialized());
        assert_legacy_result_shape(&session.send(&ping(2)));
        assert_legacy_result_shape(&session.send(&list(3)));
        assert_legacy_result_shape(&session.send(&call(4, "workspace_status", json!({}))));
        let (success, _) = session.close();
        assert!(success);
    }
}

fn expected_initialize_frame(root: &Path, profile: Profile) -> Vec<u8> {
    let service = neutral_service(root, profile);
    let instructions = serde_json::to_string(&service.instructions()).expect("instructions JSON");
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"result":{{"protocolVersion":"{LEGACY}","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"lodestar-mcp","version":"{}"}},"instructions":{instructions}}}}}
"#,
        env!("CARGO_PKG_VERSION")
    )
    .into_bytes()
}

#[test]
fn issue_38_repro_exacta() {
    let root = workspace_fixture();
    let input = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"test\",\"version\":\"0.1\"}}}\n";
    let output = raw_from_cwd(root.path(), "standard", input);
    assert!(
        output.status.success(),
        "issue #38 no debe tumbar el binario"
    );
    assert_eq!(
        output.stdout,
        expected_initialize_frame(root.path(), Profile::Standard),
        "el transcript exacto sólo puede contener el frame initialize esperado"
    );
    assert!(
        !output.stderr.is_empty(),
        "diagnósticos separados en stderr"
    );
}

#[test]
fn harness_raw_legacy_exacto() {
    let root = workspace_fixture();
    let mut session = RawSession::new(root.path(), "standard");
    let initialize_request = b"{\"jsonrpc\":\"2.0\",\"id\":1,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"raw-harness\",\"version\":\"1\"}}}\n";
    let initialized_notification =
        b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n";
    let discover_request =
        b"{\"jsonrpc\":\"2.0\",\"id\":2,\"method\":\"server/discover\",\"params\":{}}\n";
    let discover_with_modern_metadata = b"{\"jsonrpc\":\"2.0\",\"id\":3,\"method\":\"server/discover\",\"params\":{\"_meta\":{\"io.modelcontextprotocol/protocolVersion\":\"2026-07-28\",\"io.modelcontextprotocol/clientCapabilities\":{},\"io.modelcontextprotocol/clientInfo\":{\"name\":\"modern\",\"version\":\"1\"}}}}\n";
    let ping_request = b"{\"jsonrpc\":\"2.0\",\"id\":4,\"method\":\"ping\",\"params\":{}}\n";
    let expected_initialize = expected_initialize_frame(root.path(), Profile::Standard);
    let expected_discover =
        b"{\"jsonrpc\":\"2.0\",\"id\":2,\"error\":{\"code\":-32601,\"message\":\"server/discover\"}}\n";
    assert_eq!(session.request(initialize_request), expected_initialize);
    session.notification(initialized_notification);
    assert_eq!(session.request(discover_request), expected_discover);
    assert_eq!(
        session.request(discover_with_modern_metadata),
        b"{\"jsonrpc\":\"2.0\",\"id\":3,\"error\":{\"code\":-32601,\"message\":\"server/discover\"}}\n"
    );
    assert_eq!(
        session.request(ping_request),
        b"{\"jsonrpc\":\"2.0\",\"id\":4,\"result\":{}}\n"
    );
    let (success, remaining_stdout, stderr) = session.finish();
    assert!(success);
    assert!(remaining_stdout.is_empty(), "EOF no puede inventar frames");
    assert!(stderr.starts_with(b"lodestar-mcp:"));
}
