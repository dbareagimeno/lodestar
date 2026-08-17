//! E34-H06 — cierre observable de rmcp/stdio.
//!
//! Esta suite es deliberadamente independiente de las matrices H04/H05: comprueba que el
//! adaptador respeta la cancelación de rmcp antes de entrar en su FIFO, que el executor sigue
//! siendo un único escritor y que los dos modos de ciclo de vida se pueden ejercer con el cliente
//! oficial rmcp 3.1.2.

use std::future::Future;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::task::Poll;

use lodestar_app::{App, Profile};
use lodestar_mcp::{LodestarMcpService, SerialExecutor};
use serde_json::json;

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
        "---\ntype: Index\ntitle: Root\n---\n\n# Root\n\n[Note](note.md)\n",
    );
    write_file(
        root.path(),
        "note.md",
        "---\ntype: Note\ntitle: Note\n---\n\n# Note\n\nnon-empty H06 fixture\n",
    );
    root
}

const CANCEL_PROBE_CARGO: &str = r#"[package]
name = "e34-h06-cancellation-probe"
version = "0.1.0"
edition = "2024"

[dependencies]
lodestar-mcp = { path = "__MCP__" }
lodestar-app = { path = "__APP__" }
rmcp = { version = "=3.1.2", default-features = false, features = ["server", "transport-async-rw"] }
serde_json = "1"
tokio = { version = "1", features = ["macros", "rt-multi-thread", "io-util", "sync"] }
"#;

const CANCEL_PROBE_MAIN: &str = r#"use lodestar_app::{App, Profile};
use lodestar_mcp::{LodestarMcpServer, LodestarMcpService, SerialExecutor};
use rmcp::{RoleServer, Service};
use rmcp::service::{MaybeSendFuture, NotificationContext, RequestContext, ServiceRole};
use serde_json::json;
use std::{env, error::Error, path::Path, sync::mpsc as std_mpsc};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::sync::{mpsc, oneshot};

#[derive(Clone)]
struct Probe {
    inner: LodestarMcpServer,
    events: mpsc::UnboundedSender<(rmcp::model::RequestId, RequestContext<RoleServer>)>,
    done: mpsc::UnboundedSender<rmcp::model::RequestId>,
    acknowledgements: mpsc::UnboundedSender<()>,
}

impl Service<RoleServer> for Probe {
    fn handle_request(
        &self,
        request: <RoleServer as ServiceRole>::PeerReq,
        context: RequestContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<<RoleServer as ServiceRole>::Resp, rmcp::ErrorData>>
           + MaybeSendFuture + '_ {
        let id = context.id.clone();
        let saved = context.clone();
        let events = self.events.clone();
        let done = self.done.clone();
        let future = self.inner.handle_request(request, context);
        async move {
            events.send((id.clone(), saved)).map_err(|_| rmcp::ErrorData::internal_error("event channel", None))?;
            let result = future.await;
            let _ = done.send(id);
            result
        }
    }

    fn handle_notification(
        &self,
        notification: <RoleServer as ServiceRole>::PeerNot,
        context: NotificationContext<RoleServer>,
    ) -> impl std::future::Future<Output = Result<(), rmcp::ErrorData>> + MaybeSendFuture + '_ {
        let acknowledgements = self.acknowledgements.clone();
        let _ = acknowledgements.send(());
        let future = self.inner.handle_notification(notification, context);
        async move { future.await }
    }

    fn get_info(&self) -> <RoleServer as ServiceRole>::Info { self.inner.get_info() }
    fn supported_protocol_versions(&self) -> std::borrow::Cow<'static, [rmcp::model::ProtocolVersion]> {
        self.inner.supported_protocol_versions()
    }
}

fn plan_args() -> serde_json::Value {
    json!({
        "operations": [{
            "op": "patch_frontmatter",
            "ref": {"path": "note.md"},
            "patch": {"estado": "cancelled-must-not-publish"}
        }],
        "policy": {"requireValidResult": false, "allowWarnings": true}
    })
}

fn frame(id: u64, method: &str, params: serde_json::Value) -> String {
    format!("{}\n", json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}))
}

