//! E34-H03 — transporte rmcp 3.1.2 sobre stdio (fase roja).
//!
//! Estos tests arrancan el binario real y ejercitan el wire. La guarda estructural al final de
//! cada caso es deliberadamente discriminante: mientras `main.rs` conserve el lector manual de
//! `BufRead`/`read_line` y no use el transporte oficial rmcp/stdio, cada criterio permanece rojo.
//! No se inspecciona ni se modifica código de producción desde los tests.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};
use std::sync::{mpsc, Arc, Barrier};
use std::time::Duration;

use serde_json::{json, Value};

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
        "---\ntype: Note\ntitle: Nota\nestado: inicial\n---\n\n# Nota\n\ncontenido no vacío para E34-H03\n",
    );
    root
}

fn child(root: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("lodestar-mcp debe arrancar")
}

fn run_raw(root: &Path, input: &[u8]) -> Output {
    let mut process = child(root);
    process
        .stdin
        .take()
        .expect("stdin del proceso")
        .write_all(input)
        .expect("escribir bytes exactos al stdin");
    let stdout = process.stdout.take().expect("stdout del proceso");
    let stderr = process.stderr.take().expect("stderr del proceso");
    let stdout_task = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        BufReader::new(stdout)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let stderr_task = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        BufReader::new(stderr)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let status = loop {
        if let Some(status) = process.try_wait().expect("consultar estado del proceso") {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = process.kill();
            let _ = process.wait();
            panic!("lodestar-mcp no terminó dentro del timeout de 5s");
        }
        std::thread::sleep(Duration::from_millis(10));
    };
    let stdout = stdout_task
        .join()
        .expect("lector de stdout no entra en panic")
        .expect("leer stdout");
    let stderr = stderr_task
        .join()
        .expect("lector de stderr no entra en panic")
        .expect("leer stderr");
    Output {
        status,
        stdout,
        stderr,
    }
}

fn stdout_frames(output: &Output) -> Vec<Value> {
    let bytes = &output.stdout;
    assert!(
        !bytes.is_empty(),
        "stdout no puede estar vacío: falta toda respuesta MCP"
    );
    assert!(
        bytes.ends_with(b"\n"),
        "stdout debe terminar cada stream de frames con LF (se acepta CRLF), bytes={bytes:?}"
    );

    // String::lines descarta precisamente los frames vacíos que el contrato debe rechazar y
    // también oculta el terminador físico. Divide los bytes para que una línea vacía intermedia,
    // una línea extra tras el terminador o un frame sin terminador no puedan pasar desapercibidos.
    let mut lines: Vec<&[u8]> = bytes.split(|byte| *byte == b'\n').collect();
    assert_eq!(
        lines.pop(),
        Some(&b""[..]),
        "el último LF debe ser el terminador del último frame"
    );
    assert!(
        !lines.is_empty(),
        "stdout debe contener al menos un frame JSON no vacío"
    );

    lines
        .into_iter()
        .enumerate()
        .map(|(index, raw)| {
            let frame = raw.strip_suffix(b"\r").unwrap_or(raw);
            assert!(
                !frame.is_empty() && !frame.iter().all(u8::is_ascii_whitespace),
                "frame físico {index} vacío: stdout no puede ocultar líneas extra"
            );
            let value: Value = serde_json::from_slice(frame).unwrap_or_else(|error| {
                panic!("frame físico {index} debe ser JSON válido: {error}; bytes={frame:?}")
            });
            assert!(
                value.is_object(),
                "frame físico {index} debe ser un objeto JSON-RPC, no {value:?}"
            );
            value
        })
        .collect()
}

/// Error de ejecución de tool en el envelope MCP neutral y wire. La causa no puede reducirse a
/// isError: el código estable y el mensaje accionable demuestran que el rechazo proviene del
/// estado compartido que ya cambió tras la primera publicación.
fn assert_domain_conflict(response: &Value) {
    let result = response
        .get("result")
        .and_then(Value::as_object)
        .expect("un rechazo de tool debe conservar result como objeto MCP");
    assert!(
        response.get("error").is_none() || response["error"].is_null(),
        "el rechazo de dominio no puede convertirse en error JSON-RPC: {response}"
    );
    assert_eq!(
        result.get("isError"),
        Some(&Value::Bool(true)),
        "el rechazo de dominio debe marcar isError: {response}"
    );
    assert_eq!(
        result.len(),
        2,
        "envelope de error exacto: sólo content e isError, sin éxito/structuredContent: {response}"
    );
    let content = result
        .get("content")
        .and_then(Value::as_array)
        .expect("el rechazo de dominio debe llevar content");
    assert_eq!(
        content.len(),
        1,
        "el rechazo debe llevar un único mensaje de dominio"
    );
    assert_eq!(content[0]["type"], "text");
    let text = content[0]["text"]
        .as_str()
        .expect("content[0].text debe ser string")
        .trim();
    assert!(
        !text.is_empty(),
        "el mensaje de dominio no puede estar vacío"
    );
    let code = ["PLAN_STALE", "REVISION_CONFLICT"]
        .into_iter()
        .find(|code| text.starts_with(&format!("{code}:")))
        .expect("la causa debe ser PLAN_STALE o REVISION_CONFLICT con prefijo estable");
    assert!(
        text.len() > code.len() + 2,
        "el código debe ir acompañado de un mensaje accionable: {text}"
    );
    assert!(
        text.contains("change_plan") || text.contains("replanifica") || text.contains("workspace"),
        "el mensaje debe explicar el efecto/corrección del conflicto: {text}"
    );
}

