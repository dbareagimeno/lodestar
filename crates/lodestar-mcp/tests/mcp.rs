//! Test de integración del MCP (E7): handshake + tools/call sobre stdio. stdout debe ser JSON-RPC puro.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn write(dir: &std::path::Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

#[test]
fn handshake_y_tools_call_conformance() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(dir.path(), "malo.md", "# sin frontmatter\n");

    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();

    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());

    // initialize
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":1,"method":"initialize"}}"#).unwrap();
    // tools/list
    writeln!(stdin, r#"{{"jsonrpc":"2.0","id":2,"method":"tools/list"}}"#).unwrap();
    // tools/call conformance_check
    writeln!(
        stdin,
        r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"conformance_check","arguments":{{}}}}}}"#
    )
    .unwrap();
    stdin.flush().unwrap();
    drop(stdin);

    let mut lines = Vec::new();
    for line in (&mut stdout).lines().map_while(Result::ok) {
        lines.push(line);
        if lines.len() == 3 {
            break;
        }
    }
    child.wait().ok();

    // Cada línea de stdout es JSON-RPC válido (stdout puro).
    let init: serde_json::Value = serde_json::from_str(&lines[0]).unwrap();
    assert_eq!(init["result"]["serverInfo"]["name"], "lodestar-mcp");

    let list: serde_json::Value = serde_json::from_str(&lines[1]).unwrap();
    assert!(list["result"]["tools"]
        .as_array()
        .unwrap()
        .iter()
        .any(|t| t["name"] == "query"));

    let conf: serde_json::Value = serde_json::from_str(&lines[2]).unwrap();
    // malo.md sin frontmatter → hard_fail >= 1, no conforme.
    assert_eq!(conf["result"]["structuredContent"]["conform"], false);
    assert!(
        conf["result"]["structuredContent"]["hardFail"]
            .as_u64()
            .unwrap()
            >= 1
    );
}

/// Arranca el servidor sobre un bundle, envía `lines` y devuelve las primeras `expect` respuestas.
fn roundtrip(dir: &std::path::Path, lines: &[&str], expect: usize) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    for l in lines {
        writeln!(stdin, "{l}").unwrap();
    }
    stdin.flush().unwrap();
    drop(stdin);
    let mut out = Vec::new();
    for line in (&mut stdout).lines().map_while(Result::ok) {
        out.push(serde_json::from_str(&line).expect("stdout = JSON-RPC puro"));
        if out.len() == expect {
            break;
        }
    }
    child.wait().ok();
    out
}

fn bundle_min() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    dir
}

/// E2E del protocolo: parse error → -32700 (no silencio), ping → {}, método desconocido → -32601,
/// tool desconocida → -32602, error de EJECUCIÓN de tool → result con isError (no error JSON-RPC).
#[test]
fn protocolo_errores_y_ping() {
    let dir = bundle_min();
    let resp = roundtrip(
        dir.path(),
        &[
            "{esto no es json",
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"metodo/inexistente"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"no_existe","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"find_backlinks","arguments":{"concept":"../fuera.md"}}}"#,
        ],
        5,
    );
    assert_eq!(resp[0]["error"]["code"], -32700);
    assert_eq!(resp[0]["id"], serde_json::Value::Null);
    assert_eq!(resp[1]["result"], serde_json::json!({}));
    assert_eq!(resp[2]["error"]["code"], -32601);
    assert_eq!(resp[3]["error"]["code"], -32602);
    // RelPath inválido = error de ejecución de la tool → isError en el result, visible al modelo.
    assert_eq!(resp[4]["result"]["isError"], true);
    assert!(resp[4]["error"].is_null());
}

/// tools/list lleva inputSchema (obligatorio en el spec) y structuredContent siempre es objeto.
#[test]
fn tools_list_schema_y_structured_content_objeto() {
    let dir = bundle_min();
    write(
        dir.path(),
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"query","arguments":{"dsl":"is:orphan"}}}"#,
        ],
        2,
    );
    let tools = resp[0]["result"]["tools"].as_array().unwrap();
    // Conteo robusto a adiciones (E10-H08+ va añadiendo tools nuevas cada historia): el propósito
    // de este test es la FORMA (inputSchema de objeto en todas), no un total exacto. Se ancla con
    // un mínimo (las 10 heredadas + `workspace_status`) en vez de `==` para no quedar obsoleto en
    // cada historia de E10.
    assert!(
        tools.len() >= 11,
        "se esperaban al menos 11 tools (10 heredadas + workspace_status): {}",
        tools.len()
    );
    assert!(
        tools.iter().any(|t| t["name"] == "workspace_status"),
        "falta la tool «workspace_status» en tools/list: {tools:?}"
    );
    for t in tools {
        assert_eq!(
            t["inputSchema"]["type"], "object",
            "tool sin inputSchema: {}",
            t["name"]
        );
    }
    assert!(resp[1]["result"]["structuredContent"].is_object());
    assert!(resp[1]["result"]["structuredContent"]["paths"].is_array());
}