#[tokio::main(flavor = "multi_thread", worker_threads = 4)]
async fn main() -> Result<(), Box<dyn Error>> {
    let root = env::var("E34_CANCEL_ROOT")?;
    let app = App::open(Path::new(&root))?;
    let executor = SerialExecutor::new(LodestarMcpService::new(app, Profile::Standard));
    let plan = executor.call("change_plan", plan_args()).await?;
    let change_set_id = plan["structuredContent"]["changeSetId"].as_str().ok_or("missing plan id")?.to_owned();
    let late_plan = executor.call("change_plan", json!({
        "operations": [{"op":"patch_frontmatter","ref":{"path":"note.md"},"patch":{"estado":"late-publication"}}],
        "policy":{"requireValidResult":false,"allowWarnings":true}
    })).await?;
    let late_change_set_id = late_plan["structuredContent"]["changeSetId"].as_str().ok_or("missing late plan id")?.to_owned();
    let index_before = std::fs::read(Path::new(&root).join("index.md"))?;
    let note_before = std::fs::read(Path::new(&root).join("note.md"))?;

    let (entered_tx, entered_rx) = oneshot::channel();
    let (release_tx, release_rx) = std_mpsc::sync_channel(0);
    let held = executor.clone();
    std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(held.run(|_| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
    });
    entered_rx.await?;

    let (events_tx, mut events_rx) = mpsc::unbounded_channel();
    let (done_tx, mut done_rx) = mpsc::unbounded_channel();
    let (ack_tx, mut ack_rx) = mpsc::unbounded_channel();
    let server = LodestarMcpServer::new(executor.clone());
    let probe = Probe { inner: server, events: events_tx, done: done_tx, acknowledgements: ack_tx };
    let (client, server_io) = tokio::io::duplex(1024 * 1024);
    let (client_read, mut client_write) = tokio::io::split(client);
    let (server_read, server_write) = tokio::io::split(server_io);
    let mut running = rmcp::service::serve_directly(probe, (server_read, server_write), None);
    let mut lines = BufReader::new(client_read).lines();

    client_write.write_all(frame(1, "tools/call", json!({"name":"workspace_status","arguments":{}})).as_bytes()).await?;
    client_write.flush().await?;
    let (id1, _) = events_rx.recv().await.ok_or("missing first request")?;
    if id1 != rmcp::model::RequestId::Number(1) { return Err("first id mismatch".into()); }

    client_write.write_all(frame(2, "tools/call", json!({"name":"change_apply","arguments":{"changeSetId":change_set_id}})).as_bytes()).await?;
    client_write.flush().await?;
    let (id2, context2) = events_rx.recv().await.ok_or("missing second request")?;
    if id2 != rmcp::model::RequestId::Number(2) { return Err("second id mismatch".into()); }
    client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"reason\":\"missing-id-must-not-cancel\"}}\n").await?;
    client_write.flush().await?;
    ack_rx.recv().await.ok_or("missing cancellation notification ack")?;
    if context2.ct.is_cancelled() { return Err("cancel sin requestId canceló id2".into()); }
    client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":2,\"reason\":\"test\"}}\n").await?;
    client_write.flush().await?;
    context2.ct.cancelled().await;

    release_tx.send(()).map_err(|_| "release holder dropped")?;
    let response1 = lines.next_line().await?.ok_or("missing first response")?;
    let response1: serde_json::Value = serde_json::from_str(&response1)?;
    if response1["id"] != 1 { return Err(format!("first response crossed: {response1}").into()); }
    let mut finished_second = false;
    while let Some(id) = done_rx.recv().await {
        if id == rmcp::model::RequestId::Number(2) { finished_second = true; break; }
    }
    if !finished_second { return Err("cancelled request did not settle".into()); }
    if std::fs::read(Path::new(&root).join("index.md"))? != index_before
        || std::fs::read(Path::new(&root).join("note.md"))? != note_before
    { return Err("cancelación temprana alteró un Markdown".into()); }
    for directory in [".lodestar/runtime/receipts", ".lodestar/runtime/journal"] {
        let path = Path::new(&root).join(directory);
        if path.exists() && std::fs::read_dir(path)?.next().transpose()?.is_some() {
            return Err(format!("cancelación temprana dejó artefactos en {directory}").into());
        }
    }

    client_write.write_all(frame(3, "tools/call", json!({"name":"change_apply","arguments":{"changeSetId":late_change_set_id}})).as_bytes()).await?;
    client_write.flush().await?;
    let response3 = lines.next_line().await?.ok_or("missing late apply response")?;
    let response3: serde_json::Value = serde_json::from_str(&response3)?;
    if response3["id"] != 3 || response3["result"]["structuredContent"]["applied"] != true { return Err(format!("late apply failed: {response3}").into()); }
    let published_note = std::fs::read(Path::new(&root).join("note.md"))?;
    client_write.write_all(b"{\"jsonrpc\":\"2.0\",\"method\":\"notifications/cancelled\",\"params\":{\"requestId\":3,\"reason\":\"too-late\"}}\n").await?;
    client_write.flush().await?;
    client_write.write_all(frame(4, "tools/call", json!({"name":"workspace_status","arguments":{}})).as_bytes()).await?;
    client_write.flush().await?;
    let response4 = lines.next_line().await?.ok_or("missing control response")?;
    let response4: serde_json::Value = serde_json::from_str(&response4)?;
    if response4["id"] != 4 { return Err(format!("cancelled response leaked: {response4}").into()); }
    if std::fs::read(Path::new(&root).join("note.md"))? != published_note { return Err("late cancel revirtió publicación".into()); }
    let receipts = Path::new(&root).join(".lodestar/runtime/receipts");
    if !receipts.exists() || std::fs::read_dir(receipts)?.next().transpose()?.is_none() {
        return Err("publicación válida no dejó receipt".into());
    }
    let journals = Path::new(&root).join(".lodestar/runtime/journal");
    if journals.exists() && std::fs::read_dir(journals)?.next().transpose()?.is_some() {
        return Err("cancelación tardía dejó journal pendiente".into());
    }
    drop(client_write);
    let _ = running.cancel().await?;

    let bytes = std::fs::read(Path::new(&root).join("note.md"))?;
    if bytes.windows(b"cancelled-must-not-publish".len()).any(|window| window == b"cancelled-must-not-publish") {
        println!("CANCEL_PROBE_EXECUTED");
    } else {
        println!("CANCEL_PROBE_SUPPRESSED");
    }
    Ok(())
}
"#;