fn assert_success(output: &Output) {
    assert!(
        output.status.success(),
        "el proceso bajo prueba debe terminar con éxito: status={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
}

fn command_output_with_timeout(mut command: Command, timeout: Duration) -> Output {
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut process = command.spawn().expect("arrancar comando auxiliar");
    let stdout = process.stdout.take().expect("stdout del comando auxiliar");
    let stderr = process.stderr.take().expect("stderr del comando auxiliar");
    let stdout_task = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        BufReader::new(stdout)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let stderr_task = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        BufReader::new(stderr)
            .read_to_end(&mut bytes)
            .map(|_| bytes)
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        if let Some(status) = process.try_wait().expect("consultar comando auxiliar") {
            break status;
        }
        if std::time::Instant::now() >= deadline {
            let _ = process.kill();
            let _ = process.wait();
            panic!("comando auxiliar agotó su timeout de {timeout:?}");
        }
        std::thread::sleep(Duration::from_millis(25));
    };
    Output {
        status,
        stdout: stdout_task
            .join()
            .expect("lector stdout auxiliar")
            .expect("leer stdout auxiliar"),
        stderr: stderr_task
            .join()
            .expect("lector stderr auxiliar")
            .expect("leer stderr auxiliar"),
    }
}

/// Elimina comentarios antes de revisar la estructura: una cadena en un comentario no puede
/// satisfacer la guarda ni ocultar el lector manual.
fn rust_code_without_comments(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut chars = source.chars().peekable();
    let mut line_comment = false;
    let mut block_comment = false;
    while let Some(ch) = chars.next() {
        if line_comment {
            if ch == '\n' {
                line_comment = false;
                output.push(ch);
            }
            continue;
        }
        if block_comment {
            if ch == '*' && chars.peek() == Some(&'/') {
                chars.next();
                block_comment = false;
            }
            continue;
        }
        if ch == '/' && chars.peek() == Some(&'/') {
            chars.next();
            line_comment = true;
        } else if ch == '/' && chars.peek() == Some(&'*') {
            chars.next();
            block_comment = true;
        } else {
            output.push(ch);
        }
    }
    output
}

/// Guarda estructural única de H03. Inspecciona todos los módulos Rust de `src` sin compartir el
/// fallo con los otros cuatro criterios; sólo el harness raw la llama.
fn assert_rmcp_stdio_transport_is_wired() {
    let src_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut paths = vec![src_dir.clone()];
    let mut sources = Vec::new();
    while let Some(path) = paths.pop() {
        for entry in std::fs::read_dir(&path).expect("se puede recorrer src de lodestar-mcp") {
            let entry = entry.expect("entrada de src legible");
            let entry_path = entry.path();
            if entry_path.is_dir() {
                paths.push(entry_path);
            } else if entry_path.extension().is_some_and(|ext| ext == "rs") {
                let source = std::fs::read_to_string(&entry_path)
                    .unwrap_or_else(|error| panic!("leer {}: {error}", entry_path.display()));
                sources.push(rust_code_without_comments(&source));
            }
        }
    }
    let combined = sources.join("\n");
    assert!(
        !combined.contains("std::io::stdin")
            && !combined.contains("BufRead")
            && !combined.contains("read_line")
            && !combined.contains(".lines()")
            && !combined.contains("serde_json::from_str")
            && !combined.contains("serde_json::from_slice"),
        "E34-H03: src aún contiene el lector/parsing manual de stdio"
    );
    assert!(
        combined.contains("rmcp::transport::stdio()")
            && (combined.contains("serve_server") || combined.contains(".serve("))
            && combined.contains(".run("),
        "E34-H03: src debe cablear rmcp::transport::stdio(), serve y executor.run real"
    );
    let main = rust_code_without_comments(
        &std::fs::read_to_string(src_dir.join("main.rs")).expect("leer main.rs"),
    );
    let lines: Vec<&str> = main.lines().collect();
    let serve_line = lines
        .iter()
        .position(|line| line.contains(".serve("))
        .expect("main.rs debe contener el receptor .serve(...)");
    let window_start = serve_line.saturating_sub(20);
    let window_end = (serve_line + 20).min(lines.len());
    let serve_block = lines[window_start..window_end].join("\n");
    assert!(
        serve_block.contains("rmcp::transport::stdio()")
            && serve_block.contains("SerialExecutor<LodestarMcpService>"),
        "main.rs debe vincular SerialExecutor<LodestarMcpService> con rmcp::transport::stdio() +         en el mismo adaptador que recibe .serve(...): {serve_block}"
    );
}

