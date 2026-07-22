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

// ---------------------------------------------------------------------------
// E10-H09 — Tool `knowledge_search` (sustituye `query`).
//
// UBICACIÓN: los 3 criterios se ejercitan **e2e por la tool MCP** (campo Pruebas de la historia:
// `crates/lodestar-mcp/tests/`) en vez de contra `App::knowledge_search` directo. Razón deliberada:
// el contrato que importa fijar aquí es el de **wire** (nombres de campo del result, AUSENCIA de
// `body`, forma de `filters`, semántica de `nextCursor`) y probarlo por la frontera JSON-RPC lo fija
// sin acoplar los tests a los nombres internos de tipos Rust que el implementador aún no ha creado
// (`SearchFilters`/`SearchResults`/…). El parent sugirió app-directo como alternativa más simple para
// las 50 fixtures; se opta por e2e para no fijar tipos internos (el corpus de 50 se escribe en disco
// igual de fácil y el cursor autosuficiente se prueba mejor entre servidores frescos).
//
// CONTRATO fijado (fase ROJA — la tool aún NO existe, así que `tools/call` devuelve -32602 y
// `structuredContent.results` es nulo → los asserts fallan por AUSENCIA de la tool/servicio):
//   arguments: { text?: string, filters?: { types?: [...], statuses?, tags?, pathPrefix?, … },
//                sort?, limit?: 20 por defecto (máx 100), cursor?: string }
//   structuredContent: {
//     results: [ { path, id, type, title, status, description, tags, snippet, score, revision } ],
//     nextCursor: string | null,
//     totalApproximate: number
//   }
// `results[*]` NUNCA incluye la clave `body` (invariante de la historia: nunca cuerpos completos).
// La firma de servicio ASUMIDA (el implementador la crea con su propia elección de tipos):
//   App::knowledge_search(text, filters, sort, limit, cursor)
//       -> Result<{ results:[…], nextCursor, totalApproximate }, WorkspaceError>
// ---------------------------------------------------------------------------

/// Extrae los `path` de los `results` de una respuesta `knowledge_search`. Si la tool/servicio no
/// existe todavía (fase ROJA), `structuredContent.results` es nulo → panica con un mensaje que
/// documenta el porqué del rojo (la tool ausente), no un fallo espurio.
fn search_paths(resp: &serde_json::Value) -> Vec<String> {
    resp["result"]["structuredContent"]["results"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("knowledge_search debe devolver structuredContent.results (array): {resp:?}")
        })
        .iter()
        .map(|r| {
            r["path"]
                .as_str()
                .expect("cada result de knowledge_search lleva un `path` string")
                .to_string()
        })
        .collect()
}

/// Bundle con un concepto que casa el texto «autenticación» (en título y cuerpo) más un decoy que
/// NO casa: así el criterio no es vacuo (un stub que devuelva todo incluiría el decoy y fallaría).
fn bundle_autenticacion() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Auth](auth.md)\n",
    );
    write(
        dir.path(),
        "auth.md",
        "---\ntype: decision\ntitle: Autenticación con tokens\ndescription: Cómo autenticar usuarios\nstatus: accepted\ntags: [seguridad]\n---\n\n# Resumen\n\nDecidimos usar autenticación basada en tokens rotatorios.\n",
    );
    write(
        dir.path(),
        "bici.md",
        "---\ntype: concept\ntitle: Bicicletas\ndescription: sobre ruedas\n---\n\n# H\n\nnada que ver con el tema.\n",
    );
    dir
}