/// E2E de escritura: create_concept escribe el .md en disco (validado) y query lo encuentra.
#[test]
fn create_concept_escribe_y_query_lo_ve() {
    let dir = bundle_min();
    let resp = roundtrip(
        dir.path(),
        &[
            r##"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_concept","arguments":{"path":"nueva.md","type":"Nota","title":"Nueva","body":"# Resumen\n\ncuerpo\n"}}}"##,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"query","arguments":{"dsl":"type:nota"}}}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"conformance_check","arguments":{}}}"#,
        ],
        3,
    );
    assert_eq!(resp[0]["result"]["structuredContent"]["written"], true);
    assert!(dir.path().join("nueva.md").is_file(), "el .md es la verdad");
    let paths = resp[1]["result"]["structuredContent"]["paths"]
        .as_array()
        .unwrap();
    assert!(paths.iter().any(|p| p == "nueva.md"));
    assert_eq!(resp[2]["result"]["structuredContent"]["conform"], true);
}

/// Sin `body`, create_concept genera el heading por defecto `# {Tipo} - {Nombre}` en el .md.
#[test]
fn create_concept_sin_body_genera_heading_por_defecto() {
    let dir = bundle_min();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"create_concept","arguments":{"path":"otra.md","type":"Nota","title":"Otra"}}}"#,
        ],
        1,
    );
    assert_eq!(resp[0]["result"]["structuredContent"]["written"], true);
    let contenido = std::fs::read_to_string(dir.path().join("otra.md")).unwrap();
    assert!(
        contenido.contains("# Nota - Otra\n"),
        "falta el heading por defecto: {contenido}"
    );
}

/// initialize ecoa la protocolVersion del cliente si la soporta.
#[test]
fn initialize_ecoa_version_soportada() {
    let dir = bundle_min();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        ],
        1,
    );
    assert_eq!(resp[0]["result"]["protocolVersion"], "2025-03-26");
}

/// E9-H01 · Criterio `list_sin_tools_git`:
/// Dado un servidor MCP arrancado, Cuando un cliente pide `tools/list`, Entonces NO aparece
/// ninguna de las 3 tools git (`history`/`last_conforming_commit`/`commit`) en el catálogo.
#[test]
fn list_sin_tools_git() {
    let dir = bundle_min();
    let resp = roundtrip(
        dir.path(),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#],
        1,
    );
    let tools = resp[0]["result"]["tools"]
        .as_array()
        .expect("tools/list devuelve un array de tools");
    let git_tools = ["history", "last_conforming_commit", "commit"];
    let expuestas: Vec<&str> = tools
        .iter()
        .filter_map(|t| t["name"].as_str())
        .filter(|n| git_tools.contains(n))
        .collect();
    assert!(
        expuestas.is_empty(),
        "la superficie MCP no debe exponer tools git, pero aparecen: {expuestas:?}"
    );
}