/// C1 — stdout sólo contiene respuestas JSON-RPC y el log de arranque queda en stderr.
#[test]
fn stdio_stdout_puro_y_logs_en_stderr() {
    let root = workspace_fixture();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/list"
    });
    let input = format!(
        "{}\n{}\n{}\n",
        r#"{"jsonrpc":"2.0","id":0,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"e34-h03","version":"1"}}}"#,
        r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#,
        request
    );
    let output = run_raw(root.path(), input.as_bytes());
    assert_success(&output);

    let frames = stdout_frames(&output);
    assert_eq!(
        frames.len(),
        2,
        "initialize y tools/list producen exactamente dos respuestas"
    );
    let initialize = frames
        .iter()
        .find(|frame| frame["id"] == 0)
        .expect("initialize debe producir una respuesta");
    assert_eq!(initialize["jsonrpc"], "2.0");
    let tools_frame = frames
        .iter()
        .find(|frame| frame["id"] == 1)
        .expect("tools/list debe conservar id=1");
    let tools = tools_frame["result"]["tools"]
        .as_array()
        .expect("tools/list debe devolver un array real");
    assert_eq!(
        tools.len(),
        10,
        "anti-vacuidad: el catálogo no puede estar vacío"
    );
    assert!(
        tools.iter().any(|tool| tool["name"] == "knowledge_search"),
        "anti-vacuidad: tools/list debe incluir una tool real"
    );

    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stderr.trim().is_empty(),
        "el proceso debe producir logs en stderr"
    );
    assert!(!String::from_utf8_lossy(&output.stdout).contains("lodestar-mcp:"));
    assert!(!output.stdout.windows(2).any(|window| window == b"\x1b["));
}

/// C2 — EOF, línea vacía, JSON inválido y notificación no bloquean ni inventan respuestas.
#[test]
fn stdio_eof_limpio() {
    let root = workspace_fixture();
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"e34-h03\",\"version\":\"1\"}}}\r\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n",
        "\n",
        "{\"jsonrpc\":\"2.0\",\"id\":7,\"method\":\"tools/list\"}\r\n",
        "not-json\n",
        "[]\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\n"
    );
    let output = run_raw(root.path(), input.as_bytes());
    assert_success(&output);
    let frames = stdout_frames(&output);

    // Blank, `not-json` y las notificaciones son silenciosos; quedan initialize, id=7 y el
    // request JSON bien formado con shape inválido (`[]`), que exige -32600 con id null.
    assert_eq!(
        frames.len(),
        3,
        "framing exacto: initialize + id=7 + id=null; blank/not-json/notification silenciosos"
    );
    assert!(frames.iter().any(|frame| frame["id"] == 0));
    assert!(frames.iter().any(|frame| frame["id"] == 7));
    let invalid_shape = frames
        .iter()
        .find(|frame| frame["error"]["code"] == -32600)
        .expect("un array JSON-RPC bien formado pero con shape inválido exige -32600");
    assert!(invalid_shape["id"].is_null());
    assert!(frames.iter().all(|frame| frame["jsonrpc"] == "2.0"));
}

const EXECUTOR_PROBE_MAIN: &str = r#"use lodestar_app::{App, Profile};
use lodestar_mcp::{LodestarMcpService, SerialExecutor};
use lodestar_workspace::failpoints::{self, PuntoDeGancho};
use serde_json::{json, Value};
use std::{env, error::Error, path::Path, sync::{mpsc, Arc, Barrier}, thread, time::Duration};

fn plan_args(value: &str) -> Value {
    json!({
        "operations": [{
            "op": "patch_frontmatter",
            "ref": {"path": "note.md"},
            "patch": {"estado": value}
        }],
        "policy": {"requireValidResult": false, "allowWarnings": true}
    })
}

fn change_set_id(value: &Value) -> Result<String, Box<dyn Error>> {
    Ok(value["structuredContent"]["changeSetId"]
        .as_str()
        .ok_or("missing changeSetId from neutral service")?
        .to_owned())
}

fn assert_domain_conflict(value: &Value) -> Result<(), Box<dyn Error>> {
    let object = value.as_object().ok_or("domain error envelope is not an object")?;
    if object.len() != 2 || !object.contains_key("content") || !object.contains_key("isError") {
        return Err(format!("unexpected domain error envelope: {value}").into());
    }
    if value["isError"] != true || !value["structuredContent"].is_null() {
        return Err(format!("domain error must be isError without success payload: {value}").into());
    }
    let content = value["content"]
        .as_array()
        .ok_or("domain error content is not an array")?;
    if content.len() != 1 || content[0]["type"] != "text" {
        return Err(format!("domain error content shape is invalid: {value}").into());
    }
    let text = content[0]["text"]
        .as_str()
        .ok_or("domain error text is not a string")?
        .trim();
    let code = ["PLAN_STALE", "REVISION_CONFLICT"]
        .into_iter()
        .find(|code| text.starts_with(&format!("{code}:")))
        .ok_or("domain error must expose PLAN_STALE or REVISION_CONFLICT")?;
    if text.len() <= code.len() + 2
        || (!text.contains("change_plan")
            && !text.contains("replanifica")
            && !text.contains("workspace"))
    {
        return Err(format!("domain error message is not actionable: {text}").into());
    }
    Ok(())
}