/// E10-H09 · Criterio `search_sin_cuerpos` (benchmark §17: "Encontrar una decisión por significado"):
/// Dado un corpus con un concepto que casa «autenticación», Cuando se busca ese texto, Entonces
/// aparece con `snippet` y `revision`, y SIN `body`.
#[test]
fn search_sin_cuerpos() {
    let dir = bundle_autenticacion();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_search","arguments":{"text":"autenticación"}}}"#,
        ],
        1,
    );
    let sc = &resp[0]["result"]["structuredContent"];
    let results = sc["results"].as_array().unwrap_or_else(|| {
        panic!("knowledge_search debe devolver structuredContent.results (array): {resp:?}")
    });

    // El concepto que casa aparece.
    let auth = results
        .iter()
        .find(|r| r["path"] == "auth.md")
        .unwrap_or_else(|| panic!("el concepto que casa «autenticación» debe aparecer: {resp:?}"));

    // `snippet` no vacío.
    let snippet = auth["snippet"].as_str().unwrap_or("");
    assert!(
        !snippet.is_empty(),
        "el result debe traer un `snippet` no vacío: {auth:?}"
    );

    // `revision` con formato de identidad de contenido `blake3:…` (ConceptRevision, E10-H03).
    let revision = auth["revision"].as_str().unwrap_or("");
    assert!(
        revision.starts_with("blake3:"),
        "el result debe traer `revision` con formato «blake3:…»: {auth:?}"
    );

    // NUNCA cuerpos: la clave `body` debe estar AUSENTE en TODOS los results (no basta con que sea
    // corta; se verifica la ausencia de la clave).
    for r in results {
        assert!(
            r.get("body").is_none(),
            "un result de knowledge_search NUNCA debe incluir la clave `body`: {r:?}"
        );
    }

    // No vacuo: un concepto que no casa el texto NO debe aparecer.
    assert!(
        !results.iter().any(|r| r["path"] == "bici.md"),
        "un concepto que no casa «autenticación» no debe aparecer en los resultados: {resp:?}"
    );
}

/// Bundle con conceptos `type:decision` mezclados con otros tipos, todos con el mismo texto en el
/// cuerpo para que el único discriminante sea el filtro de tipo.
fn bundle_tipos_mixtos() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    for slug in ["dec-uno", "dec-dos"] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: decision\ntitle: {slug}\ndescription: arquitectura\nstatus: accepted\n---\n\n# H\n\ncuerpo sobre arquitectura.\n"
            ),
        );
    }
    write(
        dir.path(),
        "nota.md",
        "---\ntype: nota\ntitle: Nota\ndescription: arquitectura\n---\n\n# H\n\ncuerpo sobre arquitectura.\n",
    );
    write(
        dir.path(),
        "concepto.md",
        "---\ntype: concept\ntitle: Concepto\ndescription: arquitectura\n---\n\n# H\n\ncuerpo sobre arquitectura.\n",
    );
    dir
}

/// E10-H09 · Criterio `search_filtra_tipo`:
/// Dado `filters.types:[decision]`, Cuando se busca, Entonces solo aparecen conceptos `type:decision`.
#[test]
fn search_filtra_tipo() {
    let dir = bundle_tipos_mixtos();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_search","arguments":{"text":"","filters":{"types":["decision"]}}}}"#,
        ],
        1,
    );
    let results = search_paths_values(&resp[0]);

    // No vacuo: debe haber al menos un resultado (si el filtro devolviese vacío, el `all` de abajo
    // pasaría trivialmente).
    assert!(
        !results.is_empty(),
        "con `filters.types:[decision]` debe haber al menos un resultado: {resp:?}"
    );

    // TODOS los resultados son `type:decision`.
    for r in &results {
        assert_eq!(
            r["type"], "decision",
            "`filters.types:[decision]` solo debe devolver conceptos type:decision, apareció: {r:?}"
        );
    }

    // No vacuo (segunda cara): un concepto de otro tipo NO aparece.
    assert!(
        !results.iter().any(|r| r["path"] == "nota.md"),
        "un concepto `type:nota` no debe aparecer al filtrar por decision: {resp:?}"
    );
}

/// Como [`search_paths`] pero devuelve los objetos `result` completos (no solo el `path`), para
/// aseverar sobre otros campos (`type`, `snippet`, …).
fn search_paths_values(resp: &serde_json::Value) -> Vec<serde_json::Value> {
    resp["result"]["structuredContent"]["results"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("knowledge_search debe devolver structuredContent.results (array): {resp:?}")
        })
        .clone()
}

/// Bundle con **50 conceptos** que casan todos el texto «paginacion» (en `description` y cuerpo),
/// deterministas por slug (`c00`…`c49`). El `index.md` no contiene el token y no cuenta como
/// concepto, así que la búsqueda casa exactamente 50.
fn bundle_cincuenta() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    for i in 0..50 {
        let slug = format!("c{i:02}");
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: concept\ntitle: Concepto {i:02}\ndescription: paginacion\n---\n\n# H\n\ncuerpo paginacion numero {i:02}.\n"
            ),
        );
    }
    dir
}

