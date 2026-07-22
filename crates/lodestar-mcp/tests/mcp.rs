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

// ---------------------------------------------------------------------------
// E10-H10 — Tool `knowledge_get` (sustituye la lectura directa).
//
// UBICACIÓN: los 3 criterios se ejercitan **e2e por la tool MCP** (campo Pruebas de la historia:
// `crates/lodestar-mcp/tests/`), igual que E10-H09. Razón deliberada (misma que H09): lo que hay que
// fijar es el contrato de **wire** (forma de `arguments`, forma del `concept` en `structuredContent`,
// acotado de body por sección, cómo aflora el error `CONCEPT_NOT_FOUND`) sin acoplar los tests a los
// nombres de tipos Rust internos que el implementador aún no ha elegido (el tipo de retorno del
// servicio, el enum/lista de `include`, etc.). El parent ofreció como alternativa probar
// `App::knowledge_get` directo; se opta por e2e para (a) no fijar el tipo de retorno interno y (b) no
// tener que añadir un stub en `src/` (la tool ausente da un ROJO limpio en runtime, sin tocar
// producción y sin romper la compilación del resto de la suite).
//
// FASE ROJA: la tool `knowledge_get` NO existe todavía → `tools::exists("knowledge_get")` es `false`
// → `tools/call` responde el error de protocolo -32602 (sin `result`). Por eso `structuredContent`
// es nulo y los asserts fallan por AUSENCIA de la tool/servicio, no por un fallo espurio.
//
// CONTRATO DE WIRE fijado por esta historia (lo que el implementador debe respetar):
//   arguments: {
//     ref: { path: "<RelPath>" },                 // ConceptRef (E10-H04); deser de { "path": … }
//     include?: [ "frontmatter" | "body" | "revision" | "outgoingLinks" | "backlinks"
//                 | "diagnostics" | "externalReferences" ],   // selectivo: qué campos se pueblan
//     sections?: [ [ "<heading>", "<subheading>", … ] ]       // cada headingPath acota el body
//   }
//   structuredContent: {
//     concept: { path, revision, frontmatter?, body?, outgoingLinks?, backlinks?,
//                externalReferences?, diagnostics? }
//   }
// `concept.revision` == `ConceptRevision` (E10-H03), formato `blake3:…`, presente siempre (identidad).
// Un campo NO pedido en `include` NO se puebla (queda nulo/ausente).
//
// FIRMA DE SERVICIO ASUMIDA (el implementador la crea con su propia elección de tipos internos):
//   App::knowledge_get(r: &ConceptRef, include: &[…], sections: Option<&[…]>)
//       -> Result<{ concept: { path, revision, frontmatter, body, outgoingLinks, backlinks,
//                              externalReferences, diagnostics } }, ErrorCode>
//   con `CONCEPT_NOT_FOUND` cuando `resolve_ref` no encuentra el path (E10-H04).
// ---------------------------------------------------------------------------

/// Bundle con un concepto conforme `alfa.md` (frontmatter completo) para los casos que solo necesitan
/// un concepto existente al que pedirle `revision`/`frontmatter`, y para el caso inexistente (pedir un
/// path que NO está en el bundle).
fn bundle_get_revision() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Alfa](alfa.md)\n",
    );
    write(
        dir.path(),
        "alfa.md",
        "---\ntype: decision\ntitle: Alfa\ndescription: Primer concepto\nstatus: accepted\ntags: [seguridad]\n---\n\n# Resumen\n\nCuerpo del concepto alfa.\n",
    );
    dir
}

/// E10-H10 · Criterio `get_incluye_revision`:
/// Dado un concepto existente, Cuando se pide con `include:[frontmatter,revision]`, Entonces devuelve
/// la `revision` (== `ConceptRevision`, formato `blake3:…`) y el `frontmatter`. Se añade que un campo
/// NO pedido (`body`) queda sin poblar, para que el `include` selectivo sea significativo (no vacuo).
#[test]
fn get_incluye_revision() {
    let dir = bundle_get_revision();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"alfa.md"},"include":["frontmatter","revision"]}}}"#,
        ],
        1,
    );
    let concept = &resp[0]["result"]["structuredContent"]["concept"];

    // `revision` presente y con formato de identidad de contenido `blake3:…` (ConceptRevision, E10-H03).
    let revision = concept["revision"].as_str().unwrap_or_else(|| {
        panic!("knowledge_get debe devolver concept.revision (string «blake3:…»): {resp:?}")
    });
    assert!(
        revision.starts_with("blake3:"),
        "concept.revision debe tener formato «blake3:…»: {resp:?}"
    );

    // `frontmatter` presente (objeto no nulo) porque se pidió en `include`.
    assert!(
        concept["frontmatter"].is_object(),
        "con include:[frontmatter] el concept debe traer un `frontmatter` (objeto): {resp:?}"
    );

    // `include` selectivo: `body` NO se pidió ⇒ no se puebla (nulo o ausente). Sin esta comprobación
    // el criterio sería vacuo (una impl que devuelve todos los campos siempre lo cumpliría igual).
    assert!(
        concept["body"].is_null(),
        "con include:[frontmatter,revision] el `body` NO debe poblarse: {resp:?}"
    );
}

/// Bundle con un concepto cuyo cuerpo tiene una jerarquía de headings clara: `## Security` con la
/// subsección objetivo `### Token rotation`, más secciones/subsecciones hermanas que DEBEN quedar
/// fuera al acotar por `sections:[["Security","Token rotation"]]`. Cada bloque lleva un marcador único
/// para que las comprobaciones de subcadena sean inequívocas:
///   - `TOKEN-OBJETIVO-INCLUIR`  → bajo `## Security → ### Token rotation` (DEBE aparecer).
///   - `TOKEN-HERMANA-SUB-EXCLUIR` → bajo `## Security → ### Otra` (subsección hermana; DEBE quedar
///     fuera; su exclusión obliga a que el 2º nivel del headingPath cuente, no solo `## Security`).
///   - `TOKEN-HERMANA-TOP-EXCLUIR` → bajo `## Otra seccion` (sección hermana de nivel superior; fuera).
///   - `TOKEN-OVERVIEW-EXCLUIR`   → bajo `## Overview` (otra sección de nivel superior; fuera).
fn bundle_get_secciones() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Rotacion](decisiones/rotacion.md)\n",
    );
    write(
        dir.path(),
        "decisiones/rotacion.md",
        "---\n\
type: decision\n\
title: Rotacion de tokens\n\
description: Politica de rotacion de tokens\n\
status: accepted\n\
---\n\
\n\
# Rotacion de tokens\n\
\n\
Introduccion general del documento.\n\
\n\
## Overview\n\
\n\
Vision general del sistema. TOKEN-OVERVIEW-EXCLUIR.\n\
\n\
## Security\n\
\n\
Consideraciones generales de seguridad.\n\
\n\
### Token rotation\n\
\n\
Los tokens de acceso rotan cada 24 horas. TOKEN-OBJETIVO-INCLUIR.\n\
\n\
### Otra\n\
\n\
Detalle de una subseccion hermana. TOKEN-HERMANA-SUB-EXCLUIR.\n\
\n\
## Otra seccion\n\
\n\
Contenido de una seccion hermana de nivel superior. TOKEN-HERMANA-TOP-EXCLUIR.\n",
    );
    dir
}

/// E10-H10 · Criterio `get_por_seccion`:
/// Dado `sections:[["Security","Token rotation"]]`, Cuando se pide, Entonces el body devuelto es SOLO
/// esa subsección: contiene su texto y NO contiene el de sus secciones/subsecciones hermanas.
#[test]
fn get_por_seccion() {
    let dir = bundle_get_secciones();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"decisiones/rotacion.md"},"include":["body"],"sections":[["Security","Token rotation"]]}}}"#,
        ],
        1,
    );
    let concept = &resp[0]["result"]["structuredContent"]["concept"];
    let body = concept["body"].as_str().unwrap_or_else(|| {
        panic!("knowledge_get con include:[body] debe devolver concept.body (string): {resp:?}")
    });

    // CONTIENE el texto de la subsección pedida (## Security → ### Token rotation).
    assert!(
        body.contains("TOKEN-OBJETIVO-INCLUIR"),
        "el body acotado debe contener la subsección pedida «Token rotation»: {body:?}"
    );
    // NO contiene la subsección HERMANA `### Otra` (misma `## Security`): fuerza que el 2º nivel del
    // headingPath cuente (acotar solo por `## Security` dejaría entrar esta subsección).
    assert!(
        !body.contains("TOKEN-HERMANA-SUB-EXCLUIR"),
        "el body no debe incluir la subsección hermana `### Otra`: {body:?}"
    );
    // NO contiene la sección HERMANA de nivel superior `## Otra seccion`.
    assert!(
        !body.contains("TOKEN-HERMANA-TOP-EXCLUIR"),
        "el body no debe incluir la sección hermana `## Otra seccion`: {body:?}"
    );
    // NO contiene otra sección de nivel superior `## Overview`.
    assert!(
        !body.contains("TOKEN-OVERVIEW-EXCLUIR"),
        "el body no debe incluir la sección `## Overview`: {body:?}"
    );
}