fn main() -> Result<(), Box<dyn Error>> {
    let root = env::var("E34_C3_ROOT")?;
    let app = App::open(Path::new(&root))?;
    let service = LodestarMcpService::new(app, Profile::Standard);
    let executor = SerialExecutor::new(service);

    let setup = tokio::runtime::Builder::new_current_thread().enable_all().build()?;
    let (id_a, id_b) = setup.block_on(async {
        let plan_a = executor.call("change_plan", plan_args("ganador-A")).await?;
        let plan_b = executor.call("change_plan", plan_args("ganador-B")).await?;
        Ok::<_, Box<dyn Error>>((change_set_id(&plan_a)?, change_set_id(&plan_b)?))
    })?;

    let (entered_tx, entered_rx) = mpsc::channel();
    let (returned_b_tx, returned_b_rx) = mpsc::channel();
    let gate = Arc::new(Barrier::new(2));

    let executor_a = executor.clone();
    let gate_a = Arc::clone(&gate);
    let thread_a = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        runtime.block_on(async move {
            let entered_tx = entered_tx;
            failpoints::armar_gancho(PuntoDeGancho::AntesDePublicar, move || {
                entered_tx.send(()).expect("señalar entrada de change_apply A");
                gate_a.wait();
            });
            let result = executor_a.call("change_apply", json!({"changeSetId": id_a})).await;
            failpoints::desarmar_ganchos();
            result
        })
    });
    entered_rx.recv_timeout(Duration::from_secs(5))?;

    let executor_b = executor.clone();
    let thread_b = thread::spawn(move || {
        let runtime = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        let result = runtime.block_on(async move {
            executor_b.call("change_apply", json!({"changeSetId": id_b})).await
        });
        returned_b_tx.send(result).expect("entregar resultado de change_apply B");
    });
    if returned_b_rx.recv_timeout(Duration::from_millis(200)).is_ok() {
        return Err("change_apply B retornó antes de liberar A: executor no serializa".into());
    }
    gate.wait();
    let result_a = thread_a.join().map_err(|_| "thread A panicked")??;
    let result_b = returned_b_rx.recv_timeout(Duration::from_secs(5))??;
    thread_b.join().map_err(|_| "thread B panicked")?;

    if result_a["structuredContent"]["applied"] != true {
        return Err(format!("A no publicó: {result_a}").into());
    }
    assert_domain_conflict(&result_b)?;
    let contents = std::fs::read_to_string(Path::new(&root).join("note.md"))?;
    if !contents.contains("ganador-A") || contents.contains("ganador-B") {
        return Err(format!(
            "el primer apply debe observarse en el estado compartido y descartar B: {contents}"
        )
        .into());
    }

    // Control anti-implementación «rechazar siempre el segundo apply»: sobre el estado ya
    // publicado, dos planes nuevos y válidos deben poder aplicarse consecutivamente. Así el
    // rechazo de B arriba tiene que proceder de la revisión/plan obsoleto, no de la posición de
    // la llamada en una secuencia.
    let sequence = SerialExecutor::new(LodestarMcpService::new(
        App::open(Path::new(&root))?,
        Profile::Standard,
    ));
    setup.block_on(async {
        let plan_c = sequence.call("change_plan", plan_args("ganador-C")).await?;
        let id_c = change_set_id(&plan_c)?;
        let applied_c = sequence
            .call("change_apply", json!({"changeSetId": id_c}))
            .await?;
        if applied_c["structuredContent"]["applied"] != true {
            return Err(format!("el primer apply válido de control falló: {applied_c}").into());
        }
        let plan_d = sequence.call("change_plan", plan_args("ganador-D")).await?;
        let id_d = change_set_id(&plan_d)?;
        let applied_d = sequence
            .call("change_apply", json!({"changeSetId": id_d}))
            .await?;
        if applied_d["structuredContent"]["applied"] != true {
            return Err(format!(
                "el segundo apply válido no puede rechazarse por posición: {applied_d}"
            )
            .into());
        }
        Ok::<_, Box<dyn Error>>(())
    })?;
    let contents = std::fs::read_to_string(Path::new(&root).join("note.md"))?;
    if !contents.contains("ganador-D") {
        return Err(format!("el segundo apply válido no dejó su efecto en disco: {contents}").into());
    }
    println!("E34_H03_SERIAL_OK:change_apply_A_then_B");
    Ok(())
}
"#;

fn run_executor_probe(root: &Path) -> Output {
    let helper = tempfile::tempdir().expect("tempdir para probe de SerialExecutor");
    let mcp_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let app_path = mcp_path.join("../lodestar-app");
    let workspace_path = mcp_path.join("../lodestar-workspace");
    let manifest = format!(
        "[package]\nname = \"e34-h03-executor-probe\"\nversion = \"0.1.0\"\nedition = \"2024\"\n\n[dependencies]\nlodestar-mcp = {{ path = \"{}\" }}\nlodestar-app = {{ path = \"{}\", features = [\"test-failpoints\"] }}\nlodestar-workspace = {{ path = \"{}\", features = [\"test-failpoints\"] }}\nserde_json = \"1\"\ntokio = {{ version = \"1\", features = [\"rt\", \"sync\", \"time\"] }}\n",
        mcp_path.display(),
        app_path.display(),
        workspace_path.display(),
    );
    write_file(helper.path(), "Cargo.toml", &manifest);
    write_file(helper.path(), "src/main.rs", EXECUTOR_PROBE_MAIN);
    let target_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/agent-state/e34-h03/executor-target");
    std::fs::create_dir_all(&target_dir).expect("crear target persistente del probe");
    let mut cargo = Command::new("cargo");
    cargo.args([
        "run",
        "--offline",
        "--manifest-path",
        helper.path().join("Cargo.toml").to_str().unwrap(),
        "--target-dir",
        target_dir.to_str().unwrap(),
    ]);
    cargo.env("E34_C3_ROOT", root);
    command_output_with_timeout(cargo, Duration::from_secs(90))
}

struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Option<BufReader<ChildStdout>>,
}