/// E10-H09 · Criterio `search_paginacion`:
/// Dado `limit:20` y 50 resultados, Cuando se pagina con `nextCursor`, Entonces la 2ª página no
/// repite ni omite. Se recorren las 3 páginas (20+20+10) y se verifica: partición determinista,
/// `nextCursor` presente hasta agotar, unión == 50 sin repetidos, y solapamiento nulo 1↔2.
#[test]
fn search_paginacion() {
    let dir = bundle_cincuenta();

    // Construye una línea `tools/call knowledge_search` con `limit:20` y un `cursor` opcional.
    let req = |cursor: Option<&str>| -> String {
        let mut args = serde_json::json!({ "text": "paginacion", "limit": 20 });
        if let Some(c) = cursor {
            args["cursor"] = serde_json::Value::String(c.to_string());
        }
        serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "tools/call",
            "params": { "name": "knowledge_search", "arguments": args }
        })
        .to_string()
    };

    // Página 1 (sin cursor).
    let p1 = roundtrip(dir.path(), &[req(None).as_str()], 1);
    let sc1 = &p1[0]["result"]["structuredContent"];
    let paths1 = search_paths(&p1[0]);
    assert_eq!(
        paths1.len(),
        20,
        "la página 1 con limit:20 debe traer 20 resultados: {p1:?}"
    );
    assert!(
        sc1["totalApproximate"].is_number(),
        "el result debe incluir `totalApproximate` numérico: {p1:?}"
    );
    let cursor1 = sc1["nextCursor"]
        .as_str()
        .unwrap_or_else(|| panic!("con 50>20 resultados `nextCursor` debe ser no nulo: {p1:?}"))
        .to_string();

    // Determinismo: la misma petición produce la misma partición y el mismo orden.
    let p1b = roundtrip(dir.path(), &[req(None).as_str()], 1);
    assert_eq!(
        search_paths(&p1b[0]),
        paths1,
        "mismo sort ⇒ misma partición determinista (mismo orden): {p1b:?}"
    );

    // Página 2, con el cursor de la página 1. Servidor FRESCO: el cursor debe ser autosuficiente y
    // determinista (no atado al estado de una sesión), o la 2ª página divergiría.
    let p2 = roundtrip(dir.path(), &[req(Some(&cursor1)).as_str()], 1);
    let sc2 = &p2[0]["result"]["structuredContent"];
    let paths2 = search_paths(&p2[0]);
    assert_eq!(
        paths2.len(),
        20,
        "la página 2 debe traer los siguientes 20 resultados: {p2:?}"
    );
    let cursor2 = sc2["nextCursor"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("quedan 10 resultados: `nextCursor` de la página 2 debe ser no nulo: {p2:?}")
        })
        .to_string();

    // Página 3: los 10 restantes; ya sin cursor (agotados).
    let p3 = roundtrip(dir.path(), &[req(Some(&cursor2)).as_str()], 1);
    let sc3 = &p3[0]["result"]["structuredContent"];
    let paths3 = search_paths(&p3[0]);
    assert_eq!(
        paths3.len(),
        10,
        "la página 3 debe traer los 10 conceptos restantes: {p3:?}"
    );
    assert!(
        sc3["nextCursor"].is_null(),
        "agotados los 50 resultados `nextCursor` debe ser null: {p3:?}"
    );

    // No repite ni omite: la unión de las 3 páginas cubre los 50 conceptos, todos únicos.
    let mut union: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for page in [&paths1, &paths2, &paths3] {
        for path in page {
            assert!(
                union.insert(path.clone()),
                "path repetido entre páginas (la paginación no debe repetir): {path}"
            );
        }
    }
    assert_eq!(
        union.len(),
        50,
        "la unión de las 3 páginas debe cubrir los 50 conceptos sin omisiones"
    );

    // Solapamiento nulo explícito entre página 1 y 2 (redacción literal del criterio).
    let en_p1: std::collections::BTreeSet<&String> = paths1.iter().collect();
    assert!(
        paths2.iter().all(|p| !en_p1.contains(p)),
        "la 2ª página no debe solapar con la 1ª"
    );
}
