use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

struct WireSession {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
    next_id: u64,
}

impl WireSession {
    fn open(root: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
            .arg("--root")
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("arrancar lodestar-mcp real");
        let mut session = Self {
            stdin: child.stdin.take().expect("stdin de lodestar-mcp"),
            stdout: BufReader::new(child.stdout.take().expect("stdout de lodestar-mcp")),
            child,
            next_id: 1,
        };
        let initialized = session.request(
            "initialize",
            json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "patch-frontmatter-wire-test", "version": "1"}
            }),
        );
        assert_eq!(initialized["result"]["serverInfo"]["name"], "lodestar-mcp");
        session.notify("notifications/initialized", json!({}));
        session
    }

    fn notify(&mut self, method: &str, params: Value) {
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc":"2.0", "method":method, "params":params})
        )
        .expect("enviar notificación MCP");
        self.stdin.flush().expect("vaciar notificación MCP");
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next_id;
        self.next_id += 1;
        writeln!(
            self.stdin,
            "{}",
            json!({"jsonrpc":"2.0", "id":id, "method":method, "params":params})
        )
        .expect("enviar petición MCP");
        self.stdin.flush().expect("vaciar petición MCP");

        let mut line = String::new();
        let bytes = self
            .stdout
            .read_line(&mut line)
            .expect("leer respuesta MCP");
        assert!(
            bytes > 0,
            "lodestar-mcp cerró stdout antes de responder a {method}"
        );
        let response: Value =
            serde_json::from_str(line.trim_end()).expect("stdout debe ser JSON-RPC");
        assert_eq!(response["id"], id, "respuesta MCP desalineada: {response}");
        response
    }

    fn tool(&mut self, name: &str, arguments: Value) -> Value {
        let response = self.request("tools/call", json!({"name": name, "arguments": arguments}));
        assert!(
            response["error"].is_null(),
            "error de protocolo en {name}: {response}"
        );
        let result = &response["result"];
        assert_ne!(
            result["isError"], true,
            "error de ejecución en {name}: {result}"
        );
        result["structuredContent"].clone()
    }
}

impl Drop for WireSession {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn write(root: &Path, relative: &str, content: &str) {
    std::fs::write(root.join(relative), content).expect("escribir fixture Markdown");
}

fn parsed_frontmatter(raw: &str) -> serde_yaml::Value {
    let yaml = raw
        .strip_prefix("---\n")
        .expect("el documento aplicado conserva el delimitador inicial");
    let end = yaml
        .find("\n---\n")
        .expect("el documento aplicado conserva el delimitador final");
    serde_yaml::from_str(&yaml[..end]).expect("frontmatter aplicado interpretable como YAML")
}

#[test]
fn change_plan_y_change_apply_por_wire_comparten_resultado_rfc7386() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", "# Index\n");
    write(
        dir.path(),
        "doc.md",
        "---\ntype: Note\nnested:\n  a: old\n  b: survives\n  hondo:\n    keep: survives-too\n    a: old-deep\nother: untouched\n---\n\n# Body\n",
    );

    let mut wire = WireSession::open(dir.path());
    let plan = wire.tool(
        "change_plan",
        json!({
            "operations": [{
                "op": "patch_frontmatter",
                "path": "doc.md",
                "patch": {
                    "nested": {
                        "a": "new",
                        "remove_me": null,
                        "hondo": {"a": "new-deep"}
                    }
                }
            }]
        }),
    );
    assert_eq!(
        plan["canApply"], true,
        "el plan wire debe ser aplicable: {plan}"
    );
    assert_eq!(plan["noOpOperations"], json!([]));
    let planned_diff = plan["semanticDiff"].clone();
    let change_set_id = plan["changeSetId"]
        .as_str()
        .expect("changeSetId en la respuesta wire")
        .to_owned();

    let applied = wire.tool("change_apply", json!({"changeSetId": change_set_id}));
    assert_eq!(
        applied["semanticDiff"], planned_diff,
        "change_apply debe publicar el mismo resultado que change_plan simuló"
    );
    assert_eq!(applied["changedPaths"], json!(["doc.md"]));

    let raw = std::fs::read_to_string(dir.path().join("doc.md")).unwrap();
    let expected: serde_yaml::Value = serde_yaml::from_str(
        "type: Note\nnested:\n  a: new\n  b: survives\n  hondo:\n    keep: survives-too\n    a: new-deep\nother: untouched\n",
    )
    .unwrap();
    assert_eq!(parsed_frontmatter(&raw), expected);
    assert!(
        raw.ends_with("---\n\n# Body\n"),
        "el cuerpo debe conservarse: {raw:?}"
    );
}