impl Running {
    fn start(root: &Path) -> Self {
        let mut process = child(root);
        let stdin = process.stdin.take().expect("stdin del servidor");
        let stdout = BufReader::new(process.stdout.take().expect("stdout del servidor"));
        let mut running = Self {
            child: process,
            stdin: Some(stdin),
            stdout: Some(stdout),
        };
        // C3 opera sobre la misma sesión wire que un cliente MCP real: initialize válido seguido
        // de notifications/initialized antes de cualquier tools/call.
        running.send(&json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "e34-h03", "version": "1"}
            }
        }));
        let initialize = running.receive();
        assert_eq!(initialize["id"], 0);
        assert_eq!(initialize["result"]["protocolVersion"], "2025-11-25");
        running.send(&json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized"
        }));
        running
    }

    fn send(&mut self, request: &Value) {
        let stdin = self.stdin.as_mut().expect("stdin del servidor");
        writeln!(stdin, "{request}").expect("escribir request");
        stdin.flush().expect("flush de request");
    }

    /// Un único escritor conserva la propiedad de stdio (un `ChildStdin` no es clonable). Dos
    /// productores quedan liberados exactamente por la misma barrera y entregan sus requests al
    /// escritor; así el envío es concurrente sin depender de APIs no portables de duplicación.
    fn send_concurrent(&mut self, requests: [Value; 2]) {
        let stdin = self.stdin.take().expect("stdin del servidor");
        let barrier = Arc::new(Barrier::new(3));
        let (tx, rx) = mpsc::channel();
        let writer_barrier = Arc::clone(&barrier);
        let writer = std::thread::spawn(move || {
            writer_barrier.wait();
            let first = rx.recv().expect("primer request concurrente");
            let second = rx.recv().expect("segundo request concurrente");
            let mut stdin = stdin;
            writeln!(stdin, "{first}").expect("escribir primer request concurrente");
            writeln!(stdin, "{second}").expect("escribir segundo request concurrente");
            stdin.flush().expect("flush de requests concurrentes");
            stdin
        });
        let mut producers = Vec::new();
        for request in requests {
            let barrier = Arc::clone(&barrier);
            let tx = tx.clone();
            producers.push(std::thread::spawn(move || {
                barrier.wait();
                tx.send(request).expect("entregar request al escritor");
            }));
        }
        drop(tx);
        for producer in producers {
            producer
                .join()
                .expect("productor concurrente no entra en panic");
        }
        self.stdin = Some(
            writer
                .join()
                .expect("escritor concurrente no entra en panic"),
        );
    }

    fn receive(&mut self) -> Value {
        let mut stdout = self.stdout.take().expect("stdout del servidor");
        let (tx, rx) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            let mut line = String::new();
            let result = stdout.read_line(&mut line);
            tx.send((stdout, result, line))
                .expect("devolver resultado de lectura");
        });
        let (stdout_back, result, line) =
            rx.recv_timeout(Duration::from_secs(5)).unwrap_or_else(|_| {
                let _ = self.child.kill();
                let _ = self.child.wait();
                panic!("lectura de respuesta agotó el timeout de 5s")
            });
        // Keep the join explicit: the child pipe must not outlive the test thread.
        let _ = reader.join();
        self.stdout = Some(stdout_back);
        let count = result.expect("leer respuesta");
        assert!(count > 0, "el servidor cerró stdout antes de responder");
        serde_json::from_str(&line).expect("respuesta JSON-RPC válida")
    }

    fn finish(mut self) {
        drop(self.stdin.take());
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = self.child.try_wait().expect("esperar proceso") {
                break status;
            }
            if std::time::Instant::now() >= deadline {
                let _ = self.child.kill();
                panic!("el proceso no terminó dentro del timeout de 5s");
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        assert!(status.success(), "el proceso terminó con {status}");
    }
}