/// E10-H10 · Criterio `get_inexistente`:
/// Dado un path inexistente, Cuando se pide, Entonces `CONCEPT_NOT_FOUND`. En la superficie MCP un
/// concepto inexistente es un error de EJECUCIÓN de la tool (no un fallo de protocolo): aflora como
/// `result.isError == true` con el código estable `CONCEPT_NOT_FOUND` visible al agente (ErrorCode
/// wire de E10-H02, `REFACTOR §13` / invariante #4), no como un error JSON-RPC.
#[test]
fn get_inexistente() {
    let dir = bundle_get_revision(); // tiene `alfa.md`; pedimos un path que NO existe.
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"no-existe.md"},"include":["frontmatter"]}}}"#,
        ],
        1,
    );
    // Error de ejecución de la tool → isError en el result, no un error JSON-RPC de transporte.
    assert_eq!(
        resp[0]["result"]["isError"], true,
        "un ConceptRef a un path inexistente debe dar isError en knowledge_get: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "un concepto inexistente NO debe ser un error de protocolo JSON-RPC: {resp:?}"
    );
    // El código estable `CONCEPT_NOT_FOUND` debe ser visible en la respuesta (no un mensaje opaco).
    let texto = resp[0].to_string();
    assert!(
        texto.contains("CONCEPT_NOT_FOUND"),
        "el error debe exponer el código estable «CONCEPT_NOT_FOUND»: {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// E10-H11 — Tool `schema_inspect`.
//
// UBICACIÓN: los 3 criterios se ejercitan **e2e por la tool MCP** (campo Pruebas de la historia:
// `crates/lodestar-mcp/tests/`), coherente con E10-H09/H10: el contrato que importa fijar aquí es
// el de **wire** (nombres de argumento `mode`/`type`, forma del `structuredContent`), sin acoplar
// los tests a los tipos internos que el implementador aún no ha creado.
//
// FASE ROJA: la tool `schema_inspect` NO está en `tools::list()` todavía, así que `tools/call`
// devuelve el error de protocolo `-32602` (tool desconocida) y `result` es `null` → los asserts
// que leen `result.structuredContent.*` fallan por AUSENCIA de la tool/servicio (no por un valor
// erróneo). Ese es el rojo correcto: la tool + `App::schema_inspect` no existen.
//
// WIRE DE ENTRADA asumido (el implementador puede refinar los tipos internos, no el wire):
//   arguments: { mode: string, type?: string }
//     - inspect_type:    { "mode": "type", "type": "decision" }
//     - inspect_catalog: { "mode": "catalog" }
//   (modos `field`/`relation`/`diagnosticCode`/`lifecycle`/`template` del REFACTOR §9.4 quedan
//    fuera de los criterios de esta historia; solo se fijan `type` y `catalog`.)
//
// WIRE DE SALIDA asumido (`structuredContent`, `ARCHITECTURE.md §19.2`, `REFACTOR §9.4`):
//   - mode "type":    { schemaVersion, type: { name, description, requiredFields,
//                       allowedStatuses, fields, relations, rules, bodyTemplate } }
//   - mode "catalog": { schemaVersion, types: [ { name, description, requiredFields,
//                       allowedStatuses, ... } ] }  (lista de todos los DocType disponibles)
//
// La firma de servicio ASUMIDA (proyección del `Schema` cargado por `WorkspaceSchema::load`):
//   App::schema_inspect(mode) -> Result<SchemaInspection, WorkspaceError>
// ---------------------------------------------------------------------------

/// Bundle con un `.lodestar/schema.yaml` que declara DOS `DocType`s: `decision` (con
/// `requiredFields`/`allowedStatuses`/`bodyTemplate` completos, el sujeto de `inspect_type`) y
/// `note` (para que `inspect_catalog` no sea vacuo: un stub que devolviera un único tipo cableado a
/// mano fallaría al no listar los dos). Formato de wire camelCase idéntico al que ya carga el
/// loader (`crates/lodestar-workspace/tests/workspace.rs::loader_carga_schema_yaml`).
fn bundle_con_schema() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        dir.path(),
        ".lodestar/schema.yaml",
        "\
version: \"1\"
types:
  decision:
    name: decision
    description: Una decisión de arquitectura registrada
    requiredFields: [title, status, rationale]
    allowedStatuses: [proposed, accepted, rejected, deprecated]
    bodyTemplate: |
      # {title}

      ## Contexto

      ## Decisión

      ## Consecuencias
  note:
    name: note
    description: Una nota libre
    requiredFields: [title]
    allowedStatuses: [draft, published]
",
    );
    dir
}

/// E10-H11 · Criterio `inspect_type`:
/// Dado un `DocType decision`, Cuando se llama `schema_inspect(type="decision")`, Entonces devuelve
/// sus `requiredFields`/`allowedStatuses`/`bodyTemplate`.
#[test]
fn inspect_type() {
    let dir = bundle_con_schema();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"schema_inspect","arguments":{"mode":"type","type":"decision"}}}"#,
        ],
        1,
    );
    let sc = &resp[0]["result"]["structuredContent"];

    // `requiredFields` == [title, status, rationale] (orden preservado del wire).
    let required = sc["type"]["requiredFields"].as_array().unwrap_or_else(|| {
        panic!("schema_inspect(type=decision) debe devolver type.requiredFields (array): {resp:?}")
    });
    let required: Vec<&str> = required.iter().filter_map(|v| v.as_str()).collect();
    assert_eq!(
        required,
        vec!["title", "status", "rationale"],
        "type.requiredFields debe ser exactamente [title, status, rationale]: {resp:?}"
    );

    // `allowedStatuses` incluye "accepted".
    let statuses = sc["type"]["allowedStatuses"].as_array().unwrap_or_else(|| {
        panic!("schema_inspect(type=decision) debe devolver type.allowedStatuses (array): {resp:?}")
    });
    assert!(
        statuses.iter().any(|v| v == "accepted"),
        "type.allowedStatuses debe incluir «accepted»: {resp:?}"
    );

    // `bodyTemplate` presente y no vacío.
    let template = sc["type"]["bodyTemplate"].as_str().unwrap_or_else(|| {
        panic!("schema_inspect(type=decision) debe devolver type.bodyTemplate (string): {resp:?}")
    });
    assert!(
        !template.is_empty(),
        "type.bodyTemplate no debe estar vacío: {resp:?}"
    );
}

/// E10-H11 · Criterio `inspect_catalog`:
/// Dado el modo `catalog`, Cuando se llama, Entonces lista todos los `DocType` disponibles (aquí
/// `decision` y `note`).
#[test]
fn inspect_catalog() {
    let dir = bundle_con_schema();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"schema_inspect","arguments":{"mode":"catalog"}}}"#,
        ],
        1,
    );
    let sc = &resp[0]["result"]["structuredContent"];
    let types = sc["types"].as_array().unwrap_or_else(|| {
        panic!("schema_inspect(catalog) debe devolver structuredContent.types (array): {resp:?}")
    });
    let nombres: Vec<&str> = types.iter().filter_map(|t| t["name"].as_str()).collect();
    assert!(
        nombres.contains(&"decision"),
        "el catálogo debe listar el DocType «decision»: {resp:?}"
    );
    assert!(
        nombres.contains(&"note"),
        "el catálogo debe listar el DocType «note»: {resp:?}"
    );
}

/// E10-H11 · Criterio `inspect_sin_schema`:
/// Dado un bundle SIN `.lodestar/schema.yaml`, Cuando se llama `catalog`, Entonces catálogo vacío
/// (no error). El bundle es válido (tiene `index.md`) pero no declara esquema → `types == []`, sin
/// que la ausencia de esquema se convierta en un fallo.
#[test]
fn inspect_sin_schema() {
    let dir = bundle_min(); // index.md, deliberadamente SIN `.lodestar/schema.yaml`.
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"schema_inspect","arguments":{"mode":"catalog"}}}"#,
        ],
        1,
    );
    // No es un fallo: ni error JSON-RPC de transporte ni error de ejecución de la tool.
    assert!(
        resp[0]["error"].is_null(),
        "un bundle sin schema NO debe producir un error de protocolo: {resp:?}"
    );
    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "un bundle sin schema NO debe producir isError: {resp:?}"
    );
    // Catálogo vacío.
    let types = resp[0]["result"]["structuredContent"]["types"]
        .as_array()
        .unwrap_or_else(|| {
            panic!(
                "schema_inspect(catalog) debe devolver structuredContent.types (array): {resp:?}"
            )
        });
    assert!(
        types.is_empty(),
        "un bundle sin schema.yaml debe dar un catálogo vacío: {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// E10-H12 — Tool `knowledge_check` (sustituye `conformance_check`).
//
// UBICACIÓN: los 3 criterios se ejercitan **e2e por la tool MCP** (campo Pruebas de la historia:
// `crates/lodestar-mcp/tests/`), coherente con E10-H08…H11. Lo que hay que fijar es el contrato de
// **wire** (forma de `scope`, forma del `structuredContent` con `conformant`/`summary`/
// `diagnostics`/`workspaceRevision`/`nextCursor`, y que cada diagnóstico lleve `id`/`code`/`targets`)
// sin acoplar los tests a los tipos internos que el implementador aún no ha creado
// (`App::knowledge_check`, el enum de scope, etc.).
//
// FASE ROJA: la tool `knowledge_check` NO está en `tools::list()` todavía (solo existe la vieja
// `conformance_check`), así que `tools/call {name:"knowledge_check"}` devuelve el error de protocolo
// `-32602` (tool desconocida) y `result` es `null` → los asserts que leen
// `result.structuredContent.diagnostics` fallan por AUSENCIA de la tool/servicio (no por un valor
// erróneo). Ese es el rojo correcto: la tool + `App::knowledge_check` no existen.
//
// WIRE DE ENTRADA asumido (el implementador puede refinar los tipos internos, no el wire):
//   arguments: {
//     scope: { kind: "workspace" }
//          | { kind: "concept",  ref: { path } }
//          | { kind: "paths",    paths: [ "<RelPath>", … ] }
//          | { kind: "affected", refs: [ { path } ], depth: <n> },
//     minimumSeverity?: "err" | "warn" | "info",   // omitido = todos los niveles
//     includeSuggestedFixes?: bool,
//     limit?: <n>,
//     cursor?: string
//   }
//
// WIRE DE SALIDA asumido (`structuredContent`, `ARCHITECTURE.md §19.6`, `REFACTOR §10`):
//   {
//     conformant: bool,
//     summary: { errors, warnings, info },
//     diagnostics: [ { level, code, msg, targets, id, range?, related, fixes } ],  // Check (E10-H06)
//     workspaceRevision: "blake3:…",
//     nextCursor: string | null
//   }
// Cada diagnóstico lleva un `id` ESTABLE dentro de una revisión, con formato `diag:…` que embebe un
// `blake3:` (hash determinista de, p. ej., path+code+range+msg).
//
// FIRMA DE SERVICIO ASUMIDA (el implementador la crea con su propia elección de tipos internos):
//   App::knowledge_check(scope, minimum_severity, include_suggested_fixes, limit, cursor)
//       -> Result<{ conformant, summary, diagnostics, workspaceRevision, nextCursor }, _>
//   Compone `Bundle::analyze` (los 15 checks OKF) + `validate_schema(bundle, schema)` (E10-H07);
//   `affected` acota por vecindad (`Bundle::neighborhood` / `Store::blast_radius`).
// ---------------------------------------------------------------------------

/// Extrae los diagnósticos (`structuredContent.diagnostics`) de una respuesta `knowledge_check`. Si
/// la tool/servicio no existe todavía (fase ROJA), ese campo es nulo → panica con un mensaje que
/// documenta el porqué del rojo (la tool ausente), no un fallo espurio.
fn check_diagnostics(resp: &serde_json::Value) -> Vec<serde_json::Value> {
    resp["result"]["structuredContent"]["diagnostics"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("knowledge_check debe devolver structuredContent.diagnostics (array): {resp:?}")
        })
        .clone()
}