fn toml_basic_string_content(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t")
}

/// C1 — una cancelación wire real cancela el `RequestContext` mientras la request espera el FIFO.
/// La barrera es el lock real de `SerialExecutor`: al liberar el primer turno, un adaptador
/// incorrecto deja pasar la escritura cancelada y el probe devuelve un rojo observable.
#[test]
fn cancelacion_transaccional_sin_parciales() {
    let helper = tempfile::tempdir().unwrap();
    let mcp = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let manifest = CANCEL_PROBE_CARGO
        .replace("__MCP__", &toml_basic_string_content(mcp.to_str().unwrap()))
        .replace(
            "__APP__",
            &toml_basic_string_content(mcp.join("../lodestar-app").to_str().unwrap()),
        );
    write_text(&helper.path().join("Cargo.toml"), &manifest);
    write_text(&helper.path().join("src/main.rs"), CANCEL_PROBE_MAIN);
    let root = workspace_fixture();
    let target = mcp.join("../../target/agent-state/e34-h06/cancel-target");
    let output = Command::new("cargo")
        .args(["run", "--offline", "--manifest-path"])
        .arg(helper.path().join("Cargo.toml"))
        .args(["--target-dir"])
        .arg(target)
        .env("E34_CANCEL_ROOT", root.path())
        .output()
        .expect("ejecutar probe de cancelación");
    assert!(
        output.status.success(),
        "probe rmcp terminó con error: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stdout).contains("CANCEL_PROBE_SUPPRESSED"),
        "una escritura cancelada antes del FIFO no puede publicarse: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
/// C2 — cuatro combinaciones de era/perfil atraviesan lectura, plan y publicación.
///
/// La barrera conductual de exclusión y cancelación está en C1; aquí el banco verifica la misma
/// secuencia observable por ambos wires y que el perfil readonly no convierte una escritura en un
/// éxito vacío. Cada workspace es independiente para que los bytes finales sean un oráculo real.
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn serializacion_concurrente_final_coherente() {
    for modern in [false, true] {
        for profile in ["standard", "readonly"] {
            let root = workspace_fixture();
            let mut wire = Wire::start(root.path(), profile);
            if modern {
                let discover = wire.send(modern_request(1, "server/discover", json!({})));
                assert_eq!(discover["id"], 1);
                assert!(discover["result"]["supportedVersions"].is_array());
            } else {
                let init = wire.send(initialize_request(1));
                assert_eq!(init["result"]["protocolVersion"], "2025-11-25");
                wire.notify(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
            }
            let list_id = 2;
            let listed = wire.send(if modern {
                modern_request(list_id, "tools/list", json!({}))
            } else {
                request(list_id, "tools/list", json!({}))
            });
            let expected_len = if profile == "readonly" { 7 } else { 10 };
            assert_eq!(
                listed["result"]["tools"].as_array().unwrap().len(),
                expected_len
            );

            if profile == "standard" {
                let plan = wire.send(call_wire(3, modern, "change_plan", plan_args("h06-wire-A")));
                let change_set = plan["result"]["structuredContent"]["changeSetId"]
                    .as_str()
                    .expect("plan real")
                    .to_owned();
                let plan2 = wire.send(call_wire(4, modern, "change_plan", plan_args("h06-wire-B")));
                let change_set2 = plan2["result"]["structuredContent"]["changeSetId"]
                    .as_str()
                    .expect("segundo plan real")
                    .to_owned();
                let batch = wire.send_batch(&[
                    call_wire(5, modern, "change_apply", json!({"changeSetId":change_set})),
                    call_wire(
                        6,
                        modern,
                        "change_apply",
                        json!({"changeSetId":change_set2}),
                    ),
                    call_wire(7, modern, "knowledge_search", json!({"text":"Note"})),
                ]);
                assert_eq!(
                    batch
                        .iter()
                        .filter(|r| r["result"]["structuredContent"]["applied"] == true)
                        .count(),
                    1,
                    "una sola publicación: {batch:?}"
                );
                assert_eq!(
                    batch
                        .iter()
                        .filter(|r| r["result"]["isError"] == true)
                        .count(),
                    1,
                    "un conflicto de plan obsoleto: {batch:?}"
                );
                assert!(batch.iter().find(|r| r["id"] == 7).unwrap()["result"]
                    ["structuredContent"]["results"]
                    .as_array()
                    .is_some_and(|v| !v.is_empty()));
                let plan3 = wire.send(call_wire(8, modern, "change_plan", plan_args("h06-wire-C")));
                let id3 = plan3["result"]["structuredContent"]["changeSetId"]
                    .as_str()
                    .unwrap();
                assert_eq!(
                    wire.send(call_wire(
                        9,
                        modern,
                        "change_apply",
                        json!({"changeSetId":id3})
                    ))["result"]["structuredContent"]["applied"],
                    true
                );
                let plan4 = wire.send(call_wire(
                    10,
                    modern,
                    "change_plan",
                    plan_args("h06-wire-D"),
                ));
                let id4 = plan4["result"]["structuredContent"]["changeSetId"]
                    .as_str()
                    .unwrap();
                assert_eq!(
                    wire.send(call_wire(
                        11,
                        modern,
                        "change_apply",
                        json!({"changeSetId":id4})
                    ))["result"]["structuredContent"]["applied"],
                    true
                );
                let final_bytes = std::fs::read_to_string(root.path().join("note.md")).unwrap();
                assert!(
                    final_bytes.contains("h06-wire-D"),
                    "última publicación pierde orden: {final_bytes}"
                );
            } else {
                let batch = wire.send_batch(&[
                    call_wire(3, modern, "knowledge_search", json!({"text":"Note"})),
                    call_wire(
                        4,
                        modern,
                        "knowledge_get",
                        json!({"ref":{"path":"note.md"}}),
                    ),
                    call_wire(5, modern, "change_plan", plan_args("must-not-write")),
                ]);
                assert_eq!(
                    batch
                        .iter()
                        .filter(|r| r["result"]["structuredContent"].is_object())
                        .count(),
                    2,
                    "readonly lecturas reales: {batch:?}"
                );
                let rejected = batch.iter().find(|r| r["id"] == 5).unwrap();
                assert_eq!(
                    rejected["error"]["code"], -32602,
                    "readonly no puede planificar: {rejected}"
                );
                let final_bytes = std::fs::read_to_string(root.path().join("note.md")).unwrap();
                assert!(!final_bytes.contains("must-not-write"));
            }
            wire.close();
        }
    }

    // El executor neutral sigue siendo un único escritor también bajo carga de lectores.
    let root = workspace_fixture();
    let app = App::open(root.path()).expect("fixture abre");
    let executor = SerialExecutor::new(LodestarMcpService::new(app, Profile::Standard));
    let calls = (0..16)
        .map(|_| {
            let executor = executor.clone();
            tokio::spawn(async move {
                executor
                    .call("knowledge_search", json!({"text": "Note"}))
                    .await
            })
        })
        .collect::<Vec<_>>();
    for call in calls {
        let response = call.await.expect("join serial").expect("lectura válida");
        assert!(!response["structuredContent"]["results"]
            .as_array()
            .expect("la lectura no es un placeholder")
            .is_empty());
    }
    let bytes = std::fs::read(root.path().join("note.md")).unwrap();
    assert!(bytes
        .windows(b"non-empty H06 fixture".len())
        .any(|window| window == b"non-empty H06 fixture"));

    // Banco determinista de requests realmente superpuestas: el holder toma el mismo lock que
    // las tres operaciones antes de que se pollée cualquiera. No se usa sleep ni una carrera de
    // scheduler; las tres tareas deben quedar pendientes hasta liberar el canal.
    let root = workspace_fixture();
    let app = App::open(root.path()).expect("fixture concurrente abre");
    let executor = SerialExecutor::new(LodestarMcpService::new(app, Profile::Standard));
    let plan_a = executor
        .call("change_plan", plan_args("concurrent-A"))
        .await
        .unwrap()["structuredContent"]["changeSetId"]
        .as_str()
        .unwrap()
        .to_owned();
    let plan_b = executor
        .call("change_plan", plan_args("concurrent-B"))
        .await
        .unwrap()["structuredContent"]["changeSetId"]
        .as_str()
        .unwrap()
        .to_owned();
    let (entered_tx, entered_rx) = mpsc::sync_channel(3);
    let (release_tx, release_rx) = mpsc::sync_channel(0);
    let held = executor.clone();
    let holder = std::thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(held.run(|_| {
            entered_tx.send(()).unwrap();
            release_rx.recv().unwrap();
        }));
    });
    entered_rx.recv().unwrap();
    let mut futures = Vec::new();
    for operation in [
        ("change_apply", json!({"changeSetId":plan_a})),
        ("change_apply", json!({"changeSetId":plan_b})),
        ("workspace_status", json!({})),
    ] {
        let worker = executor.clone();
        let mut future = Box::pin(async move { worker.call(operation.0, operation.1).await });
        std::future::poll_fn(|cx| match future.as_mut().poll(cx) {
            Poll::Pending => Poll::Ready(()),
            Poll::Ready(_) => panic!("request terminó antes de liberar el holder"),
        })
        .await;
        futures.push(future);
    }
    release_tx.send(()).unwrap();
    holder.join().unwrap();
    let mut values = Vec::new();
    for future in futures {
        values.push(future.await.unwrap());
    }
    assert!(values
        .iter()
        .any(|value| value["structuredContent"]["applied"] == true));
    assert!(values.iter().any(|value| value["isError"] == true));
    assert!(values
        .iter()
        .any(|value| value["structuredContent"].is_object() && value["isError"].is_null()));
    // Control anti-vacuidad: dos planes nuevos, calculados después de la concurrencia, se pueden
    // publicar consecutivamente; el conflicto anterior no es un rechazo posicional.
    for value in ["concurrent-C", "concurrent-D"] {
        let plan = executor
            .call("change_plan", plan_args(value))
            .await
            .unwrap();
        let id = plan["structuredContent"]["changeSetId"].as_str().unwrap();
        let applied = executor
            .call("change_apply", json!({"changeSetId":id}))
            .await
            .unwrap();
        assert_eq!(applied["structuredContent"]["applied"], true);
    }
    assert!(std::fs::read_to_string(root.path().join("note.md"))
        .unwrap()
        .contains("concurrent-D"));
}

fn plan_args(value: &str) -> serde_json::Value {
    json!({"operations":[{"op":"patch_frontmatter","ref":{"path":"note.md"},"patch":{"estado":value}}],"policy":{"requireValidResult":false,"allowWarnings":true}})
}

fn initialize_request(id: u64) -> serde_json::Value {
    request(
        id,
        "initialize",
        json!({"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"h06","version":"1"}}),
    )
}

fn request(id: u64, method: &str, params: serde_json::Value) -> serde_json::Value {
    json!({"jsonrpc":"2.0","id":id,"method":method,"params":params})
}

fn modern_request(id: u64, method: &str, params: serde_json::Value) -> serde_json::Value {
    modern_request_version(id, method, params, "2026-07-28")
}

fn modern_request_version(
    id: u64,
    method: &str,
    params: serde_json::Value,
    version: &str,
) -> serde_json::Value {
    let mut params = params;
    params["_meta"] = json!({
        "io.modelcontextprotocol/protocolVersion":version,
        "io.modelcontextprotocol/clientCapabilities":{},
        "io.modelcontextprotocol/clientInfo":{"name":"h06","version":"1"}
    });
    request(id, method, params)
}

fn exact_frame(value: serde_json::Value) -> Vec<u8> {
    let id = value["id"].as_u64().expect("id esperado");
    let body = if value.get("result").is_some() {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"result\":{}}}\n",
            serde_json::to_string(&value["result"]).unwrap()
        )
    } else {
        format!(
            "{{\"jsonrpc\":\"2.0\",\"id\":{id},\"error\":{}}}\n",
            serde_json::to_string(&value["error"]).unwrap()
        )
    };
    body.into_bytes()
}

fn expected_legacy_initialize(root: &Path, profile: &str) -> Vec<u8> {
    let app = App::open(root).expect("App abre fixture esperado");
    let instructions = lodestar_mcp::LodestarMcpService::new(
        app,
        if profile == "readonly" {
            Profile::Readonly
        } else {
            Profile::Standard
        },
    )
    .instructions();
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{{\"tools\":{{}}}},\"serverInfo\":{{\"name\":\"lodestar-mcp\",\"version\":\"{}\"}},\"instructions\":{}}}}}\n",
        env!("CARGO_PKG_VERSION"),
        serde_json::to_string(&instructions).unwrap()
    ).into_bytes()
}

fn expected_modern_discover(root: &Path, profile: &str) -> Vec<u8> {
    let app = App::open(root).expect("App abre fixture esperado");
    let instructions = lodestar_mcp::LodestarMcpService::new(
        app,
        if profile == "readonly" {
            Profile::Readonly
        } else {
            Profile::Standard
        },
    )
    .instructions();
    format!(
        "{{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{{\"resultType\":\"complete\",\"supportedVersions\":[\"2026-07-28\"],\"capabilities\":{{\"tools\":{{}}}},\"instructions\":{},\"ttlMs\":0,\"cacheScope\":\"private\",\"_meta\":{{\"io.modelcontextprotocol/serverInfo\":{{\"name\":\"lodestar-mcp\",\"version\":\"{}\"}}}}}}}}\n",
        serde_json::to_string(&instructions).unwrap(),
        env!("CARGO_PKG_VERSION")
    ).into_bytes()
}

fn expected_error(id: u64, code: i32, message: &str, data: Option<serde_json::Value>) -> Vec<u8> {
    let error = if let Some(data) = data {
        format!(
            "{{\"code\":{code},\"message\":{},\"data\":{}}}",
            serde_json::to_string(message).unwrap(),
            serde_json::to_string(&data).unwrap()
        )
    } else {
        format!(
            "{{\"code\":{code},\"message\":{}}}",
            serde_json::to_string(message).unwrap()
        )
    };
    format!("{{\"jsonrpc\":\"2.0\",\"id\":{id},\"error\":{error}}}\n").into_bytes()
}

fn call_wire(id: u64, modern: bool, name: &str, arguments: serde_json::Value) -> serde_json::Value {
    let params = json!({"name":name,"arguments":arguments});
    if modern {
        modern_request(id, "tools/call", params)
    } else {
        request(id, "tools/call", params)
    }
}

struct Wire {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
}

impl Wire {
    fn start(root: &Path, profile: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
            .arg("--root")
            .arg(root)
            .arg("--profile")
            .arg(profile)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        Self {
            stdin: child.stdin.take().unwrap(),
            stdout: std::io::BufReader::new(child.stdout.take().unwrap()),
            child,
        }
    }
    fn send(&mut self, value: serde_json::Value) -> serde_json::Value {
        writeln!(self.stdin, "{value}").unwrap();
        self.stdin.flush().unwrap();
        let mut line = String::new();
        std::io::BufRead::read_line(&mut self.stdout, &mut line).unwrap();
        serde_json::from_str(line.trim_end()).expect("wire JSON")
    }
    fn send_batch(&mut self, values: &[serde_json::Value]) -> Vec<serde_json::Value> {
        for value in values {
            writeln!(self.stdin, "{value}").unwrap();
        }
        self.stdin.flush().unwrap();
        values
            .iter()
            .map(|_| {
                let mut line = String::new();
                std::io::BufRead::read_line(&mut self.stdout, &mut line).unwrap();
                serde_json::from_str(line.trim_end()).expect("wire JSON batch")
            })
            .collect()
    }
    fn notify(&mut self, value: serde_json::Value) {
        writeln!(self.stdin, "{value}").unwrap();
        self.stdin.flush().unwrap();
    }
    fn close(mut self) {
        drop(self.stdin);
        assert!(self.child.wait().unwrap().success());
    }
}

const OFFICIAL_CLIENT_CARGO: &str = r#"[package]
name = "e34-h06-official-client"
version = "0.1.0"
edition = "2024"

[dependencies]
rmcp = { version = "=3.1.2", default-features = false, features = ["client", "transport-async-rw"] }
tokio = { version = "1", features = ["macros", "fs", "io-util", "rt-multi-thread", "time"] }
serde_json = "1"
"#;

const OFFICIAL_CLIENT_MAIN: &str = r#"use rmcp::{ClientHandler, ClientServiceExt};
use rmcp::{ClientLifecycleMode, model::{CallToolRequestParams, ClientRequest, PingRequest, ProtocolVersion, ServerResult}};
use std::{env, error::Error, process::{Command, Stdio}};

#[derive(Default)] struct OfficialClient;
impl ClientHandler for OfficialClient {}

#[cfg(unix)] fn file_out(s: std::process::ChildStdout) -> std::fs::File {
    use std::os::unix::io::{FromRawFd, IntoRawFd};
    unsafe { std::fs::File::from_raw_fd(s.into_raw_fd()) }
}
#[cfg(unix)] fn file_in(s: std::process::ChildStdin) -> std::fs::File {
    use std::os::unix::io::{FromRawFd, IntoRawFd};
    unsafe { std::fs::File::from_raw_fd(s.into_raw_fd()) }
}
#[cfg(windows)] fn file_out(s: std::process::ChildStdout) -> std::fs::File {
    use std::os::windows::io::{FromRawHandle, IntoRawHandle};
    unsafe { std::fs::File::from_raw_handle(s.into_raw_handle()) }
}
#[cfg(windows)] fn file_in(s: std::process::ChildStdin) -> std::fs::File {
    use std::os::windows::io::{FromRawHandle, IntoRawHandle};
    unsafe { std::fs::File::from_raw_handle(s.into_raw_handle()) }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    for mode in ["modern", "legacy"] {
        let mut command = Command::new(env::var("E34_LODESTAR_BIN")?);
        command.arg("--root").arg(env::var("E34_LODESTAR_ROOT")?)
            .arg("--profile").arg(env::var("E34_LODESTAR_PROFILE")?)
            .stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
        let mut child = command.spawn()?;
        let out = tokio::fs::File::from_std(file_out(child.stdout.take().ok_or("stdout")?));
        let input = tokio::fs::File::from_std(file_in(child.stdin.take().ok_or("stdin")?));
        let mut client = if mode == "modern" {
            OfficialClient.serve_with_lifecycle((out, input), ClientLifecycleMode::Discover {
                preferred_versions: vec![ProtocolVersion::V_2026_07_28],
            }).await?
        } else {
            OfficialClient.serve_with_lifecycle((out, input), ClientLifecycleMode::Initialize {
            }).await?
        };
        if mode == "legacy" {
            let ping = client.peer().send_request(ClientRequest::PingRequest(PingRequest {
                method: Default::default(),
                extensions: Default::default(),
            })).await?;
            if !matches!(ping, ServerResult::EmptyResult(_)) {
                return Err("legacy: ping oficial no devolvió PingResult".into());
            }
        }
        let tools = client.list_all_tools().await?;
        if tools.len() != if env::var("E34_LODESTAR_PROFILE")? == "readonly" { 7 } else { 10 } {
            return Err(format!("{mode}: catálogo incorrecto: {}", tools.len()).into());
        }
        if !tools.iter().any(|tool| tool.name == "knowledge_get") {
            return Err(format!("{mode}: falta knowledge_get").into());
        }
        let read = client.call_tool(CallToolRequestParams::new("knowledge_search")
            .with_arguments(serde_json::json!({"text":"Note"}).as_object().unwrap().clone())).await?;
        if read.structured_content.as_ref().and_then(|value| value.get("results"))
            .and_then(|results| results.as_array()).is_none_or(|results| results.is_empty()) {
            return Err(format!("{mode}: tools/call no devolvió lectura real").into());
        }
        client.cancel().await?;
        if !child.wait()?.success() { return Err(format!("{mode}: servidor no terminó").into()); }
    }
    Ok(())
}
"#;