/// C3 — dos aplicaciones concurrentes se serializan y una observa el efecto de la otra.
#[test]
fn stdio_concurrencia_executor_serial() {
    let probe_root = workspace_fixture();
    let probe = run_executor_probe(probe_root.path());
    if !probe.status.success() {
        let stderr = String::from_utf8_lossy(&probe.stderr);
        assert!(
            stderr.contains("SerialExecutor"),
            "el probe C3 sólo puede fallar por la API SerialExecutor ausente: {stderr}"
        );
        panic!("E34-H03 C3 rojo: el probe SerialExecutor no compila aún; stderr={stderr}");
    }
    assert!(
        String::from_utf8_lossy(&probe.stdout).contains("E34_H03_SERIAL_OK:change_apply_A_then_B"),
        "el probe SerialExecutor debe demostrar exclusión y orden: stdout={:?}, stderr={:?}",
        String::from_utf8_lossy(&probe.stdout),
        String::from_utf8_lossy(&probe.stderr)
    );

    // El probe puede publicar A en su propio fixture; el wire A/B comienza siempre sobre una base
    // fresca y no hereda estado ni revisiones del proceso auxiliar.
    let root = workspace_fixture();
    let mut server = Running::start(root.path());
    server.send(&json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "tools/call",
        "params": {
            "name": "change_plan",
            "arguments": {
                "operations": [{
                "op": "patch_frontmatter",
                "ref": {"path": "note.md"},
                    "patch": {"estado": "ganador-A"}
                }],
                "policy": {"requireValidResult": false, "allowWarnings": true}
            }
        }
    }));
    let plan_a = server.receive();
    let id_a = plan_a["result"]["structuredContent"]["changeSetId"]
        .as_str()
        .unwrap_or_else(|| panic!("plan A debe producir un id real: {plan_a:?}"))
        .to_owned();

    server.send(&json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/call",
        "params": {
            "name": "change_plan",
            "arguments": {
                "operations": [{
                    "op": "patch_frontmatter",
                    "ref": {"path": "note.md"},
                    "patch": {"estado": "ganador-B"}
                }],
                "policy": {"requireValidResult": false, "allowWarnings": true}
            }
        }
    }));
    let plan_b = server.receive();
    let id_b = plan_b["result"]["structuredContent"]["changeSetId"]
        .as_str()
        .unwrap_or_else(|| panic!("plan B debe producir un id real: {plan_b:?}"))
        .to_owned();
    assert_ne!(
        id_a, id_b,
        "planes A/B distintos deben tener changeSetId distintos"
    );
    assert!(
        std::fs::read_to_string(root.path().join("note.md"))
            .unwrap()
            .contains("estado: inicial"),
        "planificar no puede publicar ninguno de los dos efectos"
    );

    // Dos productores liberados por una barrera entregan ambos apply antes de leer respuestas: el
    // orden observable lo decide el executor, no el arnés. El segundo apply debe observar que el
    // primero ya consumió el plan.
    server.send_concurrent([
        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "method": "tools/call",
            "params": {"name": "change_apply", "arguments": {"changeSetId": id_a}}
        }),
        json!({
            "jsonrpc": "2.0",
            "id": 4,
            "method": "tools/call",
            "params": {"name": "change_apply", "arguments": {"changeSetId": id_b}}
        }),
    ]);
    let responses = [server.receive(), server.receive()];
    let mut by_id = BTreeMap::new();
    for response in responses {
        by_id.insert(response["id"].as_u64().expect("id numérico"), response);
    }
    assert_eq!(
        by_id.len(),
        2,
        "no puede haber respuestas cruzadas ni ids duplicados"
    );
    let applied = by_id
        .values()
        .filter(|response| response["result"]["structuredContent"]["applied"] == true)
        .count();
    assert_eq!(
        applied, 1,
        "exactamente una aplicación debe publicar el cambio"
    );
    let winner = by_id
        .iter()
        .find(|(_, response)| response["result"]["structuredContent"]["applied"] == true)
        .map(|(id, _)| *id)
        .expect("una de las dos respuestas debe ser la ganadora");
    let loser = if winner == 3 { 4 } else { 3 };
    assert_domain_conflict(&by_id[&loser]);
    let contents = std::fs::read_to_string(root.path().join("note.md")).unwrap();
    let expected = if winner == 3 {
        "ganador-A"
    } else {
        "ganador-B"
    };
    let discarded = if winner == 3 {
        "ganador-B"
    } else {
        "ganador-A"
    };
    assert!(
        contents.contains(expected),
        "el changeSetId ganador debe publicar su efecto: {contents}"
    );
    assert!(
        !contents.contains(discarded),
        "el changeSetId perdedor no debe filtrarse: {contents}"
    );
    server.finish();

    // Control anti-implementación «rechazar siempre el segundo apply»: tras una primera
    // publicación, un plan nuevo sobre la revisión vigente y su segundo apply también deben
    // completarse. El rechazo observado arriba debe ser causal (plan obsoleto), no posicional.
    let sequence_root = workspace_fixture();
    let mut sequence = Running::start(sequence_root.path());
    sequence.send(&json!({
        "jsonrpc": "2.0",
        "id": 10,
        "method": "tools/call",
        "params": {
            "name": "change_plan",
            "arguments": {
                "operations": [{
                    "op": "patch_frontmatter",
                    "ref": {"path": "note.md"},
                    "patch": {"estado": "secuencia-C"}
                }],
                "policy": {"requireValidResult": false, "allowWarnings": true}
            }
        }
    }));
    let plan_c = sequence.receive();
    let id_c = plan_c["result"]["structuredContent"]["changeSetId"]
        .as_str()
        .expect("plan C de control debe producir changeSetId")
        .to_owned();
    sequence.send(&json!({
        "jsonrpc": "2.0",
        "id": 11,
        "method": "tools/call",
        "params": {"name": "change_apply", "arguments": {"changeSetId": id_c}}
    }));
    let applied_c = sequence.receive();
    assert_eq!(applied_c["result"]["structuredContent"]["applied"], true);

    sequence.send(&json!({
        "jsonrpc": "2.0",
        "id": 12,
        "method": "tools/call",
        "params": {
            "name": "change_plan",
            "arguments": {
                "operations": [{
                    "op": "patch_frontmatter",
                    "ref": {"path": "note.md"},
                    "patch": {"estado": "secuencia-D"}
                }],
                "policy": {"requireValidResult": false, "allowWarnings": true}
            }
        }
    }));
    let plan_d = sequence.receive();
    let id_d = plan_d["result"]["structuredContent"]["changeSetId"]
        .as_str()
        .expect("plan D de control debe producir changeSetId")
        .to_owned();
    sequence.send(&json!({
        "jsonrpc": "2.0",
        "id": 13,
        "method": "tools/call",
        "params": {"name": "change_apply", "arguments": {"changeSetId": id_d}}
    }));
    let applied_d = sequence.receive();
    assert_eq!(
        applied_d["result"]["structuredContent"]["applied"], true,
        "el segundo apply válido no puede rechazarse por posición: {applied_d}"
    );
    let sequence_contents = std::fs::read_to_string(sequence_root.path().join("note.md")).unwrap();
    assert!(
        sequence_contents.contains("secuencia-D"),
        "el segundo apply válido debe dejar su efecto en disco: {sequence_contents}"
    );
    sequence.finish();
}