/// Los `targets` (paths afectados) de un diagnóstico `Check` (campo `targets`, siempre presente).
fn diag_targets(d: &serde_json::Value) -> Vec<String> {
    d["targets"]
        .as_array()
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// `true` si algún diagnóstico de `diags` tiene `path` entre sus `targets`.
fn diags_cubren(diags: &[serde_json::Value], path: &str) -> bool {
    diags
        .iter()
        .any(|d| diag_targets(d).iter().any(|t| t == path))
}

/// Bundle con un `.md` **editado a mano** cuyo frontmatter es inválido: le falta el campo
/// obligatorio `type`, lo que dispara el check hard-fail `OKF-TYPE` (severidad `Err`) sobre ese
/// path (`conform.rs`: "Falta indicar de qué tipo es esta página."). El bundle es por lo demás
/// válido (tiene `index.md`), así que el ÚNICO error viene de la edición directa.
fn bundle_editado_a_mano() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Editado](editado-a-mano.md)\n",
    );
    // Frontmatter válido como bloque pero SIN `type` → OKF-TYPE (Err). Simula a alguien que editó
    // el .md a pelo y olvidó el campo obligatorio.
    write(
        dir.path(),
        "editado-a-mano.md",
        "---\ntitle: Editado a mano\ndescription: alguien lo escribió a pelo\n---\n\n# Nota\n\ncuerpo suelto sin tipo.\n",
    );
    dir
}

/// E10-H12 · Criterio `check_detecta_edicion_directa` (benchmark §17):
/// Dado un `.md` editado a mano con frontmatter inválido, Cuando se hace `knowledge_check` de scope
/// `workspace`, Entonces aparece el diagnóstico de ese path y el veredicto es no conforme.
#[test]
fn check_detecta_edicion_directa() {
    let dir = bundle_editado_a_mano();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_check","arguments":{"scope":{"kind":"workspace"}}}}"#,
        ],
        1,
    );
    let diags = check_diagnostics(&resp[0]);

    // Hay un diagnóstico sobre el fichero editado a mano.
    let del_fichero: Vec<&serde_json::Value> = diags
        .iter()
        .filter(|d| diag_targets(d).iter().any(|t| t == "editado-a-mano.md"))
        .collect();
    assert!(
        !del_fichero.is_empty(),
        "knowledge_check(workspace) debe reportar el diagnóstico de «editado-a-mano.md»: {resp:?}"
    );
    // Y es exactamente el hard-fail OKF-TYPE (frontmatter sin `type`) — no un warning cualquiera.
    assert!(
        del_fichero.iter().any(|d| d["code"] == "OKF-TYPE"),
        "el diagnóstico de «editado-a-mano.md» debe ser OKF-TYPE (falta el campo `type`): {resp:?}"
    );

    // Veredicto global: NO conforme (hay al menos un error).
    assert_eq!(
        resp[0]["result"]["structuredContent"]["conformant"],
        serde_json::Value::Bool(false),
        "con un frontmatter inválido el workspace NO debe ser conforme: {resp:?}"
    );
}

/// Bundle para `check_scope_affected`. Grafo de vecindad **bidireccional** (robusto a la dirección
/// que use el implementador — out/in/both) alrededor del ref `centro.md`:
///
///   index.md ──► centro.md ◄──► vecino.md ◄──► c.md          lejano.md   (aislado)
///
/// - `centro.md` (A): el ref; conforme. Enlaza a `vecino.md`.
/// - `vecino.md` (B, distancia 1): frontmatter sin `type` → diagnóstico OKF-TYPE. Enlaza a `centro`
///   y a `c` (así, en CUALQUIER dirección, B está a distancia 1 y C a distancia 2 de A).
/// - `c.md` (C, distancia 2): frontmatter sin `type` → diagnóstico OKF-TYPE. Enlaza a `vecino`.
/// - `lejano.md` (D, NO conectado): frontmatter sin `type` → diagnóstico OKF-TYPE. Su diagnóstico
///   DEBE quedar fuera del scope `affected {refs:[centro], depth:2}`.
///
/// El criterio es inequívoco: con `depth:2` el vecindario de A es exactamente {centro, vecino, c};
/// `lejano` está a distancia infinita y no puede colarse.
fn bundle_affected() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Centro](centro.md)\n",
    );
    // A: conforme, enlaza a B.
    write(
        dir.path(),
        "centro.md",
        "---\ntype: concept\ntitle: Centro\ndescription: nodo raíz del vecindario\n---\n\n# Centro\n\n[Vecino](vecino.md)\n",
    );
    // B (distancia 1): sin `type` → OKF-TYPE. Enlaza a A y a C (bidireccional).
    write(
        dir.path(),
        "vecino.md",
        "---\ntitle: Vecino\ndescription: a distancia 1 de centro\n---\n\n# Vecino\n\n[Centro](centro.md)\n\n[C](c.md)\n",
    );
    // C (distancia 2): sin `type` → OKF-TYPE. Enlaza a B (bidireccional).
    write(
        dir.path(),
        "c.md",
        "---\ntitle: C\ndescription: a distancia 2 de centro\n---\n\n# C\n\n[Vecino](vecino.md)\n",
    );
    // D (lejano, aislado): sin `type` → OKF-TYPE. Sin ningún enlace desde/hacia el vecindario.
    write(
        dir.path(),
        "lejano.md",
        "---\ntitle: Lejano\ndescription: desconectado del vecindario\n---\n\n# Lejano\n\ncuerpo sin enlaces.\n",
    );
    dir
}

/// E10-H12 · Criterio `check_scope_affected`:
/// Dado `scope:affected` con un ref (`centro.md`) y `depth:2`, Cuando se llama `knowledge_check`,
/// Entonces solo aparecen diagnósticos del vecindario (vecino a distancia 1 y c a distancia 2), y
/// NO el del concepto lejano y desconectado.
#[test]
fn check_scope_affected() {
    let dir = bundle_affected();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_check","arguments":{"scope":{"kind":"affected","refs":[{"path":"centro.md"}],"depth":2}}}}"#,
        ],
        1,
    );
    let diags = check_diagnostics(&resp[0]);

    // Vecino (distancia 1) DEBE aparecer.
    assert!(
        diags_cubren(&diags, "vecino.md"),
        "el diagnóstico de «vecino.md» (distancia 1) debe estar en el scope affected: {resp:?}"
    );
    // C (distancia 2) DEBE aparecer — prueba que `depth:2` alcanza el segundo salto (no vacuo).
    assert!(
        diags_cubren(&diags, "c.md"),
        "el diagnóstico de «c.md» (distancia 2) debe estar en el scope affected con depth:2: {resp:?}"
    );
    // El concepto LEJANO y desconectado NO debe aparecer: es lo que hace inequívoco el scope.
    assert!(
        !diags_cubren(&diags, "lejano.md"),
        "el diagnóstico de «lejano.md» (desconectado) NO debe estar en el scope affected: {resp:?}"
    );
}

/// Bundle con DOS ficheros no conformes (frontmatter sin `type` → OKF-TYPE), para que el conjunto de
/// `id` de diagnóstico sea significativo (≥1, aquí ≥2) al comparar estabilidad entre revisiones.
fn bundle_dos_diagnosticos() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Uno](uno.md)\n* [Dos](dos.md)\n",
    );
    for slug in ["uno", "dos"] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!("---\ntitle: {slug}\ndescription: sin type\n---\n\n# H\n\ncuerpo.\n"),
        );
    }
    dir
}

/// Reúne el conjunto de `id` de diagnóstico de una respuesta `knowledge_check`, tras verificar que
/// cada `id` está presente y con el formato estable `diag:…` que embebe `blake3:`.
fn diag_ids(resp: &serde_json::Value) -> std::collections::BTreeSet<String> {
    check_diagnostics(resp)
        .iter()
        .map(|d| {
            let id = d["id"].as_str().unwrap_or_else(|| {
                panic!("cada diagnóstico de knowledge_check debe llevar un `id` estable: {d:?}")
            });
            assert!(
                id.starts_with("diag:"),
                "el `id` de diagnóstico debe empezar por «diag:»: {id}"
            );
            assert!(
                id.contains("blake3:"),
                "el `id` de diagnóstico debe embeber un hash «blake3:»: {id}"
            );
            id.to_string()
        })
        .collect()
}

