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