fn write_text(path: &Path, value: &str) {
    std::fs::create_dir_all(path.parent().expect("path parent")).unwrap();
    std::fs::write(path, value).unwrap();
}

/// C3 — ambos clientes oficiales rmcp 3.1.2 ejercen Modern/discover y Legacy/initialize.
#[test]
fn rmcp_clientes_oficiales_moderno_y_legacy() {
    let source = std::env::var_os("E34_TOKIO_STREAM_SOURCE")
        .map(PathBuf::from)
        .expect("el auxiliar oficial no se puede omitir: falta E34_TOKIO_STREAM_SOURCE");
    let source = std::fs::canonicalize(source).expect("fuente tokio-stream válida");
    let helper = tempfile::tempdir().unwrap();
    write_text(
        &helper.path().join("Cargo.toml"),
        &format!(
            "{OFFICIAL_CLIENT_CARGO}\n[patch.crates-io]\ntokio-stream = {{ path = {:?} }}\n",
            source.to_str().expect("ruta utf8")
        ),
    );
    write_text(&helper.path().join("src/main.rs"), OFFICIAL_CLIENT_MAIN);
    let root = workspace_fixture();
    let target = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/agent-state/e34-h06/client-target");
    std::fs::create_dir_all(&target).unwrap();
    for profile in ["standard", "readonly"] {
        let output = Command::new("cargo")
            .args(["run", "--offline", "--manifest-path"])
            .arg(helper.path().join("Cargo.toml"))
            .args(["--target-dir"])
            .arg(&target)
            .env("E34_LODESTAR_BIN", env!("CARGO_BIN_EXE_lodestar-mcp"))
            .env("E34_LODESTAR_ROOT", root.path())
            .env("E34_LODESTAR_PROFILE", profile)
            .output()
            .expect("ejecutar auxiliar oficial");
        assert!(
            output.status.success(),
            "cliente oficial {profile} falló: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
    }
}

struct RawWire {
    child: std::process::Child,
    stdin: std::process::ChildStdin,
    stdout: std::io::BufReader<std::process::ChildStdout>,
}

impl RawWire {
    fn start(root: &Path, profile: &str) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
            .arg("--root")
            .arg(root)
            .arg("--profile")
            .arg(profile)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        Self {
            stdin: child.stdin.take().unwrap(),
            stdout: std::io::BufReader::new(child.stdout.take().unwrap()),
            child,
        }
    }
    fn send(&mut self, value: serde_json::Value) -> (Vec<u8>, serde_json::Value) {
        writeln!(self.stdin, "{value}").unwrap();
        self.stdin.flush().unwrap();
        let mut raw = Vec::new();
        std::io::BufRead::read_until(&mut self.stdout, b'\n', &mut raw).unwrap();
        assert!(
            raw.ends_with(b"\n") && raw.len() > 1,
            "frame raw incompleto: {raw:?}"
        );
        let parsed =
            serde_json::from_slice(raw.strip_suffix(b"\n").unwrap()).expect("frame JSON exacto");
        (raw, parsed)
    }
    fn notify(&mut self, value: serde_json::Value) {
        writeln!(self.stdin, "{value}").unwrap();
        self.stdin.flush().unwrap();
    }
    fn finish(mut self) -> (std::process::ExitStatus, Vec<u8>, Vec<u8>) {
        drop(self.stdin);
        let mut trailing = Vec::new();
        self.stdout.read_to_end(&mut trailing).unwrap();
        let mut stderr = Vec::new();
        self.child
            .stderr
            .take()
            .unwrap()
            .read_to_end(&mut stderr)
            .unwrap();
        let status = self.child.wait().unwrap();
        (status, stderr, trailing)
    }
}