/// E10-H12 · Criterio `check_ids_estables`:
/// Dada la misma revisión dos veces (dos servidores frescos sobre el MISMO bundle sin cambios),
/// Cuando se hace `knowledge_check` de scope `workspace`, Entonces el conjunto de `id` de
/// diagnóstico coincide entre ambas llamadas (misma revisión → mismos ids).
#[test]
fn check_ids_estables() {
    let dir = bundle_dos_diagnosticos();
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_check","arguments":{"scope":{"kind":"workspace"}}}}"#;

    // Dos procesos frescos sobre el mismo bundle: misma revisión de workspace.
    let a = roundtrip(dir.path(), &[call], 1);
    let b = roundtrip(dir.path(), &[call], 1);

    let ids_a = diag_ids(&a[0]);
    let ids_b = diag_ids(&b[0]);

    // Significativo: hay al menos un diagnóstico (si no, la igualdad sería vacua).
    assert!(
        !ids_a.is_empty(),
        "el bundle debe producir al menos un diagnóstico para que el criterio no sea vacuo: {a:?}"
    );
    // Misma revisión → mismos ids.
    assert_eq!(
        ids_a, ids_b,
        "los `id` de diagnóstico deben coincidir entre dos llamadas sobre la misma revisión"
    );
}

// ---------------------------------------------------------------------------
// E10-H13 — `outputSchema` (schemars).
//
// El único criterio testeable de esta historia se ejercita **e2e por stdio** (campo Pruebas:
// `crates/lodestar-mcp/tests/`):
//   `tools_declaran_outputschema`: las 5 tools de lectura/verificación de E10
//   (workspace_status/knowledge_search/knowledge_get/schema_inspect/knowledge_check) deben declarar
//   `outputSchema` (decisión D6b: derivarlo con `schemars`).
//
// FASE ROJA: las 5 tools declaran hoy `inputSchema` pero NO `outputSchema` en `tools::list()` →
// `tools_declaran_outputschema` falla por AUSENCIA de la clave `outputSchema`.
//
// DESCOPE (coordinación): la retirada de `query`/`conformance_check` (§15) queda FUERA de H13 — la
// limpieza coherente de superficie a las 10 tools objetivo requiere `graph_query` (E11) y las tools
// de cambio (E12/E13), y se hará en un único rewrite de `mcp.yml` al cerrar E13. Por eso NO hay
// aquí un test de retirada y los 3 tests heredados que usan `query`/`conformance_check` siguen
// válidos (esas tools permanecen).
//
// El criterio estructural restante («`/contrato --check` pasa contra el `mcp.yml` reescrito») lo
// verifica el guardián de contrato, no un `#[test]` (por eso no se codifica aquí).
// ---------------------------------------------------------------------------

/// E10-H13 · Criterio `tools_declaran_outputschema`:
/// Dado `tools/list`, Cuando se inspecciona cada una de las 5 tools de lectura/verificación de E10,
/// Entonces cada una incluye `outputSchema` y es un objeto de JSON Schema (con al menos una clave
/// estructural de esquema). Se exigen las 5 (no basta con `workspace_status`): un stub que solo
/// añadiera `outputSchema` a una tool no pasaría.
#[test]
fn tools_declaran_outputschema() {
    let dir = bundle_min();
    let resp = roundtrip(
        dir.path(),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#],
        1,
    );
    let tools = resp[0]["result"]["tools"]
        .as_array()
        .expect("tools/list devuelve un array de tools");

    // Las 5 tools de lectura/verificación de E10 (D6b): todas deben declarar `outputSchema`.
    let con_output = [
        "workspace_status",
        "knowledge_search",
        "knowledge_get",
        "schema_inspect",
        "knowledge_check",
    ];
    // Claves estructurales que identifican un JSON Schema derivado por schemars (raíz objeto,
    // referencia, o combinador). Basta con que aparezca una.
    let claves_schema = [
        "type",
        "$ref",
        "properties",
        "allOf",
        "oneOf",
        "anyOf",
        "$defs",
        "definitions",
    ];
    for name in con_output {
        let tool = tools
            .iter()
            .find(|t| t["name"] == name)
            .unwrap_or_else(|| panic!("falta la tool «{name}» en tools/list: {tools:?}"));
        let output = &tool["outputSchema"];
        assert!(
            output.is_object(),
            "la tool «{name}» debe declarar `outputSchema` como objeto (D6b): {tool:?}"
        );
        assert!(
            claves_schema.iter().any(|k| output.get(k).is_some()),
            "el `outputSchema` de «{name}» debe ser un JSON Schema (alguna clave estructural): {output:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// E11-H01 — Tool `graph_query` (consolida find_backlinks/neighborhood/find_orphans/find_dangling).
//
// UBICACIÓN: los 4 criterios se ejercitan **e2e por la tool MCP** (campo Pruebas de la historia:
// `crates/lodestar-mcp/tests/`), coherente con E10-H08…H13. Lo que hay que fijar es el contrato de
// **wire** (nombres del argumento `operation`/`ref`/`depth`/`direction`/`limit`/`cursor`, y la forma
// del `structuredContent` con `nodes`/`edges`/`summary{nodeCount,edgeCount,truncated}`/`nextCursor`)
// sin acoplar los tests a los tipos internos que el implementador aún no ha creado
// (`App::graph_query`, el enum de operación, el tipo del subgrafo, etc.).
//
// El criterio de PARIDAD (`graph_neighborhood_paridad`) se comprueba comparando la salida de wire de
// la tool contra la **verdad del core** (`Bundle::neighborhood`, invariante #3): se abre el MISMO
// bundle en proceso con `App::open` y se computa `neighborhood(path, 2, Both)`; los `nodes`/`edges`
// del wire deben coincidir (como conjuntos) con los del core. Esto ancla la tool a la lógica pura del
// core en vez de a una reimplementación paralela. Se hace de forma SECUENCIAL (el proceso hijo del
// `roundtrip` ya terminó — `child.wait()` — antes de abrir el `App`, así no compiten por
// `.lodestar/index.db`).
//
// FASE ROJA: la tool `graph_query` NO está en `tools::list()` todavía, así que `tools/call
// {name:"graph_query"}` devuelve el error de protocolo `-32602` (tool desconocida) y `result` es
// `null` → los helpers que leen `result.structuredContent.nodes`/`edges`/`summary` fallan por
// AUSENCIA de la tool/servicio (no por un valor erróneo). Ese es el rojo correcto: la tool +
// `App::graph_query` no existen.
//
// WIRE DE ENTRADA asumido (el implementador puede refinar los tipos internos, no el wire):
//   arguments: {
//     operation: "backlinks" | "outgoing" | "neighborhood" | "orphans" | "dangling",
//     ref?:       { path: "<RelPath>" },       // ConceptRef; obligatorio en backlinks/outgoing/neighborhood
//     depth?:     <n>,                          // solo neighborhood (por defecto 1)
//     direction?: "out" | "in" | "both",       // solo neighborhood (por defecto "out")
//     limit?:     <n>,                          // trunca el nº de nodos devueltos
//     cursor?:    string                        // cursor opaco de paginación
//   }
//
// WIRE DE SALIDA asumido (`structuredContent`, `ARCHITECTURE.md §19.6`, `REFACTOR §9.5`):
//   {
//     nodes: [ { id, ghost, type, status } ],     // GraphNode (core::types)
//     edges: [ { source, target, dangling } ],    // Edge (core::types)
//     summary: { nodeCount, edgeCount, truncated },
//     nextCursor: string | null
//   }
//
// FIRMA DE SERVICIO ASUMIDA (el implementador la crea con su propia elección de tipos internos):
//   App::graph_query(operation, ref?, depth?, direction?, limit?, cursor?)
//       -> Result<{ nodes, edges, summary{nodeCount,edgeCount,truncated}, nextCursor }, _>
//   Reusa `Bundle::backlinks`/`Bundle::neighborhood` y `Analysis::orphans`/`dangling` (verdad del
//   core, invariante #3).
// ---------------------------------------------------------------------------

/// Extrae `structuredContent.nodes` de una respuesta `graph_query`. En fase ROJA (tool ausente) ese
/// campo es nulo → panica con un mensaje que documenta el porqué del rojo, no un fallo espurio.
fn graph_nodes(resp: &serde_json::Value) -> Vec<serde_json::Value> {
    resp["result"]["structuredContent"]["nodes"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("graph_query debe devolver structuredContent.nodes (array): {resp:?}")
        })
        .clone()
}

/// Extrae `structuredContent.edges` de una respuesta `graph_query` (misma nota de ROJO que arriba).
fn graph_edges(resp: &serde_json::Value) -> Vec<serde_json::Value> {
    resp["result"]["structuredContent"]["edges"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("graph_query debe devolver structuredContent.edges (array): {resp:?}")
        })
        .clone()
}

/// Conjunto de `id` (string) de una lista de nodos de grafo (`GraphNode.id` == RelPath serializado).
fn graph_node_ids(nodes: &[serde_json::Value]) -> std::collections::BTreeSet<String> {
    nodes
        .iter()
        .map(|n| {
            n["id"]
                .as_str()
                .unwrap_or_else(|| panic!("cada nodo de graph_query lleva un `id` string: {n:?}"))
                .to_string()
        })
        .collect()
}

/// Canonicaliza una lista de objetos JSON a un conjunto de strings (para comparar `edges`/`nodes`
/// como conjuntos, sin depender del orden). Como ambos lados provienen de serializar el mismo tipo
/// del core, el orden de claves es idéntico y la comparación textual es fiel.
fn como_conjunto(vals: &[serde_json::Value]) -> std::collections::BTreeSet<String> {
    vals.iter().map(|v| v.to_string()).collect()
}

/// E11-H01 · Criterio `graph_backlinks`:
/// Dado un concepto (`objetivo.md`) con **3 backlinks**, Cuando se llama
/// `graph_query(operation:backlinks, ref:{path})`, Entonces los 3 aparecen en `nodes`/`edges`.
///
/// Bundle: `a.md`/`b.md`/`c.md` enlazan a `objetivo.md`; `d.md` es un decoy que enlaza a OTRO
/// concepto (`a.md`), no a `objetivo.md`, para que el criterio no sea vacuo (un stub que devolviera
/// todos los conceptos incluiría a `d` como fuente y fallaría). `index.md` NO lista `objetivo.md`
/// (evita que `index_refs` añada una arista desde un fichero reservado).
#[test]
fn graph_backlinks() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [A](a.md)\n* [B](b.md)\n* [C](c.md)\n* [D](d.md)\n",
    );
    write(
        dir.path(),
        "objetivo.md",
        "---\ntype: concept\ntitle: Objetivo\ndescription: recibe 3 backlinks\n---\n\n# Objetivo\n\ncuerpo.\n",
    );
    for slug in ["a", "b", "c"] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: concept\ntitle: {slug}\ndescription: enlaza al objetivo\n---\n\n# {slug}\n\n[Objetivo](objetivo.md)\n"
            ),
        );
    }
    // Decoy: enlaza a `a.md`, NUNCA a `objetivo.md`.
    write(
        dir.path(),
        "d.md",
        "---\ntype: concept\ntitle: D\ndescription: no enlaza al objetivo\n---\n\n# D\n\n[A](a.md)\n",
    );

    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"graph_query","arguments":{"operation":"backlinks","ref":{"path":"objetivo.md"}}}}"#,
        ],
        1,
    );

    let nodes = graph_nodes(&resp[0]);
    let edges = graph_edges(&resp[0]);
    let ids = graph_node_ids(&nodes);

    // Las 3 fuentes aparecen como nodos.
    for src in ["a.md", "b.md", "c.md"] {
        assert!(
            ids.contains(src),
            "el backlink «{src}» debe aparecer en nodes de graph_query(backlinks): {resp:?}"
        );
    }

    // Las aristas de backlink (target == objetivo.md) son EXACTAMENTE {a,b,c} → objetivo (3).
    let fuentes_hacia_objetivo: std::collections::BTreeSet<String> = edges
        .iter()
        .filter(|e| e["target"] == "objetivo.md")
        .map(|e| {
            e["source"]
                .as_str()
                .unwrap_or_else(|| panic!("cada arista lleva `source` string: {e:?}"))
                .to_string()
        })
        .collect();
    assert_eq!(
        fuentes_hacia_objetivo,
        ["a.md", "b.md", "c.md"]
            .iter()
            .map(|s| s.to_string())
            .collect::<std::collections::BTreeSet<String>>(),
        "los backlinks de «objetivo.md» deben ser exactamente {{a,b,c}} → objetivo: {resp:?}"
    );

    // No vacuo: el decoy `d.md` no enlaza al objetivo, así que NO es una fuente de backlink.
    assert!(
        !fuentes_hacia_objetivo.contains("d.md"),
        "el decoy «d.md» no enlaza a objetivo y no debe ser un backlink: {resp:?}"
    );

    // El `summary` es coherente con las listas devueltas.
    let summary = &resp[0]["result"]["structuredContent"]["summary"];
    assert_eq!(
        summary["nodeCount"].as_u64(),
        Some(nodes.len() as u64),
        "summary.nodeCount debe casar con nodes.len(): {resp:?}"
    );
    assert_eq!(
        summary["edgeCount"].as_u64(),
        Some(edges.len() as u64),
        "summary.edgeCount debe casar con edges.len(): {resp:?}"
    );
}