const OFFICIAL_CLIENT_CARGO: &str = r#"[package]
name = "e34-h03-official-client"
version = "0.1.0"
edition = "2024"

[dependencies]
rmcp = { version = "=3.1.2", default-features = false, features = ["client", "transport-async-rw"] }
tokio = { version = "1", features = ["macros", "fs", "io-util", "rt-multi-thread", "time"] }
"#;

fn package_field(manifest: &str, field: &str) -> Option<String> {
    let mut in_package = false;
    for line in manifest.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if !in_package || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        if key.trim() == field {
            return Some(value.trim().trim_matches('"').to_owned());
        }
    }
    None
}

fn toml_basic_string(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', "\\n")
        .replace('\r', "\\r")
        .replace('\t', "\\t");
    format!("\"{escaped}\"")
}

fn official_client_manifest(tokio_stream_source: Option<&Path>) -> String {
    let Some(source) = tokio_stream_source else {
        return OFFICIAL_CLIENT_CARGO.to_owned();
    };
    format!(
        "{OFFICIAL_CLIENT_CARGO}\n[patch.crates-io]\ntokio-stream = {{ path = {} }}\n",
        toml_basic_string(
            source
                .to_str()
                .expect("la ruta de tokio-stream debe ser UTF-8")
        )
    )
}

fn configured_tokio_stream_source() -> Option<PathBuf> {
    let raw = std::env::var_os("E34_TOKIO_STREAM_SOURCE")?;
    let source = std::fs::canonicalize(PathBuf::from(raw))
        .expect("E34_TOKIO_STREAM_SOURCE debe apuntar a un directorio existente");
    assert!(
        source.is_dir(),
        "E34_TOKIO_STREAM_SOURCE debe apuntar al directorio del crate tokio-stream"
    );
    let cargo_toml = source.join("Cargo.toml");
    let manifest = std::fs::read_to_string(&cargo_toml).unwrap_or_else(|error| {
        panic!(
            "leer el Cargo.toml de E34_TOKIO_STREAM_SOURCE ({}): {error}",
            cargo_toml.display()
        )
    });
    assert_eq!(
        package_field(&manifest, "name").as_deref(),
        Some("tokio-stream"),
        "la fuente configurada debe declarar package.name = tokio-stream"
    );
    assert_eq!(
        package_field(&manifest, "version").as_deref(),
        Some("0.1.17"),
        "la fuente configurada debe declarar package.version = 0.1.17"
    );
    let receiver_stream = source.join("src/wrappers/mpsc_bounded.rs");
    let receiver_source = std::fs::read_to_string(&receiver_stream).unwrap_or_else(|error| {
        panic!(
            "leer ReceiverStream de la fuente configurada ({}): {error}",
            receiver_stream.display()
        )
    });
    assert!(
        receiver_source.contains("ReceiverStream"),
        "la fuente configurada debe incluir el wrapper oficial ReceiverStream"
    );
    Some(source)
}

const OFFICIAL_CLIENT_MAIN: &str = r#"use rmcp::{ClientHandler, ServiceExt};
use std::{env, error::Error, process::{Command, Stdio}, time::Duration};

#[derive(Default)]
struct OfficialClient;
impl ClientHandler for OfficialClient {}

#[cfg(unix)]
fn tokio_file_from_stdout(stdout: std::process::ChildStdout) -> std::fs::File {
    use std::os::unix::io::{FromRawFd, IntoRawFd};
    // The ChildStdout owns this descriptor; ownership moves into std::fs::File exactly once.
    unsafe { std::fs::File::from_raw_fd(stdout.into_raw_fd()) }
}

#[cfg(windows)]
fn tokio_file_from_stdout(stdout: std::process::ChildStdout) -> std::fs::File {
    use std::os::windows::io::{FromRawHandle, IntoRawHandle};
    // The ChildStdout owns this handle; ownership moves into std::fs::File exactly once.
    unsafe { std::fs::File::from_raw_handle(stdout.into_raw_handle()) }
}

#[cfg(unix)]
fn tokio_file_from_stdin(stdin: std::process::ChildStdin) -> std::fs::File {
    use std::os::unix::io::{FromRawFd, IntoRawFd};
    // The ChildStdin owns this descriptor; ownership moves into std::fs::File exactly once.
    unsafe { std::fs::File::from_raw_fd(stdin.into_raw_fd()) }
}

#[cfg(windows)]
fn tokio_file_from_stdin(stdin: std::process::ChildStdin) -> std::fs::File {
    use std::os::windows::io::{FromRawHandle, IntoRawHandle};
    // The ChildStdin owns this handle; ownership moves into std::fs::File exactly once.
    unsafe { std::fs::File::from_raw_handle(stdin.into_raw_handle()) }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let mut command = Command::new(env::var("E34_LODESTAR_BIN")?);
    command.arg("--root").arg(env::var("E34_LODESTAR_ROOT")?);
    command.stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::inherit());
    let mut child = command.spawn()?;
    let stdout = tokio::fs::File::from_std(tokio_file_from_stdout(
        child.stdout.take().ok_or("missing child stdout")?,
    ));
    let stdin = tokio::fs::File::from_std(tokio_file_from_stdin(
        child.stdin.take().ok_or("missing child stdin")?,
    ));
    let mut client = tokio::time::timeout(Duration::from_secs(10), OfficialClient.serve((stdout, stdin))).await??;
    let tools = tokio::time::timeout(Duration::from_secs(10), client.list_all_tools()).await??;
    if !tools.iter().any(|tool| tool.name == "knowledge_get") {
        return Err("official rmcp client did not discover a real tool".into());
    }
    client.cancel().await?;
    let status = child.wait()?;
    if !status.success() {
        return Err(format!("lodestar-mcp exited with {status}").into());
    }
    Ok(())
}
"#;