/// E9-H01 · Criterio `call_commit_desconocida`:
/// Dado una petición `tools/call` con `name:"commit"`, Cuando se procesa, Entonces responde
/// error de tool desconocida (`-32602`) y NO la ejecuta (sin `result`).
#[test]
fn call_commit_desconocida() {
    let dir = bundle_min();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"commit","arguments":{"message":"intento"}}}"#,
        ],
        1,
    );
    // Tool desconocida = error de protocolo -32602, no una ejecución (isError o result poblado).
    assert_eq!(
        resp[0]["error"]["code"], -32602,
        "«commit» debe ser tool desconocida (-32602), no ejecutarse: {resp:?}"
    );
    assert!(
        resp[0]["result"].is_null(),
        "«commit» no debe producir result (no se ejecuta): {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// E10-H08 — Tool `workspace_status`.
//
// Ambos criterios se ejercitan e2e por stdio (campo Pruebas de la historia:
// `crates/lodestar-mcp/tests/`): `status_capabilities_readonly` DEPENDE del perfil con el que se
// arranca el servidor, así que el arnés tiene que poder lanzar el server con `--profile readonly`;
// `status_counts` va por el mismo camino para ejercitar la tool tal y como la ve un cliente MCP.
//
// CLI asumida (aún NO implementada — de ahí el ROJO): `lodestar-mcp <bundle> [--profile
// readonly|standard]`, por defecto `standard`. `capabilities.writes` = (perfil == standard).
// ---------------------------------------------------------------------------

/// Como [`roundtrip`], pero arranca el servidor con `--profile <profile>` tras el bundle.
/// El perfil aún no existe en producción: este helper documenta la superficie CLI que la historia
/// introduce y produce el ROJO cuando el flag / la tool todavía no están.
fn roundtrip_profile(
    dir: &std::path::Path,
    profile: &str,
    lines: &[&str],
    expect: usize,
) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg(dir)
        .arg("--profile")
        .arg(profile)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    for l in lines {
        writeln!(stdin, "{l}").unwrap();
    }
    stdin.flush().unwrap();
    drop(stdin);
    let mut out = Vec::new();
    for line in (&mut stdout).lines().map_while(Result::ok) {
        out.push(serde_json::from_str(&line).expect("stdout = JSON-RPC puro"));
        if out.len() == expect {
            break;
        }
    }
    child.wait().ok();
    out
}

/// Bundle con **exactamente 3 conceptos huérfanos**: un `index.md` raíz que NO enlaza a ninguno
/// (in_index vacío) más 3 `.md` conceptuales que no se enlazan entre sí ni reciben backlinks. Un
/// huérfano = concepto sin enlaces entrantes y ausente del índice (`bundle.rs` `compute_analysis`),
/// así que los 3 lo son y nadie más (index.md/log.md no cuentan como concepto).
fn bundle_con_tres_orphans() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    // index.md sin enlaces salientes: no "adopta" a ningún concepto.
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    for slug in ["uno", "dos", "tres"] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: Concept\ntitle: {slug}\ndescription: d\n---\n\n# H\n\ncuerpo suelto\n"
            ),
        );
    }
    dir
}

/// E10-H08 · Criterio `status_counts` (benchmark §17):
/// Dado un workspace con 3 orphans, Cuando se llama `workspace_status`, Entonces
/// `counts.orphans == 3` y `workspaceRevision` está presente (formato `blake3:…`).
#[test]
fn status_counts() {
    let dir = bundle_con_tres_orphans();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"workspace_status","arguments":{}}}"#,
        ],
        1,
    );
    let sc = &resp[0]["result"]["structuredContent"];
    assert_eq!(
        sc["counts"]["orphans"].as_u64(),
        Some(3),
        "workspace_status debe reportar counts.orphans == 3: {resp:?}"
    );
    let rev = sc["workspaceRevision"].as_str().unwrap_or("");
    assert!(
        rev.starts_with("blake3:"),
        "workspaceRevision ausente o mal formado (se esperaba «blake3:…»): {resp:?}"
    );
}

/// E10-H08 · Criterio `status_capabilities_readonly`:
/// Dado el perfil `readonly`, Cuando se llama `workspace_status`, Entonces
/// `capabilities.writes == false`. (Se añade el caso `standard ⇒ writes==true` para no ser vacuo:
/// que devuelva `false` siempre pasaría el criterio sin implementar la lógica del perfil.)
#[test]
fn status_capabilities_readonly() {
    let dir = bundle_con_tres_orphans();
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"workspace_status","arguments":{}}}"#;

    let ro = roundtrip_profile(dir.path(), "readonly", &[call], 1);
    assert_eq!(
        ro[0]["result"]["structuredContent"]["capabilities"]["writes"],
        serde_json::Value::Bool(false),
        "perfil readonly ⇒ capabilities.writes == false: {ro:?}"
    );

    let std = roundtrip_profile(dir.path(), "standard", &[call], 1);
    assert_eq!(
        std[0]["result"]["structuredContent"]["capabilities"]["writes"],
        serde_json::Value::Bool(true),
        "perfil standard ⇒ capabilities.writes == true: {std:?}"
    );
}

/// Un directorio que no es un bundle → exit 3 (no un servidor "feliz" sobre la nada).
#[test]
fn directorio_no_bundle_sale_con_3() {
    let dir = tempfile::tempdir().unwrap();
    let status = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap();
    assert_eq!(status.code(), Some(3));
}