/// Bundle con un vecindario dirigido no trivial alrededor de `centro.md`, con aristas de entrada y de
/// salida a distancia 1 y 2, más un `lejano.md` aislado que DEBE quedar fuera de
/// `neighborhood(centro, 2, Both)`:
///
///   abuelo.md ──► raiz.md ──► centro.md ──► vecino.md ──► c.md        lejano.md (aislado)
///
/// `neighborhood(centro, 2, Both)` = {centro, vecino, c (out, d2), raiz, abuelo (in, d2)}; `lejano`
/// a distancia infinita. `index.md` no enlaza a conceptos (evita ruido de aristas reservadas).
fn bundle_vecindario() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        dir.path(),
        "centro.md",
        "---\ntype: concept\ntitle: Centro\ndescription: raiz del vecindario\n---\n\n# Centro\n\n[Vecino](vecino.md)\n",
    );
    write(
        dir.path(),
        "vecino.md",
        "---\ntype: concept\ntitle: Vecino\ndescription: salida a distancia 1\n---\n\n# Vecino\n\n[C](c.md)\n",
    );
    write(
        dir.path(),
        "c.md",
        "---\ntype: concept\ntitle: C\ndescription: salida a distancia 2\n---\n\n# C\n\ncuerpo.\n",
    );
    write(
        dir.path(),
        "raiz.md",
        "---\ntype: concept\ntitle: Raiz\ndescription: entrada a distancia 1\n---\n\n# Raiz\n\n[Centro](centro.md)\n",
    );
    write(
        dir.path(),
        "abuelo.md",
        "---\ntype: concept\ntitle: Abuelo\ndescription: entrada a distancia 2\n---\n\n# Abuelo\n\n[Raiz](raiz.md)\n",
    );
    write(
        dir.path(),
        "lejano.md",
        "---\ntype: concept\ntitle: Lejano\ndescription: desconectado\n---\n\n# Lejano\n\ncuerpo sin enlaces.\n",
    );
    dir
}

/// E11-H01 · Criterio `graph_neighborhood_paridad`:
/// Dado `operation:neighborhood, depth:2, direction:both`, Cuando se llama, Entonces el subgrafo
/// (`nodes`/`edges`) casa **exactamente** con `Bundle::neighborhood(path, 2, Both)` del core
/// (invariante #3: el grafo es una verdad computada del core).
#[test]
fn graph_neighborhood_paridad() {
    use lodestar_core::types::{Direction, RelPath};

    let dir = bundle_vecindario();

    // 1) Salida de wire de la tool.
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"graph_query","arguments":{"operation":"neighborhood","ref":{"path":"centro.md"},"depth":2,"direction":"both"}}}"#,
        ],
        1,
    );
    let wire_nodes = como_conjunto(&graph_nodes(&resp[0]));
    let wire_edges = como_conjunto(&graph_edges(&resp[0]));

    // 2) Verdad del core: se abre el MISMO bundle en proceso (el hijo del roundtrip ya terminó) y se
    //    computa `neighborhood(centro, 2, Both)` con la lógica pura del core.
    let app = lodestar_app::App::open(dir.path()).expect("el bundle temporal debe abrir");
    let centro = RelPath::new("centro.md").unwrap();
    let nb = app
        .workspace()
        .neighborhood(&centro, 2, Direction::Both)
        .expect("el core debe computar el vecindario");
    let nb_json = serde_json::to_value(&nb).unwrap();
    let core_nodes = como_conjunto(nb_json["nodes"].as_array().unwrap());
    let core_edges = como_conjunto(nb_json["edges"].as_array().unwrap());

    // No vacuo: el vecindario es no trivial (varios nodos) y `lejano` NO forma parte de él.
    assert!(
        core_nodes.len() >= 4,
        "el fixture debe producir un vecindario no trivial (>=4 nodos): {nb_json:?}"
    );
    let core_ids = graph_node_ids(nb_json["nodes"].as_array().unwrap());
    assert!(
        !core_ids.contains("lejano.md"),
        "el concepto aislado «lejano.md» no debe estar en el vecindario del core: {nb_json:?}"
    );

    // Paridad: los nodos y aristas del wire coinciden (como conjuntos) con los del core.
    assert_eq!(
        wire_nodes, core_nodes,
        "los `nodes` de graph_query(neighborhood) deben casar con Bundle::neighborhood del core: {resp:?}"
    );
    assert_eq!(
        wire_edges, core_edges,
        "los `edges` de graph_query(neighborhood) deben casar con Bundle::neighborhood del core: {resp:?}"
    );
}

/// E11-H01 · Criterio `graph_orphans`:
/// Dado un bundle con conceptos huérfanos, Cuando se llama `graph_query(operation:orphans)`,
/// Entonces lista exactamente esos paths (los conceptos sin enlaces entrantes y ausentes del índice).
///
/// Bundle: `uno`/`dos`/`tres` son huérfanos (no listados en index, sin backlinks); `visible.md` SÍ
/// está en el índice → NO es huérfano. El no-huérfano hace el criterio no vacuo (un stub que
/// devolviera todos los conceptos incluiría `visible.md` y fallaría).
#[test]
fn graph_orphans() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Visible](visible.md)\n",
    );
    write(
        dir.path(),
        "visible.md",
        "---\ntype: concept\ntitle: Visible\ndescription: listado en el indice\n---\n\n# Visible\n\ncuerpo.\n",
    );
    for slug in ["uno", "dos", "tres"] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: concept\ntitle: {slug}\ndescription: huerfano\n---\n\n# {slug}\n\ncuerpo suelto.\n"
            ),
        );
    }

    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"graph_query","arguments":{"operation":"orphans"}}}"#,
        ],
        1,
    );

    let ids = graph_node_ids(&graph_nodes(&resp[0]));
    assert_eq!(
        ids,
        ["uno.md", "dos.md", "tres.md"]
            .iter()
            .map(|s| s.to_string())
            .collect::<std::collections::BTreeSet<String>>(),
        "graph_query(orphans) debe listar exactamente los 3 conceptos huérfanos: {resp:?}"
    );
    // No vacuo: el concepto listado en el índice NO es huérfano.
    assert!(
        !ids.contains("visible.md"),
        "«visible.md» está en el índice y no debe aparecer como huérfano: {resp:?}"
    );
}