/// C4 — transcripts Modern y Legacy exactos en ambos perfiles, sin respuesta a notificaciones.
#[test]
fn stdout_stderr_eof_final() {
    for modern in [false, true] {
        for profile in ["standard", "readonly"] {
            let root = workspace_fixture();
            let mut wire = RawWire::start(root.path(), profile);
            let mut frames = Vec::new();
            if modern {
                frames.push(wire.send(modern_request(1, "server/discover", json!({}))));
                let (ping_raw, ping) = wire.send(modern_request(2, "ping", json!({})));
                assert_eq!(ping_raw, b"{\"jsonrpc\":\"2.0\",\"id\":2,\"error\":{\"code\":-32601,\"message\":\"ping\"}}\n");
                assert_eq!(ping["error"]["code"], -32601);
                frames.push((ping_raw, ping));
                wire.notify(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}));
                frames.push(wire.send(request(3, "tools/list", json!({}))));
                frames.push(wire.send(modern_request_version(
                    4,
                    "tools/list",
                    json!({}),
                    "2099-12-31",
                )));
                frames.push(wire.send(call_wire(5, true, "tool_that_does_not_exist", json!({}))));
            } else {
                frames.push(wire.send(initialize_request(1)));
                wire.notify(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
                let (ping_raw, ping) = wire.send(request(2, "ping", json!({})));
                assert_eq!(ping_raw, b"{\"jsonrpc\":\"2.0\",\"id\":2,\"result\":{}}\n");
                frames.push((ping_raw, ping));
                wire.notify(json!({"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":2}}));
                frames.push(wire.send(request(3, "server/discover", json!({}))));
                frames.push(wire.send(call_wire(4, false, "tool_that_does_not_exist", json!({}))));
            }
            let mut transcript = Vec::new();
            for (raw, value) in frames {
                assert_eq!(value["jsonrpc"], "2.0");
                let id = value["id"].as_u64().expect("id entero en transcript");
                let expected = match (modern, id) {
                    (true, 1) => expected_modern_discover(root.path(), profile),
                    (true, 2) => exact_frame(
                        json!({"jsonrpc":"2.0","id":2,"error":{"code":-32601,"message":"ping"}}),
                    ),
                    (true, 3) => expected_error(
                        3,
                        -32602,
                        "request _meta is missing or has malformed required fields: io.modelcontextprotocol/protocolVersion, io.modelcontextprotocol/clientCapabilities",
                        None,
                    ),
                    (true, 4) => expected_error(
                        4,
                        -32022,
                        "Unsupported protocol version",
                        Some(json!({"requested":"2099-12-31","supported":["2026-07-28"]})),
                    ),
                    (true, 5) => expected_error(
                        5,
                        -32602,
                        "tool desconocida: tool_that_does_not_exist",
                        None,
                    ),
                    (false, 1) => expected_legacy_initialize(root.path(), profile),
                    (false, 2) => exact_frame(json!({"jsonrpc":"2.0","id":2,"result":{}})),
                    (false, 3) => expected_error(3, -32601, "server/discover", None),
                    (false, 4) => expected_error(
                        4,
                        -32602,
                        "tool desconocida: tool_that_does_not_exist",
                        None,
                    ),
                    _ => panic!("id no esperado en transcript: {id}"),
                };
                assert_eq!(
                    raw, expected,
                    "frame raw exacto id={id}, profile={profile}, modern={modern}"
                );
                let expected_error = if modern {
                    match id {
                        2 => Some(-32601),
                        3 => Some(-32602),
                        4 => Some(-32022),
                        5 => Some(-32602),
                        _ => None,
                    }
                } else {
                    match id {
                        3 => Some(-32601),
                        4 => Some(-32602),
                        _ => None,
                    }
                };
                if let Some(code) = expected_error {
                    assert_eq!(value["error"]["code"], code, "negativo H06: {value}");
                } else {
                    assert!(
                        value["error"].is_null(),
                        "éxito esperado en transcript: {value}"
                    );
                }
                assert!(raw.ends_with(b"\n"));
                assert!(!raw
                    .windows(b"lodestar-mcp:".len())
                    .any(|w| w == b"lodestar-mcp:"));
                transcript.extend(raw);
            }
            assert_eq!(
                transcript.iter().filter(|byte| **byte == b'\n').count(),
                if modern { 5 } else { 4 }
            );
            let (status, stderr, trailing) = wire.finish();
            assert!(status.success(), "EOF debe terminar limpio: {status}");
            assert!(
                trailing.is_empty(),
                "notificación produjo frame tardío: {trailing:?}"
            );
            assert!(
                stderr.starts_with(b"lodestar-mcp:"),
                "logs solo en stderr: {stderr:?}"
            );
            assert!(!transcript.is_empty() && transcript.iter().all(|byte| *byte != 0));
            let ids: Vec<_> = transcript
                .split(|byte| *byte == b'\n')
                .filter(|line| !line.is_empty())
                .map(|line| {
                    serde_json::from_slice::<serde_json::Value>(line).unwrap()["id"].clone()
                })
                .collect();
            let expected_ids = if modern {
                vec![json!(1), json!(2), json!(3), json!(4), json!(5)]
            } else {
                vec![json!(1), json!(2), json!(3), json!(4)]
            };
            assert_eq!(ids, expected_ids, "ids/orden exactos");
        }
    }
}