/// C4 — un crate auxiliar real, aislado del feature set de `lodestar-mcp`, usa el cliente oficial
/// rmcp para conectar el binario hijo, completar initialize/list y descubrir una tool real.
#[test]
fn rmcp_official_discovery_initialize() {
    let root = workspace_fixture();
    let helper = tempfile::tempdir().expect("tempdir para cliente rmcp oficial");
    let tokio_stream_source = configured_tokio_stream_source();
    let official_client_cargo = official_client_manifest(tokio_stream_source.as_deref());
    write_file(helper.path(), "Cargo.toml", &official_client_cargo);
    write_file(helper.path(), "src/main.rs", OFFICIAL_CLIENT_MAIN);
    assert!(
        official_client_cargo.contains("rmcp = { version = \"=3.1.2\"")
            && official_client_cargo.contains("transport-async-rw")
            && OFFICIAL_CLIENT_MAIN.contains("ClientHandler")
            && OFFICIAL_CLIENT_MAIN.contains("ServiceExt")
            && OFFICIAL_CLIENT_MAIN.contains("tokio::fs::File::from_std")
            && OFFICIAL_CLIENT_MAIN.contains("Command::new")
            && !OFFICIAL_CLIENT_MAIN.contains("tokio::process")
            && OFFICIAL_CLIENT_MAIN.contains(".serve((stdout, stdin))")
            && OFFICIAL_CLIENT_MAIN.contains("list_all_tools")
            && OFFICIAL_CLIENT_MAIN.contains("knowledge_get"),
        "el auxiliar C4 debe contener el cliente oficial rmcp real y una aserción de tool"
    );
    if let Some(source) = tokio_stream_source.as_ref() {
        assert!(
            official_client_cargo.contains("[patch.crates-io]")
                && official_client_cargo.contains(&format!(
                    "tokio-stream = {{ path = {} }}",
                    toml_basic_string(source.to_str().expect("ruta UTF-8"))
                )),
            "C4 debe usar explícitamente la fuente upstream configurada, no una dependencia implícita"
        );
    }
    let target_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/agent-state/e34-h03/client-target");
    std::fs::create_dir_all(&target_dir).expect("crear target persistente del auxiliar");

    let mut cargo = Command::new("cargo");
    cargo.args([
        "run",
        "--manifest-path",
        helper.path().join("Cargo.toml").to_str().unwrap(),
        "--target-dir",
        target_dir.to_str().unwrap(),
    ]);
    let ci = std::env::var_os("CI").is_some();
    if !ci {
        cargo.arg("--offline");
    }
    cargo
        .env("E34_LODESTAR_BIN", env!("CARGO_BIN_EXE_lodestar-mcp"))
        .env("E34_LODESTAR_ROOT", root.path());
    let output = command_output_with_timeout(cargo, Duration::from_secs(90));
    let missing_tokio_stream = String::from_utf8_lossy(&output.stderr)
        .contains("no matching package named `tokio-stream` found");
    if !ci && missing_tokio_stream {
        panic!(
            concat!(
                "E34-H03 C4 no ejecutado: el cliente oficial rmcp 3.1.2 requiere tokio-stream, ",
                "que no está disponible en el caché local; el test no puede aprobar por omisión. ",
                "stderr={}"
            ),
            String::from_utf8_lossy(&output.stderr)
        );
    }
    assert_success(&output);
}

/// Harness raw — bytes exactos, CRLF, notificación y request inválida, con captura separada.
#[test]
fn harness_raw_frames_exactos() {
    let root = workspace_fixture();
    let input = concat!(
        "{\"jsonrpc\":\"2.0\",\"id\":0,\"method\":\"initialize\",\"params\":{\"protocolVersion\":\"2025-11-25\",\"capabilities\":{},\"clientInfo\":{\"name\":\"e34-h03\",\"version\":\"1\"}}}\r\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\r\n",
        "{\"jsonrpc\":\"2.0\",\"id\":11,\"method\":\"tools/list\"}\r\n",
        "{\"jsonrpc\":\"2.0\",\"method\":\"notifications/initialized\"}\r\n",
        "{\"jsonrpc\":\"2.0\",\"id\":12,\"method\":\"method/unknown\"}\r\n"
    );
    let output = run_raw(root.path(), input.as_bytes());
    assert_success(&output);
    let frames = stdout_frames(&output);
    assert_eq!(frames.len(), 3, "initialized no debe producir frame");
    assert!(frames.iter().any(|frame| frame["id"] == 0));
    let tools_frame = frames
        .iter()
        .find(|frame| frame["id"] == 11)
        .expect("tools/list debe conservar id=11");
    let unknown_frame = frames
        .iter()
        .find(|frame| frame["id"] == 12)
        .expect("method desconocido debe conservar id=12");
    assert!(tools_frame["result"]["tools"]
        .as_array()
        .is_some_and(|tools| !tools.is_empty()));
    assert_eq!(unknown_frame["error"]["code"], -32601);
    assert!(output.stderr.starts_with(b"lodestar-mcp:"));
    assert!(output.stdout.iter().all(|byte| *byte != 0));

    assert_rmcp_stdio_transport_is_wired();
}