/// E11-H01 · Criterio `graph_truncado`:
/// Dado un `limit` menor que el nº de nodos, Cuando se llama, Entonces `summary.truncated == true` y
/// `nextCursor` está presente (no nulo).
///
/// Bundle con **10 conceptos huérfanos** (`o00`…`o09`): `graph_query(orphans, limit:5)` trunca. Para
/// que el criterio NO sea vacuo (un stub que devolviera siempre `truncated:true` lo pasaría) se hace
/// una segunda llamada con `limit:20 >= 10`: entonces `truncated == false` y `nextCursor == null`.
#[test]
fn graph_truncado() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    for i in 0..10 {
        let slug = format!("o{i:02}");
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: concept\ntitle: Orphan {i:02}\ndescription: huerfano\n---\n\n# H\n\ncuerpo suelto {i:02}.\n"
            ),
        );
    }

    // Llamada truncada: limit:5 < 10 nodos.
    let trunc = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"graph_query","arguments":{"operation":"orphans","limit":5}}}"#,
        ],
        1,
    );
    let sc = &trunc[0]["result"]["structuredContent"];
    assert_eq!(
        sc["summary"]["truncated"],
        serde_json::Value::Bool(true),
        "con limit:5 < 10 nodos, summary.truncated debe ser true: {trunc:?}"
    );
    let cursor = sc["nextCursor"].as_str().unwrap_or_else(|| {
        panic!("con la salida truncada, `nextCursor` debe ser un string no nulo: {trunc:?}")
    });
    assert!(
        !cursor.is_empty(),
        "el `nextCursor` de una página truncada no debe ser vacío: {trunc:?}"
    );
    let nodes_trunc = graph_nodes(&trunc[0]);
    assert!(
        nodes_trunc.len() <= 5,
        "la página truncada no debe exceder el `limit`: {trunc:?}"
    );

    // No vacuo: con limit:20 >= 10 nodos NO se trunca.
    let full = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"graph_query","arguments":{"operation":"orphans","limit":20}}}"#,
        ],
        1,
    );
    let sc_full = &full[0]["result"]["structuredContent"];
    assert_eq!(
        sc_full["summary"]["truncated"],
        serde_json::Value::Bool(false),
        "con limit:20 >= 10 nodos, summary.truncated debe ser false: {full:?}"
    );
    assert!(
        sc_full["nextCursor"].is_null(),
        "sin truncar, `nextCursor` debe ser null: {full:?}"
    );
    assert_eq!(
        graph_nodes(&full[0]).len(),
        10,
        "sin truncar, deben aparecer los 10 huérfanos: {full:?}"
    );
}

// ---------------------------------------------------------------------------
// E11-H05 — Tool `impact_analyze` (reusa blast-radius + neighborhood).
//
// UBICACIÓN: los criterios de comportamiento (`impacto_move_30`, `impacto_delete_bloqueos`) se
// ejercitan **e2e por la tool MCP** (campo Pruebas de la historia: `crates/lodestar-mcp/tests/`),
// coherente con E10-H08…H12 y E11-H01. Lo que hay que fijar aquí es el contrato de **wire**
// (forma de `arguments` con `ref`/`proposedOperation`/`depth`, forma del `structuredContent` con
// `summary`/`affectedConcepts`/`blockingReferences`/`recommendations`) sin acoplar los tests a los
// tipos internos que el implementador aún no ha creado (`App::impact_analyze`, el enum de `kind`,
// el struct de `summary`, etc.). El tercer criterio (`impacto_paridad_core`) NO vive aquí: es una
// paridad **store vs core** (invariante #3, el bloque que `impact_analyze` reusa), sin superficie
// de wire ni tool; está en `crates/lodestar-store/tests/store.rs::impacto_paridad_core` (ver la
// nota de rojo de este archivo, abajo, y la cabecera de ese test).
//
// FASE ROJA: la tool `impact_analyze` NO está en `tools::list()` todavía, así que
// `tools/call {name:"impact_analyze"}` devuelve el error de protocolo `-32602` (tool desconocida) y
// `result` es `null` → los asserts que leen `result.structuredContent.summary.*` /
// `result.structuredContent.blockingReferences` fallan por AUSENCIA de la tool/servicio (no por un
// valor erróneo). Ese es el rojo correcto: la tool + `App::impact_analyze` no existen.
//
// WIRE DE ENTRADA asumido (el implementador puede refinar los tipos internos, no el wire):
//   arguments: {
//     ref: { path: "<RelPath>" },                       // ConceptRef (E10-H04); deser de { path }
//     proposedOperation: {
//       kind: "move" | "delete" | "deprecate" | "transition_status"
//           | "change_relation" | "replace_concept"
//     },
//     depth?: integer                                    // profundidad del blast-radius; def. impl.
//   }
//
// WIRE DE SALIDA asumido (`structuredContent`, `ARCHITECTURE.md §19.6`, `REFACTOR §9.6`):
//   {
//     summary: {
//       directlyAffected: number,        // nº de backlinks DIRECTOS del ref (Bundle::backlinks)
//       transitivelyAffected: number,    // tamaño del blast-radius (== neighborhood(In) del core)
//       blockingReferences: number,      // == blockingReferences.len()
//       risk: "low" | "medium" | "high"  // nivel derivado de nº de afectados/bloqueos
//     },
//     affectedConcepts: [ … ],           // conceptos alcanzados (paths / nodos)
//     blockingReferences: [ { path: "<RelPath>", reason: "<texto>" } ],
//     recommendations: [ … ]             // acciones sugeridas (texto)
//   }
//
// DECISIÓN DE WIRE FIJADA POR ESTA HISTORIA (el implementador debe respetarla):
//   - `summary.risk` es un string en INGLÉS del conjunto cerrado {"low","medium","high"},
//     coherente con el resto del wire camelCase/inglés (`direction:"in"`, `minimumSeverity:"err"`,
//     claves `directlyAffected`/`blockingReferences`). El NIVEL ALTO es exactamente `"high"`.
//   - Un `blockingReference` (para `kind:"delete"`) = un concepto que declara una **relación
//     tipada del schema** (`RelationDef`, E11-H03) cuyo target es el `ref`. Cada blocker es
//     `{ path, reason }`: `path` = el concepto que depende del ref; `reason` = texto no vacío que
//     explica el bloqueo (p. ej. el nombre de la relación que quedaría rota). Esta es la lectura
//     literal del alcance de la historia ("relaciones obligatorias que quedarían rotas"): las
//     dependencias estructurales tipadas, NO los enlaces sueltos de cuerpo Markdown.
//
// FIRMA DE SERVICIO ASUMIDA (el implementador la crea con su propia elección de tipos internos):
//   App::impact_analyze(ref: &ConceptRef, proposed_operation_kind, depth: Option<u32>)
//       -> Result<{ summary, affectedConcepts, blockingReferences, recommendations }, _>
//   `directlyAffected` compone `Bundle::backlinks`; `transitivelyAffected` reusa
//   `Store::blast_radius` (verificado idéntico a `neighborhood(In)` por `impacto_paridad_core`);
//   `blockingReferences` compone `validate_relations`/`RelationDef` (E11-H03).
// ---------------------------------------------------------------------------

/// Bundle con un concepto `target.md` al que apuntan **exactamente 30** conceptos vía un enlace de
/// cuerpo Markdown (`[t](/target.md)`), y NINGÚN otro backlink. El `index.md` NO lista `target.md`
/// (así `Backlinks::index_refs` queda vacío) y los 30 emisores no reciben backlinks entre sí, de
/// modo que `directlyAffected` del target es 30 bajo cualquier lectura (inbound-solo o
/// inbound+index). Deterministas por slug (`emisor00`…`emisor29`).
fn bundle_treinta_backlinks() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    // index.md sin enlaces salientes: no "adopta" al target (index_refs vacío).
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        dir.path(),
        "target.md",
        "---\ntype: Concept\ntitle: Target\ndescription: el concepto a mover\n---\n\n# Target\n\ncuerpo\n",
    );
    for i in 0..30 {
        let slug = format!("emisor{i:02}");
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: Concept\ntitle: Emisor {i:02}\ndescription: enlaza al target\n---\n\n# H\n\nreferencia a [target](/target.md).\n"
            ),
        );
    }
    dir
}

/// E11-H05 · Criterio `impacto_move_30` (benchmark §17: "Mover un concepto con 30 backlinks"):
/// Dado un concepto con 30 backlinks, Cuando `impact_analyze(kind:move)`, Entonces
/// `summary.directlyAffected == 30`.
#[test]
fn impacto_move_30() {
    let dir = bundle_treinta_backlinks();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"impact_analyze","arguments":{"ref":{"path":"target.md"},"proposedOperation":{"kind":"move"}}}}"#,
        ],
        1,
    );
    let sc = &resp[0]["result"]["structuredContent"];
    let directly = sc["summary"]["directlyAffected"]
        .as_u64()
        .unwrap_or_else(|| {
            panic!("impact_analyze debe devolver summary.directlyAffected (número): {resp:?}")
        });
    assert_eq!(
        directly, 30,
        "un concepto con 30 backlinks debe dar summary.directlyAffected == 30: {resp:?}"
    );
}

/// Bundle con un `.lodestar/schema.yaml` que declara una relación tipada **obligatoria**
/// (estructural) `depends_on` del tipo `task` hacia tipos `component`, y **3 conceptos `task`** que
/// declaran esa relación apuntando al target `component.md`. Al borrar `component.md`, esas 3
/// relaciones tipadas quedarían rotas → 3 `blockingReferences`. Un decoy `nota.md` (tipo `note`,
/// SIN la relación) NO debe contar como bloqueo, para que el criterio no sea vacuo (un stub que
/// contara "cualquier concepto" daría 4). Wire camelCase idéntico al loader
/// (`crates/lodestar-workspace/tests/workspace.rs`), con `targetTypes`/`cardinality` de `RelationDef`.
fn bundle_delete_bloqueos() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        dir.path(),
        ".lodestar/schema.yaml",
        "\
version: \"1\"
types:
  component:
    name: component
    description: Un componente del sistema
  note:
    name: note
    description: Una nota libre
  task:
    name: task
    description: Una tarea que depende de un componente
    relations:
      depends_on:
        targetTypes: [component]
        cardinality: many
",
    );
    // El target a borrar.
    write(
        dir.path(),
        "component.md",
        "---\ntype: component\ntitle: Componente critico\ndescription: el nucleo\n---\n\n# Componente\n\ncuerpo\n",
    );
    // 3 tareas con la relación tipada OBLIGATORIA `depends_on` apuntando al target.
    for i in 1..=3 {
        write(
            dir.path(),
            &format!("tarea{i}.md"),
            &format!(
                "---\ntype: task\ntitle: Tarea {i}\ndescription: depende del componente\ndepends_on:\n  - component.md\n---\n\n# Tarea {i}\n\ncuerpo\n"
            ),
        );
    }
    // Decoy: una nota SIN relación tipada al target (no debe contar como bloqueo).
    write(
        dir.path(),
        "nota.md",
        "---\ntype: note\ntitle: Nota\ndescription: irrelevante\n---\n\n# Nota\n\nsin dependencias.\n",
    );
    dir
}

/// E11-H05 · Criterio `impacto_delete_bloqueos` (benchmark §17: "Borrar un concepto referenciado →
/// rechazo con blockers"):
/// Dado un concepto con 3 relaciones obligatorias entrantes, Cuando `impact_analyze(kind:delete)`,
/// Entonces `blockingReferences.len() == 3` y `summary.risk == "high"`.
#[test]
fn impacto_delete_bloqueos() {
    let dir = bundle_delete_bloqueos();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"impact_analyze","arguments":{"ref":{"path":"component.md"},"proposedOperation":{"kind":"delete"}}}}"#,
        ],
        1,
    );
    let sc = &resp[0]["result"]["structuredContent"];

    // `blockingReferences` es una lista de 3 blockers, uno por relación tipada entrante que rompería.
    let blockers = sc["blockingReferences"].as_array().unwrap_or_else(|| {
        panic!(
            "impact_analyze(delete) debe devolver structuredContent.blockingReferences (array): {resp:?}"
        )
    });
    assert_eq!(
        blockers.len(),
        3,
        "3 relaciones obligatorias entrantes ⇒ blockingReferences.len() == 3: {resp:?}"
    );

    // Cada blocker es `{ path, reason }`: `path` string, `reason` no vacío.
    for b in blockers {
        let path = b["path"].as_str().unwrap_or_else(|| {
            panic!("cada blockingReference debe llevar un `path` string: {b:?}")
        });
        assert!(
            path.starts_with("tarea"),
            "los blockers deben ser las 3 tareas que dependen del componente, apareció: {b:?}"
        );
        let reason = b["reason"].as_str().unwrap_or("");
        assert!(
            !reason.is_empty(),
            "cada blockingReference debe llevar un `reason` no vacío: {b:?}"
        );
    }

    // No vacuo: el decoy `nota.md` (sin relación tipada al target) NO debe ser un blocker.
    assert!(
        !blockers.iter().any(|b| b["path"] == "nota.md"),
        "un concepto sin relación tipada al target NO debe contar como bloqueo: {resp:?}"
    );

    // `summary.blockingReferences` (contador) coherente con la lista.
    assert_eq!(
        sc["summary"]["blockingReferences"].as_u64(),
        Some(3),
        "summary.blockingReferences debe ser 3 (coherente con la lista): {resp:?}"
    );

    // Nivel de riesgo ALTO fijado como `"high"` (conjunto cerrado {low,medium,high}, wire inglés).
    assert_eq!(
        sc["summary"]["risk"], "high",
        "borrar un concepto con 3 relaciones obligatorias entrantes ⇒ summary.risk == «high»: {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// E12-H08 — Tool `change_plan` (orquesta: normaliza + simula + valida, SIN escribir).
//
// UBICACIÓN: los 4 criterios se ejercitan **e2e por la tool MCP** (campo Pruebas de la historia:
// `crates/lodestar-mcp/tests/`), coherente con E10-H08…E11-H05. Lo que hay que fijar es el contrato
// de **wire** (forma de `arguments` con `expectedWorkspaceRevision?`/`operations`/`policy`, forma
// del `structuredContent` con `changeSetId`/`baseWorkspaceRevision`/`planHash`/`normalizedOperations`
// /…, y cómo aflora `REVISION_CONFLICT`) sin acoplar los tests a los tipos internos que el
// implementador aún no ha creado (`App::change_plan`, el enum de op crudas, `ChangeSet`, `PlanHash`,
// `PlanPolicy`, etc.).
//
// FASE ROJA: la tool `change_plan` NO está en `tools::list()` todavía, así que
// `tools/call {name:"change_plan"}` devuelve el error de protocolo `-32602` (tool desconocida) y
// `result` es `null` → los helpers que leen `result.structuredContent.*` fallan por AUSENCIA de la
// tool/servicio (no por un valor erróneo). Ese es el rojo correcto: la tool + `App::change_plan` no
// existen. (`plan_no_escribe` se blinda contra la vacuidad: exige PRIMERO que el plan se produjo —
// así el rojo lo dispara la tool ausente, no la ausencia de escritura, que un `-32602` cumpliría de
// balde.)
//
// WIRE DE ENTRADA asumido (el implementador puede refinar los tipos internos, no el wire):
//   arguments: {
//     expectedWorkspaceRevision?: "blake3:…",   // omitido = se toma la revisión actual del workspace
//     operations: [                              // ops CRUDAS, discriminadas por «op»
//       { "op": "create",            "path": "<RelPath>", "type": "<DocType>",
//                                    "title"?: "…", "body"?: "…" },
//       { "op": "patch_frontmatter", "ref": { "path": "<RelPath>" },
//                                    "patch": { … },               // merge-patch RFC 7386 (null borra)
//                                    "expectedRevision"?: "blake3:…" },  // control optimista por op
//       …                                        // (las 11 ops del REFACTOR §11.1)
//     ],
//     policy: { "requireConformantResult"?: bool, "allowWarnings"?: bool }
//   }
//   `expectedRevision` es OPCIONAL por op y es el `ConceptRevision` (E10-H03, «blake3:…») que el
//   agente cree vigente; si el concepto cambió (revisión actual distinta) → `REVISION_CONFLICT`.
//
// WIRE DE SALIDA asumido (`structuredContent`, `REFACTOR §11.1`, `ARCHITECTURE.md §19.5`):
//   {
//     changeSetId, baseWorkspaceRevision, planHash, canApply, expiresAt,
//     normalizedOperations: [ … ],   // una `NormalizedOperation` resuelta por cada op cruda
//     risk, semanticDiff, impact, diagnosticsBefore, diagnosticsAfter
//   }
//   `planHash` es DETERMINISTA: mismo `operations` + misma `baseWorkspaceRevision` ⇒ mismo `planHash`.
//   `change_plan` NO escribe: toda la simulación es sobre un `Bundle` en memoria (invariante #1, la
//   escritura real es E13).
//
// FIRMA DE SERVICIO ASUMIDA (el implementador la crea con su propia elección de tipos internos):
//   App::change_plan(expected_workspace_revision: Option<WorkspaceRevision>, operations, policy)
//       -> Result<ChangeSet-o-PlanResult, ErrorCode>   // con `REVISION_CONFLICT` en discrepancia
// ---------------------------------------------------------------------------

/// Bundle con un cluster de **4 conceptos relacionados** conformes (`a`/`b`/`c`/`d`, enlazados en
/// anillo y listados en el índice) sobre el que las pruebas montan una propuesta de 5 operaciones
/// (1 `create` del 5º concepto + 4 `patch_frontmatter` sobre los existentes). Todos llevan
/// `type`/`title`/`description` → el bundle base es conforme, así que un plan sin errores es posible.
fn bundle_cinco_relacionados() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [A](a.md)\n* [B](b.md)\n* [C](c.md)\n* [D](d.md)\n",
    );
    // Anillo a→b→c→d→a: un cluster relacionado (los enlaces de cuerpo los conectan).
    for (slug, next) in [("a", "b"), ("b", "c"), ("c", "d"), ("d", "a")] {
        let up = slug.to_uppercase();
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: Concept\ntitle: {up}\ndescription: nodo {slug} del cluster\n---\n\n# {up}\n\n[Siguiente]({next}.md)\n"
            ),
        );
    }
    dir
}

/// Construye la línea `tools/call change_plan` con `operations`/`policy` y un
/// `expectedWorkspaceRevision` opcional. Documenta el wire de entrada que fija esta historia.
fn change_plan_line(
    expected_ws_rev: Option<&str>,
    operations: serde_json::Value,
    policy: serde_json::Value,
) -> String {
    let mut args = serde_json::json!({ "operations": operations, "policy": policy });
    if let Some(r) = expected_ws_rev {
        args["expectedWorkspaceRevision"] = serde_json::Value::String(r.to_string());
    }
    serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "change_plan", "arguments": args }
    })
    .to_string()
}

/// Devuelve el `structuredContent` de una respuesta `change_plan`, tras verificar que es un objeto.
/// En fase ROJA (tool ausente) `result` es `null` → panica con un mensaje que documenta el porqué
/// del rojo (la tool/servicio ausente), no un fallo espurio.
fn plan_sc(resp: &serde_json::Value) -> &serde_json::Value {
    let sc = &resp["result"]["structuredContent"];
    assert!(
        sc.is_object(),
        "change_plan debe devolver structuredContent (objeto); tool/servicio ausente en fase ROJA: {resp:?}"
    );
    sc
}

/// Snapshot del conocimiento en disco: `RelPath` → contenido de cada `.md` (recursivo). Excluye
/// `.lodestar/` (cache/runtime, no conocimiento canónico — invariante #1/#5). Sirve para aseverar
/// que `change_plan` NO escribió: el mapa antes y después debe ser idéntico.
fn snapshot_md(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    fn walk(
        base: &std::path::Path,
        dir: &std::path::Path,
        map: &mut std::collections::BTreeMap<String, String>,
    ) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                // `.lodestar/` es cache/runtime (index.db, planes): no es conocimiento canónico.
                if path.file_name().and_then(|n| n.to_str()) == Some(".lodestar") {
                    continue;
                }
                walk(base, &path, map);
            } else if path.extension().and_then(|e| e.to_str()) == Some("md") {
                let rel = path
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                map.insert(rel, std::fs::read_to_string(&path).unwrap());
            }
        }
    }
    let mut map = std::collections::BTreeMap::new();
    walk(root, root, &mut map);
    map
}

/// Las 5 operaciones de la propuesta base: 1 `create` del 5º concepto + 4 `patch_frontmatter` sobre
/// el cluster `a`/`b`/`c`/`d`. Los `patch` son inocuos (actualizan `description`) para que el plan
/// pueda ser conforme; lo que fija el criterio es que salgan **5** `normalizedOperations`.
fn cinco_operaciones() -> serde_json::Value {
    serde_json::json!([
        { "op": "create", "path": "nuevo.md", "type": "Concept", "title": "Nuevo",
          "body": "# Nuevo\n\ncuerpo del quinto concepto\n" },
        { "op": "patch_frontmatter", "ref": { "path": "a.md" }, "patch": { "description": "a actualizada por el plan" } },
        { "op": "patch_frontmatter", "ref": { "path": "b.md" }, "patch": { "description": "b actualizada por el plan" } },
        { "op": "patch_frontmatter", "ref": { "path": "c.md" }, "patch": { "description": "c actualizada por el plan" } },
        { "op": "patch_frontmatter", "ref": { "path": "d.md" }, "patch": { "description": "d actualizada por el plan" } },
    ])
}

/// Política permisiva (no exige resultado conforme, admite warnings): así el criterio de
/// `plan_un_solo_changeset`/`plan_hash_determinista` no depende del veredicto de conformidad.
fn policy_permisiva() -> serde_json::Value {
    serde_json::json!({ "requireConformantResult": false, "allowWarnings": true })
}

/// E12-H08 · Criterio `plan_un_solo_changeset` (benchmark §17: "Cambiar cinco conceptos relacionados
/// → un único change set"):
/// Dado una propuesta de 5 operaciones sobre conceptos relacionados, Cuando se planifica, Entonces
/// se obtiene un **único** `ChangeSet` (un solo `changeSetId`) con `normalizedOperations` de los 5.
#[test]
fn plan_un_solo_changeset() {
    let dir = bundle_cinco_relacionados();
    let line = change_plan_line(None, cinco_operaciones(), policy_permisiva());
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);
    let sc = plan_sc(&resp[0]);

    // Un solo change set: un `changeSetId` presente y no vacío.
    let id = sc["changeSetId"]
        .as_str()
        .unwrap_or_else(|| panic!("change_plan debe devolver un `changeSetId` (string): {resp:?}"));
    assert!(
        !id.is_empty(),
        "el `changeSetId` del plan no debe estar vacío: {resp:?}"
    );

    // `normalizedOperations` con exactamente 5 entradas (una por op cruda), en un ÚNICO change set.
    let normalized = sc["normalizedOperations"].as_array().unwrap_or_else(|| {
        panic!("change_plan debe devolver structuredContent.normalizedOperations (array): {resp:?}")
    });
    assert_eq!(
        normalized.len(),
        5,
        "las 5 operaciones propuestas deben producir 5 normalizedOperations en un único change set: {resp:?}"
    );

    // Es un plan, no un error de ejecución.
    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "una propuesta válida de 5 ops no debe dar isError: {resp:?}"
    );
}

/// E12-H08 · Criterio `plan_revision_conflict` (benchmark §17: "Modificar un concepto cambiado
/// externamente → REVISION_CONFLICT"):
/// Dado el `expectedRevision` de un concepto que luego cambia EN DISCO, Cuando se planifica una op
/// sobre él con esa revisión vieja, Entonces `REVISION_CONFLICT`.
#[test]
fn plan_revision_conflict() {
    let dir = bundle_cinco_relacionados();

    // 1) Revisión actual de `a.md` (ConceptRevision, «blake3:…»), vía knowledge_get (tool existente).
    let get = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"a.md"},"include":["revision"]}}}"#,
        ],
        1,
    );
    let old_rev = get[0]["result"]["structuredContent"]["concept"]["revision"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("knowledge_get debe devolver concept.revision de «a.md»: {get:?}")
        })
        .to_string();
    assert!(
        old_rev.starts_with("blake3:"),
        "la revisión de partida debe tener formato «blake3:…»: {old_rev}"
    );

    // 2) `a.md` cambia EN DISCO (otro contenido ⇒ otra ConceptRevision): simula un cambio externo.
    write(
        dir.path(),
        "a.md",
        "---\ntype: Concept\ntitle: A\ndescription: CAMBIADA EXTERNAMENTE fuera del plan\n---\n\n# A\n\notro cuerpo distinto\n",
    );

    // 3) `change_plan` con una op sobre `a.md` que trae la revisión VIEJA ⇒ discrepancia optimista.
    let ops = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "a.md" },
          "patch": { "description": "descripción desde el plan" },
          "expectedRevision": old_rev },
    ]);
    let line = change_plan_line(None, ops, policy_permisiva());
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);

    // Es un error de EJECUCIÓN de la tool (no de protocolo): aflora como isError con el código
    // estable `REVISION_CONFLICT` visible al agente (ErrorCode wire, E10-H02 / invariante #4).
    assert_eq!(
        resp[0]["result"]["isError"], true,
        "un expectedRevision obsoleto debe dar isError en change_plan: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "un conflicto de revisión NO debe ser un error de protocolo JSON-RPC: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("REVISION_CONFLICT"),
        "el error debe exponer el código estable «REVISION_CONFLICT»: {resp:?}"
    );
}

/// E12-H08 · Criterio `plan_hash_determinista`:
/// Dado el mismo `operations` y la misma `baseWorkspaceRevision` (mismo bundle sin cambios entre
/// medias), Cuando se planifica dos veces, Entonces el `planHash` coincide. Para que NO sea vacuo
/// (un stub con hash constante lo pasaría) se añade una tercera llamada con un input DISTINTO y se
/// exige que su `planHash` difiera.
#[test]
fn plan_hash_determinista() {
    let dir = bundle_cinco_relacionados();
    let line = change_plan_line(None, cinco_operaciones(), policy_permisiva());

    // Dos servidores frescos sobre el MISMO bundle (misma baseWorkspaceRevision), mismo input.
    let a = roundtrip(dir.path(), &[line.as_str()], 1);
    let b = roundtrip(dir.path(), &[line.as_str()], 1);

    let hash_a = plan_sc(&a[0])["planHash"]
        .as_str()
        .unwrap_or_else(|| panic!("change_plan debe devolver un `planHash` (string): {a:?}"))
        .to_string();
    let hash_b = plan_sc(&b[0])["planHash"]
        .as_str()
        .unwrap_or_else(|| panic!("change_plan debe devolver un `planHash` (string): {b:?}"))
        .to_string();
    assert!(
        !hash_a.is_empty(),
        "el `planHash` no debe estar vacío: {a:?}"
    );
    assert_eq!(
        hash_a, hash_b,
        "mismo input + misma baseWorkspaceRevision ⇒ mismo planHash: {a:?} vs {b:?}"
    );

    // La base sobre la que se computa el plan también coincide (mismo bundle, misma revisión).
    assert_eq!(
        plan_sc(&a[0])["baseWorkspaceRevision"],
        plan_sc(&b[0])["baseWorkspaceRevision"],
        "sobre el mismo bundle la baseWorkspaceRevision debe coincidir: {a:?} vs {b:?}"
    );

    // No vacuo: un input DISTINTO (otras ops) debe producir un planHash distinto.
    let ops_otro = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "a.md" },
          "patch": { "description": "una descripción completamente distinta" } },
    ]);
    let line_otro = change_plan_line(None, ops_otro, policy_permisiva());
    let c = roundtrip(dir.path(), &[line_otro.as_str()], 1);
    let hash_c = plan_sc(&c[0])["planHash"]
        .as_str()
        .unwrap_or_else(|| panic!("change_plan debe devolver un `planHash` (string): {c:?}"))
        .to_string();
    assert_ne!(
        hash_a, hash_c,
        "un input distinto debe producir un planHash distinto (el hash no puede ser constante): {a:?} vs {c:?}"
    );
}

/// E12-H08 · Criterio `plan_no_escribe`:
/// Dado un `change_plan` (incluida una op `create`), Cuando termina, Entonces el disco NO cambió:
/// ningún `.md` se modificó y NO se creó el fichero del `create`. La simulación es en memoria
/// (invariante #1; la escritura real es E13).
#[test]
fn plan_no_escribe() {
    let dir = bundle_cinco_relacionados();

    // Estado del conocimiento en disco ANTES.
    let antes = snapshot_md(dir.path());

    let line = change_plan_line(None, cinco_operaciones(), policy_permisiva());
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);

    // No vacuo: primero exige que el plan SE PRODUJO (si no, un `-32602` sin escritura pasaría de
    // balde). Así el rojo lo dispara la tool ausente, no la (trivial) ausencia de escritura.
    let sc = plan_sc(&resp[0]);
    assert!(
        sc["changeSetId"].as_str().is_some(),
        "change_plan debe producir un plan (changeSetId) para que el criterio no sea vacuo: {resp:?}"
    );
    let normalized = sc["normalizedOperations"].as_array().unwrap_or_else(|| {
        panic!("change_plan debe devolver normalizedOperations (array): {resp:?}")
    });
    assert!(
        !normalized.is_empty(),
        "el plan debe incluir la op `create` (entre otras): {resp:?}"
    );

    // Estado del conocimiento en disco DESPUÉS: idéntico bit a bit.
    let despues = snapshot_md(dir.path());
    assert_eq!(
        antes, despues,
        "change_plan NO debe escribir: los .md en disco deben quedar idénticos"
    );

    // La op `create nuevo.md` NO debe materializar el fichero en disco (solo en el bundle en memoria).
    assert!(
        !dir.path().join("nuevo.md").exists(),
        "una op `create` en change_plan NO debe crear el .md en disco: {resp:?}"
    );
}
