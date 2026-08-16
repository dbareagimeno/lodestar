//! Test de integración del MCP (E7): handshake + tools/call sobre stdio. stdout debe ser JSON-RPC puro.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

fn write(dir: &std::path::Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

// NOTA E14-H06: el test `handshake_y_tools_call_conformance` se RETIRÓ al retirar la superficie
// heredada. Ejercitaba dos cosas heredadas —`query` presente en `tools/list` y la salida de
// `conformance_check` (`conform`/`hardFail`)— más una no-heredada (el `serverInfo.name` de
// `initialize`). La conformidad la cubre hoy `knowledge_check` (scope workspace) y sus tests e2e
// (`check_detecta_edicion_directa`, `check_scope_affected`, `check_ids_estables`); la presencia de
// las tools la fija `tools_list_solo_objetivo`; el `serverInfo.name` se migró a
// `initialize_ecoa_version_soportada`.

/// Arranca el servidor sobre un workspace, envía `lines` y devuelve las primeras `expect` respuestas.
///
/// **Escribir puede fallar, y no es un fallo del test**: hay casos que ejercitan precisamente el
/// gate de arranque —una config con una clave desconocida, un `--root` inválido—, donde el servidor
/// muere ANTES de leer nada. Ahí la tubería ya está cerrada y el `write` devuelve `EPIPE`; si eso
/// reventara el arnés, el test que comprueba esa muerte fallaría **por conseguir lo que buscaba**, y
/// solo a veces: es una carrera entre la muerte del hijo y la escritura del padre, así que pasa en
/// local y falla bajo la carga del CI. Un `EPIPE` se traduce en «no llegaron respuestas», que es lo
/// que esos tests aseveran. Mismo criterio que `roundtrip_con_config` (más abajo), donde ya se había
/// resuelto sin propagarlo aquí.
fn roundtrip(dir: &std::path::Path, lines: &[&str], expect: usize) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(dir)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    for l in lines {
        if writeln!(stdin, "{l}").is_err() {
            break;
        }
    }
    let _ = stdin.flush();
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

fn workspace_min() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    dir
}

/// E2E del protocolo: parse error → -32700 (no silencio), ping → {}, método desconocido → -32601,
/// tool desconocida → -32602, error de EJECUCIÓN de tool → result con isError (no error JSON-RPC).
#[test]
fn protocolo_errores_y_ping() {
    let dir = workspace_min();
    let resp = roundtrip(
        dir.path(),
        &[
            "{esto no es json",
            r#"{"jsonrpc":"2.0","id":1,"method":"ping"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"metodo/inexistente"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"no_existe","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"../fuera.md"}}}}"#,
        ],
        5,
    );
    assert_eq!(resp[0]["error"]["code"], -32700);
    assert_eq!(resp[0]["id"], serde_json::Value::Null);
    assert_eq!(resp[1]["result"], serde_json::json!({}));
    assert_eq!(resp[2]["error"]["code"], -32601);
    assert_eq!(resp[3]["error"]["code"], -32602);
    // Ruta inválida (`../` fuera del workspace) = error de EJECUCIÓN de la tool → isError en el result,
    // no un error de protocolo. Vehículo migrado en E14-H06 de la tool heredada `find_backlinks` a la
    // tool objetivo `knowledge_get` (la propiedad probada es del protocolo, no de la tool retirada).
    assert_eq!(resp[4]["result"]["isError"], true);
    assert!(resp[4]["error"].is_null());
}

/// tools/list lleva inputSchema (obligatorio en el spec) y structuredContent siempre es objeto.
#[test]
fn tools_list_schema_y_structured_content_objeto() {
    let dir = workspace_min();
    write(
        dir.path(),
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"knowledge_search","arguments":{"text":""}}}"#,
        ],
        2,
    );
    let tools = resp[0]["result"]["tools"].as_array().unwrap();
    // El propósito de este test es la FORMA (inputSchema de objeto en TODAS las tools) y que el
    // `structuredContent` de una tool sea un objeto, no el total exacto (que fija
    // `tools_list_solo_objetivo`). Se ancla con el mínimo de las 10 tools objetivo. E14-H06 migró el
    // universo desde «10 heredadas + workspace_status» a las 10 objetivo.
    assert!(
        tools.len() >= 10,
        "se esperaban al menos las 10 tools objetivo: {}",
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
    // `structuredContent` siempre es un objeto (spec MCP). Vehículo migrado en E14-H06 de la tool
    // heredada `query` a la tool objetivo `knowledge_search`.
    assert!(resp[1]["result"]["structuredContent"].is_object());
    assert!(resp[1]["result"]["structuredContent"]["results"].is_array());
}

// NOTA E14-H06: los tests `create_concept_escribe_y_query_lo_ve` y
// `create_concept_sin_body_genera_heading_por_defecto` se RETIRARON al retirar las tools heredadas
// `create_concept`/`query`/`conformance_check`. La escritura validada de un documento la cubre hoy el
// par `change_plan` + `change_apply` (`plan_un_solo_changeset`, `apply_ok`: la op `create` planifica
// y `change_apply` escribe el `.md` por el único escritor), su localización posterior la cubre
// `knowledge_search`, y la conformidad `knowledge_check`.
//
// El heading por defecto sin `body` cambia DE PROPÓSITO en la superficie objetivo: la op `create` de
// `change_plan` genera `# {título}` (`crates/lodestar-core/src/plan.rs`, `apply_one`), no el
// `# {Tipo} - {Nombre}` de la heredada `create_concept`. Esa nueva semántica es una responsabilidad
// del core (con su propia cobertura en `plan.rs`), no un hueco de la superficie MCP.

/// initialize ecoa la protocolVersion del cliente si la soporta.
#[test]
fn initialize_ecoa_version_soportada() {
    let dir = workspace_min();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26"}}"#,
        ],
        1,
    );
    assert_eq!(resp[0]["result"]["protocolVersion"], "2025-03-26");
    // Migrado desde `handshake_y_tools_call_conformance` (retirado en E14-H06 al retirar la tool
    // heredada `conformance_check`): la única propiedad no-heredada de aquel test era que
    // `initialize` identifica al servidor por nombre. Se conserva aquí.
    assert_eq!(resp[0]["result"]["serverInfo"]["name"], "lodestar-mcp");
}

/// E23-H13 · Guarda del texto `instructions` servido en `initialize`.
///
/// El `instructions` **no es documentación**: viaja por el wire y es lo primero que lee un agente,
/// así que un nombre de operación obsoleto ahí se convierte en una llamada fallida del cliente. Y
/// es justo el sitio donde nadie miró en toda E23: hasta esta historia decía «huérfanos» (la
/// operación se llama `isolated` desde E16-H02, y un `orphans` hoy es `INVALID_SCHEMA`) y
/// «conformidad» (el wire dice `valid` desde E23-H14).
///
/// Dos aserciones, ambas contra el binario real:
/// 1. el texto no contiene vocabulario RETIRADO —el de v0.2 (`bundle`/`concepto`/`orphans`/
///    `huérfanos`/`conforme`) y el que retiró la propia E23 (`sort`, `apply_fix`,
///    `externalReferences`)—;
/// 2. nombra EXACTAMENTE las tools que sirve `tools/list`, ni una de menos (un flujo que se salta
///    una tool la deja invisible) ni una de más (una tool que no existe manda al agente a un
///    `-32602`). Esto último ata el texto a la superficie viva: si un día entra o sale una tool, el
///    test cae aquí en vez de envejecer en silencio.
#[test]
fn instructions_sin_vocabulario_retirado() {
    let dir = workspace_min();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ],
        2,
    );

    let instructions = resp[0]["result"]["instructions"]
        .as_str()
        .expect("initialize sirve «instructions» (string)")
        .to_lowercase();

    // Vocabulario retirado. `huerfano` sin tilde cubre una reintroducción sin acentuar.
    for retirado in [
        "huérfano",
        "huerfano",
        "orphan",
        "conforme",
        "conformidad",
        "apply_fix",
        "externalreferences",
        "sort",
        "bundle",
        "concepto",
    ] {
        assert!(
            !instructions.contains(retirado),
            "el `instructions` que ve el agente usa vocabulario RETIRADO: «{retirado}»\n\
             (es superficie de wire, no documentación: lo que diga aquí es lo que el cliente \
             intentará llamar)\n---\n{instructions}"
        );
    }

    // Las tools nombradas en el texto == las tools servidas por `tools/list`.
    let servidas: Vec<String> = resp[1]["result"]["tools"]
        .as_array()
        .expect("tools/list devuelve un array de tools")
        .iter()
        .map(|t| {
            t["name"]
                .as_str()
                .expect("cada tool tiene «name»")
                .to_string()
        })
        .collect();
    assert_eq!(servidas.len(), 10, "la superficie objetivo es de 10 tools");
    for tool in &servidas {
        assert!(
            instructions.contains(tool.as_str()),
            "`{tool}` está en `tools/list` pero el `instructions` no la nombra"
        );
    }
    // Ninguna tool RETIRADA sobrevive en el texto: las heredadas de E14-H06, `schema_inspect`
    // (E20-H03) y las 3 git (E9-H01). Solo se listan los nombres que NO son subcadena de nada
    // vivo: `query` lo es de `graph_query`, `neighborhood` es hoy una OPERACIÓN legítima de
    // `graph_query`, y `history`/`commit` aparecen dentro de palabras corrientes.
    for retirada in [
        "conformance_check",
        "find_backlinks",
        "find_orphans",
        "find_dangling",
        "create_concept",
        "update_frontmatter",
        "generate_index",
        "generate_tag_indexes",
        "schema_inspect",
        "last_conforming_commit",
    ] {
        assert!(
            !instructions.contains(retirada),
            "el `instructions` nombra la tool RETIRADA `{retirada}`: invocarla es -32602"
        );
    }
}

/// E9-H01 · Criterio `list_sin_tools_git`:
/// Dado un servidor MCP arrancado, Cuando un cliente pide `tools/list`, Entonces NO aparece
/// ninguna de las 3 tools git (`history`/`last_conforming_commit`/`commit`) en el catálogo.
#[test]
fn list_sin_tools_git() {
    let dir = workspace_min();
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
    let dir = workspace_min();
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
// CLI asumida (aún NO implementada — de ahí el ROJO): `lodestar-mcp <workspace> [--profile
// readonly|standard]`, por defecto `standard`. `capabilities.writes` = (perfil == standard).
// ---------------------------------------------------------------------------

/// Como [`roundtrip`], pero arranca el servidor con `--profile <profile>` tras el workspace.
/// El perfil aún no existe en producción: este helper documenta la superficie CLI que la historia
/// introduce y produce el ROJO cuando el flag / la tool todavía no están.
fn roundtrip_profile(
    dir: &std::path::Path,
    profile: &str,
    lines: &[&str],
    expect: usize,
) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
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
    // Mismo criterio que `roundtrip`: un `--profile` inválido también muere en el arranque.
    for l in lines {
        if writeln!(stdin, "{l}").is_err() {
            break;
        }
    }
    let _ = stdin.flush();
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

/// Workspace con **exactamente 4 documentos aislados**: un `index.md` raíz que no enlaza a nadie más
/// 3 `.md` que no se enlazan entre sí ni reciben enlaces. Desde E16-H02 aislado = documento sin
/// enlaces internos entrantes NI salientes (`§20.7`) e `index.md` es un documento más del
/// inventario: sin enlaces de ningún tipo, también está aislado.
fn workspace_con_cuatro_aislados() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    // index.md sin enlaces salientes: no "adopta" a ningún documento.
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
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
/// Dado un workspace con 4 documentos aislados, Cuando se llama `workspace_status`, Entonces
/// `counts.isolated == 4` y `workspaceRevision` está presente (formato `blake3:…`).
///
/// El conteo cambió de 3 a 4 con E16-H02: `counts.orphans` pasó a `counts.isolated` y el
/// `index.md` del fixture —sin enlaces entrantes ni salientes— ya es un documento del inventario.
#[test]
fn status_counts() {
    let dir = workspace_con_cuatro_aislados();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"workspace_status","arguments":{}}}"#,
        ],
        1,
    );
    let sc = &resp[0]["result"]["structuredContent"];
    assert_eq!(
        sc["counts"]["isolated"].as_u64(),
        Some(4),
        "workspace_status debe reportar counts.isolated == 4: {resp:?}"
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
    let dir = workspace_con_cuatro_aislados();
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

// NOTA E15-H06: el test `directorio_no_workspace_sale_con_3` se BORRÓ. Afirmaba que un directorio
// sin `index.md`/`.lodestar/` aborta con exit 3 — exactamente el gate que esta historia elimina
// (`ARCHITECTURE.md §20.1`: «cd my-project && lodestar-mcp funciona»). Su contrario es hoy
// `arranca_en_directorio_arbitrario`.

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
//
// ESTADO HOY (este bloque describe el contrato de E10-H09, que ya NO es el vigente):
//   · `filters` (los filtros OKF privilegiados) se retiró en E19-H05 → `where`/`filter`, el
//     lenguaje de consulta tipado; con él cayeron `type`/`status`/`description`/`tags` del
//     `SearchResult`.
//   · `sort` se retiró en E23-H11: se aceptaba y se IGNORABA en silencio. El orden determinista
//     (score desc, path asc) es el único, y es la base del cursor-offset.
//   · E23-H11 añadió `include: ["frontmatter.<fieldPath>"]` y el mapa `frontmatter` del hit.
//   Firma vigente y forma del wire: `crates/lodestar-mcp/tests/descubribilidad.rs`.
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

/// Workspace con un documento que casa el texto «autenticación» (en título y cuerpo) más un decoy que
/// NO casa: así el criterio no es vacuo (un stub que devuelva todo incluiría el decoy y fallaría).
fn workspace_autenticacion() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Auth](auth.md)\n",
    );
    write(
        dir.path(),
        "auth.md",
        "---\ntype: decision\ntitle: Autenticación con tokens\ndescription: Cómo autenticar usuarios\nstatus: accepted\ntags: [seguridad]\n---\n\n# Resumen\n\nDecidimos usar autenticación basada en tokens rotatorios.\n",
    );
    write(
        dir.path(),
        "bici.md",
        "---\ntype: document\ntitle: Bicicletas\ndescription: sobre ruedas\n---\n\n# H\n\nnada que ver con el tema.\n",
    );
    dir
}

/// E10-H09 · Criterio `search_sin_cuerpos` (benchmark §17: "Encontrar una decisión por significado"):
/// Dado un corpus con un documento que casa «autenticación», Cuando se busca ese texto, Entonces
/// aparece con `snippet` y `revision`, y SIN `body`.
#[test]
fn search_sin_cuerpos() {
    let dir = workspace_autenticacion();
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

    // El documento que casa aparece.
    let auth = results
        .iter()
        .find(|r| r["path"] == "auth.md")
        .unwrap_or_else(|| panic!("el documento que casa «autenticación» debe aparecer: {resp:?}"));

    // `snippet` no vacío.
    let snippet = auth["snippet"].as_str().unwrap_or("");
    assert!(
        !snippet.is_empty(),
        "el result debe traer un `snippet` no vacío: {auth:?}"
    );

    // `revision` con formato de identidad de contenido `blake3:…` (DocumentRevision, E10-H03).
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

    // No vacuo: un documento que no casa el texto NO debe aparecer.
    assert!(
        !results.iter().any(|r| r["path"] == "bici.md"),
        "un documento que no casa «autenticación» no debe aparecer en los resultados: {resp:?}"
    );
}

/// Workspace con documentos `type:decision` mezclados con otros tipos, todos con el mismo texto en el
/// cuerpo para que el único discriminante sea el filtro de tipo.
fn workspace_tipos_mixtos() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
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
        "documento.md",
        "---\ntype: document\ntitle: Documento\ndescription: arquitectura\n---\n\n# H\n\ncuerpo sobre arquitectura.\n",
    );
    dir
}

/// E10-H09 · Criterio `search_filtra_tipo` (MIGRADO en E19-H05 al lenguaje de consulta):
/// El filtro por `type` dejó de ser un campo privilegiado (`filters.types`) y pasa por el `where`
/// tipado. Dado `where: type = "decision"`, Cuando se busca, Entonces solo aparecen los documentos
/// cuyo `type` de frontmatter es `decision` (los demás quedan fuera). El resultado ya no surfacea el
/// campo `type` —eso lo fija `search_result_sin_campos_okf`—, así que la aserción es por `path`.
#[test]
fn search_filtra_tipo() {
    let dir = workspace_tipos_mixtos();
    let resp = roundtrip(
        dir.path(),
        &[ks_call(serde_json::json!({ "where": "type = \"decision\"" })).as_str()],
        1,
    );
    let paths = search_paths(&resp[0]);
    let tiene = |p: &str| paths.iter().any(|x| x == p);

    // No vacuo: el filtro casa los documentos `type: decision` (si devolviese vacío, las
    // exclusiones de abajo pasarían trivialmente).
    assert!(
        !paths.is_empty(),
        "con `where` type=decision debe haber al menos un resultado: {resp:?}"
    );
    for decision in ["dec-uno.md", "dec-dos.md"] {
        assert!(
            tiene(decision),
            "el documento `{decision}` (type: decision) debe aparecer con `where` type=decision: {resp:?}"
        );
    }

    // Un documento de otro tipo NO aparece: el `where` filtra por metadata, sin coerción.
    for otro in ["nota.md", "documento.md", "index.md"] {
        assert!(
            !tiene(otro),
            "un documento de `type` != decision (`{otro}`) no debe aparecer al filtrar por decision: {resp:?}"
        );
    }
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

/// Workspace con **50 documentos** que casan todos el texto «paginacion» (en `description` y cuerpo),
/// deterministas por slug (`c00`…`c49`). El `index.md` no contiene el token y no cuenta como
/// documento, así que la búsqueda casa exactamente 50.
fn workspace_cincuenta() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    for i in 0..50 {
        let slug = format!("c{i:02}");
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: document\ntitle: Documento {i:02}\ndescription: paginacion\n---\n\n# H\n\ncuerpo paginacion numero {i:02}.\n"
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
    let dir = workspace_cincuenta();

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
        "la página 3 debe traer los 10 documentos restantes: {p3:?}"
    );
    assert!(
        sc3["nextCursor"].is_null(),
        "agotados los 50 resultados `nextCursor` debe ser null: {p3:?}"
    );

    // No repite ni omite: la unión de las 3 páginas cubre los 50 documentos, todos únicos.
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
        "la unión de las 3 páginas debe cubrir los 50 documentos sin omisiones"
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
// fijar es el contrato de **wire** (forma de `arguments`, forma del `document` en `structuredContent`,
// acotado de body por sección, cómo aflora el error `DOCUMENT_NOT_FOUND`) sin acoplar los tests a los
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
//     ref: { path: "<RelPath>" },                 // DocumentRef (E10-H04); deser de { "path": … }
//     include?: [ "frontmatter" | "body" | "revision" | "outgoingLinks" | "backlinks"
//                 | "diagnostics" | "externalReferences" ],   // selectivo: qué campos se pueblan
//     sections?: [ [ "<heading>", "<subheading>", … ] ]       // cada headingPath acota el body
//   }
//   structuredContent: {
//     document: { path, revision, frontmatter?, body?, outgoingLinks?, backlinks?,
//                externalReferences?, diagnostics? }
//   }
// `document.revision` == `DocumentRevision` (E10-H03), formato `blake3:…`, presente siempre (identidad).
// Un campo NO pedido en `include` NO se puebla (queda nulo/ausente).
//
// FIRMA DE SERVICIO ASUMIDA (el implementador la crea con su propia elección de tipos internos):
//   App::knowledge_get(r: &DocumentRef, include: &[…], sections: Option<&[…]>)
//       -> Result<{ document: { path, revision, frontmatter, body, outgoingLinks, backlinks,
//                              externalReferences, diagnostics } }, ErrorCode>
//   con `DOCUMENT_NOT_FOUND` cuando `resolve_ref` no encuentra el path (E10-H04).
// ---------------------------------------------------------------------------

/// Workspace con un documento conforme `alfa.md` (frontmatter completo) para los casos que solo necesitan
/// un documento existente al que pedirle `revision`/`frontmatter`, y para el caso inexistente (pedir un
/// path que NO está en el workspace).
fn workspace_get_revision() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Alfa](alfa.md)\n",
    );
    write(
        dir.path(),
        "alfa.md",
        "---\ntype: decision\ntitle: Alfa\ndescription: Primer documento\nstatus: accepted\ntags: [seguridad]\n---\n\n# Resumen\n\nCuerpo del documento alfa.\n",
    );
    dir
}

/// E10-H10 · Criterio `get_incluye_revision`:
/// Dado un documento existente, Cuando se pide con `include:[frontmatter,revision]`, Entonces devuelve
/// la `revision` (== `DocumentRevision`, formato `blake3:…`) y el `frontmatter`. Se añade que un campo
/// NO pedido (`body`) queda sin poblar, para que el `include` selectivo sea significativo (no vacuo).
#[test]
fn get_incluye_revision() {
    let dir = workspace_get_revision();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"alfa.md"},"include":["frontmatter","revision"]}}}"#,
        ],
        1,
    );
    let document = &resp[0]["result"]["structuredContent"]["document"];

    // `revision` presente y con formato de identidad de contenido `blake3:…` (DocumentRevision, E10-H03).
    let revision = document["revision"].as_str().unwrap_or_else(|| {
        panic!("knowledge_get debe devolver document.revision (string «blake3:…»): {resp:?}")
    });
    assert!(
        revision.starts_with("blake3:"),
        "document.revision debe tener formato «blake3:…»: {resp:?}"
    );

    // `frontmatter` presente (objeto no nulo) porque se pidió en `include`.
    assert!(
        document["frontmatter"].is_object(),
        "con include:[frontmatter] el documento debe traer un `frontmatter` (objeto): {resp:?}"
    );

    // `include` selectivo: `body` NO se pidió ⇒ no se puebla (nulo o ausente). Sin esta comprobación
    // el criterio sería vacuo (una impl que devuelve todos los campos siempre lo cumpliría igual).
    assert!(
        document["body"].is_null(),
        "con include:[frontmatter,revision] el `body` NO debe poblarse: {resp:?}"
    );
}

/// Workspace con un documento cuyo cuerpo tiene una jerarquía de headings clara: `## Security` con la
/// subsección objetivo `### Token rotation`, más secciones/subsecciones hermanas que DEBEN quedar
/// fuera al acotar por `sections:[["Security","Token rotation"]]`. Cada bloque lleva un marcador único
/// para que las comprobaciones de subcadena sean inequívocas:
///   - `TOKEN-OBJETIVO-INCLUIR`  → bajo `## Security → ### Token rotation` (DEBE aparecer).
///   - `TOKEN-HERMANA-SUB-EXCLUIR` → bajo `## Security → ### Otra` (subsección hermana; DEBE quedar
///     fuera; su exclusión obliga a que el 2º nivel del headingPath cuente, no solo `## Security`).
///   - `TOKEN-HERMANA-TOP-EXCLUIR` → bajo `## Otra seccion` (sección hermana de nivel superior; fuera).
///   - `TOKEN-OVERVIEW-EXCLUIR`   → bajo `## Overview` (otra sección de nivel superior; fuera).
fn workspace_get_secciones() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Rotacion](decisiones/rotacion.md)\n",
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
    let dir = workspace_get_secciones();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"decisiones/rotacion.md"},"include":["body"],"sections":[["Security","Token rotation"]]}}}"#,
        ],
        1,
    );
    let document = &resp[0]["result"]["structuredContent"]["document"];
    let body = document["body"].as_str().unwrap_or_else(|| {
        panic!("knowledge_get con include:[body] debe devolver document.body (string): {resp:?}")
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
/// Dado un path inexistente, Cuando se pide, Entonces `DOCUMENT_NOT_FOUND`. En la superficie MCP un
/// documento inexistente es un error de EJECUCIÓN de la tool (no un fallo de protocolo): aflora como
/// `result.isError == true` con el código estable `DOCUMENT_NOT_FOUND` visible al agente (ErrorCode
/// wire de E10-H02, `REFACTOR §13` / invariante #4), no como un error JSON-RPC.
#[test]
fn get_inexistente() {
    let dir = workspace_get_revision(); // tiene `alfa.md`; pedimos un path que NO existe.
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
        "un DocumentRef a un path inexistente debe dar isError en knowledge_get: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "un documento inexistente NO debe ser un error de protocolo JSON-RPC: {resp:?}"
    );
    // El código estable `DOCUMENT_NOT_FOUND` debe ser visible en la respuesta (no un mensaje opaco).
    let texto = resp[0].to_string();
    assert!(
        texto.contains("DOCUMENT_NOT_FOUND"),
        "el error debe exponer el código estable «DOCUMENT_NOT_FOUND»: {resp:?}"
    );
}

// E20-H03 — RETIRADOS los tests E10-H11 de `schema_inspect` (`inspect_type`/`inspect_catalog`/
// `inspect_sin_schema`): la tool se sustituyó por `metadata_inspect`, cuyo contrato de wire fija la
// sección E20-H03 de este fichero (`tool_es_metadata_inspect`/`metadata_inspect_catalog`/
// `metadata_inspect_field`).

// ---------------------------------------------------------------------------
// E10-H12 — Tool `knowledge_check` (sustituye `conformance_check`).
//
// UBICACIÓN: los 3 criterios se ejercitan **e2e por la tool MCP** (campo Pruebas de la historia:
// `crates/lodestar-mcp/tests/`), coherente con E10-H08…H11. Lo que hay que fijar es el contrato de
// **wire** (forma de `scope`, forma del `structuredContent` con `valid`/`summary`/
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
//          | { kind: "document",  ref: { path } }
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
//     valid: bool,
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
//       -> Result<{ valid, summary, diagnostics, workspaceRevision, nextCursor }, _>
//   Compone `DocumentSet::analyze` (los 15 checks OKF) + `validate_schema(doc_set, schema)` (E10-H07);
//   `affected` acota por vecindad (`DocumentSet::neighborhood` / `Store::blast_radius`).
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

/// Workspace con un `.md` **editado a mano** cuyo frontmatter es inválido.
///
/// MIGRADO en E16-H05: el disparador era la falta del campo `type` (`OKF-TYPE`), y un documento
/// sin `type` pasó a ser perfectamente válido. Hoy el `.md` editado a mano tiene el frontmatter
/// **sintácticamente roto** → `FM-YAML-INVALID` (severidad `Err`), que es exactamente el mismo
/// escenario contado con el catálogo mínimo de `§20.9`: alguien lo escribió a pelo y lo dejó
/// ilegible. El workspace es por lo demás válido, así que el ÚNICO error viene de la edición directa.
fn workspace_editado_a_mano() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Editado](editado-a-mano.md)\n",
    );
    // Bloque bien delimitado pero con YAML inválido → FM-YAML-INVALID (Err). Simula a alguien que
    // editó el .md a pelo y rompió la sintaxis.
    write(
        dir.path(),
        "editado-a-mano.md",
        "---\ntitle: : :\n  - roto\ndescription: alguien lo escribió a pelo\n---\n\n# Nota\n\ncuerpo suelto.\n",
    );
    dir
}

/// E10-H12 · Criterio `check_detecta_edicion_directa` (benchmark §17):
/// Dado un `.md` editado a mano con frontmatter inválido, Cuando se hace `knowledge_check` de scope
/// `workspace`, Entonces aparece el diagnóstico de ese path y el veredicto es no conforme.
#[test]
fn check_detecta_edicion_directa() {
    let dir = workspace_editado_a_mano();
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
    // Y es exactamente el hard-fail FM-YAML-INVALID — no un warning cualquiera.
    assert!(
        del_fichero.iter().any(|d| d["code"] == "FM-YAML-INVALID"),
        "el diagnóstico de «editado-a-mano.md» debe ser FM-YAML-INVALID (frontmatter ilegible): {resp:?}"
    );

    // Veredicto global: NO conforme (hay al menos un error).
    assert_eq!(
        resp[0]["result"]["structuredContent"]["valid"],
        serde_json::Value::Bool(false),
        "con un frontmatter inválido el workspace NO debe ser conforme: {resp:?}"
    );
}

/// Workspace para `check_scope_affected`. Grafo de vecindad **bidireccional** (robusto a la dirección
/// que use el implementador — out/in/both) alrededor del ref `centro.md`:
///
///   index.md ──► centro.md ◄──► vecino.md ◄──► c.md          lejano.md   (aislado)
///
/// MIGRADO en E16-H05: los tres documentos con diagnóstico lo obtenían por no tener `type`
/// (`OKF-TYPE`, retirado). Hoy lo obtienen por llevar **marcadores de merge sin resolver**
/// (`DOC-CONFLICT-MARKER`, `Err`), que es un diagnóstico del catálogo mínimo y —a diferencia de un
/// frontmatter ilegible— deja el cuerpo y sus **enlaces** intactos, que es lo que este escenario
/// necesita para que el vecindario exista.
///
/// - `centro.md` (A): el ref; sin diagnóstico. Enlaza a `vecino.md`.
/// - `vecino.md` (B, distancia 1): con marcadores → `DOC-CONFLICT-MARKER`. Enlaza a `centro`
///   y a `c` (así, en CUALQUIER dirección, B está a distancia 1 y C a distancia 2 de A).
/// - `c.md` (C, distancia 2): con marcadores → `DOC-CONFLICT-MARKER`. Enlaza a `vecino`.
/// - `lejano.md` (D, NO conectado): con marcadores → `DOC-CONFLICT-MARKER`. Su diagnóstico
///   DEBE quedar fuera del scope `affected {refs:[centro], depth:2}`.
///
/// El criterio es inequívoco: con `depth:2` el vecindario de A es exactamente {centro, vecino, c};
/// `lejano` está a distancia infinita y no puede colarse.
fn workspace_affected() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Centro](centro.md)\n",
    );
    // A: conforme, enlaza a B.
    write(
        dir.path(),
        "centro.md",
        "---\ntype: document\ntitle: Centro\ndescription: nodo raíz del vecindario\n---\n\n# Centro\n\n[Vecino](vecino.md)\n",
    );
    // B (distancia 1): con marcadores → DOC-CONFLICT-MARKER. Enlaza a A y a C (bidireccional).
    write(
        dir.path(),
        "vecino.md",
        "---\ntitle: Vecino\ndescription: a distancia 1 de centro\n---\n\n# Vecino\n\n[Centro](centro.md)\n\n[C](c.md)\n\n<<<<<<< HEAD\nuno\n=======\ndos\n>>>>>>> rama\n",
    );
    // C (distancia 2): con marcadores → DOC-CONFLICT-MARKER. Enlaza a B (bidireccional).
    write(
        dir.path(),
        "c.md",
        "---\ntitle: C\ndescription: a distancia 2 de centro\n---\n\n# C\n\n[Vecino](vecino.md)\n\n<<<<<<< HEAD\nuno\n=======\ndos\n>>>>>>> rama\n",
    );
    // D (lejano, aislado): con marcadores → DOC-CONFLICT-MARKER. Sin ningún enlace desde/hacia el
    // vecindario.
    write(
        dir.path(),
        "lejano.md",
        "---\ntitle: Lejano\ndescription: desconectado del vecindario\n---\n\n# Lejano\n\ncuerpo sin enlaces.\n\n<<<<<<< HEAD\nuno\n=======\ndos\n>>>>>>> rama\n",
    );
    dir
}

/// E10-H12 · Criterio `check_scope_affected`:
/// Dado `scope:affected` con un ref (`centro.md`) y `depth:2`, Cuando se llama `knowledge_check`,
/// Entonces solo aparecen diagnósticos del vecindario (vecino a distancia 1 y c a distancia 2), y
/// NO el del documento lejano y desconectado.
#[test]
fn check_scope_affected() {
    let dir = workspace_affected();
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
    // El documento LEJANO y desconectado NO debe aparecer: es lo que hace inequívoco el scope.
    assert!(
        !diags_cubren(&diags, "lejano.md"),
        "el diagnóstico de «lejano.md» (desconectado) NO debe estar en el scope affected: {resp:?}"
    );
}

/// Workspace con DOS ficheros con diagnóstico, para que el conjunto de `id` sea significativo (≥1,
/// aquí ≥2) al comparar estabilidad entre revisiones. MIGRADO en E16-H05: el disparador era el
/// frontmatter sin `type` (`OKF-TYPE`, retirado); hoy es un frontmatter con YAML inválido
/// (`FM-YAML-INVALID`).
fn workspace_dos_diagnosticos() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Uno](uno.md)\n* [Dos](dos.md)\n",
    );
    for slug in ["uno", "dos"] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntitle: : :\n  - {slug}\ndescription: yaml roto\n---\n\n# H\n\ncuerpo.\n"
            ),
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
/// Dada la misma revisión dos veces (dos servidores frescos sobre el MISMO workspace sin cambios),
/// Cuando se hace `knowledge_check` de scope `workspace`, Entonces el conjunto de `id` de
/// diagnóstico coincide entre ambas llamadas (misma revisión → mismos ids).
#[test]
fn check_ids_estables() {
    let dir = workspace_dos_diagnosticos();
    let call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_check","arguments":{"scope":{"kind":"workspace"}}}}"#;

    // Dos procesos frescos sobre el mismo workspace: misma revisión de workspace.
    let a = roundtrip(dir.path(), &[call], 1);
    let b = roundtrip(dir.path(), &[call], 1);

    let ids_a = diag_ids(&a[0]);
    let ids_b = diag_ids(&b[0]);

    // Significativo: hay al menos un diagnóstico (si no, la igualdad sería vacua).
    assert!(
        !ids_a.is_empty(),
        "el workspace debe producir al menos un diagnóstico para que el criterio no sea vacuo: {a:?}"
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
//   (workspace_status/knowledge_search/knowledge_get/metadata_inspect/knowledge_check) deben declarar
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
//
// ENDURECIDO tras el defecto del `outputSchema` de `metadata_inspect`: tal como nació, este test
// miraba 5 de las 10 tools y aceptaba como válida CUALQUIERA de
// `["type","$ref","properties","allOf","oneOf","anyOf","$defs","definitions"]` en la raíz. Con
// `anyOf` en esa allowlist, pasaba en verde sobre un schema que un cliente MCP estricto (Claude
// Code) rechazaba —y al rechazar una tool inválida deja de registrar las diez, así que el servidor
// entero quedaba inutilizable—. La laxitud no era descuido: el criterio original solo pedía
// «parece un JSON Schema». Hoy exige lo que el spec exige de verdad, en las 10: la raíz declara
// `type: "object"`. La guardia gemela en proceso vive en
// `tools.rs::tools_list_lleva_output_schema_de_tipo_object`, junto a la de `inputSchema` que sí
// llevaba esta comprobación desde el principio.
// ---------------------------------------------------------------------------

/// E10-H13 · Criterio `tools_declaran_outputschema` (ENDURECIDO, ver la nota de arriba):
/// Dado `tools/list`, Cuando se inspecciona **cada una de las 10** tools activas, Entonces cada una
/// incluye `outputSchema` y su raíz declara `type: "object"`, como exige el spec MCP.
///
/// Se exigen las 10 (no basta con `workspace_status`): un stub que solo añadiera `outputSchema` a
/// una tool no pasaría. Y se exige `type: "object"` en la raíz, no «alguna clave estructural»: un
/// `anyOf` pelado es lo que un cliente estricto rechaza.
#[test]
fn tools_declaran_outputschema() {
    let dir = workspace_min();
    let resp = roundtrip(
        dir.path(),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#],
        1,
    );
    let tools = resp[0]["result"]["tools"]
        .as_array()
        .expect("tools/list devuelve un array de tools");

    // Las 10 tools objetivo (`§19.6`): TODAS deben declarar `outputSchema` (D6b), no solo las 5 de
    // lectura/verificación de E10 con las que nació este criterio.
    let con_output = [
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
    assert_eq!(
        tools.len(),
        con_output.len(),
        "la superficie debe ser exactamente las 10 tools objetivo: {tools:?}"
    );
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
        assert_eq!(
            output["type"], "object",
            "el `outputSchema` de «{name}» debe declarar `type: \"object\"` en la raíz: el spec \
             MCP lo exige y un cliente estricto que rechaza una tool inválida deja de registrar \
             LAS DIEZ: {output:?}"
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
// la tool contra la **verdad del core** (`DocumentSet::neighborhood`, invariante #3): se abre el MISMO
// workspace en proceso con `App::open` y se computa `neighborhood(path, 2, Both)`; los `nodes`/`edges`
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
//     operation: "backlinks" | "outgoing" | "neighborhood" | "isolated" | "dangling",
//     ref?:       { path: "<RelPath>" },       // DocumentRef; obligatorio en backlinks/outgoing/neighborhood
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
//   Reusa `DocumentSet::backlinks`/`DocumentSet::neighborhood` y `Analysis::isolated`/`dangling` (verdad del
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
/// Dado un documento (`objetivo.md`) con **3 backlinks**, Cuando se llama
/// `graph_query(operation:backlinks, ref:{path})`, Entonces los 3 aparecen en `nodes`/`edges`.
///
/// Workspace: `a.md`/`b.md`/`c.md` enlazan a `objetivo.md`; `d.md` es un decoy que enlaza a OTRO
/// documento (`a.md`), no a `objetivo.md`, para que el criterio no sea vacuo (un stub que devolviera
/// todos los documentos incluiría a `d` como fuente y fallaría). `index.md` NO lista `objetivo.md`
/// (así el índice no aporta aristas entrantes al target).
#[test]
fn graph_backlinks() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [A](a.md)\n* [B](b.md)\n* [C](c.md)\n* [D](d.md)\n",
    );
    write(
        dir.path(),
        "objetivo.md",
        "---\ntype: document\ntitle: Objetivo\ndescription: recibe 3 backlinks\n---\n\n# Objetivo\n\ncuerpo.\n",
    );
    for slug in ["a", "b", "c"] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: document\ntitle: {slug}\ndescription: enlaza al objetivo\n---\n\n# {slug}\n\n[Objetivo](objetivo.md)\n"
            ),
        );
    }
    // Decoy: enlaza a `a.md`, NUNCA a `objetivo.md`.
    write(
        dir.path(),
        "d.md",
        "---\ntype: document\ntitle: D\ndescription: no enlaza al objetivo\n---\n\n# D\n\n[A](a.md)\n",
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

/// Workspace con un vecindario dirigido no trivial alrededor de `centro.md`, con aristas de entrada y de
/// salida a distancia 1 y 2, más un `lejano.md` aislado que DEBE quedar fuera de
/// `neighborhood(centro, 2, Both)`:
///
///   abuelo.md ──► raiz.md ──► centro.md ──► vecino.md ──► c.md        lejano.md (aislado)
///
/// `neighborhood(centro, 2, Both)` = {centro, vecino, c (out, d2), raiz, abuelo (in, d2)}; `lejano`
/// a distancia infinita. `index.md` no enlaza a documentos (evita ruido de aristas reservadas).
fn workspace_vecindario() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        dir.path(),
        "centro.md",
        "---\ntype: document\ntitle: Centro\ndescription: raiz del vecindario\n---\n\n# Centro\n\n[Vecino](vecino.md)\n",
    );
    write(
        dir.path(),
        "vecino.md",
        "---\ntype: document\ntitle: Vecino\ndescription: salida a distancia 1\n---\n\n# Vecino\n\n[C](c.md)\n",
    );
    write(
        dir.path(),
        "c.md",
        "---\ntype: document\ntitle: C\ndescription: salida a distancia 2\n---\n\n# C\n\ncuerpo.\n",
    );
    write(
        dir.path(),
        "raiz.md",
        "---\ntype: document\ntitle: Raiz\ndescription: entrada a distancia 1\n---\n\n# Raiz\n\n[Centro](centro.md)\n",
    );
    write(
        dir.path(),
        "abuelo.md",
        "---\ntype: document\ntitle: Abuelo\ndescription: entrada a distancia 2\n---\n\n# Abuelo\n\n[Raiz](raiz.md)\n",
    );
    write(
        dir.path(),
        "lejano.md",
        "---\ntype: document\ntitle: Lejano\ndescription: desconectado\n---\n\n# Lejano\n\ncuerpo sin enlaces.\n",
    );
    dir
}

/// E11-H01 · Criterio `graph_neighborhood_paridad`:
/// Dado `operation:neighborhood, depth:2, direction:both`, Cuando se llama, Entonces el subgrafo
/// (`nodes`/`edges`) casa **exactamente** con `DocumentSet::neighborhood(path, 2, Both)` del core
/// (invariante #3: el grafo es una verdad computada del core).
#[test]
fn graph_neighborhood_paridad() {
    use lodestar_core::types::{Direction, RelPath};

    let dir = workspace_vecindario();

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

    // 2) Verdad del core: se abre el MISMO workspace en proceso (el hijo del roundtrip ya terminó) y se
    //    computa `neighborhood(centro, 2, Both)` con la lógica pura del core.
    let app = lodestar_app::App::open(dir.path()).expect("el workspace temporal debe abrir");
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
        "el documento aislado «lejano.md» no debe estar en el vecindario del core: {nb_json:?}"
    );

    // Paridad: los nodos y aristas del wire coinciden (como conjuntos) con los del core.
    assert_eq!(
        wire_nodes, core_nodes,
        "los `nodes` de graph_query(neighborhood) deben casar con DocumentSet::neighborhood del core: {resp:?}"
    );
    assert_eq!(
        wire_edges, core_edges,
        "los `edges` de graph_query(neighborhood) deben casar con DocumentSet::neighborhood del core: {resp:?}"
    );
}

/// E11-H01 · Criterio `graph_orphans`, MIGRADO a `isolated` en E16-H02:
/// Dado un workspace con documentos aislados, Cuando se llama `graph_query(operation:isolated)`,
/// Entonces lista exactamente esos paths (los documentos sin enlaces internos entrantes NI
/// salientes, `§20.7`).
///
/// Workspace: `uno`/`dos`/`tres` no tienen enlaces de ningún tipo → aislados; `visible.md` recibe uno
/// (desde `index.md`) e `index.md` emite uno → **ninguno de los dos** está aislado. Esos dos hacen
/// el criterio no vacuo por partida doble: excluyen tanto al stub que devolviera todos los
/// documentos como al que confundiera «aislado» con «sin entrantes» (que incluiría `index.md`).
#[test]
fn graph_isolated() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Visible](visible.md)\n",
    );
    write(
        dir.path(),
        "visible.md",
        "---\ntype: document\ntitle: Visible\ndescription: listado en el indice\n---\n\n# Visible\n\ncuerpo.\n",
    );
    for slug in ["uno", "dos", "tres"] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: document\ntitle: {slug}\ndescription: huerfano\n---\n\n# {slug}\n\ncuerpo suelto.\n"
            ),
        );
    }

    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"graph_query","arguments":{"operation":"isolated"}}}"#,
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
        "graph_query(isolated) debe listar exactamente los 3 documentos aislados: {resp:?}"
    );
    // No vacuo: quien recibe un enlace y quien lo emite NO están aislados.
    assert!(
        !ids.contains("visible.md") && !ids.contains("index.md"),
        "«visible.md» (entrante) e «index.md» (saliente) no están aislados: {resp:?}"
    );
}

/// E11-H01 · Operación `dangling` de `graph_query`.
/// Dado un workspace con un enlace colgante (a una página inexistente), Cuando se llama
/// `graph_query(operation:dangling)`, Entonces el target colgante aparece listado como nodo (fantasma)
/// y un target que sí resuelve NO aparece.
///
/// Aserción MIGRADA en E14-H06 desde el golden heredado `golden_orphans_y_dangling_igual_workspace`
/// (que ejercitaba la tool retirada `find_dangling` comparando su salida con `Analysis::dangling`, la
/// LISTA de targets colgantes): su mitad de huérfanos ya la cubre `graph_orphans`, pero la de dangling
/// no tenía equivalente en la superficie objetivo. Se conserva aquí sobre `graph_query(dangling)`, su
/// reemplazo semántico (`contracts/mcp.yml §15`), sobre la misma propiedad: la lista de targets
/// colgantes son los nodos devueltos (que es como `graph_query(dangling)` proyecta `Analysis::dangling`,
/// invariante #3).
///
/// Workspace: `fuente.md` enlaza a `inexistente.md` (colgante) y `otro.md` enlaza a `existe.md` (que sí
/// existe → NO colgante). El enlace que resuelve hace el criterio no vacuo (un stub que devolviera
/// todos los targets incluiría `existe.md` y fallaría).
#[test]
fn graph_dangling() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        dir.path(),
        "fuente.md",
        "---\ntype: document\ntitle: Fuente\ndescription: enlaza a algo inexistente\n---\n\n# Fuente\n\n[Roto](inexistente.md)\n",
    );
    write(
        dir.path(),
        "otro.md",
        "---\ntype: document\ntitle: Otro\ndescription: enlaza a algo que existe\n---\n\n# Otro\n\n[Existe](existe.md)\n",
    );
    write(
        dir.path(),
        "existe.md",
        "---\ntype: document\ntitle: Existe\ndescription: destino real\n---\n\n# Existe\n\ncuerpo.\n",
    );

    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"graph_query","arguments":{"operation":"dangling"}}}"#,
        ],
        1,
    );

    // El target colgante aparece listado como nodo.
    let ids = graph_node_ids(&graph_nodes(&resp[0]));
    assert!(
        ids.contains("inexistente.md"),
        "graph_query(dangling) debe listar el target colgante «inexistente.md» como nodo: {resp:?}"
    );

    // No vacuo: un target que SÍ resuelve no es colgante y NO debe aparecer.
    assert!(
        !ids.contains("existe.md"),
        "«existe.md» existe y no es un target colgante; no debe aparecer en graph_query(dangling): {resp:?}"
    );
}

/// E11-H01 · Criterio `graph_truncado`:
/// Dado un `limit` menor que el nº de nodos, Cuando se llama, Entonces `summary.truncated == true` y
/// `nextCursor` está presente (no nulo).
///
/// Workspace con **11 documentos aislados** (`o00`…`o09` más el `index.md`, que desde E16-H02 es un
/// documento más y tampoco tiene enlaces): `graph_query(isolated, limit:5)` trunca. Para que el
/// criterio NO sea vacuo (un stub que devolviera siempre `truncated:true` lo pasaría) se hace una
/// segunda llamada con `limit:20 >= 11`: entonces `truncated == false` y `nextCursor == null`.
#[test]
fn graph_truncado() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    for i in 0..10 {
        let slug = format!("o{i:02}");
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!(
                "---\ntype: document\ntitle: Orphan {i:02}\ndescription: huerfano\n---\n\n# H\n\ncuerpo suelto {i:02}.\n"
            ),
        );
    }

    // Llamada truncada: limit:5 < 10 nodos.
    let trunc = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"graph_query","arguments":{"operation":"isolated","limit":5}}}"#,
        ],
        1,
    );
    let sc = &trunc[0]["result"]["structuredContent"];
    assert_eq!(
        sc["summary"]["truncated"],
        serde_json::Value::Bool(true),
        "con limit:5 < 11 nodos, summary.truncated debe ser true: {trunc:?}"
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
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"graph_query","arguments":{"operation":"isolated","limit":20}}}"#,
        ],
        1,
    );
    let sc_full = &full[0]["result"]["structuredContent"];
    assert_eq!(
        sc_full["summary"]["truncated"],
        serde_json::Value::Bool(false),
        "con limit:20 >= 11 nodos, summary.truncated debe ser false: {full:?}"
    );
    assert!(
        sc_full["nextCursor"].is_null(),
        "sin truncar, `nextCursor` debe ser null: {full:?}"
    );
    assert_eq!(
        graph_nodes(&full[0]).len(),
        11,
        "sin truncar, deben aparecer los 11 documentos aislados: {full:?}"
    );
}

// ---------------------------------------------------------------------------
// E11-H05 — Tool `impact_analyze` (reusa blast-radius + neighborhood).
//
// UBICACIÓN: los criterios de comportamiento (`impacto_move_30`, `impacto_delete_bloqueos`) se
// ejercitan **e2e por la tool MCP** (campo Pruebas de la historia: `crates/lodestar-mcp/tests/`),
// coherente con E10-H08…H12 y E11-H01. Lo que hay que fijar aquí es el contrato de **wire**
// (forma de `arguments` con `ref`/`proposedOperation`/`depth`, forma del `structuredContent` con
// `summary`/`affectedDocuments`/`blockingReferences`/`recommendations`) sin acoplar los tests a los
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
//     ref: { path: "<RelPath>" },                       // DocumentRef (E10-H04); deser de { path }
//     proposedOperation: {
//       kind: "move" | "delete"                            // E21-H01: solo las de impacto (§20.10)
//     },
//     depth?: integer                                    // profundidad del blast-radius; def. impl.
//   }
//
// WIRE DE SALIDA asumido (`structuredContent`, `ARCHITECTURE.md §19.6`, `REFACTOR §9.6`):
//   {
//     summary: {
//       directlyAffected: number,        // nº de backlinks DIRECTOS del ref (DocumentSet::backlinks)
//       transitivelyAffected: number,    // tamaño del blast-radius (== neighborhood(In) del core)
//       blockingReferences: number,      // == blockingReferences.len()
//       risk: "low" | "medium" | "high"  // nivel derivado de nº de afectados/bloqueos
//     },
//     affectedDocuments: [ … ],           // documentos alcanzados (paths / nodos)
//     blockingReferences: [ { path: "<RelPath>", reason: "<texto>" } ],
//     recommendations: [ … ]             // acciones sugeridas (texto)
//   }
//
// DECISIÓN DE WIRE FIJADA POR ESTA HISTORIA (el implementador debe respetarla):
//   - `summary.risk` es un string en INGLÉS del conjunto cerrado {"low","medium","high"},
//     coherente con el resto del wire camelCase/inglés (`direction:"in"`, `minimumSeverity:"err"`,
//     claves `directlyAffected`/`blockingReferences`). El NIVEL ALTO es exactamente `"high"`.
//   - Un `blockingReference` (para `kind:"delete"`) = un documento que declara una **relación
//     tipada del schema** (`RelationDef`, E11-H03) cuyo target es el `ref`. Cada blocker es
//     `{ path, reason }`: `path` = el documento que depende del ref; `reason` = texto no vacío que
//     explica el bloqueo (p. ej. el nombre de la relación que quedaría rota). Esta es la lectura
//     literal del alcance de la historia ("relaciones obligatorias que quedarían rotas"): las
//     dependencias estructurales tipadas, NO los enlaces sueltos de cuerpo Markdown.
//
// FIRMA DE SERVICIO ASUMIDA (el implementador la crea con su propia elección de tipos internos):
//   App::impact_analyze(ref: &DocumentRef, proposed_operation_kind, depth: Option<u32>)
//       -> Result<{ summary, affectedDocuments, blockingReferences, recommendations }, _>
//   `directlyAffected` compone `DocumentSet::backlinks`; `transitivelyAffected` reusa
//   `Store::blast_radius` (verificado idéntico a `neighborhood(In)` por `impacto_paridad_core`);
//   `blockingReferences` compone `validate_relations`/`RelationDef` (E11-H03).
// ---------------------------------------------------------------------------

/// Workspace con un documento `target.md` al que apuntan **exactamente 30** documentos vía un enlace de
/// cuerpo Markdown (`[t](/target.md)`), y NINGÚN otro backlink. El `index.md` NO lista `target.md`
/// (así el índice no aporta entrantes) y los 30 emisores no reciben backlinks entre sí, de
/// modo que `directlyAffected` del target es 30 bajo cualquier lectura (inbound-solo o
/// inbound+index). Deterministas por slug (`emisor00`…`emisor29`).
fn workspace_treinta_backlinks() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    // index.md sin enlaces salientes: no aporta ningún entrante al target.
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        dir.path(),
        "target.md",
        "---\ntype: Concept\ntitle: Target\ndescription: el documento a mover\n---\n\n# Target\n\ncuerpo\n",
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

/// E11-H05 · Criterio `impacto_move_30` (benchmark §17: "Mover un documento con 30 backlinks"):
/// Dado un documento con 30 backlinks, Cuando `impact_analyze(kind:move)`, Entonces
/// `summary.directlyAffected == 30`.
#[test]
fn impacto_move_30() {
    let dir = workspace_treinta_backlinks();
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
        "un documento con 30 backlinks debe dar summary.directlyAffected == 30: {resp:?}"
    );
}

// E17-H05 RETIRÓ `impacto_delete_bloqueos` (y su fixture `workspace_delete_bloqueos`): era *el*
// test de los `blockingReferences` derivados de relaciones tipadas del `.lodestar/schema.yaml`, y
// esa noción desapareció con el modelo que la definía (`§20.10`: una relación es un enlace
// Markdown y nada más). El campo `blockingReferences` sigue en el wire, siempre vacío, hasta que
// E20 retire `core::schema`; que el impacto NO mire tipos ni relaciones —ni siquiera con un
// `schema.yaml` presente y relaciones declaradas hacia el documento— lo fija ahora
// `crates/lodestar-app/tests/grafo.rs::impacto_sin_tipos_okf`.

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
//       { "op": "create",            "path": "<RelPath>",
//                                    "frontmatter"?: { … },   // YAML ARBITRARIO y opcional (E23-H02)
//                                    "body"?: "…" },
//       { "op": "patch_frontmatter", "ref": { "path": "<RelPath>" },
//                                    "patch": { … },               // merge-patch RFC 7386 (null borra)
//                                    "expectedRevision"?: "blake3:…" },  // control optimista por op
//       …                                        // (las 11 ops del REFACTOR §11.1)
//     ],
//     policy: { "requireValidResult"?: bool, "allowWarnings"?: bool }
//   }
//   `expectedRevision` es OPCIONAL por op y es el `DocumentRevision` (E10-H03, «blake3:…») que el
//   agente cree vigente; si el documento cambió (revisión actual distinta) → `REVISION_CONFLICT`.
//
// WIRE DE SALIDA asumido (`structuredContent`, `REFACTOR §11.1`, `ARCHITECTURE.md §19.5`):
//   {
//     changeSetId, baseWorkspaceRevision, planHash, canApply, expiresAt,
//     normalizedOperations: [ … ],   // una `NormalizedOperation` resuelta por cada op cruda
//     risk, semanticDiff, impact, diagnosticsBefore, diagnosticsAfter
//   }
//   `planHash` es DETERMINISTA: mismo `operations` + misma `baseWorkspaceRevision` ⇒ mismo `planHash`.
//   `change_plan` NO escribe: toda la simulación es sobre un `DocumentSet` en memoria (invariante #1, la
//   escritura real es E13).
//
// FIRMA DE SERVICIO ASUMIDA (el implementador la crea con su propia elección de tipos internos):
//   App::change_plan(expected_workspace_revision: Option<WorkspaceRevision>, operations, policy)
//       -> Result<ChangeSet-o-PlanResult, ErrorCode>   // con `REVISION_CONFLICT` en discrepancia
// ---------------------------------------------------------------------------

/// Workspace con un cluster de **4 documentos relacionados** conformes (`a`/`b`/`c`/`d`, enlazados en
/// anillo y listados en el índice) sobre el que las pruebas montan una propuesta de 5 operaciones
/// (1 `create` del 5º documento + 4 `patch_frontmatter` sobre los existentes). Todos llevan
/// `type`/`title`/`description` → el workspace base es conforme, así que un plan sin errores es posible.
fn workspace_cinco_relacionados() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [A](a.md)\n* [B](b.md)\n* [C](c.md)\n* [D](d.md)\n",
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

/// Las 5 operaciones de la propuesta base: 1 `create` del 5º documento + 4 `patch_frontmatter` sobre
/// el cluster `a`/`b`/`c`/`d`. Los `patch` son inocuos (actualizan `description`) para que el plan
/// pueda ser conforme; lo que fija el criterio es que salgan **5** `normalizedOperations`.
fn cinco_operaciones() -> serde_json::Value {
    serde_json::json!([
        { "op": "create", "path": "nuevo.md",
          "body": "# Nuevo\n\ncuerpo del quinto documento\n" },
        { "op": "patch_frontmatter", "ref": { "path": "a.md" }, "patch": { "description": "a actualizada por el plan" } },
        { "op": "patch_frontmatter", "ref": { "path": "b.md" }, "patch": { "description": "b actualizada por el plan" } },
        { "op": "patch_frontmatter", "ref": { "path": "c.md" }, "patch": { "description": "c actualizada por el plan" } },
        { "op": "patch_frontmatter", "ref": { "path": "d.md" }, "patch": { "description": "d actualizada por el plan" } },
    ])
}

/// Política permisiva (no exige resultado conforme, admite warnings): así el criterio de
/// `plan_un_solo_changeset`/`plan_hash_determinista` no depende del veredicto de conformidad.
fn policy_permisiva() -> serde_json::Value {
    serde_json::json!({ "requireValidResult": false, "allowWarnings": true })
}

/// E12-H08 · Criterio `plan_un_solo_changeset` (benchmark §17: "Cambiar cinco documentos relacionados
/// → un único change set"):
/// Dado una propuesta de 5 operaciones sobre documentos relacionados, Cuando se planifica, Entonces
/// se obtiene un **único** `ChangeSet` (un solo `changeSetId`) con `normalizedOperations` de los 5.
#[test]
fn plan_un_solo_changeset() {
    let dir = workspace_cinco_relacionados();
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

/// E12-H08 · Criterio `plan_revision_conflict` (benchmark §17: "Modificar un documento cambiado
/// externamente → REVISION_CONFLICT"):
/// Dado el `expectedRevision` de un documento que luego cambia EN DISCO, Cuando se planifica una op
/// sobre él con esa revisión vieja, Entonces `REVISION_CONFLICT`.
#[test]
fn plan_revision_conflict() {
    let dir = workspace_cinco_relacionados();

    // 1) Revisión actual de `a.md` (DocumentRevision, «blake3:…»), vía knowledge_get (tool existente).
    let get = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"a.md"},"include":["revision"]}}}"#,
        ],
        1,
    );
    let old_rev = get[0]["result"]["structuredContent"]["document"]["revision"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("knowledge_get debe devolver document.revision de «a.md»: {get:?}")
        })
        .to_string();
    assert!(
        old_rev.starts_with("blake3:"),
        "la revisión de partida debe tener formato «blake3:…»: {old_rev}"
    );

    // 2) `a.md` cambia EN DISCO (otro contenido ⇒ otra DocumentRevision): simula un cambio externo.
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
/// Dado el mismo `operations` y la misma `baseWorkspaceRevision` (mismo workspace sin cambios entre
/// medias), Cuando se planifica dos veces, Entonces el `planHash` coincide. Para que NO sea vacuo
/// (un stub con hash constante lo pasaría) se añade una tercera llamada con un input DISTINTO y se
/// exige que su `planHash` difiera.
#[test]
fn plan_hash_determinista() {
    let dir = workspace_cinco_relacionados();
    let line = change_plan_line(None, cinco_operaciones(), policy_permisiva());

    // Dos servidores frescos sobre el MISMO workspace (misma baseWorkspaceRevision), mismo input.
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

    // La base sobre la que se computa el plan también coincide (mismo workspace, misma revisión).
    assert_eq!(
        plan_sc(&a[0])["baseWorkspaceRevision"],
        plan_sc(&b[0])["baseWorkspaceRevision"],
        "sobre el mismo workspace la baseWorkspaceRevision debe coincidir: {a:?} vs {b:?}"
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
    let dir = workspace_cinco_relacionados();

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

    // La op `create nuevo.md` NO debe materializar el fichero en disco (solo en el workspace en memoria).
    assert!(
        !dir.path().join("nuevo.md").exists(),
        "una op `create` en change_plan NO debe crear el .md en disco: {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// E21-H01 — Retirar las 5 operaciones semánticas del contrato transaccional.
//
// UBICACIÓN: los 3 criterios se ejercitan **e2e por la frontera MCP** (campo Pruebas de la historia:
// `crates/lodestar-mcp/tests/`), porque lo que E21-H01 cambia es el CONTRATO DE WIRE: qué `op`
// acepta `change_plan` y qué `kind` acepta `impact_analyze.proposedOperation`. Se fija aquí para que
// la superficie observable (¿plan o error?, ¿qué código?) quede clavada, no los tipos internos que
// el implementador retirará (`NormalizedOperation::{AddRelation,RemoveRelation,TransitionStatus}`,
// las ramas de `normalize_raw_op`, los `kind` del `inputSchema`).
//
// FASE ROJA (estado ANTES de E21-H01, todo commiteado y verde):
//   - `transition_status_retirada`: HOY `normalize_raw_op` SÍ despacha `op:"transition_status"`
//     (produce un `PatchFrontmatter{status:to}`), así que `change_plan` devuelve un PLAN válido, sin
//     `isError`. El test exige un error → hace ROJO porque el plan tiene éxito. Tras retirar la rama
//     del despacho, `op:"transition_status"` cae en `_ => Err(ErrorCode::InvalidSchema)` → verde.
//   - `impact_sin_ops_semanticas`: HOY `App::impact_analyze` NO valida `kind` (lo usa como texto de
//     recomendación), así que `kind:"deprecate"` devuelve un INFORME válido, sin `isError`. El test
//     exige un error → hace ROJO. Tras restringir `kind` a {move, delete} (los que `§20.10` lista
//     para impacto) → `INVALID_SCHEMA` → verde.
//   - `patch_hace_de_transicion`: NO es un test rojo. Es el GUARDIÁN de equivalencia que `§Fase 12`
//     promete ("un transition_status es un patch_frontmatter"): pasa HOY y debe seguir pasando tras
//     E21-H01, porque `patch_frontmatter` —operación universal— no cambia. Documenta que retirar las
//     semánticas NO pierde capacidad. (Reportado como no-rojo en la salida de esta fase.)
//
// FORMA DEL ERROR PARA UNA OP RETIRADA (decisión de criterio, ver informe): el código estable de
// wire es `INVALID_SCHEMA` (`ErrorCode::InvalidSchema`, E10-H02 / invariante #4). Es el MISMO código
// con que `normalize_raw_op` ya rechaza hoy cualquier `op` no reconocida (`_ => InvalidSchema`) y
// cualquier parámetro mal formado; retirar `transition_status`/`add_relation`/`remove_relation` del
// match las convierte, sin más, en "op desconocida" → `INVALID_SCHEMA`. Para `impact_analyze` se fija
// el mismo código: un `kind` fuera de {move, delete} es un esquema de entrada inválido. Aflora como
// error de EJECUCIÓN de la tool (`result.isError == true`), NUNCA como error de protocolo JSON-RPC.
// ---------------------------------------------------------------------------

/// Workspace mínimo y válido con un documento `decision.md` que lleva `status: draft` en su
/// frontmatter (más un `index.md` que lo enlaza). El `type`/`status` son metadata YAML arbitraria
/// (modelo universal, `§20.10`), no semántica OKF. Base común de los tres criterios de E21-H01:
/// la op semántica retirada (`transition_status`), el `kind` semántico retirado de `impact_analyze`
/// (`deprecate`) y la equivalencia sin pérdida (`patch_frontmatter` que fija `status: accepted`).
fn workspace_decision_draft() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntitle: Índice\n---\n\n# Índice\n\n* [Decisión](decision.md)\n",
    );
    write(
        dir.path(),
        "decision.md",
        "---\ntype: decision\ntitle: Decisión de autenticación\nstatus: draft\n---\n\n# Decisión\n\ncuerpo\n",
    );
    dir
}

/// E21-H01 · Criterio `transition_status_retirada`:
/// **Dado** un `change_plan` con `op: "transition_status"`, **Cuando** se procesa, **Entonces** es un
/// error (op desconocida) — no un plan. Fija que la operación semántica retirada deja de existir en la
/// superficie transaccional (`§Fase 12`).
#[test]
fn transition_status_retirada() {
    let dir = workspace_decision_draft();
    // Un change_plan cuyo ÚNICO op es la operación semántica retirada. El `ref.path` resuelve a un
    // documento EXISTENTE, así que la única razón posible de error es que `transition_status` ya no
    // sea una op reconocida (no un objetivo inexistente ni un parámetro mal formado).
    let ops = serde_json::json!([
        { "op": "transition_status", "ref": { "path": "decision.md" }, "to": "accepted" }
    ]);
    let line = change_plan_line(None, ops, policy_permisiva());
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);

    // Superficie observable: es un error de EJECUCIÓN de la tool, NO un plan.
    assert_eq!(
        resp[0]["result"]["isError"], true,
        "una op retirada (`transition_status`) debe dar isError en change_plan, no un plan: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "una op desconocida es un error de EJECUCIÓN de la tool, no un error de protocolo JSON-RPC: {resp:?}"
    );
    assert!(
        resp[0]["result"]["structuredContent"].is_null(),
        "una op retirada NO debe producir structuredContent (no hay change set): {resp:?}"
    );
    // Código estable de wire: op desconocida = INVALID_SCHEMA (el mismo con que `normalize_raw_op`
    // rechaza hoy cualquier `op` no reconocida).
    let texto = resp[0].to_string();
    assert!(
        texto.contains("INVALID_SCHEMA"),
        "el error de una op retirada debe exponer el código estable «INVALID_SCHEMA»: {resp:?}"
    );
}

/// E21-H01 · Criterio `impact_sin_ops_semanticas`:
/// **Dado** un `impact_analyze` con `proposedOperation.kind: "deprecate"`, **Entonces** es un error.
/// El `kind` queda restringido a las operaciones que `§20.10` lista para impacto (`move`/`delete`);
/// los `kind` semánticos (`deprecate`/`transition_status`/`change_relation`/`replace_document`) se
/// retiran del `inputSchema` y se rechazan.
#[test]
fn impact_sin_ops_semanticas() {
    let dir = workspace_decision_draft();
    // El `ref` apunta a un documento EXISTENTE: la única razón posible de error es el `kind`
    // semántico retirado (`deprecate`), no un objetivo irresoluble.
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"impact_analyze","arguments":{"ref":{"path":"decision.md"},"proposedOperation":{"kind":"deprecate"}}}}"#,
        ],
        1,
    );

    assert_eq!(
        resp[0]["result"]["isError"], true,
        "un `kind` semántico retirado (`deprecate`) debe dar isError en impact_analyze: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "un `kind` no soportado es un error de EJECUCIÓN de la tool, no un error de protocolo JSON-RPC: {resp:?}"
    );
    assert!(
        resp[0]["result"]["structuredContent"].is_null(),
        "un `kind` retirado NO debe producir un informe de impacto: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("INVALID_SCHEMA"),
        "el error de un `kind` retirado debe exponer el código estable «INVALID_SCHEMA»: {resp:?}"
    );
}

/// E21-H01 · Criterio `patch_hace_de_transicion` (equivalencia sin pérdida de capacidad, `§Fase 12`):
/// **Dado** un `change_plan` con `op: "patch_frontmatter"` que fija `status: "accepted"` sobre un
/// documento en `status: "draft"`, **Entonces** funciona: el plan resultante fija `status: accepted`.
/// La «transición» sobrevive como patch de una propiedad de frontmatter arbitraria. NO es un test
/// rojo: `patch_frontmatter` no cambia con E21-H01; es el guardián de que retirar `transition_status`
/// no pierde capacidad.
#[test]
fn patch_hace_de_transicion() {
    let dir = workspace_decision_draft();
    let ops = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "decision.md" }, "patch": { "status": "accepted" } }
    ]);
    let line = change_plan_line(None, ops, policy_permisiva());
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);
    let sc = plan_sc(&resp[0]);

    // No es un error: el patch produce un plan.
    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "un patch_frontmatter que fija `status` debe producir un plan, no isError: {resp:?}"
    );

    // El plan lleva UNA op normalizada `patch_frontmatter` que fija `status: accepted` sobre
    // `decision.md`: la transición `draft → accepted` expresada como patch.
    let normalized = sc["normalizedOperations"].as_array().unwrap_or_else(|| {
        panic!("change_plan debe devolver normalizedOperations (array): {resp:?}")
    });
    assert_eq!(
        normalized.len(),
        1,
        "una sola op cruda ⇒ una sola op normalizada: {resp:?}"
    );
    let op = &normalized[0];
    assert_eq!(
        op["op"], "patch_frontmatter",
        "la op normalizada debe ser un patch_frontmatter (no una op semántica): {resp:?}"
    );
    assert_eq!(
        op["path"], "decision.md",
        "el patch debe apuntar a decision.md: {resp:?}"
    );
    assert_eq!(
        op["patch"]["status"], "accepted",
        "el patch debe fijar `status` a «accepted» (la transición como patch): {resp:?}"
    );

    // Efecto observable en el diff semántico: `decision.md` está entre los documentos con cambio de
    // frontmatter — corrobora que la propiedad `status` cambia sin operación semántica dedicada.
    let fm_changes = sc["semanticDiff"]["frontmatterChanges"]
        .as_array()
        .unwrap_or_else(|| {
            panic!("el semanticDiff debe traer frontmatterChanges (array): {resp:?}")
        });
    let cambia_decision = fm_changes.iter().any(|p| p == "decision.md");
    assert!(
        cambia_decision,
        "el semanticDiff debe registrar decision.md como cambio de frontmatter (draft→accepted): {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// E13-H08 — Tool `change_apply` (orquestación del proceso de 15 pasos, `REFACTOR §11.2/§17`).
//
// UBICACIÓN: los 4 criterios se ejercitan **e2e por la tool MCP** (campo Pruebas de la historia:
// `crates/lodestar-mcp/tests/`), coherente con E12-H08 (`change_plan`). `change_apply` es la
// integración de toda E13 (staging/journal/copias/publicación/receipt), pero lo que hay que fijar
// AQUÍ es su contrato de **wire**: qué `arguments` toma, qué `structuredContent` devuelve al aplicar,
// y cómo afloran `PLAN_STALE`/`PLAN_EXPIRED`/`PERMISSION_DENIED`. La mecánica interna la testean las
// historias de `lodestar-workspace` (E13-H01…H07).
//
// FASE ROJA: la tool `change_apply` NO está en `tools::list()` todavía, así que
// `tools/call {name:"change_apply"}` devuelve el error de protocolo `-32602` (tool desconocida, vía
// `tools::exists`) y `result` es `null`. Por eso los asserts que leen `result.structuredContent.*` o
// `result.isError` fallan por AUSENCIA de la tool/servicio (`App::change_apply`), no por un valor
// erróneo. Ese es el rojo correcto. (El paso previo `change_plan` SÍ existe desde E12-H08, así que el
// flujo `change_plan → change_apply` deja el rojo en la segunda llamada, no en la primera.)
//
// WIRE DE ENTRADA asumido (el implementador puede refinar los tipos internos, no el wire):
//   arguments: {
//     changeSetId: "changeset:<hash>",           // el id que devolvió change_plan (E12-H08)
//     expectedWorkspaceRevision?: "blake3:…"      // control optimista a nivel de workspace; si se
//   }                                             // omite, se adopta la revisión actual del workspace
//
// WIRE DE SALIDA asumido (`structuredContent`, `REFACTOR §11.2`, `ARCHITECTURE.md §19.5/§19.6`):
//   {
//     receiptId, applied,                         // applied:true al publicar; receiptId del recibo (H07)
//     previousWorkspaceRevision, workspaceRevision,   // «blake3:…» antes/después de la transacción
//     changedPaths, semanticDiff,
//     validation: { valid, errors, warnings }
//   }
//   El `workspaceRevision` devuelto es la revisión resultante: tras un apply OK el workspace «queda
//   en» ella (comprobado contra `workspace_status`). Los errores de EJECUCIÓN (`PLAN_STALE`,
//   `PLAN_EXPIRED`, `PERMISSION_DENIED`) afloran como `result.isError == true` con el código estable
//   wire visible (ErrorCode `as_str()`, E10-H02 / invariante #4 / `REFACTOR §13`), NUNCA como error
//   JSON-RPC de transporte — mismo patrón que `DOCUMENT_NOT_FOUND`/`REVISION_CONFLICT`.
//
// FIRMA DE SERVICIO ASUMIDA (el implementador la crea con su propia elección de tipos internos):
//   App::change_apply(change_set_id: &ChangeSetId, expected_workspace_revision: Option<WorkspaceRevision>)
//       -> Result<ApplyResult, ErrorCode>
//   que carga el plan persistido (E12-H09), verifica caducidad (`PLAN_EXPIRED`) y `planHash`
//   (`PLAN_STALE`), y publica por el ÚNICO ESCRITOR con assert_writable (E11-H04 → `PERMISSION_DENIED`
//   fuera de `writableRoots`).
//
// FLUJO change_plan → change_apply: `change_plan` PERSISTE el plan en `.lodestar/runtime/plans/<hash>.json`
// (E12-H09), así que `change_apply` puede recuperarlo por `changeSetId` desde un servidor FRESCO (no
// hace falta la misma sesión stdio). Todos los tests hacen: (1) un `roundtrip` con `change_plan` para
// obtener el `changeSetId` y la `baseWorkspaceRevision`; (2) —tras la manipulación que fije el
// escenario— un segundo `roundtrip` (servidor fresco, mismo workspace) con `change_apply`.
// ---------------------------------------------------------------------------

/// Construye la línea `tools/call change_apply` con el `changeSetId` y un `expectedWorkspaceRevision`
/// opcional. Documenta el wire de entrada que fija esta historia.
fn change_apply_line(change_set_id: &str, expected_ws_rev: Option<&str>) -> String {
    let mut args = serde_json::json!({ "changeSetId": change_set_id });
    if let Some(r) = expected_ws_rev {
        args["expectedWorkspaceRevision"] = serde_json::Value::String(r.to_string());
    }
    serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "change_apply", "arguments": args }
    })
    .to_string()
}

/// `structuredContent` de una respuesta `change_apply`, tras verificar que es un objeto. En fase
/// ROJA (tool ausente) `result` es `null` → panica con un mensaje que documenta el porqué del rojo
/// (la tool/servicio `change_apply` ausente), no un fallo espurio.
fn apply_sc(resp: &serde_json::Value) -> &serde_json::Value {
    let sc = &resp["result"]["structuredContent"];
    assert!(
        sc.is_object(),
        "change_apply debe devolver structuredContent (objeto); tool/servicio ausente en fase ROJA: {resp:?}"
    );
    sc
}

/// El `changeSetId` (string, «changeset:<hash>») de una respuesta `change_plan`. Panica —documentando
/// el rojo— si el plan no se produjo (tool/servicio ausente ⇒ `structuredContent` nulo).
fn plan_change_set_id(resp: &serde_json::Value) -> String {
    plan_sc(resp)["changeSetId"]
        .as_str()
        .unwrap_or_else(|| panic!("change_plan debe devolver un `changeSetId` (string): {resp:?}"))
        .to_string()
}

/// Regresión secuencial en la frontera MCP: plan y apply ocurren en procesos stdio distintos y el
/// plan persistido debe conservar el `working` acumulado de las tres sustituciones.
#[test]
fn change_plan_apply_en_procesos_mcp_distintos_conserva_tres_sustituciones() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", "# Index\n");
    write(
        dir.path(),
        "doc.md",
        "---\ntype: Note\ntitle: Doc\n---\n\nAAA BBB CCC\n",
    );
    let ops = serde_json::json!([
        {"op":"replace_text","path":"doc.md","find":"AAA","replace":"A1"},
        {"op":"replace_text","path":"doc.md","find":"BBB","replace":"B1"},
        {"op":"replace_text","path":"doc.md","find":"CCC","replace":"C1"}
    ]);
    let plan_resp = roundtrip(
        dir.path(),
        &[&change_plan_line(None, ops, policy_permisiva())],
        1,
    );
    let change_set_id = plan_change_set_id(&plan_resp[0]);
    let apply_resp = roundtrip(dir.path(), &[&change_apply_line(&change_set_id, None)], 1);
    assert_eq!(apply_sc(&apply_resp[0])["applied"], true);
    let final_raw = std::fs::read_to_string(dir.path().join("doc.md")).unwrap();
    assert!(
        final_raw.contains("A1") && final_raw.contains("B1") && final_raw.contains("C1"),
        "plan/apply MCP en procesos distintos debe conservar las tres sustituciones: {final_raw}"
    );
}

/// Fuerza la caducidad de un plan persistido reescribiendo su `expiresAt` a un instante pasado
/// («0» epoch), como haría el paso de caducidad de E12-H09 al comparar contra `now`. El fichero es
/// `.lodestar/runtime/plans/<hash>.json`, donde `<hash>` es el `changeSetId` sin el prefijo
/// `changeset:` (mismo saneado que `plan_file_name` en `lodestar-app`). Solo toca `expiresAt`; el
/// resto del plan (incl. `planHash`) queda intacto para que la caducidad sea el ÚNICO discriminante.
fn force_plan_expired(root: &std::path::Path, change_set_id: &str) {
    let hex = change_set_id
        .strip_prefix("changeset:")
        .unwrap_or(change_set_id);
    let path = root
        .join(".lodestar")
        .join("runtime")
        .join("plans")
        .join(format!("{hex}.json"));
    let raw = std::fs::read_to_string(&path).unwrap_or_else(|e| {
        panic!("el plan persistido debe existir en {path:?} tras change_plan: {e}")
    });
    let mut plan: serde_json::Value =
        serde_json::from_str(&raw).expect("el plan persistido debe ser JSON válido");
    plan["expiresAt"] = serde_json::Value::String("0".to_string());
    std::fs::write(&path, serde_json::to_vec(&plan).unwrap()).unwrap();
}

/// E13-H08 · Criterio `apply_ok` (benchmark §17: "Crear un documento válido → plan aceptado y aplicado"):
/// Dado un plan válido y vigente, Cuando se aplica, Entonces `applied:true` y el workspace queda en el
/// `resultWorkspaceRevision` que el plan previó. Se comprueba (a) `applied==true`; (b) que
/// `previousWorkspaceRevision` == la `baseWorkspaceRevision` del plan (se aplicó sobre la base
/// prevista); (c) que la revisión AVANZÓ (`previous != workspaceRevision`, para no ser vacuo); (d) que
/// el `.md` canónico del `create` existe en disco (la escritura real ocurrió, invariante #1); y (e)
/// que un `workspace_status` posterior reporta EXACTAMENTE el `workspaceRevision` devuelto (el
/// workspace «queda en» esa revisión resultante).
#[test]
fn apply_ok() {
    let dir = workspace_min();

    // (1) Plan válido: crear un documento conforme (type/title/body ⇒ conforme, cf.
    // `create_concept_escribe_y_query_lo_ve`). Servidor fresco; el plan se persiste en runtime.
    let ops = serde_json::json!([
        { "op": "create", "path": "nuevo.md",
          "body": "# Resumen\n\ncuerpo del documento nuevo\n" },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );
    let id = plan_change_set_id(&plan[0]);
    let base = plan_sc(&plan[0])["baseWorkspaceRevision"]
        .as_str()
        .unwrap_or_else(|| panic!("change_plan debe devolver `baseWorkspaceRevision`: {plan:?}"))
        .to_string();

    // (2) Aplicar el plan por su `changeSetId` (servidor fresco, mismo workspace) + `workspace_status`.
    let status_line = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"workspace_status","arguments":{}}}"#;
    let resp = roundtrip(
        dir.path(),
        &[change_apply_line(&id, None).as_str(), status_line],
        2,
    );
    let sc = apply_sc(&resp[0]);

    // (a) El plan se aplicó.
    assert_eq!(
        sc["applied"],
        serde_json::Value::Bool(true),
        "un plan válido y vigente debe aplicarse (applied:true): {resp:?}"
    );
    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "un apply exitoso no debe dar isError: {resp:?}"
    );

    // (b) Se aplicó SOBRE la base prevista por el plan.
    let ws_rev = sc["workspaceRevision"].as_str().unwrap_or("");
    let prev = sc["previousWorkspaceRevision"].as_str().unwrap_or("");
    assert_eq!(
        prev, base,
        "previousWorkspaceRevision debe ser la baseWorkspaceRevision que previó el plan: {resp:?}"
    );

    // (c) No vacuo: la revisión resultante AVANZÓ respecto de la previa (hubo cambio real).
    assert!(
        ws_rev.starts_with("blake3:"),
        "workspaceRevision resultante ausente o mal formado («blake3:…»): {resp:?}"
    );
    assert_ne!(
        prev, ws_rev,
        "el apply debe hacer avanzar la WorkspaceRevision (prev != resultado): {resp:?}"
    );

    // (d) La escritura real ocurrió: el `.md` canónico existe con su cuerpo (invariante #1).
    let creado = dir.path().join("nuevo.md");
    assert!(
        creado.is_file(),
        "el apply debe materializar el `.md` del create en disco: {resp:?}"
    );
    let contenido = std::fs::read_to_string(&creado).unwrap();
    assert!(
        contenido.contains("cuerpo del documento nuevo"),
        "el `.md` canónico debe reflejar el cuerpo del plan: {contenido:?}"
    );

    // (e) El workspace «queda en» la revisión resultante: `workspace_status` reporta la misma.
    let status_rev = resp[1]["result"]["structuredContent"]["workspaceRevision"]
        .as_str()
        .unwrap_or("");
    assert_eq!(
        status_rev, ws_rev,
        "tras el apply, workspace_status debe reportar la workspaceRevision resultante: {resp:?}"
    );
}

/// E13-H08 · Criterio `apply_plan_stale`:
/// Dado un plan cuya `planHash` ya no casa (el workspace cambió bajo él), Cuando se aplica, Entonces
/// `PLAN_STALE` y no escribe. El drift se fuerza reescribiendo EN DISCO un `.md` que el plan toca
/// (`a.md`): cambia la `baseWorkspaceRevision` actual ⇒ el `planHash` recomputado en
/// `change_apply` (paso «re-normalizar y validar → verificar planHash», `REFACTOR §11.2`) difiere del
/// persistido ⇒ `PLAN_STALE`. Se NO pasa `expectedWorkspaceRevision` para que el discriminante sea el
/// `planHash` (no un `REVISION_CONFLICT` del control optimista de workspace).
#[test]
fn apply_plan_stale() {
    let dir = workspace_cinco_relacionados();

    // (1) Plan: un patch sobre `a.md` que fijaría una descripción reconocible.
    let ops = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "a.md" },
          "patch": { "description": "PLANNED-DESC-STALE" } },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );
    let id = plan_change_set_id(&plan[0]);

    // (2) El workspace cambia BAJO el plan: `a.md` se reescribe en disco con OTRO contenido (otra
    // WorkspaceRevision base ⇒ otro planHash recomputado).
    write(
        dir.path(),
        "a.md",
        "---\ntype: Concept\ntitle: A\ndescription: EXTERNAL-STALE-CHANGE\n---\n\n# A\n\ncuerpo cambiado por fuera del plan\n",
    );

    // (3) Aplicar (servidor fresco): el planHash ya no casa ⇒ PLAN_STALE.
    let resp = roundtrip(dir.path(), &[change_apply_line(&id, None).as_str()], 1);
    assert_eq!(
        resp[0]["result"]["isError"], true,
        "un plan con planHash obsoleto debe dar isError en change_apply: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "un plan obsoleto NO debe ser un error de protocolo JSON-RPC: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("PLAN_STALE"),
        "el error debe exponer el código estable «PLAN_STALE»: {resp:?}"
    );

    // No escribe: `a.md` conserva el contenido externo, NO la descripción que fijaba el plan.
    let en_disco = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
    assert!(
        en_disco.contains("EXTERNAL-STALE-CHANGE"),
        "un apply rechazado por PLAN_STALE no debe tocar `a.md`: {en_disco:?}"
    );
    assert!(
        !en_disco.contains("PLANNED-DESC-STALE"),
        "el patch del plan obsoleto NO debe aplicarse: {en_disco:?}"
    );
}

/// E13-H08 · Criterio `apply_plan_expired`:
/// Dado un plan caducado, Cuando se aplica, Entonces `PLAN_EXPIRED`. Se fuerza la caducidad
/// reescribiendo el `expiresAt` del plan persistido a un instante pasado (E12-H09), SIN tocar el
/// workspace (así el discriminante es la caducidad, no un PLAN_STALE por drift).
#[test]
fn apply_plan_expired() {
    let dir = workspace_cinco_relacionados();

    // (1) Plan válido sobre `a.md`.
    let ops = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "a.md" },
          "patch": { "description": "PLANNED-DESC-EXPIRED" } },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );
    let id = plan_change_set_id(&plan[0]);
    let antes = std::fs::read_to_string(dir.path().join("a.md")).unwrap();

    // (2) Caducar el plan persistido (expiresAt en el pasado).
    force_plan_expired(dir.path(), &id);

    // (3) Aplicar (servidor fresco): plan vencido ⇒ PLAN_EXPIRED.
    let resp = roundtrip(dir.path(), &[change_apply_line(&id, None).as_str()], 1);
    assert_eq!(
        resp[0]["result"]["isError"], true,
        "un plan caducado debe dar isError en change_apply: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "un plan caducado NO debe ser un error de protocolo JSON-RPC: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("PLAN_EXPIRED"),
        "el error debe exponer el código estable «PLAN_EXPIRED»: {resp:?}"
    );

    // No escribe: `a.md` queda idéntico (el plan vencido no se aplica).
    let despues = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
    assert_eq!(
        antes, despues,
        "un apply rechazado por PLAN_EXPIRED no debe tocar `a.md`"
    );
}

/// Workspace con `writableRoots:[knowledge]` y `referenceRoots:[src]`: `knowledge/` es la única raíz
/// escribible; `src/` es una raíz de referencia (visible, NUNCA escribible, E11-H04). Un plan que
/// intente CREAR un `.md` bajo `src/` debe rechazarse al aplicar (`PERMISSION_DENIED`).
fn workspace_writable_restringido() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    // Marcador de workspace en la raíz.
    write(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    // Un documento conforme dentro de la raíz escribible.
    write(
        dir.path(),
        "knowledge/documento.md",
        "---\ntype: Concept\ntitle: Documento\ndescription: dentro de knowledge\n---\n\n# H\n\ncuerpo\n",
    );
    // Un fichero cualquiera bajo la raíz de referencia (código adoptado, no conocimiento).
    write(dir.path(), "src/existente.rs", "fn main() {}\n");
    // Config: solo `knowledge` escribible; `src` de referencia.
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "workspace:\n  writableRoots: [knowledge]\n  referenceRoots: [src]\n",
    );
    dir
}

/// E13-H08 · Criterio `apply_fuera_de_writable` (benchmark §17: "Intentar escribir fuera de
/// writableRoots → rechazo"):
/// Dado un intento de escribir fuera de `writableRoots` en las ops, Cuando se aplica, Entonces
/// `PERMISSION_DENIED` y no escribe.
///
/// DÓNDE se rechaza: `change_plan` NO valida `writableRoots` — normaliza y simula en memoria
/// (`plan::normalize_create` es del core PURO, sin config; verificado en el árbol actual). Por eso el
/// plan del `create` bajo `src/` se PRODUCE (hay `changeSetId`), y el rechazo corresponde a
/// `change_apply`, que publica por el único escritor con `assert_writable` (E11-H04) ⇒
/// `PERMISSION_DENIED`. El test asevera el rechazo EN APPLY (la redacción literal del criterio) y, de
/// forma independiente del punto de rechazo, que NO se materializa nada bajo `src/`.
#[test]
fn apply_fuera_de_writable() {
    let dir = workspace_writable_restringido();

    // (1) Plan con un create bajo `src/` (fuera de `writableRoots`). change_plan no valida writable,
    // así que produce el plan (documentado arriba): exigimos un `changeSetId` para que el rojo lo
    // dispare la ausencia de `change_apply`, no un rechazo prematuro en el plan.
    let ops = serde_json::json!([
        { "op": "create", "path": "src/malicioso.md",
          "body": "# Malo\n\nintento de escribir fuera de writableRoots\n" },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );
    let id = plan_change_set_id(&plan[0]);

    // (2) Aplicar: escribir bajo `src/` (referenceRoot / fuera de writableRoots) ⇒ PERMISSION_DENIED.
    let resp = roundtrip(dir.path(), &[change_apply_line(&id, None).as_str()], 1);
    assert_eq!(
        resp[0]["result"]["isError"], true,
        "un apply que escribe fuera de writableRoots debe dar isError: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "un rechazo por permisos NO debe ser un error de protocolo JSON-RPC: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("PERMISSION_DENIED"),
        "el error debe exponer el código estable «PERMISSION_DENIED»: {resp:?}"
    );

    // No escribe: nada se materializa bajo la raíz de referencia `src/`.
    assert!(
        !dir.path().join("src/malicioso.md").exists(),
        "el apply rechazado NO debe crear ningún `.md` bajo `src/`: {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// E13-H09 — Tool `change_revert` (reversión de una transacción reciente y no alterada,
// `REFACTOR §11.3/§17`, `ARCHITECTURE.md §19.5/§19.6`).
//
// UBICACIÓN: los 3 criterios se ejercitan **e2e por la tool MCP** (campo Pruebas de la historia:
// `crates/lodestar-mcp/tests/`), coherente con E13-H08 (`change_apply`). Lo que se fija AQUÍ es el
// contrato de **wire** de `change_revert`: qué `arguments` toma, qué `structuredContent` devuelve al
// revertir, y cómo afloran `WRITE_CONFLICT` (fichero afectado alterado) y el código de «receipt no
// disponible» (caducado/purgado). La mecánica interna (restauración por el único escritor desde las
// copias de recuperación, transacción inversa con su propio journal) la testean las historias de
// `lodestar-workspace` / `lodestar-app`; aquí se fija la SUPERFICIE.
//
// FASE ROJA: la tool `change_revert` NO está en `tools::list()` todavía, así que
// `tools/call {name:"change_revert"}` devuelve el error de protocolo `-32602` (tool desconocida, vía
// `tools::exists`) y `result` es `null`. Por eso los asserts que leen `result.structuredContent.*` o
// `result.isError` fallan por AUSENCIA de la tool/servicio (`App::change_revert`), no por un valor
// erróneo. Ese es el rojo correcto. Los pasos previos `change_plan`/`change_apply` SÍ existen desde
// E12-H08/E13-H08, así que el flujo `change_plan → change_apply → change_revert` deja el rojo SIEMPRE
// en la ÚLTIMA llamada (la reversión), no antes.
//
// WIRE DE ENTRADA asumido (`REFACTOR §11.3`; el implementador puede refinar los tipos internos, no el
// wire):
//   arguments: {
//     receiptId: "<el receiptId que devolvió change_apply>",  // requerido; localiza el receipt (E13-H07)
//     expectedWorkspaceRevision?: "blake3:…"                  // control optimista a nivel de workspace;
//   }                                                         // si se omite, se adopta la revisión actual
//
// WIRE DE SALIDA asumido (`structuredContent`, salida «razonable» de la historia: la reversión es una
// transacción inversa por el único escritor):
//   {
//     reverted: true,                              // la reversión se publicó
//     previousWorkspaceRevision, workspaceRevision,   // «blake3:…» antes/después de la transacción INVERSA:
//       // `previousWorkspaceRevision` == la `resultRevision` que dejó el apply (el estado del que parte
//       //  la reversión); `workspaceRevision` == la `previousRevision` original del apply (el estado
//       //  restaurado). Es decir: revertir devuelve el workspace a `previousRevision` (criterio).
//     receiptId, changedPaths, …
//   }
//   Los errores de EJECUCIÓN afloran como `result.isError == true` con el código estable wire visible
//   (ErrorCode `as_str()`, E10-H02 / invariante #4 / `REFACTOR §13`), NUNCA como error JSON-RPC de
//   transporte — mismo patrón que `PLAN_STALE`/`REVISION_CONFLICT` en `change_apply`.
//
// FIRMA DE SERVICIO ASUMIDA (el implementador la crea con su propia elección de tipos internos):
//   App::change_revert(receipt_id: &ReceiptId, expected_workspace_revision: Option<WorkspaceRevision>)
//       -> Result<RevertResult, ErrorCode>
//   que carga el receipt persistido (E13-H07), verifica que existe/no caducó, que el workspace sigue en
//   la `resultRevision` y que los ficheros afectados no cambiaron (si no → `WRITE_CONFLICT`), y restaura
//   desde las copias de recuperación (E13-H04) por el ÚNICO ESCRITOR (invariante #5).
//
// CÓDIGO DE «RECEIPT NO DISPONIBLE» (caducado/purgado): el catálogo de `ErrorCode` (invariante #4,
// `core::types`) está CONGELADO en 16 variantes y NO tiene una específica de «receipt no encontrado».
// Se REUSA `PLAN_EXPIRED` —igual que `change_apply` reusa `PLAN_EXPIRED`/`PLAN_STALE` para el plan
// persistido ausente/vencido— por ser el match semántico más cercano a «la transacción registrada ya
// no está disponible por retención» y por alinear con el nombre del criterio (`revert_caducado`).
// ASUNCIÓN documentada y sujeta a ratificación por el implementador/juez: si se prefiere otro código
// del catálogo, es una decisión de contrato a cerrar antes de la fase verde (no la cierro aquí).
// ---------------------------------------------------------------------------

/// Construye la línea `tools/call change_revert` con el `receiptId` y un `expectedWorkspaceRevision`
/// opcional. Documenta el wire de entrada que fija esta historia.
fn change_revert_line(receipt_id: &str, expected_ws_rev: Option<&str>) -> String {
    let mut args = serde_json::json!({ "receiptId": receipt_id });
    if let Some(r) = expected_ws_rev {
        args["expectedWorkspaceRevision"] = serde_json::Value::String(r.to_string());
    }
    serde_json::json!({
        "jsonrpc": "2.0", "id": 3, "method": "tools/call",
        "params": { "name": "change_revert", "arguments": args }
    })
    .to_string()
}

/// `structuredContent` de una respuesta `change_revert`, tras verificar que es un objeto. En fase
/// ROJA (tool ausente) `result` es `null` → panica con un mensaje que documenta el porqué del rojo
/// (la tool/servicio `change_revert` ausente), no un fallo espurio.
fn revert_sc(resp: &serde_json::Value) -> &serde_json::Value {
    let sc = &resp["result"]["structuredContent"];
    assert!(
        sc.is_object(),
        "change_revert debe devolver structuredContent (objeto); tool/servicio ausente en fase ROJA: {resp:?}"
    );
    sc
}

/// El `receiptId` (string; `ReceiptId` es un newtype `#[serde(transparent)]`) de una respuesta
/// `change_apply`. Panica —documentando el rojo— si el apply no lo produjo (tool/servicio
/// `change_apply` ausente ⇒ `structuredContent` nulo). Como `change_apply` YA existe (E13-H08), en
/// la práctica esto siempre devuelve un id: el rojo lo dispara la ausencia de `change_revert`.
fn apply_receipt_id(resp: &serde_json::Value) -> String {
    apply_sc(resp)["receiptId"]
        .as_str()
        .unwrap_or_else(|| panic!("change_apply debe devolver un `receiptId` (string): {resp:?}"))
        .to_string()
}

/// Purga los recibos persistidos borrando `.lodestar/runtime/receipts/` entero (como haría un GC de
/// retención al caducar, E13-H07): tras esto, ningún `receiptId` es localizable ⇒ «no disponible».
/// Se borra el directorio completo (no un fichero concreto) para no acoplar el test al saneado del
/// nombre del receipt (`receipt_file_name`). Las copias de recuperación se dejan intactas: el
/// discriminante del criterio `revert_caducado` es que el RECEIPT ya no está.
fn purge_receipts(root: &std::path::Path) {
    let dir = root.join(".lodestar").join("runtime").join("receipts");
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

/// E13-H09 · Criterio `revert_reciente` (benchmark §17: "Recuperar un cambio reciente → change_revert"):
/// Dado un receipt reciente y el workspace intacto, Cuando se revierte, Entonces el workspace vuelve a
/// `previousRevision`. Flujo: `change_plan` (create) → `change_apply` (captura `receiptId` y la
/// `previousWorkspaceRevision` original) → `change_revert(receiptId)`. Se comprueba (a) `reverted==true`
/// y no `isError`; (b) que la `workspaceRevision` resultante de la reversión == la
/// `previousWorkspaceRevision` que tenía el apply (se volvió al estado previo); (c) empírico: el `.md`
/// creado por el apply YA NO existe en disco (la reversión de un `create` lo borra, invariante #1); y
/// (d) que un `workspace_status` posterior reporta EXACTAMENTE esa revisión restaurada.
#[test]
fn revert_reciente() {
    let dir = workspace_min();

    // (1) Plan válido: crear un documento conforme (mismo patrón que `apply_ok`).
    let ops = serde_json::json!([
        { "op": "create", "path": "nuevo.md",
          "body": "# Resumen\n\ncuerpo del documento nuevo\n" },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );
    let id = plan_change_set_id(&plan[0]);

    // (2) Aplicar (servidor fresco): captura el `receiptId` y la revisión ORIGINAL previa al apply.
    let applied = roundtrip(dir.path(), &[change_apply_line(&id, None).as_str()], 1);
    let receipt_id = apply_receipt_id(&applied[0]);
    let revision_original = apply_sc(&applied[0])["previousWorkspaceRevision"]
        .as_str()
        .unwrap_or_else(|| {
            panic!("change_apply debe devolver `previousWorkspaceRevision`: {applied:?}")
        })
        .to_string();
    // El apply materializó el `.md` (precondición: si no, el criterio sería vacuo).
    assert!(
        dir.path().join("nuevo.md").is_file(),
        "precondición: el apply debe haber creado `nuevo.md` antes de revertir: {applied:?}"
    );

    // (3) Revertir por el `receiptId` (servidor fresco, mismo workspace) + `workspace_status`.
    let status_line = r#"{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"workspace_status","arguments":{}}}"#;
    let resp = roundtrip(
        dir.path(),
        &[change_revert_line(&receipt_id, None).as_str(), status_line],
        2,
    );
    let sc = revert_sc(&resp[0]);

    // (a) La reversión se publicó.
    assert_eq!(
        sc["reverted"],
        serde_json::Value::Bool(true),
        "un receipt reciente y el workspace intacto deben revertirse (reverted:true): {resp:?}"
    );
    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "una reversión exitosa no debe dar isError: {resp:?}"
    );

    // (b) El workspace VUELVE a la revisión previa al apply (criterio literal).
    let ws_rev = sc["workspaceRevision"].as_str().unwrap_or("");
    assert!(
        ws_rev.starts_with("blake3:"),
        "workspaceRevision restaurada ausente o mal formada («blake3:…»): {resp:?}"
    );
    assert_eq!(
        ws_rev, revision_original,
        "revertir debe devolver el workspace a la previousRevision del apply: {resp:?}"
    );

    // (c) Empírico: la reversión del `create` borró el `.md` del disco (invariante #1).
    assert!(
        !dir.path().join("nuevo.md").exists(),
        "revertir un `create` debe borrar el `.md` del canónico: {resp:?}"
    );

    // (d) El workspace «queda en» la revisión restaurada: `workspace_status` reporta la misma.
    let status_rev = resp[1]["result"]["structuredContent"]["workspaceRevision"]
        .as_str()
        .unwrap_or("");
    assert_eq!(
        status_rev, ws_rev,
        "tras revertir, workspace_status debe reportar la workspaceRevision restaurada: {resp:?}"
    );
}

/// E13-H09 · Criterio `revert_fichero_alterado`:
/// Dado que un fichero afectado cambió tras el apply, Cuando se revierte, Entonces `WRITE_CONFLICT` y
/// no revierte. Flujo: `change_plan`(create) → `change_apply` → se REESCRIBE en disco el `.md`
/// afectado (`nuevo.md`) → `change_revert`. Se NO pasa `expectedWorkspaceRevision` para que el
/// discriminante sea la comprobación de fichero-afectado-alterado (`WRITE_CONFLICT`), no un
/// `REVISION_CONFLICT` del control optimista de workspace.
#[test]
fn revert_fichero_alterado() {
    let dir = workspace_min();

    // (1) Plan + apply de un `create` (el único fichero afectado es `nuevo.md`).
    let ops = serde_json::json!([
        { "op": "create", "path": "nuevo.md",
          "body": "# Resumen\n\ncuerpo del documento nuevo\n" },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );
    let id = plan_change_set_id(&plan[0]);
    let applied = roundtrip(dir.path(), &[change_apply_line(&id, None).as_str()], 1);
    let receipt_id = apply_receipt_id(&applied[0]);

    // (2) Un fichero AFECTADO cambia tras el apply: `nuevo.md` se reescribe EN DISCO con otro
    //     contenido (marcador reconocible). Ahora el workspace ya NO está en la `resultRevision` que
    //     dejó el apply y el afectado no casa con el receipt.
    write(
        dir.path(),
        "nuevo.md",
        "---\ntype: Nota\ntitle: Nuevo\n---\n\n# Resumen\n\nALTERADO-A-MANO-TRAS-EL-APPLY\n",
    );

    // (3) Revertir (servidor fresco): fichero afectado alterado ⇒ WRITE_CONFLICT.
    let resp = roundtrip(
        dir.path(),
        &[change_revert_line(&receipt_id, None).as_str()],
        1,
    );
    assert_eq!(
        resp[0]["result"]["isError"], true,
        "revertir con un fichero afectado alterado debe dar isError: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "un conflicto de escritura NO debe ser un error de protocolo JSON-RPC: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("WRITE_CONFLICT"),
        "el error debe exponer el código estable «WRITE_CONFLICT»: {resp:?}"
    );

    // No revierte: `nuevo.md` conserva el contenido alterado a mano (si hubiera revertido el
    // `create`, el fichero estaría BORRADO). El estado permanece intacto.
    let en_disco = std::fs::read_to_string(dir.path().join("nuevo.md")).unwrap();
    assert!(
        en_disco.contains("ALTERADO-A-MANO-TRAS-EL-APPLY"),
        "una reversión rechazada por WRITE_CONFLICT no debe tocar el fichero afectado: {en_disco:?}"
    );
}

/// E13-H09 · Criterio `revert_caducado`:
/// Dado un receipt caducado/purgado, Cuando se revierte, Entonces error (no disponible). Flujo:
/// `change_plan`(create) → `change_apply` (captura `receiptId`) → se PURGA el receipt persistido
/// (borra `.lodestar/runtime/receipts/`, como un GC de retención, E13-H07) → `change_revert`.
///
/// CÓDIGO ASUMIDO: `PLAN_EXPIRED` (reuso documentado del catálogo congelado de 16 `ErrorCode`, cf. la
/// nota de sección). Además de exigir el código, se comprueba que (a) es un error de EJECUCIÓN
/// (isError, no JSON-RPC); (b) NO es `WRITE_CONFLICT` (así el receipt-no-disponible se distingue del
/// fichero-alterado y el test no es vacuo); y (c) no revierte: el `.md` del apply permanece en disco.
#[test]
fn revert_caducado() {
    let dir = workspace_min();

    // (1) Plan + apply de un `create`.
    let ops = serde_json::json!([
        { "op": "create", "path": "nuevo.md",
          "body": "# Resumen\n\ncuerpo del documento nuevo\n" },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );
    let id = plan_change_set_id(&plan[0]);
    let applied = roundtrip(dir.path(), &[change_apply_line(&id, None).as_str()], 1);
    let receipt_id = apply_receipt_id(&applied[0]);
    assert!(
        dir.path().join("nuevo.md").is_file(),
        "precondición: el apply debe haber creado `nuevo.md`: {applied:?}"
    );

    // (2) Caducar/purgar el receipt: se borra el directorio de recibos (como un GC de retención).
    purge_receipts(dir.path());

    // (3) Revertir (servidor fresco): el receipt ya no está ⇒ error «no disponible».
    let resp = roundtrip(
        dir.path(),
        &[change_revert_line(&receipt_id, None).as_str()],
        1,
    );
    assert_eq!(
        resp[0]["result"]["isError"], true,
        "revertir un receipt caducado/purgado debe dar isError: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "un receipt no disponible NO debe ser un error de protocolo JSON-RPC: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("PLAN_EXPIRED"),
        "el error debe exponer el código estable de «no disponible» (asumido «PLAN_EXPIRED»): {resp:?}"
    );
    // No es un WRITE_CONFLICT: el receipt-no-disponible se distingue del fichero-alterado.
    assert!(
        !texto.contains("WRITE_CONFLICT"),
        "un receipt purgado no es un WRITE_CONFLICT (debe ser «no disponible»): {resp:?}"
    );

    // No revierte: el `.md` creado por el apply sigue en disco (nada se restauró).
    assert!(
        dir.path().join("nuevo.md").is_file(),
        "una reversión de receipt no disponible no debe tocar el canónico: {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// E14-H03 — Instrucciones del servidor + perfiles para agentes
// (`requirements/epica-14-integracion-evaluacion.md` E14-H03; `ARCHITECTURE.md §19.6`;
// `REFACTOR §7, §12`). Fase ROJA: hoy el servidor arranca con `--profile` y refleja el perfil en
// `workspace_status.capabilities` (E10-H08), pero (a) `tools/list` NO se filtra por perfil, (b)
// `initialize` NO devuelve `instructions`, y (c) no hay gating al INVOCAR una tool de cambio bajo
// `readonly`. Los tres tests de abajo fijan ese comportamiento pendiente.
//
// Las **3 tools de cambio** (las que el perfil `readonly` debe ocultar) son, según `contracts/mcp.yml`
// (`perfil: standard` en las tres) y la superficie objetivo de 10 tools: `change_plan`,
// `change_apply`, `change_revert`. `change_plan` SÍ cuenta como tool de cambio (planifica un cambio,
// aunque no escriba; el contrato la marca `perfil: standard`).
// ---------------------------------------------------------------------------

/// Las 3 tools de cambio que `readonly` debe ocultar de `tools/list` (todas `perfil: standard` en
/// `contracts/mcp.yml`; `change_plan` incluido — es una tool de cambio aunque no escriba).
const TOOLS_DE_CAMBIO: [&str; 3] = ["change_plan", "change_apply", "change_revert"];

/// Tools de lectura/consulta que deben seguir presentes en CUALQUIER perfil (muestra representativa
/// de la superficie objetivo de lectura, `REFACTOR §8`).
const TOOLS_DE_LECTURA: [&str; 7] = [
    "workspace_status",
    "knowledge_search",
    "knowledge_get",
    "metadata_inspect",
    "knowledge_check",
    "graph_query",
    "impact_analyze",
];

/// Nombres de tool presentes en la respuesta `tools/list` de `resp`.
fn nombres_de_tools(resp: &serde_json::Value) -> std::collections::BTreeSet<String> {
    resp["result"]["tools"]
        .as_array()
        .expect("tools/list devuelve un array de tools")
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_string))
        .collect()
}

/// E14-H03 · Criterio `perfil_readonly_sin_cambio`:
/// Dado el servidor con `--profile readonly`, Cuando un cliente pide `tools/list`, Entonces NO
/// aparecen las 3 tools de cambio (y SÍ las de lectura). Con `--profile standard` SÍ aparecen las 3
/// (control para no ser vacuo: si el perfil se ignorase, standard también las ocultaría/mostraría).
#[test]
fn perfil_readonly_sin_cambio() {
    let dir = workspace_min();
    let list = r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#;

    // --- readonly: sin tools de cambio, con tools de lectura ---
    let ro = roundtrip_profile(dir.path(), "readonly", &[list], 1);
    let ro_tools = nombres_de_tools(&ro[0]);
    for cambio in TOOLS_DE_CAMBIO {
        assert!(
            !ro_tools.contains(cambio),
            "perfil readonly NO debe exponer la tool de cambio «{cambio}» en tools/list: {ro_tools:?}"
        );
    }
    for lectura in TOOLS_DE_LECTURA {
        assert!(
            ro_tools.contains(lectura),
            "perfil readonly DEBE seguir exponiendo la tool de lectura «{lectura}»: {ro_tools:?}"
        );
    }

    // --- standard: con las 3 tools de cambio (y las de lectura) ---
    let std = roundtrip_profile(dir.path(), "standard", &[list], 1);
    let std_tools = nombres_de_tools(&std[0]);
    for cambio in TOOLS_DE_CAMBIO {
        assert!(
            std_tools.contains(cambio),
            "perfil standard DEBE exponer la tool de cambio «{cambio}» en tools/list: {std_tools:?}"
        );
    }
    for lectura in TOOLS_DE_LECTURA {
        assert!(
            std_tools.contains(lectura),
            "perfil standard DEBE exponer la tool de lectura «{lectura}»: {std_tools:?}"
        );
    }
}

/// E14-H03 · Criterio `instrucciones_flujo`:
/// Dado el arranque, Cuando el cliente lee las instrucciones del servidor (campo `instructions` de
/// la respuesta `initialize`), Entonces describen el flujo de 10 pasos
/// `workspace_status → knowledge_search → knowledge_get → metadata_inspect →
/// graph_query/impact_analyze → change_plan → change_apply → knowledge_check → change_revert`,
/// mencionando las 10 tools EN ORDEN (no solo un string no vacío).
#[test]
fn instrucciones_flujo() {
    let dir = workspace_min();
    let resp = roundtrip(
        dir.path(),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize"}"#],
        1,
    );
    let instructions = resp[0]["result"]["instructions"]
        .as_str()
        .unwrap_or_else(|| {
            panic!(
                "initialize debe devolver `instructions` (string) con el flujo recomendado: {resp:?}"
            )
        });
    assert!(
        !instructions.trim().is_empty(),
        "las instrucciones del servidor no deben estar vacías: {resp:?}"
    );

    // Índice de la primera aparición de cada tool en el texto (None si no aparece).
    let pos = |tool: &str| -> usize {
        instructions.find(tool).unwrap_or_else(|| {
            panic!("las instrucciones deben mencionar la tool «{tool}» del flujo: {instructions:?}")
        })
    };

    // La "columna vertebral" del flujo debe aparecer en orden estrictamente creciente.
    let espina = [
        "workspace_status",
        "knowledge_search",
        "knowledge_get",
        "metadata_inspect",
        "change_plan",
        "change_apply",
        "change_revert",
    ];
    let mut previo: Option<(&str, usize)> = None;
    for tool in espina {
        let aqui = pos(tool);
        if let Some((antes, idx)) = previo {
            assert!(
                idx < aqui,
                "el flujo debe listar «{antes}» antes de «{tool}» (10 pasos en orden): {instructions:?}"
            );
        }
        previo = Some((tool, aqui));
    }

    // `graph_query`/`impact_analyze` son el paso de análisis: entre metadata_inspect y change_plan.
    let (ini, fin) = (pos("metadata_inspect"), pos("change_plan"));
    for tool in ["graph_query", "impact_analyze"] {
        let idx = pos(tool);
        assert!(
            ini < idx && idx < fin,
            "«{tool}» debe situarse tras metadata_inspect y antes de change_plan en el flujo: {instructions:?}"
        );
    }

    // `knowledge_check` es la verificación tras aplicar: entre change_apply y change_revert.
    let (ini, fin) = (pos("change_apply"), pos("change_revert"));
    let idx = pos("knowledge_check");
    assert!(
        ini < idx && idx < fin,
        "«knowledge_check» debe situarse tras change_apply y antes de change_revert en el flujo: {instructions:?}"
    );
}

/// E14-H03 (endurecimiento; cierra la reserva de «gating por perfil» de E13-H08):
/// Dado el servidor con `--profile readonly`, Cuando un cliente INVOCA directamente una tool de
/// cambio (las 3: `change_plan`, `change_apply`, `change_revert`), Entonces la invocación se RECHAZA
/// sin ejecutarse: ocultarla de `tools/list` no basta si el cliente la llama igualmente.
///
/// Cubre las tres tools de cambio, no solo la que planifica: `change_apply`/`change_revert` son las
/// que SÍ escriben, así que la aserción de seguridad de más valor es que un cliente que ignore
/// `tools/list` NO pueda **aplicar** ni **revertir** bajo `readonly`.
///
/// No vacuidad — cada rama se contrasta con `standard` para atribuir el rechazo AL PERFIL, no a una
/// petición malformada:
/// - `change_plan` (con ops válidas): bajo `standard` produce un plan (`changeSetId`); bajo
///   `readonly` no debe devolver ninguno.
/// - `change_apply`/`change_revert` (con `changeSetId`/`receiptId` INEXISTENTES): el gating debe
///   cortar ANTES de tocar el argumento → `-32602` (tool no disponible). Bajo `standard` la MISMA
///   llamada SÍ llega a ejecutarse y falla por el id inexistente como error de aplicación
///   (`isError`, sin `-32602`). Ese contraste prueba que el `-32602` de `readonly` es gating de
///   perfil, no validación de argumento.
#[test]
fn perfil_readonly_rechaza_cambio() {
    let dir = workspace_cinco_relacionados();

    // --- change_plan: bajo standard produce un plan válido; bajo readonly no debe producirlo ---
    let plan_line = change_plan_line(None, cinco_operaciones(), policy_permisiva());

    // Control: bajo `standard` la MISMA llamada produce un plan válido (changeSetId presente).
    let std = roundtrip_profile(dir.path(), "standard", &[plan_line.as_str()], 1);
    let std_id = std[0]["result"]["structuredContent"]["changeSetId"].as_str();
    assert!(
        std_id.is_some_and(|s| !s.is_empty()),
        "control: bajo standard, change_plan debe devolver un changeSetId (la petición es válida): {std:?}"
    );

    // Bajo `readonly`, la misma invocación debe rechazarse: ni changeSetId ni ejecución silenciosa.
    let ro = roundtrip_profile(dir.path(), "readonly", &[plan_line.as_str()], 1);
    assert!(
        ro[0]["result"]["structuredContent"]["changeSetId"].is_null(),
        "perfil readonly NO debe ejecutar change_plan (no debe devolver un changeSetId): {ro:?}"
    );
    let rechazado =
        ro[0]["error"].get("code").is_some() || ro[0]["result"]["isError"].as_bool() == Some(true);
    assert!(
        rechazado,
        "perfil readonly debe RECHAZAR change_plan con un error claro (protocolo -32602 o result.isError), no ignorarlo: {ro:?}"
    );

    // --- change_apply / change_revert: las tools que SÍ escriben. Con ids INEXISTENTES ---
    // El gating de perfil debe cortar ANTES de intentar ejecutar (tool no disponible = -32602), sin
    // llegar siquiera a validar el argumento. Ids deliberadamente inexistentes: si el gating NO
    // cortara, la ejecución fallaría con un error de aplicación (isError), NO con -32602 — por eso
    // aseverar el `-32602` distingue «rechazado por perfil» de «falló por otra razón».
    let escrituras = [
        (
            "change_apply",
            change_apply_line("changeset:inexistente0000", None),
        ),
        (
            "change_revert",
            change_revert_line("receipt:inexistente0000", None),
        ),
    ];
    for (tool, line) in escrituras {
        // readonly: gating de perfil → -32602 (tool no disponible), sin ejecutar (sin result).
        let ro = roundtrip_profile(dir.path(), "readonly", &[line.as_str()], 1);
        assert_eq!(
            ro[0]["error"]["code"], -32602,
            "perfil readonly debe rechazar «{tool}» con -32602 (tool no disponible), no ejecutarla: {ro:?}"
        );
        assert!(
            ro[0]["result"].is_null(),
            "perfil readonly NO debe ejecutar «{tool}» (sin result, corta antes del despacho): {ro:?}"
        );

        // Control de no-vacuidad: bajo `standard` la MISMA llamada SÍ llega a ejecutarse y falla por
        // el id inexistente como error de aplicación (isError), NUNCA como -32602 de gating. Así el
        // -32602 de readonly se atribuye al perfil, no a un argumento inválido.
        let st = roundtrip_profile(dir.path(), "standard", &[line.as_str()], 1);
        assert_ne!(
            st[0]["error"]["code"], -32602,
            "control: bajo standard «{tool}» debe llegar a ejecutarse (no el -32602 de gating): {st:?}"
        );
        assert_eq!(
            st[0]["result"]["isError"], true,
            "control: bajo standard «{tool}» con id inexistente debe fallar como error de aplicación (isError): {st:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// E14-H06 — Retirada de la superficie heredada (10 tools heredadas → 10 objetivo).
//
// Cierra el giro headless: la superficie MCP converge a EXACTAMENTE las 10 tools objetivo
// (`ARCHITECTURE.md §19.6`, `contracts/mcp.yml §15`). Las 10 heredadas (`query`,
// `conformance_check`, `find_backlinks`, `find_orphans`, `find_dangling`, `neighborhood`,
// `create_concept`, `update_frontmatter`, `generate_index`, `generate_tag_indexes`) desaparecen de
// `tools/list` y del despacho.
//
// FASE ROJA: hoy las 10 heredadas siguen en `tools::list()` y en el `match` de `tools::call()`, así
// que `tools_list_solo_objetivo` falla (la lista NO es solo las 10 objetivo: hay 20) y
// `tool_heredada_retirada` falla (invocar `query`/`conformance_check`/… SÍ ejecuta en vez de dar
// `-32602`). La retirada real en `src/tools.rs` es del implementador.
// ---------------------------------------------------------------------------

/// Las 10 tools objetivo del giro headless (superficie de largo plazo, perfil `standard`).
const TOOLS_OBJETIVO: [&str; 10] = [
    "workspace_status",
    "knowledge_search",
    "knowledge_get",
    "metadata_inspect",
    "graph_query",
    "impact_analyze",
    "knowledge_check",
    "change_plan",
    "change_apply",
    "change_revert",
];

/// Las 10 tools heredadas que E14-H06 retira (su reemplazo semántico vive en las objetivo,
/// `contracts/mcp.yml §15`).
const TOOLS_HEREDADAS: [&str; 10] = [
    "query",
    "conformance_check",
    "find_backlinks",
    "find_orphans",
    "find_dangling",
    "neighborhood",
    "create_concept",
    "update_frontmatter",
    "generate_index",
    "generate_tag_indexes",
];

/// E14-H06 · Criterio `tools_list_solo_objetivo`:
/// Dado el servidor MCP (perfil standard), Cuando un cliente pide `tools/list`, Entonces devuelve
/// EXACTAMENTE las 10 tools objetivo y NINGUNA heredada. Se asevera el CONJUNTO exacto (las 10
/// presentes Y las 10 heredadas ausentes), no solo el conteo: un conteo por sí solo no distinguiría
/// «10 objetivo» de «5 objetivo + 5 heredadas».
#[test]
fn tools_list_solo_objetivo() {
    let dir = workspace_min();
    let resp = roundtrip_profile(
        dir.path(),
        "standard",
        &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#],
        1,
    );
    let presentes = nombres_de_tools(&resp[0]);

    // Conjunto EXACTO: la superficie es exactamente las 10 objetivo.
    let objetivo: std::collections::BTreeSet<String> =
        TOOLS_OBJETIVO.iter().map(|s| s.to_string()).collect();
    assert_eq!(
        presentes, objetivo,
        "tools/list (standard) debe devolver EXACTAMENTE las 10 tools objetivo: {presentes:?}"
    );

    // Redundante pero explícito (redacción literal del criterio): las 10 objetivo presentes…
    for objetivo in TOOLS_OBJETIVO {
        assert!(
            presentes.contains(objetivo),
            "falta la tool objetivo «{objetivo}» en tools/list: {presentes:?}"
        );
    }
    // …y NINGUNA de las 10 heredadas.
    for heredada in TOOLS_HEREDADAS {
        assert!(
            !presentes.contains(heredada),
            "la tool heredada «{heredada}» NO debe aparecer en tools/list tras E14-H06: {presentes:?}"
        );
    }
    // Y el conteo exacto, por si acaso (ni más ni menos que 10).
    assert_eq!(
        presentes.len(),
        10,
        "la superficie objetivo es de EXACTAMENTE 10 tools: {presentes:?}"
    );
}

/// E14-H06 · Criterio `tool_heredada_retirada`:
/// Dado el servidor, Cuando un cliente invoca una tool heredada (se cubren las 10, incluidas
/// `query`/`conformance_check`/`find_backlinks`/`create_concept`/`generate_index`), Entonces se
/// rechaza como tool desconocida SIN ejecutarla (sin `result`).
///
/// CÓDIGO DE ERROR — `-32602` (ratificado en la spec): una tool inexistente en `tools/call` se mapea
/// a `-32602` («Invalid params»: `tools/call` SÍ es un método válido, lo desconocido es el *nombre de
/// tool* = un parámetro); `-32601` queda reservado para un *método* de alto nivel desconocido (p. ej.
/// `foo/bar`). Convención coherente con los tests `call_commit_desconocida` (E9, retirada de la tool
/// git `commit` → `-32602`) y `protocolo_errores_y_ping` (tool `no_existe` → `-32602`). Una tool
/// heredada RETIRADA es, tras la retirada, exactamente el mismo caso que una tool inexistente.
#[test]
fn tool_heredada_retirada() {
    let dir = workspace_min();
    // Un argumento plausible por tool heredada, para descartar que el rechazo venga de un argumento
    // ausente en vez de la retirada de la tool.
    let args = |name: &str| -> &'static str {
        match name {
            "query" => r#"{"dsl":"is:orphan"}"#,
            "conformance_check" => r#"{}"#,
            "find_backlinks" => r#"{"concept":"alfa.md"}"#,
            "find_orphans" => r#"{}"#,
            "find_dangling" => r#"{}"#,
            "neighborhood" => r#"{"concept":"alfa.md"}"#,
            "create_concept" => r#"{"path":"nueva.md","type":"Nota"}"#,
            "update_frontmatter" => r#"{"path":"alfa.md","patch":{}}"#,
            "generate_index" => r#"{"dir":""}"#,
            "generate_tag_indexes" => r#"{}"#,
            _ => r#"{}"#,
        }
    };

    for heredada in TOOLS_HEREDADAS {
        let line = format!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"{heredada}","arguments":{args}}}}}"#,
            args = args(heredada)
        );
        let resp = roundtrip(dir.path(), &[line.as_str()], 1);

        // Tool desconocida (tras la retirada): -32602, coherente con `call_commit_desconocida` (E9).
        // Sin ejecutar la tool (sin result). Ver la nota sobre el código -32602 arriba.
        assert_eq!(
            resp[0]["error"]["code"], -32602,
            "la tool heredada «{heredada}» debe rechazarse como desconocida (-32602): {resp:?}"
        );
        assert!(
            resp[0]["result"].is_null(),
            "la tool heredada «{heredada}» NO debe producir result (no se ejecuta): {resp:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// E15-H06 — La raíz del workspace es el `cwd`
// (`requirements/epica-15-workspace-universal.md`, `ARCHITECTURE.md §20.1`/`§20.5`).
//
// SUPERFICIE FIJADA (fase ROJA — todavía NO implementada):
//   `lodestar-mcp [--root <dir>] [--profile readonly|standard]`
//   · sin `--root` ⇒ la raíz es `std::env::current_dir()`;
//   · `--root <dir>` la fija explícitamente y gana sobre el cwd;
//   · **no hay argumento posicional**: v0.3 es incompatible con v0.2 y la historia declara que no
//     hace falta conservarlo como alias deprecado. Por eso NINGUNO de los tests de este bloque
//     usa `roundtrip`/`roundtrip_profile` (que pasan la raíz como posicional): usan
//     [`roundtrip_en`], que ejercita la superficie nueva y que sigue siendo válida cuando el
//     posicional desaparezca.
//   · desaparece el gate «esto no es un workspace lodestar» (exit 3): cualquier directorio vale.
// ---------------------------------------------------------------------------

/// Como [`roundtrip`], pero arranca el servidor con el **cwd** en `cwd` y exactamente los
/// argumentos `args` (ninguno posicional). Es el arnés de la superficie de arranque de E15-H06:
/// `&[]` ejercita «la raíz es el cwd» y `&["--root", dir]` ejercita la raíz explícita.
///
/// Si el servidor aborta al arrancar (hoy: exit 3 por el gate de workspace), el vector devuelto sale
/// **vacío** — de ahí que cada test asserte primero cuántas respuestas llegaron, para que el rojo
/// se lea como «el servidor no arrancó» y no como un índice fuera de rango.
fn roundtrip_en(
    cwd: &std::path::Path,
    args: &[&str],
    lines: &[&str],
    expect: usize,
) -> Vec<serde_json::Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    for l in lines {
        // El servidor puede haber muerto ya (gate de arranque): un EPIPE al escribir no debe
        // reventar el arnés, debe traducirse en «no llegaron respuestas».
        if writeln!(stdin, "{l}").is_err() {
            break;
        }
    }
    let _ = stdin.flush();
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

/// E15-H06 · Criterio `arranca_en_directorio_arbitrario`:
/// **Dado** un directorio con `notas.md` y **sin** `index.md` ni `.lodestar/`, **Cuando** se lanza
/// `lodestar-mcp` con el cwd ahí (y sin argumentos), **Entonces** arranca y responde `tools/list`.
///
/// Es el criterio de aceptación central de la épica (`§20.1`: «`cd my-project && lodestar-mcp`
/// funciona»). El documento no tiene frontmatter a propósito: el arranque no puede depender del
/// modelo documental.
///
/// Fase ROJA: hoy `main.rs` comprueba `index.md`/`.lodestar/` y aborta con exit 3 antes de leer
/// stdin, así que no llega ninguna respuesta.
#[test]
fn arranca_en_directorio_arbitrario() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notas.md", "Notas sueltas, sin frontmatter.\n");
    // Precondiciones del escenario: nada de lodestar en el directorio (si no, sería vacuo).
    assert!(
        !dir.path().join("index.md").exists(),
        "el escenario exige un directorio SIN index.md"
    );
    assert!(
        !dir.path().join(".lodestar").exists(),
        "el escenario exige un directorio SIN .lodestar/"
    );

    let resp = roundtrip_en(
        dir.path(),
        &[],
        &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#],
        1,
    );

    assert_eq!(
        resp.len(),
        1,
        "el servidor debe arrancar en un directorio arbitrario y responder tools/list; \
         no llegó respuesta (¿abortó al arrancar?): {resp:?}"
    );
    let tools = resp[0]["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools/list debe devolver un array de tools: {resp:?}"));
    assert!(
        !tools.is_empty(),
        "tools/list debe listar el catálogo también sobre un directorio arbitrario: {resp:?}"
    );
}

/// E15-H06 · Criterio `root_explicito_gana`:
/// **Dado** `lodestar-mcp --root /otro/dir`, **Cuando** arranca, **Entonces** opera sobre ese
/// directorio aunque el cwd del proceso sea otro.
///
/// Se comprueba por dos vías independientes, porque «operar sobre ese directorio» son dos cosas:
///  1. la raíz que reporta `workspace_status` es la pedida — canonicalizada (`§20.5`: «canonicalizar
///     la raíz una sola vez al arrancar»), por eso se compara `canonicalize` contra `canonicalize`
///     y no cadena contra cadena (en macOS `/var/...` ⇒ `/private/var/...`);
///  2. el inventario es el de `--root` (`alfa.md`) y **no** el del cwd (`beta.md`) — sin esta
///     segunda parte, un servidor que solo guardara la ruta pero leyera el cwd pasaría el test.
///
/// Fase ROJA: hoy la raíz es un argumento POSICIONAL, así que `--root` se toma como si fuera la
/// ruta del workspace (`root = "--root"`), el gate de workspace falla y el proceso sale con 3.
#[test]
fn root_explicito_gana() {
    let raiz = tempfile::tempdir().unwrap();
    let cwd = tempfile::tempdir().unwrap();
    write(
        raiz.path(),
        "alfa.md",
        "---\ntype: Nota\ntitle: Alfa\ndescription: d\n---\n\n# Alfa\n\ncuerpo\n",
    );
    write(
        cwd.path(),
        "beta.md",
        "---\ntype: Nota\ntitle: Beta\ndescription: d\n---\n\n# Beta\n\ncuerpo\n",
    );
    let root_arg = raiz.path().to_str().expect("tempdir con ruta UTF-8");

    let resp = roundtrip_en(
        cwd.path(),
        &["--root", root_arg],
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"workspace_status","arguments":{}}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"knowledge_search","arguments":{"text":""}}}"#,
        ],
        2,
    );

    assert_eq!(
        resp.len(),
        2,
        "`lodestar-mcp --root <dir>` debe arrancar sobre <dir>; no llegaron las 2 respuestas \
         (¿`--root` no se reconoce?): {resp:?}"
    );

    // 1. La raíz reportada es la pedida (no la del cwd).
    let reportada = resp[0]["result"]["structuredContent"]["root"]
        .as_str()
        .unwrap_or_else(|| panic!("workspace_status debe reportar `root`: {resp:?}"));
    let reportada = std::fs::canonicalize(reportada)
        .unwrap_or_else(|e| panic!("`root` reportada «{reportada}» no es un directorio: {e}"));
    assert_eq!(
        reportada,
        std::fs::canonicalize(raiz.path()).unwrap(),
        "la raíz debe ser la de `--root`, no la del cwd ({}): {resp:?}",
        cwd.path().display()
    );

    // 2. Y el inventario es el de esa raíz, no el del cwd.
    let paths = search_paths(&resp[1]);
    assert!(
        paths.iter().any(|p| p == "alfa.md"),
        "el inventario debe ser el de `--root` (falta «alfa.md»): {paths:?}"
    );
    assert!(
        !paths.iter().any(|p| p == "beta.md"),
        "el inventario NO debe incluir documentos del cwd («beta.md»): {paths:?}"
    );
}

/// Workspace mínimo **dentro de un directorio padre controlado**, para poder comprobar que un path
/// rechazado no escribe nada FUERA de la raíz. Devuelve `(padre, raíz)`; el padre se conserva vivo
/// porque su `Drop` borra el árbol.
///
/// Lleva `index.md` a propósito: estos dos tests son **guardas** que deben estar en verde ANTES de
/// E15-H06 (no dependen de que se retire el gate de arranque). Se lanzan con el cwd en la raíz y sin
/// argumentos, que es la forma de arranque válida tanto hoy como después de la historia.
fn ws_con_padre() -> (tempfile::TempDir, std::path::PathBuf) {
    let padre = tempfile::tempdir().unwrap();
    let ws = padre.path().join("ws");
    std::fs::create_dir_all(&ws).unwrap();
    write(
        &ws,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n",
    );
    write(
        &ws,
        "a.md",
        "---\ntype: Nota\ntitle: A\ndescription: d\n---\n\n# H\n\ncuerpo\n",
    );
    // Cebo FUERA de la raíz: un `.md` legible y bien formado, al alcance tanto de una ruta
    // absoluta como de un `..`. Sin él la guarda no mordería: un `knowledge_get` que dejara de
    // validar la ruta respondería igualmente «no encontrado» (isError) y el test pasaría en falso.
    // Con él, un escape que prosperase devolvería el cuerpo y [`SECRETO`] aparecería en la
    // respuesta.
    write(
        padre.path(),
        "secreto.md",
        "---\ntype: Nota\ntitle: Secreto\ndescription: d\n---\n\n# S\n\nCONTENIDO-SECRETO-FUERA\n",
    );
    (padre, ws)
}

/// Marca del cebo de [`ws_con_padre`]: nunca debe aparecer en una respuesta del servidor.
const SECRETO: &str = "CONTENIDO-SECRETO-FUERA";

/// Todos los ficheros (rutas absolutas) bajo `dir`, recursivamente. Se usa para verificar que una
/// operación rechazada **no tocó disco**: ni dentro ni fuera del workspace.
fn ficheros_bajo(dir: &std::path::Path) -> Vec<String> {
    let mut out = Vec::new();
    let mut pila = vec![dir.to_path_buf()];
    while let Some(d) = pila.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else {
            continue;
        };
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                pila.push(p);
            } else {
                out.push(p.to_string_lossy().to_string());
            }
        }
    }
    out.sort();
    out
}

/// Llamada `tools/call` a `knowledge_get` (camino de LECTURA) con la ruta cruda `path`.
///
/// El JSON se **serializa**, no se interpola en un literal: `path` es una ruta cruda del sistema y
/// en Windows lleva backslashes (`C:\Users\runneradmin\AppData\Local\Temp\…`). Interpolada dentro
/// de una cadena JSON, `\U`/`\A`/`\L`/`\T` no son escapes válidos y la línea entera dejaría de
/// parsear: el servidor respondería `-32700` (parse error de protocolo) y el test estaría
/// aseverando sobre una respuesta que no es la que quiere probar. `serde_json` escapa la ruta.
fn llamada_get(id: u32, path: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "knowledge_get",
            "arguments": { "ref": { "path": path }, "include": ["body"] }
        }
    })
    .to_string()
}

/// Llamada `tools/call` a `change_plan` con una op `create` sobre la ruta cruda `path` (camino de
/// ESCRITURA: es el que podría materializar un fichero fuera del workspace).
///
/// Serializada con `serde_json` por la misma razón que [`llamada_get`].
fn llamada_plan_create(id: u32, path: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": {
            "name": "change_plan",
            "arguments": { "operations": [{ "op": "create", "path": path }] }
        }
    })
    .to_string()
}

/// Comprueba que una respuesta de tool es un **rechazo de ejecución reconocible**: `isError: true`
/// en el `result` (no un error de protocolo, que el modelo no puede corregir), con un mensaje no
/// vacío, y sin filtrar contenido del fichero apuntado.
fn asserta_rechazo(resp: &serde_json::Value, ruta: &str) {
    assert!(
        resp["error"].is_null(),
        "una ruta inválida es un error de EJECUCIÓN de la tool (isError), no de protocolo: {resp:?}"
    );
    assert_eq!(
        resp["result"]["isError"],
        serde_json::Value::Bool(true),
        "la tool debe rechazar la ruta «{ruta}» con isError: {resp:?}"
    );
    let texto = resp["result"]["content"][0]["text"].as_str().unwrap_or("");
    assert!(
        !texto.trim().is_empty(),
        "el rechazo de «{ruta}» debe llevar un mensaje legible para el modelo: {resp:?}"
    );
    // Nada del fichero apuntado puede viajar de vuelta: ni el cebo plantado fuera de la raíz
    // (`secreto.md`) ni el contenido de `/etc/passwd` (que empieza por líneas «root:…»).
    let entera = resp.to_string();
    assert!(
        !entera.contains(SECRETO) && !entera.contains("root:"),
        "el rechazo de «{ruta}» no debe filtrar contenido de fuera del workspace: {resp:?}"
    );
}

/// E15-H06 · Criterio `rechaza_absoluta` — **guarda: ya verde, fija el contrato de la frontera**.
/// **Dado** un servidor arrancado, **Cuando** una tool recibe `path: "/etc/passwd"`, **Entonces**
/// responde error de path inválido sin tocar disco.
///
/// No es código nuevo: `RelPath::new` (`crates/lodestar-core/src/types.rs:33`) ya rechaza las
/// absolutas. Lo que fija este test es que ese rechazo es **contrato de la frontera MCP** (llega al
/// cliente como `isError` reconocible) y no un detalle de implementación que un refactor pueda
/// perder. Se cubren los dos caminos: lectura (`knowledge_get`) y escritura (`change_plan`).
///
/// Se prueban DOS rutas absolutas: la literal del criterio (`/etc/passwd`) y la del cebo
/// `<padre>/secreto.md` — esta última existe, es un `.md` bien formado y **sería legible** si la
/// validación desapareciera, así que es la que hace que la guarda muerda de verdad.
#[test]
fn rechaza_absoluta() {
    let (padre, ws) = ws_con_padre();
    let antes = ficheros_bajo(padre.path());
    let cebo = padre.path().join("secreto.md");
    let cebo = cebo.to_str().expect("tempdir con ruta UTF-8").to_string();

    let l1 = llamada_get(1, "/etc/passwd");
    let l2 = llamada_plan_create(2, "/etc/passwd");
    let l3 = llamada_get(3, &cebo);
    let l4 = llamada_plan_create(4, &cebo);
    let resp = roundtrip_en(&ws, &[], &[&l1, &l2, &l3, &l4], 4);

    assert_eq!(
        resp.len(),
        4,
        "el servidor debe seguir vivo tras rechazar rutas absolutas: {resp:?}"
    );
    asserta_rechazo(&resp[0], "/etc/passwd");
    asserta_rechazo(&resp[1], "/etc/passwd");
    asserta_rechazo(&resp[2], &cebo);
    asserta_rechazo(&resp[3], &cebo);

    // Sin tocar disco: ningún `.md` nuevo y ningún rastro de la ruta absoluta reconstruida
    // dentro del workspace (`ws/etc/passwd`).
    let despues = ficheros_bajo(padre.path());
    let nuevos_md: Vec<&String> = despues
        .iter()
        .filter(|p| p.ends_with(".md") && !antes.contains(*p))
        .collect();
    assert!(
        nuevos_md.is_empty(),
        "una ruta absoluta rechazada no debe escribir ningún .md: {nuevos_md:?}"
    );
    assert!(
        !ws.join("etc").exists(),
        "la ruta absoluta no debe reinterpretarse como relativa dentro del workspace"
    );
    assert!(
        !despues.iter().any(|p| p.ends_with("/passwd")),
        "no debe aparecer ningún fichero «passwd» bajo el árbol de pruebas: {despues:?}"
    );
}

/// E15-H06 · Criterio `rechaza_escape` — **guarda: ya verde, fija el contrato de la frontera**.
/// **Dado** un servidor arrancado, **Cuando** una tool recibe `path: "../fuera.md"`, **Entonces**
/// responde error de path inválido sin tocar disco.
///
/// Aquí «sin tocar disco» es literal y comprobable en las dos direcciones, porque el workspace vive
/// en un subdirectorio de un padre temporal y `..` apunta a un directorio real bajo control del
/// test: si el escape prosperase, la LECTURA de `../secreto.md` devolvería el cebo y la ESCRITURA
/// materializaría `<padre>/fuera.md`.
#[test]
fn rechaza_escape() {
    let (padre, ws) = ws_con_padre();
    let antes = ficheros_bajo(padre.path());

    let l1 = llamada_get(1, "../fuera.md");
    let l2 = llamada_plan_create(2, "../fuera.md");
    let l3 = llamada_get(3, "../secreto.md");
    let resp = roundtrip_en(&ws, &[], &[&l1, &l2, &l3], 3);

    assert_eq!(
        resp.len(),
        3,
        "el servidor debe seguir vivo tras rechazar un escape con «..»: {resp:?}"
    );
    asserta_rechazo(&resp[0], "../fuera.md");
    asserta_rechazo(&resp[1], "../fuera.md");
    asserta_rechazo(&resp[2], "../secreto.md");

    assert!(
        !padre.path().join("fuera.md").exists(),
        "el escape con «..» no debe materializar nada fuera de la raíz del workspace"
    );
    let despues = ficheros_bajo(padre.path());
    let nuevos_md: Vec<&String> = despues
        .iter()
        .filter(|p| p.ends_with(".md") && !antes.contains(*p))
        .collect();
    assert!(
        nuevos_md.is_empty(),
        "un escape rechazado no debe escribir ningún .md: {nuevos_md:?}"
    );
}

// =============================================================================
// E19-H05 — Cablear el lenguaje de consulta a `knowledge_search`
// =============================================================================
//
// UBICACIÓN (decisión del autor de tests). Los 4 criterios se ejercitan **e2e por la tool MCP**
// (frontera JSON-RPC), no contra `App::knowledge_search` directo, por tres razones:
//   1. Lo que la historia cambia es el **contrato de wire**: los `arguments` (`where`/`filter`
//      sustituyen a `filters`) y la **forma del `SearchResult`** (pierde `type`/`status`/
//      `description`/`tags`). Probarlo por JSON-RPC fija ESE contrato sin acoplar los tests a la
//      firma Rust interna que el implementador aún ha de diseñar (`knowledge_search(text, where,
//      filter, …)`, la retirada de `SearchFilters`) — la misma razón deliberada que ya movió a e2e
//      los tests de E10-H09 (ver el bloque de esa historia, arriba).
//   2. Al no referenciar ningún símbolo Rust nuevo, estos tests **compilan contra el binario MCP
//      actual** y el ROJO es puro fallo de aserción (no `todo!()` ni símbolo inexistente): NO hace
//      falta ningún stub de producción. El dispatcher actual (`main.rs`/`tools.rs`) lee solo
//      `text`/`filters`/`sort`/`limit`/`cursor` y NO valida `additionalProperties`, así que hoy
//      IGNORA `where`/`filter` → la búsqueda devuelve TODOS los documentos (`text` vacío) y las
//      aserciones muerden por la razón correcta (el lenguaje no está cableado).
//   3. El test insignia `search_propiedad_de_grafo` es más fuerte e2e: demuestra que TODA la tubería
//      (dispatch MCP → `App` → evaluador del core que ve el `Analysis`/grafo) está cableada, no solo
//      una unidad.
//
// CONTRATO nuevo fijado (fase ROJA — el cableado del lenguaje aún NO existe):
//   arguments: { text?: string, where?: string, filter?: object(§20.10), sort?, limit?, cursor? }
//     · `where` (textual) y `filter` (JSON) → el MISMO `Expression` (E19-H01…H04) → filtro,
//       intersectado con el FTS de `text` (como hoy).
//   structuredContent.results[*]: conserva `path`, `title` (derivado) y `snippet` (+ `revision`,
//     `score`, `id` genéricos); NO lleva `type`/`status`/`description`/`tags` privilegiados.
//
// NOTA para el implementador (fuera de mi alcance — NO son tests):
//   · `contracts/mcp.yml` cambia: el `inputSchema` de `knowledge_search` pasa de `filters` a
//     `where`/`filter`, y el `SearchResult` pierde los 4 campos OKF. Es superficie, no test; lo
//     verifica `/contrato --check`, no un `#[test]`.
//   · Rompen al retirar `SearchFilters`/`query`/DSL vieja (inventario en el informe): en este mismo
//     fichero, `search_filtra_tipo`; en `lodestar-app/tests/escala.rs`,
//     `bench_search_payload_acotado`; en `lodestar-core/tests/core.rs`, los tests de `DocumentSet::
//     query`. Su migración/retirada es trabajo del implementador (no los toco: solo añado).

/// Construye la línea JSON-RPC de un `tools/call` a `knowledge_search` con `arguments` arbitrarios.
/// Usa `serde_json::json!` para no pelear con el escapado de comillas del `where`.
fn ks_call(arguments: serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "knowledge_search", "arguments": arguments }
    })
    .to_string()
}

/// Workspace con documentos de distinto `status` en frontmatter y, por lo demás, texto/metadata
/// indistinguibles: el único criterio que separa los aceptados es el valor de la propiedad `status`.
fn workspace_estados() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntitle: Índice\n---\n\n# Índice\n\ncontenido común.\n",
    );
    write(
        dir.path(),
        "aceptada-uno.md",
        "---\ntitle: Aceptada uno\nstatus: accepted\n---\n\n# Aceptada uno\n\ncontenido común.\n",
    );
    write(
        dir.path(),
        "aceptada-dos.md",
        "---\ntitle: Aceptada dos\nstatus: accepted\n---\n\n# Aceptada dos\n\ncontenido común.\n",
    );
    write(
        dir.path(),
        "borrador.md",
        "---\ntitle: Borrador\nstatus: draft\n---\n\n# Borrador\n\ncontenido común.\n",
    );
    write(
        dir.path(),
        "revision.md",
        "---\ntitle: Revisión\nstatus: review\n---\n\n# Revisión\n\ncontenido común.\n",
    );
    dir
}

/// E19-H05 · Criterio `search_where`:
/// Dado `knowledge_search {where: "status = \"accepted\""}`, Cuando se busca, Entonces solo aparecen
/// los documentos cuyo `status` de frontmatter es `accepted` (los demás, y `index.md` sin `status`,
/// quedan fuera). NO vacuo: los documentos de otro `status` deben quedar EXCLUIDOS (un stub que hoy
/// ignora `where` los devuelve todos y muerde aquí).
#[test]
fn search_where() {
    let dir = workspace_estados();
    let resp = roundtrip(
        dir.path(),
        &[ks_call(serde_json::json!({ "where": "status = \"accepted\"" })).as_str()],
        1,
    );
    let paths = search_paths(&resp[0]);
    let tiene = |p: &str| paths.iter().any(|x| x == p);

    assert!(
        !paths.is_empty(),
        "el `where` `status = \"accepted\"` debe devolver al menos un documento: {resp:?}"
    );
    for aceptada in ["aceptada-uno.md", "aceptada-dos.md"] {
        assert!(
            tiene(aceptada),
            "el documento `{aceptada}` (status: accepted) debe aparecer con `where` status=accepted: {resp:?}"
        );
    }
    for otra in ["borrador.md", "revision.md"] {
        assert!(
            !tiene(otra),
            "el documento `{otra}` (status != accepted) NO debe aparecer: el `where` filtra por metadata: {resp:?}"
        );
    }
    assert!(
        !tiene("index.md"),
        "`index.md` no tiene `status`: un campo ausente en una comparación es `false`, no debe casar `status = \"accepted\"`: {resp:?}"
    );
}

/// E19-H05 · Criterio `search_filter_equivalente`:
/// Dado el `filter` JSON equivalente al `where` anterior, Cuando se busca por ambas vías, Entonces
/// devuelven EL MISMO conjunto de documentos — y ese conjunto es exactamente los aceptados. La
/// equivalencia se prueba de PUNTA A PUNTA por la superficie de `knowledge_search` (la equivalencia
/// a nivel de AST ya la cubre E19-H03; aquí NO se duplica). El ancla al conjunto esperado impide el
/// pase vacuo de «ambas vías ignoran el filtro y devuelven todo».
#[test]
fn search_filter_equivalente() {
    let dir = workspace_estados();

    let por_where = roundtrip(
        dir.path(),
        &[ks_call(serde_json::json!({ "where": "status = \"accepted\"" })).as_str()],
        1,
    );
    let mut set_where = search_paths(&por_where[0]);
    set_where.sort();

    let por_filter = roundtrip(
        dir.path(),
        &[ks_call(serde_json::json!({
            "filter": { "field": "frontmatter.status", "operator": "equals", "value": "accepted" }
        }))
        .as_str()],
        1,
    );
    let mut set_filter = search_paths(&por_filter[0]);
    set_filter.sort();

    // (1) Mismo conjunto por ambas vías: `where` textual y `filter` JSON son equivalentes.
    assert_eq!(
        set_where, set_filter,
        "`where` y `filter` equivalentes deben devolver EL MISMO conjunto de documentos: \
         where={por_where:?} filter={por_filter:?}"
    );

    // (2) Y ese conjunto es exactamente los aceptados (ancla no vacía ⇒ no vacuo).
    let mut esperado = vec!["aceptada-dos.md".to_string(), "aceptada-uno.md".to_string()];
    esperado.sort();
    assert_eq!(
        set_filter, esperado,
        "el `filter` equivalente debe seleccionar exactamente los documentos con status=accepted, \
         no todos los documentos: {por_filter:?}"
    );
}

/// Workspace con documentos enlazados y no enlazados, indistinguibles por texto/metadata: solo el
/// GRAFO los separa. `index.md` enlaza a `enlazado.md` (1 backlink); `huerfano.md` no recibe enlaces.
fn workspace_enlaces() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntitle: Índice\n---\n\n# Índice\n\n* [Enlazado](enlazado.md)\n",
    );
    write(
        dir.path(),
        "enlazado.md",
        "---\ntitle: Enlazado\n---\n\n# Enlazado\n\ncontenido idéntico.\n",
    );
    write(
        dir.path(),
        "huerfano.md",
        "---\ntitle: Huérfano\n---\n\n# Huérfano\n\ncontenido idéntico.\n",
    );
    dir
}

/// E19-H05 · Criterio `search_propiedad_de_grafo` (TEST INSIGNIA):
/// Dado `knowledge_search {where: "graph.backlinks = 0"}`, Cuando se busca, Entonces devuelve los
/// documentos NO enlazados (`huerfano.md`) y EXCLUYE los enlazados (`enlazado.md`, con 1 backlink
/// desde `index.md`). Es la prueba de que detrás está el evaluador NUEVO —que ve el grafo— y no un
/// grep de subcadena, que no puede expresar `graph.backlinks = 0`. `enlazado.md` y `huerfano.md`
/// tienen cuerpo/metadata idénticos: solo la propiedad calculada del grafo los distingue.
#[test]
fn search_propiedad_de_grafo() {
    let dir = workspace_enlaces();
    let resp = roundtrip(
        dir.path(),
        &[ks_call(serde_json::json!({ "where": "graph.backlinks = 0" })).as_str()],
        1,
    );
    let paths = search_paths(&resp[0]);
    let tiene = |p: &str| paths.iter().any(|x| x == p);

    assert!(
        !paths.is_empty(),
        "`graph.backlinks = 0` debe devolver los documentos no enlazados: {resp:?}"
    );
    assert!(
        tiene("huerfano.md"),
        "`huerfano.md` no recibe enlaces (backlinks 0): debe aparecer con `graph.backlinks = 0`: {resp:?}"
    );
    assert!(
        !tiene("enlazado.md"),
        "`enlazado.md` recibe 1 backlink desde `index.md`: NO debe aparecer con `graph.backlinks = 0` \
         (una consulta de subcadena no podría excluirlo — es la prueba del evaluador de grafo): {resp:?}"
    );
}

/// Workspace con un documento cuyo frontmatter TIENE los antiguos campos privilegiados OKF
/// (`type`/`status`/`description`/`tags`) poblados y un cuerpo que casa «autenticación».
fn workspace_con_metadata() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "index.md",
        "---\ntitle: Índice\n---\n\n# Índice\n\n* [doc](doc.md)\n",
    );
    write(
        dir.path(),
        "doc.md",
        "---\ntype: decision\ntitle: Documento con metadata\nstatus: accepted\ndescription: Una descripción\ntags:\n  - seguridad\n  - redes\n---\n\n# Documento\n\ncuerpo sobre autenticación y redes.\n",
    );
    dir
}

/// E19-H05 · Criterio `search_result_sin_campos_okf`:
/// Dado un documento cuyo frontmatter lleva `type`/`status`/`description`/`tags`, Cuando aparece en
/// `knowledge_search`, Entonces el resultado del wire NO surfacea esos campos como privilegiados —
/// aunque estén en el frontmatter— y sí conserva `path`, `title` (derivado) y `snippet`.
#[test]
fn search_result_sin_campos_okf() {
    let dir = workspace_con_metadata();
    let resp = roundtrip(
        dir.path(),
        &[ks_call(serde_json::json!({ "text": "autenticación" })).as_str()],
        1,
    );
    let results = search_paths_values(&resp[0]);

    let doc = results
        .iter()
        .find(|r| r["path"] == "doc.md")
        .unwrap_or_else(|| panic!("el documento que casa «autenticación» debe aparecer: {resp:?}"));

    // Conserva `path`, `title` (derivado) y `snippet`.
    assert_eq!(
        doc["path"], "doc.md",
        "el resultado conserva `path` (identidad del documento): {doc:?}"
    );
    assert!(
        !doc["title"].as_str().unwrap_or("").is_empty(),
        "el resultado conserva un `title` derivado no vacío: {doc:?}"
    );
    assert!(
        !doc["snippet"].as_str().unwrap_or("").is_empty(),
        "el resultado conserva un `snippet` no vacío: {doc:?}"
    );

    // NO surfacea los antiguos campos privilegiados OKF, aunque estén en el frontmatter del documento.
    for campo in ["type", "status", "description", "tags"] {
        assert!(
            doc.get(campo).is_none(),
            "el resultado de `knowledge_search` NO debe llevar el campo privilegiado OKF `{campo}`: \
             está en el frontmatter, pero deja de ser un campo de wire (el filtrado por metadata pasa \
             por el lenguaje): {doc:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// E20-H03 — Sustituir `schema_inspect` por `metadata_inspect` y retirar `core::schema`.
//
// UBICACIÓN: los 3 criterios se ejercitan **e2e por la tool MCP** (campo Pruebas de la historia +
// coherente con E10-H11 y E19-H05): lo que importa fijar aquí es el contrato de **wire** (nombre de
// la tool en `tools/list`, forma del `structuredContent` del catálogo y de la inspección) sin
// acoplar los tests a la firma interna que el implementador elija (`App::metadata_inspect`, un tipo
// wire de `lodestar-app` o un `derive` directo sobre los tipos de `core::types`). Por eso NO hay
// stub en `src/`: la tool ausente da un ROJO limpio en runtime.
//
// FASE ROJA: la tool `metadata_inspect` NO está en `tools::list()` todavía y la vieja
// `schema_inspect` SÍ, así que:
//   · `tool_es_metadata_inspect` falla por partida doble: `metadata_inspect` ausente Y
//     `schema_inspect` presente.
//   · `metadata_inspect_catalog`/`metadata_inspect_field` invocan una tool inexistente →
//     `tools/call` responde `-32602` y `result` es `null` → los asserts que leen
//     `structuredContent.*` fallan por AUSENCIA de la tool/servicio (no por un valor erróneo).
//
// CONTRATO DE WIRE que fija esta fase (los 4 tipos de retorno de E20-H01/H02
// —`MetadataCatalog`/`FieldStats`/`FieldInspection`/`ValueCount`— quedaron SIN serde a propósito
// para que H03 lo clave):
//   arguments: { mode: "catalog" }                       -> catálogo de propiedades
//            | { mode: "field", field: "<dot.path>" }    -> inspección de un campo
//   structuredContent (mode "catalog"): {
//     fields: [ { name: "<dot.path>", presentIn: N, inferredTypes: { "<tipo>": N, … } } ]
//   }
//   structuredContent (mode "field"): {
//     field: "<dot.path>", presentIn: N, missingIn: N,
//     inferredTypes: { "<tipo>": N, … },
//     values: [ { value: <valor en su tipo JSON natural>, count: N } ]
//   }
// Decisiones de wire (autor de tests, clavadas por los asserts de abajo):
//   · El path del campo es la clave `name` en el CATÁLOGO y `field` en la INSPECCIÓN (§Fase 6:
//     `{"name":"status",…}` vs `{"field":"status",…}`); un `FieldPath` rinde a su string punteado
//     (`"service.tier"`), no a un array de segmentos.
//   · `inferredTypes` es un OBJETO `{nombre-de-tipo-en-minúscula: conteo}`, NO un array de pares:
//     el `BTreeMap<ValueType, usize>` interno se aplana a `{ "string": N, "number": N }`.
//   · `values[*].value` conserva su tipo JSON natural: un número es número, un string es string
//     (sin coerción — el número `5` y el string `"5"` son valores distintos).
// ---------------------------------------------------------------------------

/// Workspace con metadata heterogénea sobre el campo `status`, servible por AMBOS modos de
/// `metadata_inspect` (catálogo e inspección) con números coherentes entre sí:
///   · `status` presente en 6 de 8 documentos (2 sin frontmatter → ausente);
///   · tipos: 5 string (`accepted`×3, `draft`×2) + 1 number (`status: 5`);
///   · valores escalares: `accepted`×3, `draft`×2, `5`(número)×1.
///
/// No vacuo por diseño: los conteos discriminan (present 6 ≠ total 8), los tipos son mixtos
/// (string y number) y hay un valor NUMÉRICO — así el wire de `inferredTypes` (objeto por tipo) y
/// el de `values` (tipo JSON natural) se ejercitan de verdad, no con un único string uniforme.
fn workspace_metadata() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for (slug, status) in [
        ("p1", "accepted"),
        ("p2", "accepted"),
        ("p3", "accepted"),
        ("p4", "draft"),
        ("p5", "draft"),
    ] {
        write(
            dir.path(),
            &format!("{slug}.md"),
            &format!("---\nstatus: {status}\n---\n\n# {slug}\n\ncuerpo.\n"),
        );
    }
    // `status` NUMÉRICO: un `5` a secas es un número YAML, no un string (sin coerción).
    write(
        dir.path(),
        "p6.md",
        "---\nstatus: 5\n---\n\n# p6\n\ncuerpo.\n",
    );
    // 2 documentos SIN frontmatter → `status` ausente (missingIn == 2).
    write(dir.path(), "p7.md", "# p7\n\nsin frontmatter.\n");
    write(dir.path(), "p8.md", "# p8\n\nsin frontmatter.\n");
    dir
}

/// E20-H03 · Criterio `tool_es_metadata_inspect`:
/// Dado el MCP, Cuando se pide `tools/list`, Entonces aparece `metadata_inspect` y NO
/// `schema_inspect`. Se asevera lo uno Y lo otro (presencia de la nueva, ausencia de la vieja): no
/// basta con añadir `metadata_inspect` dejando `schema_inspect` en la superficie.
#[test]
fn tool_es_metadata_inspect() {
    let dir = workspace_min();
    let resp = roundtrip(
        dir.path(),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#],
        1,
    );
    let tools = nombres_de_tools(&resp[0]);
    assert!(
        tools.contains("metadata_inspect"),
        "tools/list debe incluir la tool «metadata_inspect»: {tools:?}"
    );
    assert!(
        !tools.contains("schema_inspect"),
        "tools/list NO debe incluir la tool retirada «schema_inspect»: {tools:?}"
    );
}

/// E20-H03 · Criterio `metadata_inspect_catalog`:
/// Dado `metadata_inspect {mode: "catalog"}`, Cuando se llama, Entonces devuelve el catálogo de H01:
/// `fields` con `name`/`presentIn`/`inferredTypes` (§Fase 6).
#[test]
fn metadata_inspect_catalog() {
    let dir = workspace_metadata();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"metadata_inspect","arguments":{"mode":"catalog"}}}"#,
        ],
        1,
    );
    let sc = &resp[0]["result"]["structuredContent"];
    let fields = sc["fields"].as_array().unwrap_or_else(|| {
        panic!("metadata_inspect(catalog) debe devolver structuredContent.fields (array): {resp:?}")
    });

    // El campo `status` aparece con clave `name` (§Fase 6: `{"name":"status",…}`), no `field`.
    let status = fields
        .iter()
        .find(|f| f["name"] == "status")
        .unwrap_or_else(|| {
            panic!("el catálogo debe listar el campo «status» bajo la clave `name`: {resp:?}")
        });

    // `presentIn`: en 6 de los 8 documentos (los conteos discriminan → no vacuo).
    assert_eq!(
        status["presentIn"].as_u64(),
        Some(6),
        "status.presentIn debe ser 6 (presente en 6 de 8 documentos): {status:?}"
    );

    // `inferredTypes` es un OBJETO {tipo-en-minúscula: conteo}, NO un array de pares.
    let tipos = &status["inferredTypes"];
    assert!(
        tipos.is_object(),
        "inferredTypes debe ser un objeto {{tipo: conteo}}, no un array de pares: {status:?}"
    );
    assert_eq!(
        tipos["string"].as_u64(),
        Some(5),
        "inferredTypes.string debe ser 5 (accepted×3 + draft×2): {status:?}"
    );
    assert_eq!(
        tipos["number"].as_u64(),
        Some(1),
        "inferredTypes.number debe ser 1 (`status: 5` es un número, sin coerción a string): {status:?}"
    );
}

/// E20-H03 · Criterio `metadata_inspect_field`:
/// Dado `metadata_inspect {mode: "field", field: "status"}`, Cuando se llama, Entonces devuelve la
/// inspección de H02: `presentIn`/`missingIn`/`inferredTypes`/`values` con `value`/`count` (§Fase 6).
#[test]
fn metadata_inspect_field() {
    let dir = workspace_metadata();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"metadata_inspect","arguments":{"mode":"field","field":"status"}}}"#,
        ],
        1,
    );
    let sc = &resp[0]["result"]["structuredContent"];
    // Rojo limpio si la tool no existe: `structuredContent` nulo → panic explicando el porqué.
    assert!(
        sc.is_object(),
        "metadata_inspect(field) debe devolver structuredContent (objeto): {resp:?}"
    );

    // El path inspeccionado viaja con la clave `field` (§Fase 6: `{"field":"status",…}`), no `name`.
    assert_eq!(
        sc["field"], "status",
        "la inspección debe llevar `field` == «status» (string punteado del FieldPath): {resp:?}"
    );

    // Presencia/ausencia sobre el total: present 6 + missing 2 == 8 documentos.
    assert_eq!(
        sc["presentIn"].as_u64(),
        Some(6),
        "status.presentIn debe ser 6: {resp:?}"
    );
    assert_eq!(
        sc["missingIn"].as_u64(),
        Some(2),
        "status.missingIn debe ser 2 (2 documentos sin frontmatter): {resp:?}"
    );

    // `inferredTypes` como objeto {tipo: conteo}, misma forma que en el catálogo.
    let tipos = &sc["inferredTypes"];
    assert!(
        tipos.is_object(),
        "inferredTypes debe ser un objeto {{tipo: conteo}}: {resp:?}"
    );
    assert_eq!(
        tipos["string"].as_u64(),
        Some(5),
        "inferredTypes.string: {resp:?}"
    );
    assert_eq!(
        tipos["number"].as_u64(),
        Some(1),
        "inferredTypes.number: {resp:?}"
    );

    // `values`: lista de `{value, count}` con el valor en su TIPO JSON natural.
    let values = sc["values"].as_array().unwrap_or_else(|| {
        panic!(
            "metadata_inspect(field) debe devolver `values` (array de {{value, count}}): {resp:?}"
        )
    });

    // Un valor STRING conserva su tipo string y su conteo (accepted×3).
    let accepted = values
        .iter()
        .find(|v| v["value"] == "accepted")
        .unwrap_or_else(|| panic!("`values` debe incluir el string «accepted»: {resp:?}"));
    assert!(
        accepted["value"].is_string(),
        "el valor «accepted» debe viajar como string JSON: {accepted:?}"
    );
    assert_eq!(
        accepted["count"].as_u64(),
        Some(3),
        "«accepted» aparece en 3 documentos: {accepted:?}"
    );

    // Un valor NUMÉRICO conserva su tipo número (no se convierte a `"5"`): clava el tipo JSON natural.
    let numerico = values
        .iter()
        .find(|v| v["value"].is_number())
        .unwrap_or_else(|| {
            panic!("`values` debe incluir el valor NUMÉRICO como número JSON (no como string): {resp:?}")
        });
    assert_eq!(
        numerico["value"].as_i64(),
        Some(5),
        "el valor numérico debe ser 5 (número, sin coerción a string): {numerico:?}"
    );
    assert_eq!(
        numerico["count"].as_u64(),
        Some(1),
        "el valor numérico 5 aparece en 1 documento: {numerico:?}"
    );
}

// ---------------------------------------------------------------------------
// E23-H04 — `pendingTransaction` real
// (`requirements/epica-23-cierre-migracion.md`, `crates/lodestar-workspace/src/recovery.rs`)
//
// SÍNTOMA: `workspace_status.recovery.pendingTransaction` es un `false` LITERAL en
// `crates/lodestar-app/src/lib.rs` (`StatusRecovery { pending_transaction: false }`), pese a que
// `Workspace::recovery_pending()` existe y funciona desde E13-H06. Tras un crash, la primera tool
// que llama un agente le miente: planifica con normalidad y solo descubre el problema cuando
// `change_apply` explota con `WORKSPACE_RECOVERY_REQUIRED`.
//
// UBICACIÓN: por la **frontera MCP JSON-RPC real** (binario `lodestar-mcp` sobre stdio), no por la
// capa `App`. Es criterio DURO de la historia: éste es el sexto hueco de cableado de la misma
// familia (E17 `other_files`, E20-H04 diagnósticos, E22-H04 selección masiva) y todos los anteriores
// se escaparon precisamente porque se probaban en `App`.
//
// MONTAJE del estado «transacción a medias»: SIN la feature `test-failpoints` (que vive en
// `lodestar-workspace` y no está activada aquí). Se compone con las MISMAS primitivas públicas y
// durables que usa `simular_caida` en `crates/lodestar-workspace/tests/transactions.rs`
// (`backup_originals` de E13-H04 + `create_journal`/`mark_applied` de E13-H03), deteniéndose en el
// equivalente a `FailPoint::EntreRenames`: journal `applying`, 1 de 2 renames hechos, copias de
// recuperación listas y ningún `done`. Es exactamente lo que un crash real deja en disco.
// ---------------------------------------------------------------------------

use lodestar_core::types::RelPath;
use lodestar_workspace::Workspace;

/// Id de la transacción interrumpida. Un mismo id nombra el journal (`<id>.json`) y las copias de
/// recuperación (`recovery/<id>/`), como fija la convención de E13-H06.
const TXN_A_MEDIAS: &str = "txn-e23-h04-a-medias";

/// Monta un workspace con una **transacción a medias** durable en disco: dos documentos canónicos,
/// copias de recuperación de ambos, un write-ahead journal en estado `applying` con el primer rename
/// ya marcado y el segundo `pending`, y el canónico reflejando solo ese primer rename.
///
/// El `Workspace` se abre y se **dropea** dentro de esta función: eso es la «caída». Nada sella el
/// journal a `done`, así que `Workspace::recovery_pending()` queda en `true` para cualquier proceso
/// que abra después el mismo directorio — incluido el servidor MCP.
///
/// Nota sobre las revisiones del journal: se pasa la misma `WorkspaceRevision` como base y como
/// resultado porque la recuperación (`JournalHeader` en `recovery.rs`) solo lee `txnId` y `state`
/// del JSON — los campos de revisión no participan en la detección de recuperación pendiente.
fn workspace_transaccion_a_medias() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    write(root, "notas/uno.md", "# Uno\n\ncuerpo original de uno\n");
    write(root, "notas/dos.md", "# Dos\n\ncuerpo original de dos\n");

    let ws = Workspace::open(root).expect("el workspace de prueba debe abrir");
    let uno = RelPath::new("notas/uno.md").unwrap();
    let dos = RelPath::new("notas/dos.md").unwrap();
    let afectados = [uno.clone(), dos];
    let base = ws
        .workspace_revision()
        .expect("revisión base del workspace");

    // (H04) Copias de recuperación de los originales afectados: preceden a todo rename.
    ws.backup_originals(TXN_A_MEDIAS, &afectados)
        .expect("preparar las copias de recuperación");
    // (H03) Write-ahead journal `prepared`, fsynced antes de tocar el canónico.
    let mut journal = ws
        .create_journal(TXN_A_MEDIAS, &afectados, &base, &base)
        .expect("crear el write-ahead journal");
    // (H05) Primer rename «hecho»: el canónico ya refleja el cambio de `notas/uno.md`…
    std::fs::write(
        root.join("notas/uno.md"),
        "# Uno\n\ncuerpo NUEVO a medio publicar\n",
    )
    .unwrap();
    journal
        .mark_applied(&uno)
        .expect("marcar el primer rename en el journal");
    // …y aquí «se cae»: el segundo rename nunca ocurre y el journal nunca llega a `done`.

    dir
}

/// **E23-H04** · Criterio `status_reporta_recovery_pendiente`:
/// **Dado** un workspace con una transacción a medias (journal presente), **Cuando** se llama a
/// `workspace_status` **por MCP**, **Entonces** `recovery.pendingTransaction` es `true`.
///
/// ROJO hoy: `App::workspace_status` construye `StatusRecovery { pending_transaction: false }` con un
/// literal, sin consultar `Workspace::recovery_pending()`, así que la tool responde `false` sobre un
/// workspace que sí necesita recuperación.
///
/// La precondición (`recovery_pending()` directo sobre el mismo directorio) NO es decorativa: prueba
/// que el fixture montó de verdad el estado interrumpido, de modo que un `false` en la respuesta solo
/// puede ser el hueco de cableado y nunca un fixture mal montado.
#[test]
fn status_reporta_recovery_pendiente() {
    let dir = workspace_transaccion_a_medias();

    // Precondición no vacua: el estado en disco es realmente una recuperación pendiente.
    let ws = Workspace::open(dir.path()).expect("reabrir el workspace de prueba");
    assert!(
        ws.recovery_pending(),
        "precondición: el fixture debe dejar una recuperación PENDIENTE (journal no-`done` bajo \
         .lodestar/runtime/journal/)"
    );
    drop(ws);

    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"workspace_status","arguments":{}}}"#,
        ],
        1,
    );
    let sc = &resp[0]["result"]["structuredContent"];
    assert_eq!(
        sc["recovery"]["pendingTransaction"],
        serde_json::Value::Bool(true),
        "con una transacción a medias en disco, workspace_status debe reportar \
         recovery.pendingTransaction == true (hoy es un `false` literal en \
         `App::workspace_status`, sin consultar `Workspace::recovery_pending()`): {resp:?}"
    );
}

/// **E23-H04** · Criterio `status_sin_recovery_pendiente` (**control anti-vacuo**):
/// **Dado** un workspace limpio, **Cuando** se llama a `workspace_status`, **Entonces**
/// `recovery.pendingTransaction` es `false`.
///
/// Impide que el criterio anterior se satisfaga cableando un `true` literal (el defecto simétrico al
/// de hoy). GUARDA verde en la fase roja —el literal actual ya devuelve `false`—; su valor es de
/// regresión sobre la implementación futura.
#[test]
fn status_sin_recovery_pendiente() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notas/uno.md", "# Uno\n\ncuerpo original\n");
    write(dir.path(), "notas/dos.md", "# Dos\n\ncuerpo original\n");

    // Precondición no vacua: este workspace NO tiene recuperación pendiente (el par exacto del
    // fixture de `status_reporta_recovery_pendiente`, sin el journal interrumpido).
    let ws = Workspace::open(dir.path()).expect("el workspace limpio debe abrir");
    assert!(
        !ws.recovery_pending(),
        "precondición: un workspace recién creado no tiene ninguna transacción a medias"
    );
    drop(ws);

    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"workspace_status","arguments":{}}}"#,
        ],
        1,
    );
    let sc = &resp[0]["result"]["structuredContent"];
    assert_eq!(
        sc["recovery"]["pendingTransaction"],
        serde_json::Value::Bool(false),
        "sobre un workspace limpio, workspace_status debe reportar \
         recovery.pendingTransaction == false: {resp:?}"
    );
}

// ===========================================================================
// E23-H09 · Bordes (`requirements/epica-23-cierre-migracion.md`)
//
// Lo que separa «funciona en un tempdir limpio» de «funciona en un repo real». Es cobertura que
// faltaba, no fase roja: se espera que estos tests pasen.
//
// La concurrencia ENTRE PROCESOS (`dos_procesos_un_ganador`, `lock_huerfano`) vive en
// `crates/lodestar-mcp/tests/concurrencia.rs`, que necesita mantener dos servidores vivos a la vez
// y no puede usar el `roundtrip` de este fichero. El Unicode en rutas, en
// `crates/lodestar-workspace/tests/discovery.rs`.
// ===========================================================================

/// El texto del error de EJECUCIÓN de una tool: `crates/lodestar-mcp/src/tools.rs` pone ahí el
/// código estable (`ErrorCode::as_str()`), nunca el `Debug` de la variante. Verifica de paso que el
/// rechazo llegó como `isError` y **no** como error de protocolo JSON-RPC.
fn texto_de_error(resp: &serde_json::Value) -> String {
    assert_eq!(
        resp["result"]["isError"], true,
        "se esperaba un error de EJECUCIÓN de la tool (isError en el result): {resp:?}"
    );
    assert!(
        resp["error"].is_null(),
        "un rechazo del motor NO debe viajar como error de protocolo JSON-RPC: {resp:?}"
    );
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("el error debe traer su código estable como texto: {resp:?}"))
        .to_string()
}

/// Frontmatter **sintácticamente inválido** dentro de un bloque bien delimitado: el mismo
/// disparador de `FM-YAML-INVALID` que ya usa `workspace_editado_a_mano`, aquí como dato de
/// entrada del camino de ESCRITURA.
const FM_ROTO: &str =
    "---\ntitle: : :\n  - roto\nestado: escrito a pelo\n---\n\n# Nota rota\n\ncuerpo valioso.\n";

/// Bloque de frontmatter que abre y **nunca cierra** (`FM-UNCLOSED`): la otra mitad de «ilegible».
/// Va en el mismo test porque son dos ramas distintas de `model::patch_frontmatter`
/// (`SplitFront::SinCerrar` vs. YAML que no parsea) y arreglar una no arregla la otra.
const FM_SIN_CERRAR: &str =
    "---\nestado: abierto\n\n# Nota sin cerrar\n\notro cuerpo valioso que no se puede perder.\n";

/// **E23-H09** · Criterio `patch_sobre_frontmatter_ilegible`:
/// **Dado** un documento cuyo frontmatter no se puede interpretar, **Cuando** se le aplica un
/// `patch_frontmatter`, **Entonces** la operación falla limpio (`INVALID_SCHEMA`) y el `.md` queda
/// **byte a byte** como estaba.
///
/// Es la vía más directa a pérdida de datos del usuario y hasta esta historia solo estaba probada
/// en el camino de LECTURA (`check_detecta_edicion_directa` → `FM-YAML-INVALID`). En escritura, la
/// alternativa peligrosa sería «reconstruir el bloque encima» con las claves que sí se entienden:
/// eso borraría en silencio lo que el usuario había escrito. El core lo evita a propósito
/// (`crates/lodestar-core/src/plan.rs`, rama `PatchFrontmatter`: «un frontmatter ilegible hace
/// fallar la operación en vez de reconstruirse encima»); aquí se fija **por la frontera MCP** y
/// **verificando el disco**, que es donde se pierde el dato.
///
/// El fallo ocurre ya en `change_plan` —que simula el cambio en memoria— así que no llega a existir
/// un plan que aplicar: mejor todavía, el agente se entera antes de pedir la escritura.
///
/// Anti-vacuo: la tercera parte del test aplica el MISMO patch a un documento sano y comprueba que
/// ese sí planifica y escribe. Sin ella, un `patch_frontmatter` roto de raíz pasaría este test.
#[test]
fn patch_sobre_frontmatter_ilegible() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "roto-yaml.md", FM_ROTO);
    write(dir.path(), "sin-cerrar.md", FM_SIN_CERRAR);
    write(
        dir.path(),
        "sano.md",
        "---\nestado: borrador\n---\n\n# Sano\n\ncuerpo.\n",
    );

    // Copia byte a byte del estado previo de los dos documentos ilegibles.
    let antes: Vec<(String, Vec<u8>)> = ["roto-yaml.md", "sin-cerrar.md"]
        .iter()
        .map(|p| {
            (
                (*p).to_string(),
                std::fs::read(dir.path().join(p)).expect("leer el documento ilegible"),
            )
        })
        .collect();

    for (ruta, contenido_previo) in &antes {
        let ops = serde_json::json!([
            { "op": "patch_frontmatter", "ref": { "path": ruta },
              "patch": { "estado": "PATCH-QUE-NO-DEBE-APLICARSE" } }
        ]);
        let resp = roundtrip(
            dir.path(),
            &[change_plan_line(None, ops, policy_permisiva()).as_str()],
            1,
        );

        // (a) Falla limpio, con el código estable del catálogo — no un panic ni un plan silencioso.
        let codigo = texto_de_error(&resp[0]);
        assert!(
            codigo.contains("INVALID_SCHEMA"),
            "parchear el frontmatter de «{ruta}» (ilegible) debe rechazarse con INVALID_SCHEMA \
             —precondición de la operación incumplida por el dato de entrada—, no con «{codigo}»: \
             {resp:?}"
        );

        // (b) LO ESENCIAL: el fichero del usuario sigue **byte a byte** como estaba.
        let ahora = std::fs::read(dir.path().join(ruta)).expect("releer el documento ilegible");
        assert_eq!(
            &ahora,
            contenido_previo,
            "un patch rechazado NO puede tocar el documento: «{ruta}» debe quedar byte a byte \
             igual. Antes: {:?} · Ahora: {:?}",
            String::from_utf8_lossy(contenido_previo),
            String::from_utf8_lossy(&ahora)
        );
        // Redundante a propósito y legible en el informe de fallo: el bloque original sobrevive.
        let texto = String::from_utf8_lossy(&ahora);
        assert!(
            !texto.contains("PATCH-QUE-NO-DEBE-APLICARSE"),
            "la clave del patch no puede haberse colado en «{ruta}»: {texto:?}"
        );
    }

    // (c) Anti-vacuo: el mismo patch sobre un documento con frontmatter legible SÍ planifica.
    let ops_sanas = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "sano.md" },
          "patch": { "estado": "PATCH-QUE-SI-SE-APLICA" } }
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops_sanas, policy_permisiva()).as_str()],
        1,
    );
    assert!(
        plan[0]["result"]["isError"].as_bool() != Some(true),
        "el MISMO patch sobre un documento sano debe planificar sin error (si no, el rechazo de \
         arriba no probaría nada sobre el frontmatter ilegible): {plan:?}"
    );
    let id = plan_change_set_id(&plan[0]);
    let aplicado = roundtrip(dir.path(), &[change_apply_line(&id, None).as_str()], 1);
    assert_eq!(
        apply_sc(&aplicado[0])["applied"],
        serde_json::Value::Bool(true),
        "y debe poder aplicarse: {aplicado:?}"
    );
    let sano = std::fs::read_to_string(dir.path().join("sano.md")).unwrap();
    assert!(
        sano.contains("PATCH-QUE-SI-SE-APLICA"),
        "el documento sano sí recibe el patch: {sano:?}"
    );
    // Y la transacción no arrastró a los ilegibles, que ni siquiera estaban en su change set.
    for (ruta, contenido_previo) in &antes {
        assert_eq!(
            &std::fs::read(dir.path().join(ruta)).unwrap(),
            contenido_previo,
            "publicar un cambio sobre otro documento no puede reescribir «{ruta}»"
        );
    }
}

/// **E23-H09** · Criterio «códigos del catálogo sin emisor», primer caso alcanzable:
/// **Dado** `lodestar-mcp --root <ruta que no existe>`, **Cuando** se arranca, **Entonces** falla
/// con un mensaje legible por stderr y exit code 3, sin panic y sin ensuciar stdout.
///
/// El arranque es el único punto de la superficie donde «no hay workspace» puede ocurrir: dentro de
/// una sesión, la raíz ya está canonicalizada y fija (`§20.5`). Se asevera lo que el producto
/// promete de verdad —stdout es JSON-RPC **puro**, así que un fallo de arranque no puede escribir
/// nada ahí— y que no hay panic (un `unwrap` en el arranque sería un fallo de robustez visible para
/// cualquier cliente MCP, que vería el proceso morir sin explicación).
///
/// **HALLAZGO registrado por este test**: el catálogo congelado de 16 códigos tiene
/// `WORKSPACE_NOT_FOUND`, pero **ningún camino del producto lo emite** — el arranque sale por
/// `std::process::exit(3)` con texto plano, no por el envelope de error. Aquí se fija el
/// comportamiento REAL (exit 3 + mensaje que nombra la ruta), no el que el catálogo insinúa.
#[test]
fn root_inexistente_falla_legible_sin_panic() {
    let base = tempfile::tempdir().unwrap();
    let inexistente = base.path().join("no-existe").join("ni-de-lejos");
    assert!(
        !inexistente.exists(),
        "precondición: la ruta del test no debe existir"
    );

    let salida = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(&inexistente)
        .stdin(Stdio::null())
        .output()
        .expect("ejecutar lodestar-mcp");

    let stderr = String::from_utf8_lossy(&salida.stderr).into_owned();
    let stdout = String::from_utf8_lossy(&salida.stdout).into_owned();
    eprintln!(
        "[root-inexistente] exit={:?} stderr={stderr:?}",
        salida.status.code()
    );

    assert_eq!(
        salida.status.code(),
        Some(3),
        "una raíz que no existe es un fallo de runtime/IO: exit 3 (stderr: {stderr:?})"
    );
    assert!(
        !stderr.contains("panicked"),
        "el arranque no puede paniquear ante una ruta inexistente: {stderr:?}"
    );
    assert!(
        stderr.contains("lodestar-mcp"),
        "el mensaje debe identificar al programa: {stderr:?}"
    );
    assert!(
        stderr.contains("ni-de-lejos"),
        "el mensaje debe nombrar la ruta que no se pudo resolver (si no, el usuario no sabe qué \
         arreglar): {stderr:?}"
    );
    assert!(
        stdout.is_empty(),
        "stdout es JSON-RPC PURO: un fallo de arranque no puede escribir nada ahí, ni siquiera un \
         mensaje de ayuda: {stdout:?}"
    );
}

/// **E23-H09** · Criterio «códigos del catálogo sin emisor», caso `INTERNAL_IO_ERROR`:
/// **Dado** un workspace en el que `.lodestar/runtime/plans/` **no puede existir** como directorio,
/// **Cuando** se pide un `change_plan`, **Entonces** la tool responde `INTERNAL_IO_ERROR` en vez de
/// paniquear o de devolver un plan que no se podrá aplicar.
///
/// Es el único de los cuatro códigos huérfanos del catálogo (`AMBIGUOUS_REFERENCE`,
/// `RESULT_TOO_LARGE`, `RECOVERY_FAILED`, `INTERNAL_IO_ERROR`) con un camino alcanzable desde la
/// superficie: los otros tres no tienen productor en el árbol (ver el informe de la historia).
///
/// El escenario se monta plantando un **fichero** donde el motor espera un directorio, que es la
/// forma portable de provocar un fallo de I/O real (los permisos POSIX no se comportan igual en
/// Windows ni bajo root). Modela un caso de campo: un `.lodestar/` corrupto o un volumen que
/// rechaza la escritura.
///
/// De paso fija dos propiedades de robustez: abrir el workspace **no** aborta por esto (desde
/// E23-H12 la apertura ni siquiera mira el runtime — el scaffold se retiró y cada consumidor crea su
/// directorio al escribir), y la lectura sigue funcionando pese al runtime roto.
#[test]
fn plan_con_runtime_no_escribible_da_internal_io_error() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "nota.md", "# Nota\n\ncuerpo.\n");

    // Un FICHERO donde `persist_plan` necesita un directorio: `create_dir_all` fallará.
    std::fs::create_dir_all(dir.path().join(".lodestar/runtime")).unwrap();
    std::fs::write(
        dir.path().join(".lodestar/runtime/plans"),
        b"no soy un directorio\n",
    )
    .unwrap();

    let ops = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "nota.md" }, "patch": { "estado": "x" } }
    ]);
    let resp = roundtrip(
        dir.path(),
        &[
            change_plan_line(None, ops, policy_permisiva()).as_str(),
            // El servidor sigue vivo y sirviendo lecturas pese al runtime roto.
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"nota.md"}}}}"#,
        ],
        2,
    );

    let codigo = texto_de_error(&resp[0]);
    assert!(
        codigo.contains("INTERNAL_IO_ERROR"),
        "un fallo de I/O al persistir el plan debe reportarse como INTERNAL_IO_ERROR (fallo del \
         motor/entorno, no del agente), no como «{codigo}»: {resp:?}"
    );

    // No se corrompió nada ni se abortó el proceso: la lectura posterior responde con normalidad.
    assert!(
        resp[1]["result"]["isError"].as_bool() != Some(true),
        "el servidor debe seguir sirviendo lecturas con el runtime roto: {resp:?}"
    );
    assert_eq!(
        resp[1]["result"]["structuredContent"]["document"]["path"], "nota.md",
        "y devolver el documento pedido: {resp:?}"
    );

    // El obstáculo sigue siendo un fichero: el motor no lo ha borrado por su cuenta para hacerse
    // sitio (eso sería destruir algo del usuario para poder escribir su scratch).
    assert!(
        dir.path().join(".lodestar/runtime/plans").is_file(),
        "el motor no debe borrar lo que encuentra en su ruta de runtime"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("nota.md")).unwrap(),
        "# Nota\n\ncuerpo.\n",
        "planificar no escribe el canónico, ni siquiera cuando falla"
    );
}

// ===========================================================================
// E23-H12 — Higiene de efectos secundarios y retirada de las claves privilegiadas
// (`requirements/epica-23-cierre-migracion.md §E23-H12`). Fase ROJA.
//
// DOS DEFECTOS, UNA HISTORIA:
//
// 1. `Workspace::open` (`crates/lodestar-workspace/src/lib.rs:83-84`) ejecuta
//    `gitignore::ensure_gitignore(root)` + `runtime::ensure_runtime_scaffold(root)` ANTES de leer
//    nada, así que **arrancar el MCP —incluso en perfil `readonly`— reescribe el `.gitignore` del
//    proyecto** y le crea `.lodestar/runtime/{plans,receipts,staging}`. Para el pitch «cd
//    my-project && lodestar-mcp sobre cualquier proyecto» es una escritura no solicitada; en CI,
//    un working tree sucio. Los dos efectos pasan a ser perezosos: el scaffold se borra sin
//    sustituto (sus ocho consumidores ya hacen su `create_dir_all`) y el `.gitignore` se ajusta en
//    los cuatro chokepoints de escritura (`enable_cache`, `acquire_lock`, `persist_plan`,
//    `try_append_audit`).
//
// 2. `implemented_by`/`verified_by` son las últimas claves de frontmatter con **semántica impuesta
//    y no configurable** (`crates/lodestar-workspace/src/external_refs.rs:25`), contra el
//    invariante 3 de `§20.2`. Se retiran sin sustituto (decisión del usuario, 2026-07-26) y con
//    ellas la opción `include:["externalReferences"]` de `knowledge_get`, que se quedaría sin
//    fuente: una opción que siempre devuelve vacío es el patrón que E23 está saldando.
// ===========================================================================

/// Snapshot determinista del árbol bajo `dir`: `(ruta relativa, bytes)` ordenado. Captura contenido
/// Y existencia, así que detecta lo mismo un fichero modificado que uno creado o borrado.
///
/// Solo recoge FICHEROS: los subdirectorios vacíos del scaffold de runtime no dejan rastro aquí y
/// hay que aseverarlos aparte (por eso los tests preguntan además por `.lodestar/`).
fn snapshot_arbol(dir: &std::path::Path) -> Vec<(String, Vec<u8>)> {
    fn recorrer(
        base: &std::path::Path,
        actual: &std::path::Path,
        acc: &mut Vec<(String, Vec<u8>)>,
    ) {
        let mut entradas: Vec<std::path::PathBuf> = std::fs::read_dir(actual)
            .unwrap()
            .map(|e| e.unwrap().path())
            .collect();
        entradas.sort();
        for p in entradas {
            if p.is_dir() {
                recorrer(base, &p, acc);
            } else {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().into_owned();
                acc.push((rel, std::fs::read(&p).unwrap()));
            }
        }
    }
    let mut acc = Vec::new();
    recorrer(dir, dir, &mut acc);
    acc.sort();
    acc
}

/// Una línea `tools/call` serializada con `serde_json` (nunca interpolada: los argumentos llevan
/// rutas y texto libre).
fn linea_call(id: u32, tool: &str, args: serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0",
        "id": id,
        "method": "tools/call",
        "params": { "name": tool, "arguments": args }
    })
    .to_string()
}

/// **E23-H12** · Criterio `readonly_no_escribe_nada`: **Dado** un proyecto con un `.gitignore`
/// propio, **Cuando** se arranca el MCP en perfil `readonly` y se hace una sesión de solo lectura,
/// **Entonces** el proyecto queda intacto: ni el `.gitignore` cambia ni aparece `.lodestar/`.
///
/// El criterio de la historia lo enuncia como «`git status --porcelain` sale vacío»; aquí se asevera
/// la propiedad que hay debajo, y de forma más estricta que `git status`: el árbol ENTERO byte a
/// byte (`git status` no vería un fichero ya ignorado) más la no-existencia de `.lodestar/`, que es
/// donde caería el scaffold de runtime (directorios vacíos que un snapshot de ficheros no ve).
///
/// El `.gitignore` lleva CRLF y línea en blanco final a propósito: son los dos detalles que la
/// primera reescritura de `ensure_gitignore` normaliza, o sea que el fichero volvería con otros
/// bytes conservando todas sus reglas. Comparar contenido lógico no vería el defecto.
///
/// NO-VACUIDAD: la sesión tiene que haber SERVIDO de verdad (`workspace_status` con su
/// `workspaceRevision`, `knowledge_search` encontrando el documento y `knowledge_check` con
/// veredicto), no simplemente arrancar y morir; un servidor que rechazara todo también dejaría el
/// árbol intacto.
#[test]
fn readonly_no_escribe_nada() {
    let dir = tempfile::tempdir().unwrap();
    let gitignore_original: &[u8] = b"target/\r\n*.log\r\n\r\n";
    std::fs::write(dir.path().join(".gitignore"), gitignore_original).unwrap();
    write(
        dir.path(),
        "guia.md",
        "---\nestado: vigente\n---\n\n# Guía\n\nVer [alfa](notas/alfa.md).\n",
    );
    write(dir.path(), "notas/alfa.md", "# Alfa\n\nCuerpo de alfa.\n");

    let antes = snapshot_arbol(dir.path());

    // Sesión de SOLO LECTURA: las 7 tools que el perfil `readonly` sigue exponiendo.
    let lineas = [
        r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#.to_string(),
        r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#.to_string(),
        linea_call(3, "workspace_status", serde_json::json!({})),
        linea_call(4, "knowledge_search", serde_json::json!({ "text": "alfa" })),
        linea_call(
            5,
            "knowledge_get",
            serde_json::json!({
                "ref": { "path": "guia.md" },
                "include": ["frontmatter", "body", "outgoingLinks", "backlinks", "diagnostics"]
            }),
        ),
        linea_call(6, "metadata_inspect", serde_json::json!({ "mode": "catalog" })),
        linea_call(
            7,
            "knowledge_check",
            serde_json::json!({ "scope": { "kind": "workspace" } }),
        ),
        linea_call(
            8,
            "graph_query",
            serde_json::json!({ "operation": "isolated" }),
        ),
    ];
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip_profile(dir.path(), "readonly", &refs, lineas.len());

    // --- guarda anti-vacua: la sesión de lectura funcionó de verdad -----------------------------
    assert_eq!(
        resp.len(),
        lineas.len(),
        "el servidor debe responder a las {} peticiones de la sesión: {resp:?}",
        lineas.len()
    );
    assert!(
        resp[2]["result"]["structuredContent"]["workspaceRevision"]
            .as_str()
            .unwrap_or("")
            .starts_with("blake3:"),
        "workspace_status debe haber servido el estado del workspace: {resp:?}"
    );
    assert!(
        search_paths(&resp[3]).contains(&"notas/alfa.md".to_string()),
        "knowledge_search debe haber encontrado `notas/alfa.md`: {resp:?}"
    );
    assert_eq!(
        resp[4]["result"]["structuredContent"]["document"]["path"], "guia.md",
        "knowledge_get debe haber servido el documento: {resp:?}"
    );
    assert_eq!(
        resp[6]["result"]["structuredContent"]["valid"],
        serde_json::Value::Bool(true),
        "knowledge_check debe haber emitido veredicto sobre un workspace válido: {resp:?}"
    );

    // --- el criterio: cero escrituras -----------------------------------------------------------
    let gitignore_tras_sesion = std::fs::read(dir.path().join(".gitignore")).unwrap();
    assert_eq!(
        gitignore_tras_sesion,
        gitignore_original,
        "una sesión `readonly` no puede tocar el `.gitignore` del proyecto: era {:?} y quedó {:?}",
        String::from_utf8_lossy(gitignore_original),
        String::from_utf8_lossy(&gitignore_tras_sesion)
    );
    assert!(
        !dir.path().join(".lodestar").exists(),
        "una sesión `readonly` no puede hacer aparecer `.lodestar/` (ni siquiera el scaffold vacío \
         de runtime): sobre el proyecto de un tercero es una escritura no solicitada"
    );
    assert_eq!(
        snapshot_arbol(dir.path()),
        antes,
        "una sesión `readonly` no debe modificar, crear ni borrar NINGÚN fichero del proyecto"
    );
}

/// El `document` de una respuesta `knowledge_get`, normalizado para comparar dos documentos gemelos
/// que solo se diferencian en el NOMBRE de una clave de frontmatter: se quita `revision` (blake3 del
/// contenido — difiere por fuerza, porque el nombre de la clave forma parte de los bytes) y se
/// sustituyen la ruta y la clave por marcadores.
fn documento_normalizado(resp: &serde_json::Value, ruta: &str, clave: &str) -> String {
    let mut doc = resp["result"]["structuredContent"]["document"].clone();
    assert!(
        doc.is_object(),
        "knowledge_get debe devolver `document` como objeto: {resp:?}"
    );
    if let Some(obj) = doc.as_object_mut() {
        obj.remove("revision");
    }
    doc.to_string().replace(ruta, "DOC").replace(clave, "CLAVE")
}

/// **E23-H12** · Criterio `claves_de_frontmatter_sin_semantica_impuesta`: **Dado** un documento con
/// `implemented_by: María`, **Cuando** se audita el workspace, **Entonces** no se emite ningún
/// diagnóstico — ningún nombre de campo tiene semántica impuesta.
///
/// SE EJERCE POR `knowledge_check` scope `workspace`, que es el mismo motor que `lodestar check`
/// (invariante #3: desde E23-H01 ambos salen de `App::full_analysis`), y por la superficie donde el
/// privilegio es OBSERVABLE, que es `knowledge_get`.
///
/// POR QUÉ NO BASTA LA MITAD LITERAL: el diagnóstico `EXTREF-MISSING` ya murió en E20-H03, así que
/// «`check` no dice nada de `implemented_by`» es hoy trivialmente cierto y un test que solo aseverase
/// eso sería VACUO. Lo que sigue vivo —y lo que esta historia mata— es que Lodestar interpreta el
/// valor de esas dos claves como una **ruta de fichero** y la resuelve contra disco: con
/// `implemented_by: María` (un nombre de persona), `knowledge_get(include:[externalReferences])`
/// devuelve hoy `[{path:"María", exists:false}]`, o sea que trata a María como un fichero de código
/// que falta.
///
/// FORMA DEL TEST — DIFERENCIAL: dos documentos gemelos, idénticos salvo el NOMBRE de la clave
/// (`implemented_by` vs `autor_favorito`). «Ningún nombre de campo tiene semántica impuesta»
/// significa exactamente que sus proyecciones son indistinguibles módulo ese nombre. Hoy difieren
/// (uno trae `externalReferences` poblado, el otro vacío) ⇒ ROJO. La formulación es robusta frente a
/// cómo se implemente la retirada: da igual que la opción `externalReferences` pase a rechazarse o a
/// no existir, mientras los gemelos se traten igual.
#[test]
fn claves_de_frontmatter_sin_semantica_impuesta() {
    let dir = tempfile::tempdir().unwrap();
    // `referenceRoots` configurado: el escenario en el que la maquinaria de refs externas está más
    // «viva» posible, para que su retirada no pueda pasar por casualidad.
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "workspace:\n  referenceRoots: [src]\n",
    );
    write(dir.path(), "src/lib.rs", "// código del proyecto\n");
    write(
        dir.path(),
        "docs/con-clave-privilegiada.md",
        "---\nimplemented_by: María\n---\n\n# Ficha\n\ncuerpo.\n",
    );
    write(
        dir.path(),
        "docs/con-clave-cualquiera.md",
        "---\nautor_favorito: María\n---\n\n# Ficha\n\ncuerpo.\n",
    );

    let include = serde_json::json!([
        "frontmatter",
        "body",
        "outgoingLinks",
        "backlinks",
        "diagnostics",
        "externalReferences"
    ]);
    let lineas = [
        linea_call(
            1,
            "knowledge_check",
            serde_json::json!({ "scope": { "kind": "workspace" } }),
        ),
        linea_call(
            2,
            "knowledge_get",
            serde_json::json!({ "ref": { "path": "docs/con-clave-privilegiada.md" }, "include": include }),
        ),
        linea_call(
            3,
            "knowledge_get",
            serde_json::json!({ "ref": { "path": "docs/con-clave-cualquiera.md" }, "include": include }),
        ),
    ];
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip(dir.path(), &refs, lineas.len());

    // --- mitad literal del criterio: auditar no emite NINGÚN diagnóstico ------------------------
    let diags = check_diagnostics(&resp[0]);
    assert!(
        diags.is_empty(),
        "un documento con `implemented_by: María` (un nombre de persona, no una ruta) no puede \
         producir diagnóstico alguno: ningún nombre de campo tiene semántica impuesta. \
         Diagnósticos: {diags:?}"
    );

    // --- la mitad que muerde: los gemelos son indistinguibles módulo el nombre de la clave ------
    let privilegiada =
        documento_normalizado(&resp[1], "docs/con-clave-privilegiada.md", "implemented_by");
    let cualquiera =
        documento_normalizado(&resp[2], "docs/con-clave-cualquiera.md", "autor_favorito");
    assert_eq!(
        privilegiada, cualquiera,
        "`implemented_by` debe ser metadata del usuario como `autor_favorito`: sus proyecciones \
         tienen que ser idénticas módulo el nombre de la clave. Hoy no lo son porque Lodestar \
         interpreta el valor de `implemented_by` como una ruta y lo resuelve contra disco"
    );

    // Guarda anti-vacua: la comparación de arriba solo significa algo si el frontmatter del usuario
    // viajó de verdad (dos `document` vacíos también serían iguales).
    assert_eq!(
        resp[1]["result"]["structuredContent"]["document"]["frontmatter"]["implemented_by"],
        serde_json::Value::String("María".to_string()),
        "el frontmatter del usuario debe viajar VERBATIM, con su clave y su valor: {resp:?}"
    );
}

/// **E23-H12** · Criterio `external_references_retirada_del_wire`: **Dado** un `knowledge_get`,
/// **Entonces** `externalReferences` no está en el enum de `include` ni en `contracts/mcp.yml`.
///
/// Se aseveran las TRES caras de la retirada, porque cada una se puede incumplir por separado:
///   1. el **schema declarado** a los clientes (`tools/list` → `inputSchema` de `knowledge_get`),
///      que es lo que un agente lee para saber qué puede pedir;
///   2. el **contrato** `contracts/mcp.yml`, la spec de la frontera. Se compara contra el YAML
///      **parseado** (sección `tools:`), no contra el texto crudo: los comentarios `#` del fichero
///      son memoria histórica de la migración y documentar ahí la retirada debe seguir siendo
///      legítimo — lo que no puede sobrevivir es la superficie declarada;
///   3. el **comportamiento**: pedir el campo retirado no puede resucitarlo en la respuesta. Sin
///      esto, quitarlo del enum y dejar el código vivo pasaría por «hecho» y el wire seguiría
///      devolviendo un campo indocumentado.
#[test]
fn external_references_retirada_del_wire() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "workspace:\n  referenceRoots: [src]\n",
    );
    write(dir.path(), "src/existe.rs", "fn main() {}\n");
    write(
        dir.path(),
        "tarea.md",
        "---\nimplemented_by:\n  - src/existe.rs\n---\n\n# Tarea\n\ncuerpo.\n",
    );

    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#,
            &linea_call(
                2,
                "knowledge_get",
                serde_json::json!({
                    "ref": { "path": "tarea.md" },
                    "include": ["frontmatter", "externalReferences"]
                }),
            ),
        ],
        2,
    );

    // --- 1. el enum de `include` que se le declara al cliente ------------------------------------
    let tools = resp[0]["result"]["tools"]
        .as_array()
        .expect("tools/list devuelve un array");
    let get = tools
        .iter()
        .find(|t| t["name"] == "knowledge_get")
        .unwrap_or_else(|| panic!("`knowledge_get` debe seguir en el catálogo: {tools:?}"));
    let valores: Vec<String> = get["inputSchema"]["properties"]["include"]["items"]["enum"]
        .as_array()
        .unwrap_or_else(|| panic!("`include` debe declarar su enum de valores: {get}"))
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    assert!(
        !valores.iter().any(|v| v == "externalReferences"),
        "`externalReferences` no puede seguir en el enum de `include` de `knowledge_get`: sin \
         `implemented_by`/`verified_by` no tiene fuente, y una opción que siempre devolvería vacío \
         es el patrón que E23 salda. Enum: {valores:?}"
    );
    // Guarda anti-vacua: el enum sigue existiendo y con sus valores vivos.
    for vivo in [
        "frontmatter",
        "body",
        "outgoingLinks",
        "backlinks",
        "diagnostics",
    ] {
        assert!(
            valores.iter().any(|v| v == vivo),
            "el enum de `include` debe conservar «{vivo}»: {valores:?}"
        );
    }

    // --- 2. el contrato de la frontera -----------------------------------------------------------
    let contrato = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/mcp.yml");
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(&contrato)
            .unwrap_or_else(|e| panic!("no se pudo leer {}: {e}", contrato.display())),
    )
    .expect("`contracts/mcp.yml` debe ser YAML válido");
    let superficie = serde_yaml::to_string(&yaml["tools"]).unwrap();
    assert!(
        !superficie.contains("externalReferences"),
        "la sección `tools:` de `contracts/mcp.yml` no puede seguir declarando `externalReferences` \
         en `knowledge_get`"
    );
    // Guarda anti-vacua: se está mirando la sección correcta y con contenido.
    assert!(
        superficie.contains("knowledge_get") && superficie.contains("outgoingLinks"),
        "el test debe estar leyendo la superficie real de `contracts/mcp.yml`"
    );

    // --- 3. el comportamiento: pedirlo no lo resucita --------------------------------------------
    let documento = &resp[1]["result"]["structuredContent"]["document"];
    assert!(
        documento.get("externalReferences").is_none(),
        "pedir el campo retirado no puede hacer que reaparezca en la respuesta: {documento}"
    );
    assert!(
        documento["frontmatter"]["implemented_by"].is_array(),
        "guarda anti-vacua: el documento se sirvió y su frontmatter viajó tal cual (`implemented_by` \
         es hoy una lista más de metadata del usuario): {documento}"
    );
}

/// **E23-H12** · Regresión de SEGURIDAD migrada desde `ref_externa_traversal`
/// (`crates/lodestar-workspace/tests/reference_roots.rs`, hallada por un juez ciego): `knowledge_get`
/// **no puede ser un oráculo de existencia de ficheros arbitrarios del host**.
///
/// El vector original era `implemented_by: [/etc/hosts]` / `verified_by: [../secreto.txt]`: si
/// Lodestar resuelve contra disco una cadena cruda del frontmatter con un `join` ingenuo, un agente
/// puede preguntar por cualquier ruta del sistema y leer la respuesta `exists:true/false`. E23-H12
/// retira la resolución entera, así que el contrato se ENDURECE: antes se prohibía `exists:true`,
/// ahora se prohíbe **cualquier** resolución.
///
/// El escenario es determinista con independencia del entorno: el workspace vive en un
/// subdirectorio y el `secreto.txt` está en su PADRE, fuera de él (con un `root.join` crudo,
/// `../secreto.txt` alcanza un fichero REAL).
///
/// Las claves siguen viajando como metadata —`frontmatter` las ecoa verbatim, que es justo el
/// punto: son datos del usuario, no instrucciones para el motor—; lo que no puede viajar es su
/// resolución.
#[test]
fn frontmatter_no_es_oraculo_de_ficheros_del_host() {
    let base = tempfile::tempdir().unwrap();
    let root = base.path().join("workspace");
    std::fs::create_dir_all(&root).unwrap();
    // El "secreto" vive FUERA del workspace, en el directorio padre.
    std::fs::write(base.path().join("secreto.txt"), "datos sensibles\n").unwrap();

    write(
        &root,
        ".lodestar/config.yaml",
        "workspace:\n  referenceRoots: [src]\n",
    );
    write(
        &root,
        "ficha.md",
        "---\nimplemented_by:\n  - /etc/hosts\nverified_by:\n  - ../secreto.txt\n---\n\n# Ficha\n\ncuerpo.\n",
    );

    let resp = roundtrip(
        &root,
        &[&linea_call(
            1,
            "knowledge_get",
            serde_json::json!({
                "ref": { "path": "ficha.md" },
                "include": ["frontmatter", "body", "diagnostics", "externalReferences"]
            }),
        )],
        1,
    );

    let documento = &resp[0]["result"]["structuredContent"]["document"];
    let texto = documento.to_string();
    assert!(
        documento.get("externalReferences").is_none(),
        "ninguna resolución de rutas del frontmatter puede viajar en la respuesta: {documento}"
    );
    assert!(
        !texto.contains("exists"),
        "la respuesta no puede llevar veredictos de existencia de ficheros del host (oráculo, \
         invariante #6): {documento}"
    );
    assert!(
        !texto.contains("datos sensibles"),
        "jamás puede viajar el CONTENIDO de un fichero de fuera del workspace: {documento}"
    );
    // Las claves son metadata del usuario y viajan como tal (guarda anti-vacua: el documento se
    // sirvió de verdad).
    assert_eq!(
        documento["frontmatter"]["verified_by"][0],
        serde_json::Value::String("../secreto.txt".to_string()),
        "el frontmatter se ecoa verbatim: son datos del usuario, no instrucciones para el motor: \
         {documento}"
    );
}

/// **E23-H12** · Guarda del OTRO lado del criterio: hacer perezoso el ajuste del `.gitignore` no es
/// lo mismo que retirarlo. **Dado** un proyecto cuyo `.gitignore` no menciona a lodestar, **Cuando**
/// se ejerce un camino de ESCRITURA (`change_plan` → `change_apply`), **Entonces** el bloque
/// gestionado aparece y el contenido propio del usuario se conserva.
///
/// Sin esto, «que abrir no escriba» se podría satisfacer borrando `ensure_gitignore`, y el proyecto
/// acabaría versionando `.lodestar/index.db` (una base SQLite derivada) y `.lodestar/runtime/`
/// (planes, recibos y staging), que es el problema que aquel ajuste resolvía.
///
/// ## Cada chokepoint por separado (corrección de un hallazgo de juez ciego)
///
/// La versión anterior de este test comprobaba el `.gitignore` tras `change_plan` y **luego** tras
/// `change_apply`, sin restaurarlo entre medias: cuando llegaba a la segunda comprobación el bloque
/// ya estaba puesto por `persist_plan`, así que la segunda no podía distinguir nada (borrar el
/// ajuste de `acquire_lock` o el de `try_append_audit` dejaba la suite entera en verde). Ahora el
/// `.gitignore` se **restaura a su estado original entre fase y fase**, de modo que cada fase solo
/// puede pasar si el chokepoint que ejerce hace el ajuste por sí mismo:
///
///   1. `change_plan` → `persist_plan`, que persiste el plan bajo `.lodestar/runtime/plans/` **sin**
///      tomar el lock.
///   2. `change_apply` con un `changeSetId` INEXISTENTE → falla en el paso 1 (plan no encontrado),
///      o sea que no llega ni a `acquire_lock` ni a `persist_plan`… pero **audita igual** (cada
///      intento, con éxito o sin él, anexa a `.lodestar/runtime/audit.jsonl`). Es el único camino de
///      la superficie que ejerce `try_append_audit` en solitario.
///   3. `change_apply` real: el camino completo end-to-end (lock + publicación + auditoría), que es
///      la propiedad que ve el usuario. Esta fase **no discrimina** entre `acquire_lock` y
///      `try_append_audit` —el segundo corre siempre al final del primero— y no pretende hacerlo:
///      `acquire_lock` se aísla donde sí se puede, en `workspace.rs::lock_ajusta_el_gitignore`
///      (crate sin auditoría). `enable_cache` lo cubren `gitignore_parte_lodestar` y
///      `adopcion_ajusta_gitignore`.
///
/// NO es fase roja: es regresión sobre la implementación de esta historia.
#[test]
fn escribir_si_ajusta_el_gitignore() {
    let dir = tempfile::tempdir().unwrap();
    /// El `.gitignore` del usuario, sin rastro de lodestar: el estado de partida de cada fase.
    const GITIGNORE_USUARIO: &str = "target/\n";
    write(
        dir.path(),
        "nota.md",
        "---\nestado: borrador\n---\n\n# Nota\n",
    );

    /// Las entradas que el bloque gestionado garantiza (`workspace/src/gitignore.rs`).
    const ENTRADAS: [&str; 2] = [".lodestar/index.db", ".lodestar/runtime/"];
    // Devuelve el `.gitignore` al estado del usuario: sin esto, una fase heredaría el ajuste de la
    // anterior y pasaría sin ejercer su propio chokepoint (el defecto que halló el juez).
    let restaura = || {
        write(dir.path(), ".gitignore", GITIGNORE_USUARIO);
    };
    let comprueba = |momento: &str| {
        let gi = std::fs::read_to_string(dir.path().join(".gitignore"))
            .unwrap_or_else(|e| panic!("{momento}: el `.gitignore` debe existir: {e}"));
        assert_ne!(
            gi, GITIGNORE_USUARIO,
            "{momento}: el `.gitignore` sigue exactamente como lo dejó el usuario, así que este \
             camino de escritura NO ajustó nada"
        );
        for entrada in ENTRADAS {
            assert!(
                gi.lines().any(|l| l.trim() == entrada),
                "{momento}: el `.gitignore` debe ignorar «{entrada}» (la cache y el runtime son \
                 derivados y desechables: versionarlos es el defecto que este ajuste evita). \
                 Era:\n{gi}"
            );
        }
        assert!(
            gi.lines().any(|l| l.trim() == "target/"),
            "{momento}: el ajuste debe preservar el `.gitignore` propio del usuario. Era:\n{gi}"
        );
        assert!(
            !gi.lines().any(|l| l.trim() == ".lodestar/config.yaml"),
            "{momento}: la config canónica NO se ignora (va versionada). Era:\n{gi}"
        );
    };

    // (1) `persist_plan` aislado: `change_plan` persiste el plan sin tomar el lock ni auditar.
    restaura();
    let ops = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "nota.md" }, "patch": { "estado": "vigente" } }
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );
    let id = plan_change_set_id(&plan[0]);
    comprueba("tras change_plan (persist_plan)");

    // (2) `try_append_audit` aislado: un apply de un plan que NO existe aborta antes de tocar el
    // lock, pero el intento se audita igual — y auditar hace nacer runtime desechable.
    restaura();
    let fallido = roundtrip(
        dir.path(),
        &[change_apply_line("changeset:no-existe", None).as_str()],
        1,
    );
    assert_eq!(
        fallido[0]["result"]["isError"],
        serde_json::Value::Bool(true),
        "guarda: el apply de un plan inexistente debe FALLAR (si se ejecutara, la fase dejaría de \
         aislar la auditoría): {fallido:?}"
    );
    assert!(
        dir.path().join(".lodestar/runtime/audit.jsonl").is_file(),
        "guarda: el intento fallido tiene que haber auditado de verdad; si no hay `audit.jsonl`, \
         esta fase no está ejerciendo `try_append_audit`"
    );
    comprueba("tras un change_apply fallido (try_append_audit)");

    // (3) El camino completo, tal y como lo vive el usuario: lock + publicación + auditoría.
    restaura();
    let applied = roundtrip(dir.path(), &[change_apply_line(&id, None).as_str()], 1);
    assert_eq!(
        apply_sc(&applied[0])["applied"],
        serde_json::Value::Bool(true),
        "guarda de no vacuidad: el apply tiene que haberse ejecutado de verdad: {applied:?}"
    );
    comprueba("tras change_apply");
}

// ---------------------------------------------------------------------------
// E24-H01 — Un BOM deja de tragarse el frontmatter (por el WIRE).
// `requirements/epica-24-cierre-defectos-v031.md §E24-H01` · `ARCHITECTURE.md §20.4` ·
// `CLAUDE.md` invariante #1 («los `.md` en disco son la única fuente de verdad»). Fase ROJA.
//
// Estos tres tests son la reproducción EXACTA del síntoma de la historia, ejercida sobre el
// binario real por JSON-RPC (el arnés `roundtrip`), porque es así como se descubrió: el fichero
// con BOM se escribe en disco, y lo que se juzga son los BYTES que quedan en disco después.
// La misma semántica, en el núcleo puro, la fijan los tests homónimos de
// `crates/lodestar-core/tests/documento.rs`.
//
// SÍNTOMA verificado hoy contra `lodestar-mcp` (v0.3.0):
//   knowledge_get  -> frontmatter {} · body = el fichero ENTERO (BOM y bloque incluidos)
//   where "document.has_frontmatter = true"  -> []      (y `= false` -> ['bom.md'])
//   patch_frontmatter {"status":"review"} + apply deja en disco
//     b'---\nstatus: review\n---\n\n\xef\xbb\xbf---\nstatus: draft\nowner: ana\n---\n\n# Con BOM…'
//   Dos bloques; `owner: ana` degradado a texto del cuerpo, listo para que el siguiente
//   `replace_body` lo borre para siempre.
//
// ROJO esperado HOY: por ASERCIÓN en los tres (ninguna API nueva, ningún stub).
// ---------------------------------------------------------------------------

/// El documento del síntoma, byte a byte: BOM UTF-8 (`EF BB BF`) + frontmatter de **dos** claves.
/// `owner` es la clave testigo: es la que la corrupción de hoy destruye.
const DOC_CON_BOM: &str =
    "\u{feff}---\nstatus: draft\nowner: ana\n---\n\n# Con BOM\n\ncuerpo original\n";

/// Workspace con el documento del síntoma **y** un gemelo sin BOM. El gemelo es el control de no
/// vacuidad de las consultas: `document.has_frontmatter = true` ya lo devuelve hoy, así que si un
/// arreglo rompiera el caso normal, estos tests lo verían.
fn workspace_con_bom() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "bom.md", DOC_CON_BOM);
    write(
        dir.path(),
        "sin_bom.md",
        "---\nstatus: draft\nowner: ana\n---\n\n# Sin BOM\n\ncuerpo original\n",
    );
    dir
}

/// Los BYTES de un `.md` del workspace. Se leen como bytes —y no como `String`— porque el BOM es
/// precisamente lo que se juzga y una lectura descuidada lo escondería.
fn bytes_de(root: &std::path::Path, rel: &str) -> Vec<u8> {
    std::fs::read(root.join(rel)).unwrap_or_else(|e| panic!("`{rel}` debe existir en disco: {e}"))
}

/// Rendición legible de unos bytes para los mensajes de aserción (el BOM se ve como `\u{feff}`).
fn como_texto(bytes: &[u8]) -> String {
    format!("{:?}", String::from_utf8_lossy(bytes))
}

/// Número de líneas delimitadoras de frontmatter (`---`) de un `.md`, tolerando el BOM en la
/// primera. Un documento con **un** bloque tiene exactamente dos; la corrupción produce cuatro.
fn delimitadores(bytes: &[u8]) -> usize {
    String::from_utf8_lossy(bytes)
        .lines()
        .filter(|l| l.trim_start_matches('\u{feff}') == "---")
        .count()
}

/// La `workspaceRevision` que reporta `workspace_status` sobre el workspace de `root`.
fn revision_de(root: &std::path::Path) -> String {
    let status = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"workspace_status","arguments":{}}}"#;
    let resp = roundtrip(root, &[status], 1);
    resp[0]["result"]["structuredContent"]["workspaceRevision"]
        .as_str()
        .unwrap_or_else(|| panic!("workspace_status debe devolver `workspaceRevision`: {resp:?}"))
        .to_string()
}

/// Planifica **una** operación y la aplica, exigiendo que el apply tenga éxito (guarda de no
/// vacuidad: si la operación no llega a ejecutarse, el estado de disco no prueba nada).
fn planifica_y_aplica(root: &std::path::Path, op: serde_json::Value) -> serde_json::Value {
    let plan = roundtrip(
        root,
        &[change_plan_line(None, serde_json::json!([op]), policy_permisiva()).as_str()],
        1,
    );
    let id = plan_change_set_id(&plan[0]);
    let apply = roundtrip(root, &[change_apply_line(&id, None).as_str()], 1);
    assert_eq!(
        apply_sc(&apply[0])["applied"],
        serde_json::Value::Bool(true),
        "guarda: la operación debe aplicarse de verdad para que el criterio no sea vacuo: {apply:?}"
    );
    apply_sc(&apply[0]).clone()
}

/// E24-H01 · Criterio `bom_no_se_traga_el_frontmatter`:
/// Dado un `.md` con BOM y frontmatter válido, Cuando se lee con `knowledge_get`, Entonces
/// `frontmatter` trae las claves reales y `document.has_frontmatter` es `true`.
#[test]
fn bom_no_se_traga_el_frontmatter() {
    let dir = workspace_con_bom();
    // Guarda del fixture: el fichero que se acaba de escribir empieza de verdad por `EF BB BF`.
    assert_eq!(
        bytes_de(dir.path(), "bom.md").get(..3),
        Some([0xEF, 0xBB, 0xBF].as_slice()),
        "guarda del fixture: `bom.md` debe empezar por el BOM UTF-8"
    );

    let get = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"bom.md"},"include":["frontmatter","body"]}}}"#;
    let con_fm = ks_call(serde_json::json!({ "where": "document.has_frontmatter = true" }));
    let sin_fm = ks_call(serde_json::json!({ "where": "document.has_frontmatter = false" }));
    let resp = roundtrip(dir.path(), &[get, con_fm.as_str(), sin_fm.as_str()], 3);

    // (a) El frontmatter que viaja son las CLAVES REALES, no el mapa vacío.
    let doc = &resp[0]["result"]["structuredContent"]["document"];
    assert_eq!(
        doc["frontmatter"]["status"],
        serde_json::json!("draft"),
        "`knowledge_get` sobre un `.md` con BOM debe devolver su `status` real; hoy devuelve el \
         frontmatter vacío y la metadata del usuario es invisible: {resp:?}"
    );
    assert_eq!(
        doc["frontmatter"]["owner"],
        serde_json::json!("ana"),
        "…y su `owner`, que es la clave que la corrupción de esta historia destruye: {resp:?}"
    );

    // (b) El `body` es el CUERPO, no el fichero entero con su bloque dentro.
    let body = doc["body"].as_str().unwrap_or_else(|| {
        panic!("`knowledge_get` con include=[body] debe traer `body`: {resp:?}")
    });
    assert!(
        !body.contains("status: draft"),
        "el bloque de frontmatter NO puede viajar dentro del `body`: body = {body:?}"
    );
    assert!(
        !body.starts_with('\u{feff}'),
        "el BOM pertenece a la cabecera del fichero, no al cuerpo: body = {body:?}"
    );
    assert_eq!(
        body, "\n# Con BOM\n\ncuerpo original\n",
        "el cuerpo debe ser exactamente el que sigue al bloque, igual que en un `.md` sin BOM"
    );

    // (c) `document.has_frontmatter` es `true` para el documento con BOM.
    let con: std::collections::BTreeSet<String> = search_paths(&resp[1]).into_iter().collect();
    assert!(
        con.contains("sin_bom.md"),
        "guarda de no vacuidad: el gemelo SIN BOM ya casa `document.has_frontmatter = true` en \
         v0.3.0 y debe seguir casando: {resp:?}"
    );
    assert!(
        con.contains("bom.md"),
        "`document.has_frontmatter` debe ser `true` para el `.md` con BOM: su bloque está \
         presente y cerrado. Casaron {con:?}: {resp:?}"
    );

    // (d) …y por tanto NO casa la consulta complementaria (control anti-vacuo de la anterior).
    let sin: std::collections::BTreeSet<String> = search_paths(&resp[2]).into_iter().collect();
    assert!(
        !sin.contains("bom.md"),
        "`bom.md` no puede aparecer como documento SIN frontmatter: casaron {sin:?}: {resp:?}"
    );
}

/// E24-H01 · Criterio `patch_sobre_bom_no_duplica_bloque`:
/// Dado ese documento, Cuando se le aplica `patch_frontmatter`, Entonces el resultado tiene un
/// solo bloque, conserva las claves que no se tocaron y empieza por el BOM.
///
/// Se juzga sobre los BYTES publicados en disco (invariante #1), no sobre la respuesta de la tool.
#[test]
fn patch_sobre_bom_no_duplica_bloque() {
    let dir = workspace_con_bom();

    planifica_y_aplica(
        dir.path(),
        serde_json::json!({
            "op": "patch_frontmatter", "ref": { "path": "bom.md" },
            "patch": { "status": "review" }
        }),
    );

    let publicado = bytes_de(dir.path(), "bom.md");

    // (a) Empieza por el BOM: no se le antepone un bloque por delante.
    assert_eq!(
        publicado.get(..3),
        Some([0xEF, 0xBB, 0xBF].as_slice()),
        "el `.md` publicado debe seguir empezando por el BOM UTF-8 (EF BB BF); hoy el patch le \
         antepone un bloque nuevo. En disco quedó: {}",
        como_texto(&publicado)
    );

    // (b) UN SOLO bloque de frontmatter.
    assert_eq!(
        delimitadores(&publicado),
        2,
        "el `.md` publicado debe tener UN solo bloque (2 líneas «---»); la corrupción deja 4 y \
         degrada el bloque original a texto del cuerpo. En disco quedó: {}",
        como_texto(&publicado)
    );

    // (c) La clave no tocada sobrevive, una sola vez, y la tocada tiene el valor nuevo.
    let texto = String::from_utf8_lossy(&publicado).to_string();
    assert_eq!(
        texto.matches("owner: ana").count(),
        1,
        "`owner: ana` debe aparecer EXACTAMENTE una vez: ni borrada ni duplicada en un segundo \
         bloque muerto. En disco quedó: {}",
        como_texto(&publicado)
    );
    assert!(
        !texto.contains("status: draft"),
        "el valor viejo de `status` no puede sobrevivir en un bloque degradado a cuerpo. En disco \
         quedó: {}",
        como_texto(&publicado)
    );

    // (d) Y el motor vuelve a leer el documento entero: nada quedó fuera de su alcance, así que el
    // siguiente `replace_body` (el segundo paso del síntoma) no tiene nada que destruir.
    let get = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"bom.md"},"include":["frontmatter"]}}}"#;
    let resp = roundtrip(dir.path(), &[get], 1);
    let fm = &resp[0]["result"]["structuredContent"]["document"]["frontmatter"];
    assert_eq!(
        fm["status"],
        serde_json::json!("review"),
        "tras el patch, `status` debe valer «review»: {resp:?}"
    );
    assert_eq!(
        fm["owner"],
        serde_json::json!("ana"),
        "…y `owner: ana` debe seguir siendo metadata VIVA, no texto muerto del cuerpo: {resp:?}"
    );
}

/// E24-H01 · Criterio `bom_roundtrip_byte_a_byte` (**control anti-vacuo de la historia**):
/// Dado ese documento, Cuando se lee y se reescribe sin cambios, Entonces los bytes son idénticos
/// y la `WorkspaceRevision` no cambia.
///
/// Sin este criterio, un arreglo que se limitase a **strippear** el BOM al leer pasaría los otros
/// dos. Aquí no: `workspace_revision` hashea los bytes crudos del `FileMap`, así que strippear al
/// leer y no al reemitir declararía un cambio espurio en cada round-trip — justo lo que el alcance
/// de la historia prohíbe («el BOM NO se normaliza en la lectura de disco»).
///
/// Dos rutas, las dos que un agente usa de verdad:
///   - **A. `replace_body`** con el `body` que acaba de devolver `knowledge_get`: obliga a
///     **reemitir** el BOM al reconstruir el documento. Lleva una precondición explícita (el
///     cuerpo leído debe ser el cuerpo) sin la cual sería vacua: si el `body` fuese el fichero
///     entero —el defecto de hoy—, reescribirlo devolvería los mismos bytes por accidente.
///   - **B. `patch_frontmatter`** escribiendo en `status` el valor que **ya** tenía: obliga a
///     **conservarlo**. Es roja hoy por sí sola.
#[test]
fn bom_roundtrip_byte_a_byte() {
    let dir = workspace_con_bom();
    let antes = bytes_de(dir.path(), "bom.md");
    let revision_antes = revision_de(dir.path());

    // --- Ruta A: leer el cuerpo por el wire y volver a escribirlo tal cual.
    let get = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"bom.md"},"include":["body"]}}}"#;
    let leido = roundtrip(dir.path(), &[get], 1);
    let body = leido[0]["result"]["structuredContent"]["document"]["body"]
        .as_str()
        .unwrap_or_else(|| panic!("`knowledge_get` debe traer el `body` de `bom.md`: {leido:?}"))
        .to_string();
    assert!(
        !body.starts_with('\u{feff}') && !body.contains("status: draft"),
        "precondición del round-trip: el `body` leído debe ser el cuerpo, no el fichero entero — \
         si no, reescribirlo devolvería los mismos bytes por accidente y el criterio sería vacuo. \
         body = {body:?}"
    );

    planifica_y_aplica(
        dir.path(),
        serde_json::json!({
            "op": "replace_body", "ref": { "path": "bom.md" }, "body": body
        }),
    );

    let tras_a = bytes_de(dir.path(), "bom.md");
    assert_eq!(
        tras_a.get(..3),
        Some([0xEF, 0xBB, 0xBF].as_slice()),
        "reescribir el cuerpo debe REEMITIR el BOM: es del usuario, no ruido a normalizar. En \
         disco quedó: {}",
        como_texto(&tras_a)
    );
    assert_eq!(
        como_texto(&tras_a),
        como_texto(&antes),
        "leer y reescribir sin cambios debe dejar los MISMOS bytes en disco"
    );
    assert_eq!(
        revision_de(dir.path()),
        revision_antes,
        "un round-trip sin cambios no puede mover la `WorkspaceRevision`: declararía un cambio \
         espurio —y con él conflictos de escritura— en cada lectura-escritura de un `.md` con BOM"
    );

    // --- Ruta B: escribir en `status` el valor que ya tenía.
    planifica_y_aplica(
        dir.path(),
        serde_json::json!({
            "op": "patch_frontmatter", "ref": { "path": "bom.md" },
            "patch": { "status": "draft" }
        }),
    );

    let tras_b = bytes_de(dir.path(), "bom.md");
    assert_eq!(
        como_texto(&tras_b),
        como_texto(&antes),
        "escribir en `status` el valor que ya tenía no cambia el documento: mismos bytes, BOM \
         incluido"
    );
    assert_eq!(
        revision_de(dir.path()),
        revision_antes,
        "un patch que no cambia ningún valor no puede mover la `WorkspaceRevision`"
    );
}

// ---------------------------------------------------------------------------
// E24-H09/H10 — La superficie de error deja de mentir
//
// H09: `contracts/mcp.yml` («regla de la casa») dice que el servidor **valida los VALORES de los
// parámetros que declara**. No lo hacía: `limit: "10"`, `limit: 0` (el schema declara `minimum: 1`)
// o `includeSuggestedFixes: "true"` caían al default EN SILENCIO. El peor caso es `limit: 0`, que
// devolvía 0 resultados — indistinguible de «no hay nada».
//
// H10: 10 de los 21 errores de superficie viajaban como texto suelto, sin código del catálogo, y la
// MISMA consulta malformada daba dos códigos distintos según la tool (`INTERNAL_IO_ERROR` por
// `knowledge_search`, `INVALID_SCHEMA` por la selección de `change_plan`).
//
// Lo que NO cambia: los parámetros **no declarados** se siguen ignorando. Es la regla de la casa,
// escrita en tres sitios, y revisarla es un cambio de política, no un bugfix.
// ---------------------------------------------------------------------------

/// Workspace mínimo con dos documentos, para que un `limit` mal puesto sea observable.
fn ws_dos_docs() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.md", "---\ns: 1\n---\n\n# A\n");
    write(dir.path(), "b.md", "---\ns: 2\n---\n\n# B\n");
    dir
}

/// Texto del error de ejecución de una tool, o `None` si la llamada tuvo éxito.
fn error_de(resp: &serde_json::Value) -> Option<String> {
    let res = resp.get("result")?;
    res.get("isError")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
        .then(|| res["content"][0]["text"].as_str().unwrap_or("").to_string())
}

/// **E24-H09** — un `limit` fuera del rango declarado o de otro tipo se RECHAZA.
#[test]
fn limit_fuera_de_rango_es_invalid_schema() {
    let dir = ws_dos_docs();
    let casos = [
        serde_json::json!({"text": "", "limit": 0}),
        serde_json::json!({"text": "", "limit": 9999}),
        serde_json::json!({"text": "", "limit": -5}),
        serde_json::json!({"text": "", "limit": "10"}),
    ];
    let lineas: Vec<String> = casos
        .iter()
        .enumerate()
        .map(|(i, args)| {
            serde_json::json!({"jsonrpc":"2.0","id": i + 1,"method":"tools/call",
                "params":{"name":"knowledge_search","arguments": args}})
            .to_string()
        })
        .collect();
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip(dir.path(), &refs, casos.len());

    for (i, r) in resp.iter().enumerate() {
        let err = error_de(r).unwrap_or_else(|| {
            panic!(
                "el caso {i} ({}) debe RECHAZARSE: el `inputSchema` declara `minimum: 1, \
                 maximum: 100`, y hasta E24-H09 estos valores caían al default en silencio. \
                 `limit: 0` devolvía 0 resultados, indistinguible de «no hay nada». Respuesta: {r}",
                casos[i]
            )
        });
        assert!(
            err.starts_with("INVALID_SCHEMA"),
            "el rechazo debe llevar el código estable del catálogo: {err}"
        );
    }
}

/// **E24-H09** — control anti-vacuo: un `limit` válido sigue funcionando exactamente igual.
#[test]
fn limit_valido_sigue_funcionando() {
    let dir = ws_dos_docs();
    let linea = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"knowledge_search","arguments":{"text":"","limit":1}}})
    .to_string();
    let resp = roundtrip(dir.path(), &[linea.as_str()], 1);
    assert!(
        error_de(&resp[0]).is_none(),
        "un `limit` dentro del rango no puede rechazarse: {}",
        resp[0]
    );
    assert_eq!(
        resp[0]["result"]["structuredContent"]["results"]
            .as_array()
            .map(Vec::len),
        Some(1),
        "y debe seguir acotando la página: {}",
        resp[0]
    );
}

/// **E24-H09** — un booleano y un entero con el tipo equivocado se rechazan.
#[test]
fn tipo_incorrecto_es_invalid_schema() {
    let dir = ws_dos_docs();
    let l1 = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"knowledge_check","arguments":{"scope":{"kind":"workspace"},
                  "includeSuggestedFixes":"true"}}})
    .to_string();
    let l2 = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name":"graph_query","arguments":{"operation":"backlinks",
                  "ref":{"path":"a.md"},"depth":"3"}}})
    .to_string();
    let resp = roundtrip(dir.path(), &[l1.as_str(), l2.as_str()], 2);

    for (i, r) in resp.iter().enumerate() {
        let err = error_de(r).unwrap_or_else(|| {
            panic!("el caso {i} debe rechazarse en vez de caer al default en silencio: {r}")
        });
        assert!(
            err.starts_with("INVALID_SCHEMA"),
            "con código estable: {err}"
        );
    }
}

/// **E24-H10** — un `where` malformado sale con `INVALID_SCHEMA`, no con `INTERNAL_IO_ERROR`.
#[test]
fn where_malformado_es_invalid_schema() {
    let dir = ws_dos_docs();
    let linea = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"knowledge_search","arguments":{"text":"","where":"status ="}}})
    .to_string();
    let resp = roundtrip(dir.path(), &[linea.as_str()], 1);
    let err = error_de(&resp[0]).expect("una consulta malformada debe fallar");
    assert!(
        err.starts_with("INVALID_SCHEMA"),
        "un typo del agente en su consulta es entrada inválida, no un error interno de I/O del \
         motor (hasta v0.3.0 salía INTERNAL_IO_ERROR): {err}"
    );
}

/// **E24-H10** — la MISMA consulta malformada da el MISMO código por las dos tools que la aceptan.
///
/// Es la asimetría concreta que cierra la historia: por `knowledge_search` salía
/// `INTERNAL_IO_ERROR` y por la selección masiva de `change_plan`, `INVALID_SCHEMA`.
#[test]
fn misma_consulta_mismo_codigo_en_las_dos_tools() {
    let dir = ws_dos_docs();
    let l1 = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"knowledge_search","arguments":{"text":"","where":"))) and and"}}})
    .to_string();
    let l2 = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name":"change_plan","arguments":{
            "selection":{"where":"))) and and"},
            "operation":{"patch_frontmatter":{"patch":{"x":1}}}}}})
    .to_string();
    let resp = roundtrip(dir.path(), &[l1.as_str(), l2.as_str()], 2);

    let e1 = error_de(&resp[0]).expect("knowledge_search debe fallar");
    let e2 = error_de(&resp[1]).expect("change_plan debe fallar");
    assert!(
        e1.starts_with("INVALID_SCHEMA") && e2.starts_with("INVALID_SCHEMA"),
        "la misma consulta malformada debe dar el mismo código por las dos tools.\n\
         knowledge_search: {e1}\nchange_plan:       {e2}"
    );
}

/// **E24-H10** — un parámetro obligatorio ausente lleva código estable.
#[test]
fn parametro_obligatorio_ausente_es_invalid_schema() {
    let dir = ws_dos_docs();
    let casos: [(&str, serde_json::Value); 5] = [
        ("knowledge_get", serde_json::json!({})),
        ("metadata_inspect", serde_json::json!({})),
        ("knowledge_check", serde_json::json!({})),
        ("graph_query", serde_json::json!({})),
        ("change_apply", serde_json::json!({})),
    ];
    let lineas: Vec<String> = casos
        .iter()
        .enumerate()
        .map(|(i, (n, args))| {
            serde_json::json!({"jsonrpc":"2.0","id": i + 1,"method":"tools/call",
                "params":{"name": n,"arguments": args}})
            .to_string()
        })
        .collect();
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip(dir.path(), &refs, casos.len());

    for (i, r) in resp.iter().enumerate() {
        let err = error_de(r)
            .unwrap_or_else(|| panic!("{} sin su parámetro obligatorio debe fallar", casos[i].0));
        assert!(
            err.starts_with("INVALID_SCHEMA"),
            "«falta el parámetro X» viajaba como texto suelto, sin nada por lo que un agente \
             pueda ramificar. Tool {}: {err}",
            casos[i].0
        );
    }
}

/// **E24-H10** — los mensajes de error no filtran internos de serde.
///
/// **Ampliado en E26-H07** a `change_plan`: la misma `mensaje_de_filtro` que sanea el `FilterError`
/// para `knowledge_search` debe servir a la selección masiva, que hasta v0.4.0 devolvía
/// «INVALID_SCHEMA» pelado (no filtraba el interno de serde… porque no decía nada). La exigencia
/// es la misma para las dos tools, y por eso el caso se añade a la tabla en vez de a un test aparte:
/// una segunda copia del saneado sería justo lo que prohíbe el invariante #3.
#[test]
fn errores_no_filtran_internos_de_serde() {
    let dir = ws_dos_docs();
    let casos: [(&str, serde_json::Value); 2] = [
        (
            "knowledge_search",
            serde_json::json!({"text": "", "filter": {"nope": 1}}),
        ),
        (
            "change_plan",
            serde_json::json!({"selection": {"filter": {"nope": 1}},
                               "operation": {"patch_frontmatter": {"patch": {"x": 1}}}}),
        ),
    ];
    let lineas: Vec<String> = casos
        .iter()
        .enumerate()
        .map(|(i, (n, args))| {
            serde_json::json!({"jsonrpc":"2.0","id": i + 1,"method":"tools/call",
                "params":{"name": n,"arguments": args}})
            .to_string()
        })
        .collect();
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip(dir.path(), &refs, casos.len());

    for (i, (tool, _)) in casos.iter().enumerate() {
        let err = error_de(&resp[i]).unwrap_or_else(|| {
            panic!("un filtro malformado debe fallar por «{tool}»: {}", resp[i])
        });
        assert!(
            !err.contains("untagged enum"),
            "«data did not match any variant of untagged enum WireNode» es un interno de \
             implementación que no le dice a nadie qué arreglar en su filtro. Tool «{tool}»: {err}"
        );
        assert!(
            err.contains("field") && err.contains("operator"),
            "el mensaje debe decir qué forma se esperaba. Tool «{tool}»: {err}"
        );
    }
}

// ---------------------------------------------------------------------------
// E24-H11 — `knowledge_get` devuelve el título derivado
//
// La derivación (`frontmatter.title` → primer H1 → nombre del fichero, §20.2) funcionaba y viajaba
// en `knowledge_search` y en `graph_query`, pero NO en la tool que lee un documento: su
// `DocumentView` traía `path`/`revision`/`frontmatter`/`body`/`outgoingLinks`/`backlinks`/
// `diagnostics` y nada más. Un agente que siguiera el flujo recomendado (buscar → leer) perdía el
// título al leer, y el `include` cerrado tampoco le dejaba pedirlo.
// ---------------------------------------------------------------------------

/// **E24-H11** — las tres fuentes de la cascada de `§20.2` llegan por `knowledge_get`.
#[test]
fn get_devuelve_titulo_derivado() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "con_fm.md",
        "---\ntitle: Desde Frontmatter\n---\n\n# Otro H1\n",
    );
    write(dir.path(), "con_h1.md", "# Desde H1\n\ncuerpo\n");
    write(dir.path(), "pelado.md", "solo texto, sin heading\n");

    let esperado = [
        ("con_fm.md", "Desde Frontmatter"),
        ("con_h1.md", "Desde H1"),
        ("pelado.md", "pelado"),
    ];
    let lineas: Vec<String> = esperado
        .iter()
        .enumerate()
        .map(|(i, (p, _))| {
            serde_json::json!({"jsonrpc":"2.0","id": i + 1,"method":"tools/call",
                "params":{"name":"knowledge_get","arguments":{"ref":{"path": p}}}})
            .to_string()
        })
        .collect();
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip(dir.path(), &refs, esperado.len());

    for (i, (path, titulo)) in esperado.iter().enumerate() {
        let doc = &resp[i]["result"]["structuredContent"]["document"];
        assert_eq!(
            doc["title"].as_str(),
            Some(*titulo),
            "`knowledge_get` debe traer el título derivado por la cascada de §20.2 \
             (frontmatter.title → primer H1 → nombre del fichero) para {path}: {}",
            resp[i]
        );
    }
}

/// **E24-H11** — control anti-vacuo: el título de `knowledge_get` coincide con el de
/// `knowledge_search`.
///
/// Es lo que impide que sea una SEGUNDA implementación del título (invariante #3): si alguien
/// derivara el título aquí con otro criterio, este test lo caza.
#[test]
fn titulo_coincide_entre_get_y_search() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "con_fm.md",
        "---\ntitle: Desde Frontmatter\n---\n\n# Otro H1\n",
    );
    write(dir.path(), "con_h1.md", "# Desde H1\n\ncuerpo\n");
    write(dir.path(), "pelado.md", "solo texto\n");

    let l_search = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"knowledge_search","arguments":{"text":""}}})
    .to_string();
    let l_get: Vec<String> = ["con_fm.md", "con_h1.md", "pelado.md"]
        .iter()
        .enumerate()
        .map(|(i, p)| {
            serde_json::json!({"jsonrpc":"2.0","id": i + 2,"method":"tools/call",
                "params":{"name":"knowledge_get","arguments":{"ref":{"path": p}}}})
            .to_string()
        })
        .collect();
    let mut refs: Vec<&str> = vec![l_search.as_str()];
    refs.extend(l_get.iter().map(String::as_str));
    let resp = roundtrip(dir.path(), &refs, 4);

    let de_search: std::collections::BTreeMap<String, String> = resp[0]["result"]
        ["structuredContent"]["results"]
        .as_array()
        .expect("results")
        .iter()
        .map(|r| {
            (
                r["path"].as_str().unwrap_or_default().to_string(),
                r["title"].as_str().unwrap_or_default().to_string(),
            )
        })
        .collect();

    for r in resp.iter().skip(1) {
        let doc = &r["result"]["structuredContent"]["document"];
        let path = doc["path"].as_str().expect("path").to_string();
        assert_eq!(
            doc["title"].as_str().map(str::to_string),
            de_search.get(&path).cloned(),
            "el título de `knowledge_get` y el de `knowledge_search` deben salir de la MISMA \
             función del core, no de dos implementaciones (invariante #3). Path: {path}"
        );
    }
}

// ---------------------------------------------------------------------------
// E24-H15 — El `structuredContent` CONFORMA el `outputSchema` declarado
//
// Los 10 `outputSchema` se derivan con `schemars` desde el tipo Rust que sirve cada servicio, así
// que en teoría no pueden divergir. En la práctica nadie lo comprobaba: `tools_declaran_outputschema`
// mira 5 de las 10 y solo que el schema «tenga alguna clave estructural», y un brazo del despachador
// que construya el JSON a mano (como hace `knowledge_get` con su envoltorio `{document}`) puede
// apartarse del tipo sin que nada lo note.
//
// Un cliente MCP estricto SÍ valida. Esto es una guardia anti-drift, no la corrección de un defecto:
// al escribirla, las 10 conformaban.
// ---------------------------------------------------------------------------

/// Valida `instancia` contra `schema`, devolviendo los errores en texto.
fn errores_de_schema(schema: &serde_json::Value, instancia: &serde_json::Value) -> Vec<String> {
    let validador = jsonschema::validator_for(schema)
        .unwrap_or_else(|e| panic!("el propio outputSchema no es un JSON Schema válido: {e}"));
    validador
        .iter_errors(instancia)
        .map(|e| format!("{} en {}", e, e.instance_path()))
        .collect()
}

/// **E24-H15** — la salida real de las 10 tools conforma su `outputSchema`.
#[test]
fn structured_content_conforma_output_schema() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "alfa.md",
        "---\nstatus: draft\npriority: 2\ntags:\n  - uno\n---\n\n# Alfa\n\nVer [beta](notas/beta.md) y [roto](no-existe.md).\n",
    );
    write(
        dir.path(),
        "notas/beta.md",
        "---\nstatus: accepted\n---\n\n# Beta\n\n## Sección\n\ncuerpo\n\n[alfa](../alfa.md)\n",
    );

    // Llamadas que cubren las 10 tools (varias formas de las polimórficas).
    let llamadas: Vec<(&str, serde_json::Value)> = vec![
        ("workspace_status", serde_json::json!({})),
        (
            "knowledge_search",
            serde_json::json!({"text": "", "include": ["frontmatter.status"]}),
        ),
        (
            "knowledge_get",
            serde_json::json!({"ref": {"path": "alfa.md"},
            "include": ["frontmatter","body","revision","outgoingLinks","backlinks","diagnostics"]}),
        ),
        ("metadata_inspect", serde_json::json!({"mode": "catalog"})),
        (
            "metadata_inspect",
            serde_json::json!({"mode": "field", "field": "tags"}),
        ),
        (
            "knowledge_check",
            serde_json::json!({"scope": {"kind": "workspace"}, "includeSuggestedFixes": true}),
        ),
        (
            "graph_query",
            serde_json::json!({"operation": "neighborhood", "ref": {"path": "alfa.md"}, "depth": 2, "direction": "both"}),
        ),
        (
            "graph_query",
            serde_json::json!({"operation": "components"}),
        ),
        ("graph_query", serde_json::json!({"operation": "dangling"})),
        (
            "impact_analyze",
            serde_json::json!({"ref": {"path": "alfa.md"}, "proposedOperation": {"kind": "move"}}),
        ),
    ];

    let mut lineas =
        vec![serde_json::json!({"jsonrpc":"2.0","id":0,"method":"tools/list"}).to_string()];
    for (i, (nombre, args)) in llamadas.iter().enumerate() {
        lineas.push(
            serde_json::json!({"jsonrpc":"2.0","id": i + 1,"method":"tools/call",
                "params":{"name": nombre,"arguments": args}})
            .to_string(),
        );
    }
    // Las 3 de cambio, encadenadas: plan -> apply -> revert.
    lineas.push(
        serde_json::json!({"jsonrpc":"2.0","id":100,"method":"tools/call","params":{
            "name":"change_plan","arguments":{
                "operations":[{"op":"patch_frontmatter","path":"alfa.md","patch":{"status":"review"}}],
                "policy":{"requireValidResult":false,"allowWarnings":true}}}})
        .to_string(),
    );
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip(dir.path(), &refs, lineas.len());

    let tools: std::collections::BTreeMap<String, serde_json::Value> = resp[0]["result"]["tools"]
        .as_array()
        .expect("tools/list")
        .iter()
        .map(|t| {
            (
                t["name"].as_str().unwrap_or_default().to_string(),
                t["outputSchema"].clone(),
            )
        })
        .collect();
    assert_eq!(tools.len(), 10, "deben ser las 10 tools objetivo");

    let mut validadas: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for (i, (nombre, args)) in llamadas.iter().enumerate() {
        let r = &resp[i + 1];
        let sc = &r["result"]["structuredContent"];
        assert!(
            !r["result"]["isError"].as_bool().unwrap_or(false),
            "la llamada a «{nombre}» con {args} debe tener éxito para poder validar su salida: {r}"
        );
        let errores = errores_de_schema(&tools[*nombre], sc);
        assert!(
            errores.is_empty(),
            "el `structuredContent` de «{nombre}» NO conforma su `outputSchema` declarado. Un \
             cliente MCP estricto lo rechazaría.\nViolaciones: {errores:#?}"
        );
        validadas.insert((*nombre).to_string());
    }

    // change_plan (la última respuesta): su salida también conforma.
    let plan = &resp[lineas.len() - 1]["result"]["structuredContent"];
    let errores = errores_de_schema(&tools["change_plan"], plan);
    assert!(
        errores.is_empty(),
        "el `structuredContent` de «change_plan» no conforma su `outputSchema`: {errores:#?}"
    );
    validadas.insert("change_plan".to_string());

    // change_apply y change_revert, en una sesión aparte (necesitan el changeSetId del plan).
    let cs = plan["changeSetId"].as_str().expect("changeSetId");
    let l_apply = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"change_apply","arguments":{"changeSetId": cs}}})
    .to_string();
    let resp2 = roundtrip(dir.path(), &[l_apply.as_str()], 1);
    let apply = &resp2[0]["result"]["structuredContent"];
    let errores = errores_de_schema(&tools["change_apply"], apply);
    assert!(
        errores.is_empty(),
        "el `structuredContent` de «change_apply» no conforma su `outputSchema`: {errores:#?}"
    );
    validadas.insert("change_apply".to_string());

    let receipt = apply["receiptId"].as_str().expect("receiptId");
    let l_revert = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"change_revert","arguments":{"receiptId": receipt}}})
    .to_string();
    let resp3 = roundtrip(dir.path(), &[l_revert.as_str()], 1);
    let revert = &resp3[0]["result"]["structuredContent"];
    let errores = errores_de_schema(&tools["change_revert"], revert);
    assert!(
        errores.is_empty(),
        "el `structuredContent` de «change_revert» no conforma su `outputSchema`: {errores:#?}"
    );
    validadas.insert("change_revert".to_string());

    // Cobertura: las 10, sin excepción. Es lo que impide que este test se degrade con el tiempo a
    // «las que resultaron fáciles de llamar».
    let declaradas: std::collections::BTreeSet<String> = tools.keys().cloned().collect();
    assert_eq!(
        validadas, declaradas,
        "se deben validar TODAS las tools declaradas, no un subconjunto"
    );
}

/// **E24-H15** — control anti-vacuo del validador: una salida deliberadamente incoherente falla.
///
/// Sin esto, un `errores_de_schema` que devolviera siempre la lista vacía —por un schema mal
/// cargado, por ejemplo— haría pasar el test de arriba sin validar nada.
#[test]
fn el_validador_de_schema_muerde() {
    let schema = serde_json::json!({
        "type": "object",
        "properties": { "results": { "type": "array" } },
        "required": ["results"]
    });
    assert!(
        errores_de_schema(&schema, &serde_json::json!({"results": []})).is_empty(),
        "una instancia correcta no puede producir errores"
    );
    assert!(
        !errores_de_schema(&schema, &serde_json::json!({"results": "no soy un array"})).is_empty(),
        "una instancia que viola el schema DEBE producir errores: si no, el test de conformidad no \
         está validando nada"
    );
    assert!(
        !errores_de_schema(&schema, &serde_json::json!({})).is_empty(),
        "un campo requerido ausente debe detectarse"
    );
}

// ---------------------------------------------------------------------------
// E26-H07 — Todo error de superficie lleva código Y mensaje
//
// E24-H10 puso el código estable al frente del mensaje… en `knowledge_search` y en las
// comprobaciones locales del despachador. Las otras OCHO tools siguen haciendo
// `.map_err(|e| e.as_str().to_string())`: el agente recibe literalmente «INVALID_SCHEMA», sin una
// palabra sobre QUÉ parámetro, QUÉ valor o QUÉ se esperaba. No es un descuido del despachador —los
// productores de `lodestar-app` son `Result<_, ErrorCode>` y no tienen dónde poner el mensaje—, y
// por eso el arreglo es de la fachada entera, no de ocho `format!`.
//
// Dos consecuencias concretas de no tener sitio para el mensaje:
//  - `graph_query` sin `ref` responde `DOCUMENT_NOT_FOUND` (el rustdoc lo admite: «no hay un código
//    de falta-parámetro en el catálogo»), así que quien OLVIDA el `ref` recibe el mismo error que
//    quien apunta a un documento que no existe, y toma el camino de recuperación equivocado;
//  - `build_selection_expression` tira el `ParseError` del core con `map_err(|_| …)`, así que la
//    MISMA consulta malformada se diagnostica por `knowledge_search` y se calla por `change_plan`.
//
// El catálogo NO lo toca ESTA historia (E26-H11): añade mensaje, no reclasifica códigos — salvo el
// único caso que declara, `graph_query` sin `ref`/`to`. Cuando se escribió tenía 16 filas; hoy tiene
// 17 (`catalogo_de_errores_tiene_diecisiete_filas`, en `lodestar-core`) porque E28-H02 lo abrió a
// conciencia para `DOCUMENT_ALREADY_EXISTS`, ajeno a lo que fija este bloque.
// ---------------------------------------------------------------------------

/// Parte el texto de error en `(código, mensaje)` **solo** si tiene la forma «CÓDIGO: mensaje» con
/// el código estable de `ErrorCode::as_str()` (SCREAMING_SNAKE) al frente. `None` si el texto es el
/// código pelado, o si lo que abre no es un código del catálogo.
fn codigo_y_mensaje(err: &str) -> Option<(&str, &str)> {
    let (codigo, mensaje) = err.split_once(": ")?;
    (!codigo.is_empty()
        && codigo
            .chars()
            .all(|c| c.is_ascii_uppercase() || c == '_' || c.is_ascii_digit()))
    .then(|| (codigo, mensaje.trim()))
}

/// El código que abre el texto de error, o el texto entero si viene pelado (que es justo lo que
/// esta historia arregla): sirve para juzgar el CÓDIGO con independencia de si ya trae mensaje.
fn codigo_de(err: &str) -> &str {
    codigo_y_mensaje(err).map_or(err, |(codigo, _)| codigo)
}

/// ¿El mensaje **nombra** ese identificador (parámetro, operación) como token, y no como trozo de
/// otra palabra? Acepta cualquier delimitador —«ref», "ref", `ref` o ref suelto— porque lo que el
/// criterio exige es que el mensaje lo nombre, no una tipografía concreta; pero rechaza
/// «referencia» o «documento» como forma de «nombrar» `ref` o `to`.
fn menciona(texto: &str, token: &str) -> bool {
    texto
        .split(|c: char| !(c.is_alphanumeric() || c == '_'))
        .any(|t| t == token)
}

/// **E26-H07** — las 8 tools que hoy devuelven el código pelado emiten «CÓDIGO: mensaje».
///
/// Cada caso provoca el error **más común** de su tool por el camino que hoy no tiene mensaje: el
/// del productor de `lodestar-app` (no las comprobaciones locales del despachador, que desde
/// E24-H10 ya lo llevan). El código esperado se asevera además tal cual sale hoy: esta historia
/// AÑADE mensaje y no reclasifica el catálogo, así que envolver el arreglo en un «todo es
/// INVALID_SCHEMA» sería otro defecto, no el arreglo.
#[test]
fn todas_las_tools_dan_codigo_y_mensaje() {
    let dir = ws_dos_docs();
    let casos: [(&str, serde_json::Value, &str); 8] = [
        (
            "knowledge_get",
            serde_json::json!({"ref": {"path": "no-existe.md"}}),
            "DOCUMENT_NOT_FOUND",
        ),
        (
            "metadata_inspect",
            serde_json::json!({"mode": "field"}),
            "INVALID_SCHEMA",
        ),
        (
            "knowledge_check",
            serde_json::json!({"scope": {"kind": "document", "ref": {"path": "no-existe.md"}}}),
            "DOCUMENT_NOT_FOUND",
        ),
        (
            "graph_query",
            serde_json::json!({"operation": "chorizo"}),
            "INVALID_SCHEMA",
        ),
        (
            "impact_analyze",
            serde_json::json!({"ref": {"path": "no-existe.md"},
                               "proposedOperation": {"kind": "move"}}),
            "DOCUMENT_NOT_FOUND",
        ),
        (
            "change_plan",
            serde_json::json!({"operations": [
                {"op": "patch_frontmatter", "path": "no-existe.md", "patch": {"x": 1}}]}),
            "DOCUMENT_NOT_FOUND",
        ),
        (
            "change_apply",
            serde_json::json!({"changeSetId": "cs:no-existe"}),
            "PLAN_STALE",
        ),
        (
            "change_revert",
            serde_json::json!({"receiptId": "rc:no-existe"}),
            "PLAN_EXPIRED",
        ),
    ];

    let lineas: Vec<String> = casos
        .iter()
        .enumerate()
        .map(|(i, (n, args, _))| {
            serde_json::json!({"jsonrpc":"2.0","id": i + 1,"method":"tools/call",
                "params":{"name": n,"arguments": args}})
            .to_string()
        })
        .collect();
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip(dir.path(), &refs, casos.len());

    // Se acumulan TODAS las tools que incumplen antes de fallar: el defecto es de las ocho a la
    // vez, y un panic en la primera obligaría a descubrirlas de una en una.
    let mut incumplen: Vec<String> = Vec::new();
    for (i, (tool, args, codigo_esperado)) in casos.iter().enumerate() {
        let err = error_de(&resp[i]).unwrap_or_else(|| {
            panic!(
                "«{tool}» con {args} debe fallar para poder juzgar su error: {}",
                resp[i]
            )
        });
        match codigo_y_mensaje(&err) {
            None => incumplen.push(format!(
                "{tool}: CÓDIGO PELADO «{err}» (se esperaba «{codigo_esperado}: <mensaje>»)"
            )),
            Some((codigo, mensaje)) => {
                if codigo != *codigo_esperado {
                    incumplen.push(format!(
                        "{tool}: código «{codigo}», se esperaba «{codigo_esperado}» — E26-H07 \
                         AÑADE mensaje, no reclasifica el catálogo (su único cambio de código es \
                         `graph_query` sin «ref»)"
                    ));
                } else if mensaje.len() < 10 || mensaje == codigo {
                    incumplen.push(format!(
                        "{tool}: el mensaje debe ser accionable, no un relleno ni el código \
                         repetido: «{err}»"
                    ));
                }
            }
        }
    }
    assert!(
        incumplen.is_empty(),
        "estas tools no emiten «CÓDIGO: mensaje». El agente puede ramificar por el código, pero \
         no tiene una palabra sobre qué parámetro, qué valor o qué se esperaba — que es lo que \
         necesita para CORREGIR. La forma es la que `knowledge_search` emite desde E24-H10:\n  {}",
        incumplen.join("\n  ")
    );
}

/// **E26-H07** — olvidar el parámetro NO es «el documento no existe».
///
/// Las cuatro operaciones que exigen un extremo (`backlinks`/`outgoing`/`neighborhood` piden `ref`;
/// `path_between` pide además `to`) devuelven hoy `DOCUMENT_NOT_FOUND` cuando el parámetro ni
/// siquiera viene. El agente que olvidó el `ref` recibe el mismo error que el que apuntó a un
/// documento inexistente, y toma el camino de recuperación equivocado (buscar el documento, en vez
/// de mirar su llamada).
#[test]
fn graph_query_sin_ref_es_invalid_schema() {
    let dir = ws_dos_docs();
    // (argumentos, operación, parámetro ausente)
    let casos: [(serde_json::Value, &str, &str); 4] = [
        (
            serde_json::json!({"operation": "backlinks"}),
            "backlinks",
            "ref",
        ),
        (
            serde_json::json!({"operation": "outgoing"}),
            "outgoing",
            "ref",
        ),
        (
            serde_json::json!({"operation": "neighborhood"}),
            "neighborhood",
            "ref",
        ),
        (
            serde_json::json!({"operation": "path_between", "ref": {"path": "a.md"}}),
            "path_between",
            "to",
        ),
    ];

    let lineas: Vec<String> = casos
        .iter()
        .enumerate()
        .map(|(i, (args, _, _))| {
            serde_json::json!({"jsonrpc":"2.0","id": i + 1,"method":"tools/call",
                "params":{"name":"graph_query","arguments": args}})
            .to_string()
        })
        .collect();
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip(dir.path(), &refs, casos.len());

    for (i, (args, operacion, parametro)) in casos.iter().enumerate() {
        let err = error_de(&resp[i])
            .unwrap_or_else(|| panic!("{args} debe fallar: falta «{parametro}»: {}", resp[i]));
        // El CÓDIGO se juzga primero y con independencia del mensaje: el defecto U2 es que
        // «falta el parámetro» y «el documento no existe» son hoy el mismo error.
        assert_eq!(
            codigo_de(&err),
            "INVALID_SCHEMA",
            "que FALTE «{parametro}» es un esquema de entrada inválido, no un documento que no \
             existe: hasta v0.4.0 salía DOCUMENT_NOT_FOUND, indistinguible de un «{parametro}» \
             presente que no resuelve. Error completo: «{err}»"
        );
        let (_, mensaje) = codigo_y_mensaje(&err)
            .unwrap_or_else(|| panic!("el error de {args} debe llevar código Y mensaje: «{err}»"));
        assert!(
            menciona(mensaje, parametro),
            "el mensaje debe NOMBRAR el parámetro que falta («{parametro}»): «{err}»"
        );
        assert!(
            menciona(mensaje, operacion),
            "…y la operación que lo exige («{operacion}»), porque no todas lo exigen: «{err}»"
        );
    }
}

/// **E26-H07** — control anti-vacuo: el arreglo no puede consistir en mapear todo a
/// `INVALID_SCHEMA`.
///
/// Un `ref` (o un `to`) PRESENTE que no resuelve es exactamente lo que dice
/// `DOCUMENT_NOT_FOUND`, y debe seguir siéndolo: es la distinción que la historia existe para
/// crear. Lo que sí cambia es que ahora también trae mensaje.
#[test]
fn ref_que_no_resuelve_sigue_siendo_not_found() {
    let dir = ws_dos_docs();
    let casos: [serde_json::Value; 2] = [
        serde_json::json!({"operation": "backlinks", "ref": {"path": "no-existe.md"}}),
        serde_json::json!({"operation": "path_between", "ref": {"path": "a.md"},
                           "to": {"path": "no-existe.md"}}),
    ];

    let lineas: Vec<String> = casos
        .iter()
        .enumerate()
        .map(|(i, args)| {
            serde_json::json!({"jsonrpc":"2.0","id": i + 1,"method":"tools/call",
                "params":{"name":"graph_query","arguments": args}})
            .to_string()
        })
        .collect();
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip(dir.path(), &refs, casos.len());

    for (i, args) in casos.iter().enumerate() {
        let err = error_de(&resp[i])
            .unwrap_or_else(|| panic!("{args} apunta a un documento inexistente: {}", resp[i]));
        // Primero el control (el CÓDIGO no puede cambiar), después la exigencia nueva (mensaje).
        assert_eq!(
            codigo_de(&err),
            "DOCUMENT_NOT_FOUND",
            "un extremo PRESENTE que no resuelve es lo que su nombre dice. Si esto pasara a \
             INVALID_SCHEMA, la distinción que introduce E26-H07 se habría perdido por el otro \
             lado. Error completo: «{err}»"
        );
        assert!(
            codigo_y_mensaje(&err).is_some_and(|(_, m)| !m.is_empty()),
            "…y también él lleva ahora mensaje, en la forma «CÓDIGO: mensaje»: «{err}»"
        );
    }
}

/// **E26-H07** — la misma consulta malformada se diagnostica igual por las dos tools que la aceptan.
///
/// `build_search_expression` (`knowledge_search`) entrega el texto del `ParseError` del core;
/// `build_selection_expression` (la selección masiva de `change_plan`) lo tira con
/// `map_err(|_| ErrorCode::InvalidSchema)`. E24-H10 cerró esa asimetría para el CÓDIGO y la dejó
/// abierta para el MENSAJE.
///
/// El diagnóstico esperado se toma del **core**, no de un literal: así el test no fija la redacción
/// del parser, solo exige que llegue entera a las dos superficies (invariante #3, una sola verdad).
#[test]
fn change_plan_conserva_el_error_del_parser() {
    let dir = ws_dos_docs();
    const CONSULTA: &str = "status =";
    let diagnostico = lodestar_core::parse::parse(CONSULTA)
        .expect_err("«status =» es una consulta malformada: al parser le falta el valor")
        .message;

    let l_search = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"knowledge_search","arguments":{"text":"","where": CONSULTA}}})
    .to_string();
    let l_plan = serde_json::json!({"jsonrpc":"2.0","id":2,"method":"tools/call",
        "params":{"name":"change_plan","arguments":{
            "selection":{"where": CONSULTA},
            "operation":{"patch_frontmatter":{"patch":{"x":1}}}}}})
    .to_string();
    let resp = roundtrip(dir.path(), &[l_search.as_str(), l_plan.as_str()], 2);

    let e_search = error_de(&resp[0]).expect("knowledge_search debe fallar");
    let e_plan = error_de(&resp[1]).expect("change_plan debe fallar");

    // Control: por `knowledge_search` el diagnóstico ya viaja (E24-H10). Si esto fallara, el
    // esperado se habría desalineado del core y el test de abajo no probaría nada.
    assert!(
        e_search.contains(&diagnostico),
        "el diagnóstico del parser («{diagnostico}») debe seguir llegando por knowledge_search: \
         «{e_search}»"
    );
    let (codigo, mensaje) = codigo_y_mensaje(&e_plan)
        .unwrap_or_else(|| panic!("change_plan debe emitir «CÓDIGO: mensaje»: «{e_plan}»"));
    assert_eq!(codigo, "INVALID_SCHEMA", "mismo código por las dos tools");
    assert!(
        mensaje.contains(&diagnostico),
        "el MISMO `where` malformado debe dar el MISMO diagnóstico por las dos tools que lo \
         aceptan: `build_selection_expression` lo descarta con `map_err(|_| …)`, así que \
         change_plan devolvía «INVALID_SCHEMA» a secas.\n\
         diagnóstico del core: «{diagnostico}»\n\
         knowledge_search:     «{e_search}»\n\
         change_plan:          «{e_plan}»"
    );
}

// ---------------------------------------------------------------------------
// E26-H08 — Un `TypeError` de consulta se REPORTA, no excluye documentos en silencio
//
// Los dos consumidores del lenguaje descartan la evaluación con
// `if !matches!(evaluate(...), Ok(true)) { continue; }` —`knowledge_search` y la selección masiva de
// `change_plan` (`expand_selection`)—, así que un `Err(TypeError)` cae en el MISMO `continue` que un
// `Ok(false)`: el documento se excluye. Efectos que estos tests reproducen:
//   · una consulta con un error de tipo real devuelve `[]` sin un solo aviso, indistinguible de «no
//     hay resultados»;
//   · sobre una base heterogénea la exclusión se decide documento a documento, así que la respuesta
//     es una lista RECORTADA que nadie puede distinguir de la correcta;
//   · en `change_plan` es peor: una selección masiva salta documentos en silencio y el plan afecta a
//     menos ficheros de los que el agente cree haber seleccionado.
//
// Es el principio de E24-H07 («una respuesta silenciosamente equivocada es peor que un error»)
// aplicado a la EVALUACIÓN, después de que E24-H07/H08 lo aplicaran al parseo. Los rustdoc de
// `lib.rs` consagran hoy lo contrario («sin propagarse a la búsqueda entera» / «sin abortar el
// plan»): E26-H08 revisa ese criterio, igual que E24-H07 revisó el de E19-H04.
//
// Lo que NO cambia (y por eso hay dos controles anti-vacuos): `Ok(false)` sigue siendo AUSENCIA —no
// casar no es un error—, y un campo ausente sigue excluyendo su documento sin ruido.
// ---------------------------------------------------------------------------

/// La consulta del defecto: orden (`>=`) entre un campo numérico y un literal string. Es
/// `TypeError::OrderNotDefined` en el core desde E19-H01 (`error_de_tipo_orden_cruzado`), y hasta
/// v0.4.0 se traducía a «este documento no casa».
const ORDEN_CRUZADO: &str = "priority >= \"high\"";

/// Grafías admisibles del tipo `number` en el mensaje: la del wire (`ValueType` serializa en
/// minúscula, y es la que ve el agente en `metadata_inspect.inferredTypes`) o su nombre en español
/// —los mensajes van en español (E26-H07)—. Lo que el criterio exige es que el mensaje NOMBRE el
/// tipo, no una tipografía concreta.
const GRAFIAS_NUMBER: &[&str] = &["number", "numero", "número", "numerico", "numérico"];

/// Ídem para `string`.
const GRAFIAS_STRING: &[&str] = &["string", "cadena", "texto"];

/// ¿El mensaje nombra ese tipo en alguna de sus grafías admisibles?
fn nombra_tipo(mensaje: &str, grafias: &[&str]) -> bool {
    let bajo = mensaje.to_lowercase();
    grafias.iter().any(|g| menciona(&bajo, g))
}

/// Juzga el error de un `TypeError` de orden cruzado: código estable + un mensaje que permita
/// CORREGIR la consulta (campo, operador y los dos tipos que chocan). `contexto` identifica la tool
/// en el fallo.
fn juzga_error_de_tipo(err: &str, contexto: &str) {
    assert_eq!(
        codigo_de(err),
        "INVALID_SCHEMA",
        "un error de TIPO al evaluar es una consulta que el motor no puede responder sobre estos \
         datos, y el catálogo ya tiene el código para eso ({contexto}): «{err}»"
    );
    let (_, mensaje) = codigo_y_mensaje(err)
        .unwrap_or_else(|| panic!("{contexto} debe emitir «CÓDIGO: mensaje» (E26-H07): «{err}»"));
    assert!(
        menciona(&mensaje.to_lowercase(), "priority"),
        "el mensaje debe NOMBRAR el campo que choca ({contexto}): «{err}»"
    );
    assert!(
        nombra_tipo(mensaje, GRAFIAS_NUMBER),
        "…el tipo que tiene el campo en el documento (number) ({contexto}): «{err}»"
    );
    assert!(
        nombra_tipo(mensaje, GRAFIAS_STRING),
        "…y el tipo del literal con el que se le comparó (string), que es la mitad del diagnóstico \
         sin la cual el agente no sabe qué lado corregir ({contexto}): «{err}»"
    );
    assert!(
        mensaje.contains(">=") || menciona(&mensaje.to_lowercase(), "greater_than_or_equal"),
        "…y el operador, porque `=` sobre los MISMOS operandos es legal (es `false`, no error): \
         solo el ORDEN yerra ({contexto}): «{err}»"
    );
}

/// Workspace **homogéneo**: los dos documentos tienen `priority` numérico, así que
/// `priority >= "high"` es un error de tipo en todos. Hoy la consulta devuelve `[]` —«no hay
/// resultados»— sin un solo aviso.
fn ws_priority_numerico() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "alfa.md",
        "---\npriority: 2\nstatus: draft\n---\n\n# Alfa\n",
    );
    write(
        dir.path(),
        "beta.md",
        "---\npriority: 5\nstatus: draft\n---\n\n# Beta\n",
    );
    dir
}

/// Workspace **heterogéneo** —el escenario real del defecto—: `priority` es string en unos
/// documentos y número en otros, de modo que `priority >= "high"` casa unos, yerra en otros y hoy
/// devuelve una lista recortada.
///
/// El reparto es DISCRIMINANTE para el criterio de determinismo:
///   · `alfa.md` es el primero del orden total y **no** yerra (string vs string): quien reporte «el
///     primer documento» sin más, o el primero que casa, nombrará el documento equivocado;
///   · `bravo.md` es el primero del orden total que **sí** yerra → es el que debe salir nombrado;
///   · `zulu.md` yerra también, pero el ÚLTIMO: quien acumule los errores y reporte el último (o
///     todos) hará que el mensaje dependa de dónde paró el motor, no del workspace.
fn ws_priority_heterogeneo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "alfa.md",
        "---\npriority: high\nstatus: draft\n---\n\n# Alfa\n",
    );
    write(
        dir.path(),
        "bravo.md",
        "---\npriority: 2\nstatus: draft\n---\n\n# Bravo\n",
    );
    write(
        dir.path(),
        "charlie.md",
        "---\npriority: urgent\nstatus: draft\n---\n\n# Charlie\n",
    );
    write(
        dir.path(),
        "zulu.md",
        "---\npriority: 9\nstatus: draft\n---\n\n# Zulu\n",
    );
    dir
}

/// La llamada JSON-RPC a `knowledge_search` con un `where` (y opcionalmente un `limit`).
fn linea_search(id: u32, donde: &str, limit: Option<u32>) -> String {
    let mut args = serde_json::json!({"text": "", "where": donde});
    if let Some(l) = limit {
        args["limit"] = serde_json::json!(l);
    }
    serde_json::json!({"jsonrpc":"2.0","id": id,"method":"tools/call",
        "params":{"name":"knowledge_search","arguments": args}})
    .to_string()
}

/// La llamada JSON-RPC a `change_plan` con la MISMA consulta como `selection.where`. La política es
/// permisiva a propósito: lo que se juzga es la EXPANSIÓN de la selección, no el veredicto de
/// conformidad del resultado.
fn linea_plan(id: u32, donde: &str) -> String {
    serde_json::json!({"jsonrpc":"2.0","id": id,"method":"tools/call",
        "params":{"name":"change_plan","arguments":{
            "selection": {"where": donde},
            "operation": {"patch_frontmatter": {"status": "review"}},
            "policy": {"requireValidResult": false, "allowWarnings": true}}}})
    .to_string()
}

/// **E26-H08** — un error de TIPO aborta la consulta con `INVALID_SCHEMA`, en vez de devolver `[]`.
///
/// Hoy `knowledge_search` responde `{"results": []}`: una lista vacía que el agente lee como «no hay
/// documentos con esa prioridad», cuando lo que ocurrió es que su consulta no es respondible sobre
/// estos datos. Es la respuesta silenciosamente equivocada que E24-H07 declaró peor que un error.
#[test]
fn type_error_de_orden_es_error_de_consulta() {
    let dir = ws_priority_numerico();
    let linea = linea_search(1, ORDEN_CRUZADO, None);
    let resp = roundtrip(dir.path(), &[linea.as_str()], 1);

    let err = error_de(&resp[0]).unwrap_or_else(|| {
        panic!(
            "`{ORDEN_CRUZADO}` sobre documentos con `priority` NUMÉRICO no es una consulta que el \
             motor pueda responder: comparar el orden de un número con un string es \
             `TypeError::OrderNotDefined` en el core desde E19-H01. Hasta v0.4.0 cada documento que \
             erraba se EXCLUÍA, así que la respuesta era `[]` —indistinguible de «no hay \
             resultados»— y el agente no tenía forma de enterarse.\nRespuesta recibida: {}",
            resp[0]
        )
    });
    juzga_error_de_tipo(&err, "knowledge_search");
}

/// **E26-H08** — la misma consulta da el mismo error por las dos superficies que la aceptan.
///
/// `knowledge_search` y `change_plan.selection` comparten el lenguaje (`§20.10`: `where`/`filter`
/// significan lo mismo en las dos), así que también deben compartir el veredicto y su redacción. En
/// `change_plan` el defecto es además el más caro: la selección masiva salta documentos en silencio
/// y el plan toca menos ficheros de los que el agente cree haber seleccionado.
#[test]
fn misma_consulta_mismo_error_en_search_y_en_plan() {
    let dir = ws_priority_numerico();
    let l_search = linea_search(1, ORDEN_CRUZADO, None);
    let l_plan = linea_plan(2, ORDEN_CRUZADO);
    let resp = roundtrip(dir.path(), &[l_search.as_str(), l_plan.as_str()], 2);

    let e_search = error_de(&resp[0]).unwrap_or_else(|| {
        panic!(
            "`knowledge_search` con `{ORDEN_CRUZADO}` debe fallar (ver \
             `type_error_de_orden_es_error_de_consulta`): {}",
            resp[0]
        )
    });
    let e_plan = error_de(&resp[1]).unwrap_or_else(|| {
        panic!(
            "`change_plan` con `selection.where: {ORDEN_CRUZADO}` debe fallar: hoy `expand_selection` \
             se salta en silencio TODO documento cuya evaluación yerra, así que planifica sobre un \
             subconjunto que nadie pidió (aquí, el conjunto vacío) y lo presenta como un plan \
             legítimo.\nRespuesta recibida: {}",
            resp[1]
        )
    });

    juzga_error_de_tipo(&e_plan, "change_plan");
    assert_eq!(
        e_search, e_plan,
        "el MISMO `where` sobre el MISMO workspace debe dar el MISMO código y el MISMO mensaje por \
         las dos tools que aceptan el lenguaje: si divergen, el agente aprende a corregir con una \
         superficie y se queda a ciegas con la otra (§20.10, invariante #3)"
    );
}

/// **E26-H08** — el error reportado es determinista: mismo workspace y misma consulta, mismo error
/// palabra por palabra, y siempre sobre el mismo documento.
///
/// Sobre una base heterogénea hay VARIOS documentos que yerran, así que «cuál se reporta» tiene que
/// estar decidido por el workspace y no por el camino que tomó el motor. El criterio de la historia
/// es el **primer documento del orden total ya existente** (`Analysis::documents`, ordenado por
/// `RelPath`) — la premisa está clavada en el core por `primer_type_error_en_el_orden_total`
/// (`lodestar-core/tests/consulta.rs`).
///
/// Cuatro observaciones distintas del mismo workspace, que es lo que hace al test discriminante:
/// dos procesos frescos, una página más pequeña y la otra tool. Ninguna puede cambiar el veredicto.
#[test]
fn el_type_error_reportado_es_determinista() {
    let dir = ws_priority_heterogeneo();
    let linea = linea_search(1, ORDEN_CRUZADO, None);

    // (1) y (2): dos servidores recién arrancados, sin estado compartido.
    let r1 = roundtrip(dir.path(), &[linea.as_str()], 1);
    let r2 = roundtrip(dir.path(), &[linea.as_str()], 1);
    let e1 = error_de(&r1[0]).unwrap_or_else(|| {
        panic!(
            "sobre una base heterogénea la consulta `{ORDEN_CRUZADO}` devuelve hoy los documentos \
             de `priority` textual y CALLA sobre los numéricos: una lista recortada, decidida \
             documento a documento, que nadie puede distinguir de la correcta.\nRespuesta: {}",
            r1[0]
        )
    });
    let e2 = error_de(&r2[0]).expect("la segunda ejecución debe fallar igual que la primera");
    assert_eq!(
        e1, e2,
        "dos ejecuciones de la misma consulta sobre el mismo workspace deben dar el MISMO error, \
         palabra por palabra"
    );
    juzga_error_de_tipo(&e1, "knowledge_search (base heterogénea)");

    // El documento nombrado es el PRIMERO del orden total que yerra, no el primero del orden
    // (`alfa.md`, que no yerra) ni el último que yerra (`zulu.md`).
    assert!(
        e1.contains("bravo.md"),
        "el error debe nombrar el documento sobre el que se produjo, y ser el PRIMERO del orden \
         total de `Analysis::documents` que yerra: `alfa.md` va antes pero su `priority` es string \
         (no yerra), así que el nombrado es `bravo.md`. Error: «{e1}»"
    );
    assert!(
        !e1.contains("zulu.md"),
        "…y SOLO ese: reportar el último documento que yerra (o todos) hace que el mensaje dependa \
         de dónde paró el motor en vez de del workspace, que es justo el no-determinismo que la \
         historia cierra. Error: «{e1}»"
    );

    // (3) El tamaño de página no puede cambiar el veredicto: si el motor evaluara perezosamente
    //     hasta llenar la página, con `limit: 1` le bastaría `alfa.md` (que casa) para responder
    //     antes de llegar a `bravo.md`, y la MISMA consulta tendría éxito o fracaso según el
    //     `limit` — un resultado que depende de un parámetro invisible para el problema.
    let l_limit = linea_search(1, ORDEN_CRUZADO, Some(1));
    let r3 = roundtrip(dir.path(), &[l_limit.as_str()], 1);
    let e3 = error_de(&r3[0]).unwrap_or_else(|| {
        panic!(
            "la misma consulta con `limit: 1` debe fallar igual: el veredicto de una consulta no \
             puede depender del tamaño de la página.\nRespuesta: {}",
            r3[0]
        )
    });
    assert_eq!(e3, e1, "…y con el mismo texto exacto");

    // (4) Y por `change_plan`, cuyo bucle es OTRO: el documento reportado lo fija el orden total,
    //     no el orden en que el planificador toque los documentos.
    let l_plan = linea_plan(1, ORDEN_CRUZADO);
    let r4 = roundtrip(dir.path(), &[l_plan.as_str()], 1);
    let e4 = error_de(&r4[0]).unwrap_or_else(|| {
        panic!(
            "la selección masiva sobre la base heterogénea debe fallar en vez de planificar sobre \
             los documentos que «sí casaron».\nRespuesta: {}",
            r4[0]
        )
    });
    assert_eq!(
        e4, e1,
        "las dos tools deben nombrar el MISMO documento con el MISMO texto: el criterio es el orden \
         total de `Analysis::documents`, no el orden de evaluación de cada consumidor"
    );
}

/// **E26-H08** — control anti-vacuo: no casar sigue siendo AUSENCIA, no error.
///
/// El arreglo no puede consistir en convertir cualquier resultado vacío en un fallo: `Ok(false)` es
/// exclusión y solo `Err` cambia de tratamiento. El control lleva su propio control: la misma
/// consulta con el valor que SÍ está en los documentos devuelve los dos, de modo que el `[]` de
/// arriba significa «no casa» y no «la búsqueda está rota».
#[test]
fn no_casar_sigue_siendo_ausencia() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "alfa.md",
        "---\nstatus: draft\ntitle: Alfa\n---\n\n# Alfa\n",
    );
    write(
        dir.path(),
        "beta.md",
        "---\nstatus: draft\ntitle: Beta\n---\n\n# Beta\n",
    );

    let l_vacia = linea_search(1, "status = borrador", None);
    let l_control = linea_search(2, "status = draft", None);
    let resp = roundtrip(dir.path(), &[l_vacia.as_str(), l_control.as_str()], 2);

    assert_eq!(
        error_de(&resp[0]),
        None,
        "`status = borrador` sobre documentos con `status: draft` es una comparación PERFECTAMENTE \
         tipada (string vs string) que simplemente no casa: eso es ausencia, no error"
    );
    assert!(
        search_paths(&resp[0]).is_empty(),
        "…y su respuesta es la lista vacía: {:?}",
        search_paths(&resp[0])
    );

    let mut casan = search_paths(&resp[1]);
    casan.sort();
    assert_eq!(
        casan,
        vec!["alfa.md".to_string(), "beta.md".to_string()],
        "control del control: la MISMA maquinaria con el valor que sí está devuelve los dos \
         documentos, así que el `[]` de arriba es un veredicto y no una búsqueda rota"
    );
}

/// **E26-H08** — control anti-vacuo: un campo AUSENTE excluye su documento sin error, como hasta
/// ahora.
///
/// La ausencia cortocircuita antes de comprobar tipos (`campo_inexistente`, E19-H01): no se puede
/// errar sobre un tipo que no se tiene. El documento sin `priority` va PRIMERO en el orden total a
/// propósito: una implementación que abortara ante todo lo que no sea `Ok(true)` moriría en él.
#[test]
fn campo_ausente_no_es_type_error() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a-sin-priority.md",
        "---\nstatus: draft\ntitle: Sin prioridad\n---\n\n# Sin prioridad\n",
    );
    write(
        dir.path(),
        "b-con-priority.md",
        "---\nstatus: draft\npriority: 5\n---\n\n# Con prioridad\n",
    );

    let linea = linea_search(1, "priority >= 3", None);
    let resp = roundtrip(dir.path(), &[linea.as_str()], 1);

    assert_eq!(
        error_de(&resp[0]),
        None,
        "preguntar por una clave que un documento no tiene es legítimo (el frontmatter es metadata \
         arbitraria): la ausencia excluye el documento, no rompe la consulta"
    );
    assert_eq!(
        search_paths(&resp[0]),
        vec!["b-con-priority.md".to_string()],
        "…y el documento que SÍ tiene la clave, con el tipo correcto, sigue casando"
    );
}

/// La llamada JSON-RPC a `knowledge_search` combinando un `text` NO vacío con un `where`.
fn linea_search_con_texto(id: u32, texto: &str, donde: &str) -> String {
    serde_json::json!({"jsonrpc":"2.0","id": id,"method":"tools/call",
        "params":{"name":"knowledge_search","arguments":{"text": texto, "where": donde}}})
    .to_string()
}

/// **E26-H08** — un `text` más estrecho NO puede tapar el error de tipo.
///
/// El resto de tests de la familia usan `text: ""`, así que ninguno fija el ORDEN entre los dos
/// criterios de `knowledge_search`, y el orden es justo lo que decide el alcance del error: si el
/// `text` se aplicase primero, un documento descartado por texto nunca llegaría a evaluarse y su
/// `TypeError` desaparecería. La misma consulta sería entonces legal o ilegal según un parámetro que
/// no habla de tipos, y **añadir palabras a la búsqueda arreglaría la consulta** — el mismo
/// resultado-que-depende-de-lo-invisible que la historia cierra.
///
/// El criterio es que el error es de la CONSULTA («este `where` no es respondible sobre este
/// workspace»), no del subconjunto que el `text` deja pasar: el `where` se evalúa sobre el orden
/// total de `Analysis::documents`, y por eso el veredicto es idéntico —byte a byte— al de `text: ""`.
///
/// Discriminante por construcción: `text: "alfa"` casa **solo** `alfa.md` (que NO yerra: su
/// `priority` es string) y descarta `bravo.md` (que sí yerra, y es el que debe seguir saliendo
/// nombrado).
#[test]
fn el_text_no_tapa_el_type_error() {
    let dir = ws_priority_heterogeneo();

    // Control de la premisa: con ese `text` y SIN `where`, la búsqueda devuelve solo `alfa.md` —de
    // modo que el `text` de verdad descarta a `bravo.md`, y el test no es vacuo.
    let l_solo_texto = serde_json::json!({"jsonrpc":"2.0","id":1,"method":"tools/call",
        "params":{"name":"knowledge_search","arguments":{"text":"alfa"}}})
    .to_string();
    let l_texto_y_where = linea_search_con_texto(2, "alfa", ORDEN_CRUZADO);
    let l_solo_where = linea_search(3, ORDEN_CRUZADO, None);
    let resp = roundtrip(
        dir.path(),
        &[
            l_solo_texto.as_str(),
            l_texto_y_where.as_str(),
            l_solo_where.as_str(),
        ],
        3,
    );

    assert_eq!(
        search_paths(&resp[0]),
        vec!["alfa.md".to_string()],
        "premisa del test: `text: \"alfa\"` acota la búsqueda a `alfa.md` y deja fuera a `bravo.md` \
         (el documento que yerra). Si esto cambiara, el caso de abajo dejaría de discriminar"
    );

    let con_texto = error_de(&resp[1]).unwrap_or_else(|| {
        panic!(
            "un `text` que descarta al documento mal tipado NO puede convertir una consulta \
             imposible en una respuesta: el `where` se evalúa sobre el orden total, no sobre lo que \
             el `text` deje pasar.\nRespuesta: {}",
            resp[1]
        )
    });
    juzga_error_de_tipo(
        &con_texto,
        "knowledge_search (con `text` que excluye el documento)",
    );
    assert!(
        con_texto.contains("bravo.md"),
        "…y sigue nombrando `bravo.md`, aunque el `text` lo hubiera descartado: «{con_texto}»"
    );

    let sin_texto = error_de(&resp[2]).expect("y con `text: \"\"` también falla");
    assert_eq!(
        con_texto, sin_texto,
        "el veredicto y su texto deben ser IDÉNTICOS con y sin `text`: si un `text` más estrecho \
         cambiara el error (o lo hiciera desaparecer), el agente podría «arreglar» una consulta mal \
         tipada añadiendo palabras a la búsqueda"
    );
}

/// **E26-H08** — la otra variante de `TypeError` también aborta: `NotAList`.
///
/// `OrderNotDefined` (el `>=` cruzado) es el generador más probable en una base real, pero el enum
/// del core tiene DOS variantes y las dos llegan por el mismo camino. Sin este caso, la rama
/// `NotAList` del traductor de la fachada no la ejercita nadie: podría no emitir mensaje —o emitir
/// el del orden— y la suite no se enteraría.
///
/// **Ojo con la semántica real de `contains`** (verificada contra el evaluador del core antes de
/// escribir el test, `eval_contains`): sobre un **string** `contains` es SUBCADENA, no error, así
/// que `tags contains "x"` sobre `tags: solo` es `Ok(false)` y **no** dispara nada. `NotAList` sale
/// de los dos casos que este test usa:
///   · `contains` sobre un escalar **no string** (aquí un número);
///   · `contains_any`/`contains_all` sobre cualquier no-lista (aquí un string) — son exclusivos de
///     listas, y es el caso realista de quien escribió un tag suelto sin lista.
#[test]
fn type_error_de_lista_tambien_es_error_de_consulta() {
    let dir = tempfile::tempdir().unwrap();
    // Orden total: `a-lista.md` < `b-escalar.md` < `c-numero.md`. El primero del orden NO yerra en
    // ninguno de los dos casos, así que el documento nombrado no puede salir por accidente.
    write(
        dir.path(),
        "a-lista.md",
        "---\ntags:\n  - uno\n  - dos\n---\n\n# Con lista\n",
    );
    write(
        dir.path(),
        "b-escalar.md",
        "---\ntags: solo\n---\n\n# Tag suelto\n",
    );
    write(
        dir.path(),
        "c-numero.md",
        "---\npriority: 2\n---\n\n# Número\n",
    );

    let l_contains_num = linea_search(1, "priority contains \"2\"", None);
    let l_contains_any = linea_search(2, "tags contains_any [\"uno\"]", None);
    let l_subcadena = linea_search(3, "tags contains \"sol\"", None);
    let resp = roundtrip(
        dir.path(),
        &[
            l_contains_num.as_str(),
            l_contains_any.as_str(),
            l_subcadena.as_str(),
        ],
        3,
    );

    // (1) `contains` sobre un número: el operador de lista sobre un escalar no-string.
    let e_num = error_de(&resp[0]).unwrap_or_else(|| {
        panic!(
            "`priority contains \"2\"` sobre `priority: 2` es `TypeError::NotAList` en el core: un \
             operador de lista sobre un número. Debe abortar la consulta igual que el orden \
             cruzado, no devolver una lista recortada.\nRespuesta: {}",
            resp[0]
        )
    });
    let (codigo, mensaje) = codigo_y_mensaje(&e_num)
        .unwrap_or_else(|| panic!("debe emitir «CÓDIGO: mensaje»: «{e_num}»"));
    assert_eq!(
        codigo, "INVALID_SCHEMA",
        "mismo código que el otro TypeError"
    );
    assert!(
        menciona(&mensaje.to_lowercase(), "priority"),
        "el mensaje debe nombrar el campo: «{e_num}»"
    );
    assert!(
        nombra_tipo(mensaje, GRAFIAS_NUMBER),
        "…y el tipo REAL del campo (number), que es lo que le dice al agente por qué su `contains` \
         no aplica: «{e_num}»"
    );
    assert!(
        menciona(&mensaje.to_lowercase(), "contains"),
        "…y el operador que lo exigía: «{e_num}»"
    );
    assert!(
        e_num.contains("c-numero.md"),
        "…y el documento donde chocó: «{e_num}»"
    );

    // (2) `contains_any` sobre un string: exclusivo de listas. El primero del orden (`a-lista.md`)
    //     casa sin errar, así que el nombrado es el segundo.
    let e_any = error_de(&resp[1]).unwrap_or_else(|| {
        panic!(
            "`tags contains_any [\"uno\"]` sobre `tags: solo` (string) es `NotAList`: \
             `contains_any` es exclusivo de listas. Hasta v0.4.0 ese documento se excluía en \
             silencio, así que la respuesta era la lista de los que SÍ tenían lista.\nRespuesta: {}",
            resp[1]
        )
    });
    let (codigo, mensaje) = codigo_y_mensaje(&e_any)
        .unwrap_or_else(|| panic!("debe emitir «CÓDIGO: mensaje»: «{e_any}»"));
    assert_eq!(codigo, "INVALID_SCHEMA", "mismo código");
    assert!(
        menciona(&mensaje.to_lowercase(), "tags") && nombra_tipo(mensaje, GRAFIAS_STRING),
        "el mensaje debe nombrar el campo y su tipo real (string): «{e_any}»"
    );
    assert!(
        menciona(&mensaje.to_lowercase(), "contains_any"),
        "…y el operador exacto, que es distinto del `contains` a secas: «{e_any}»"
    );
    assert!(
        e_any.contains("b-escalar.md") && !e_any.contains("c-numero.md"),
        "…y el documento donde chocó, que es el primero del orden total que yerra (`a-lista.md` va \
         antes y casa sin errar): «{e_any}»"
    );

    // (3) Control anti-vacuo: `contains` sobre un STRING es subcadena, no error. El arreglo no
    //     puede consistir en hacer ilegal todo `contains` sobre lo que no sea una lista.
    assert_eq!(
        error_de(&resp[2]),
        None,
        "`tags contains \"sol\"` sobre `tags: solo` es SUBCADENA (el tipo del campo decide el \
         significado del operador, `eval_contains`): eso no es un error de tipo"
    );
    assert_eq!(
        search_paths(&resp[2]),
        vec!["b-escalar.md".to_string()],
        "…y casa el documento del tag suelto, sin que la lista de `a-lista.md` (que no contiene la \
         subcadena) ni la ausencia de `tags` en `c-numero.md` lo estropeen"
    );
}

// ---------------------------------------------------------------------------
// E29-H04 — `starts_with`/`ends_with` sobre un campo no-string es TYPE ERROR (por el wire)
//
// `requirements/epica-29-honestidad-superficie.md §E29-H04` · `decisiones §23/A-04` (criterio
// **ratificado por el usuario el 2026-08-06**) · caso **G1-20** del testbench homelab
// (`docs/qa/informe-homelab-2026-08-06.md §3`), que es como se observó el hallazgo: por el wire.
//
// SÍNTOMA medido hoy (v0.5.0) sobre un workspace con `priority: 3` (número):
//   · `knowledge_search {where: "priority starts_with \"3\""}` → `{"results": [], …}` **sin error**;
//   · `change_plan {selection: {where: <lo mismo>}}` → un plan con `normalizedOperations: []`,
//     `impact.affectedCount: 0` y **`canApply: true`**, presentado como un plan legítimo.
// En el homelab eran 7 documentos con `priority: 3` desapareciendo de la respuesta sin un aviso.
//
// CAUSA: `core::eval::eval_afijo` devuelve `bool` (no `Result`), así que un campo no-string es
// `false` y cae en el mismo `continue` que «no casa» — el defecto que E26-H08 cerró para el ORDEN,
// abierto todavía para los dos operadores de afijo.
//
// LO QUE ESTOS TESTS FIJAN, y por qué aquí y no solo en el core: el criterio de la historia es que el
// error llegue al agente por las DOS superficies que evalúan consultas, con el `INVALID_SCHEMA` del
// catálogo (no hay código nuevo) y con un mensaje que nombre campo, operador y tipo hallado —el
// mismo contrato de redacción que `juzga_error_de_tipo` ya exige para `OrderNotDefined`—, y que en
// `change_plan` **aborte el plan** en vez de reducir la selección en silencio (coherencia con
// E26-H08). El core solo puede probar que el evaluador yerra; que ese `Err` no se pierda entre el
// evaluador y el wire solo se ve desde aquí.
//
// ROJO esperado HOY: por ASERCIÓN (`error_de` devuelve `None` porque la respuesta es un éxito).
// ---------------------------------------------------------------------------

/// La consulta del caso **G1-20**: operador de afijo sobre un campo numérico.
const AFIJO_SOBRE_NUMERO: &str = "priority starts_with \"3\"";

/// Grafías admisibles del tipo `list` en el mensaje (las de `GRAFIAS_NUMBER`/`GRAFIAS_STRING`, para
/// el criterio de la lista).
const GRAFIAS_LIST: &[&str] = &["list", "lista", "array", "secuencia", "sequence"];

/// Juzga el error de un type error de **afijo**: código estable + mensaje que permita CORREGIR la
/// consulta (campo, operador y el tipo real del campo). Es el gemelo de `juzga_error_de_tipo`, con
/// el tipo esperado parametrizado porque el defecto se manifiesta sobre varias familias.
fn juzga_error_de_afijo(err: &str, campo: &str, operador: &str, grafias: &[&str], contexto: &str) {
    assert_eq!(
        codigo_de(err),
        "INVALID_SCHEMA",
        "un type error de afijo es el MISMO tipo de fallo que el del orden y usa el MISMO código del \
         catálogo —la historia no abre `ErrorCode` ({contexto})—: «{err}»"
    );
    let (_, mensaje) = codigo_y_mensaje(err)
        .unwrap_or_else(|| panic!("{contexto} debe emitir «CÓDIGO: mensaje» (E26-H07): «{err}»"));
    let bajo = mensaje.to_lowercase();
    assert!(
        menciona(&bajo, campo),
        "el mensaje debe NOMBRAR el campo que choca ({contexto}): «{err}»"
    );
    assert!(
        nombra_tipo(mensaje, grafias),
        "…y el tipo REAL que el campo tiene en el documento, que es lo que le dice al agente por qué \
         su operador de texto no aplica ({contexto}): «{err}»"
    );
    assert!(
        menciona(&bajo, operador),
        "…y el operador exacto: `starts_with` y `ends_with` fallan por la misma razón pero el agente \
         corrige uno u otro ({contexto}): «{err}»"
    );
}

/// Workspace del caso **G1-20**: `priority` numérico en dos documentos y **textual** en un tercero,
/// más un `tags` lista.
///
/// La heterogeneidad es la que hace grave el defecto y discriminante el test: hoy
/// `priority starts_with "3"` NO devuelve la lista vacía, devuelve `["z-textual.md"]` — una respuesta
/// recortada y perfectamente creíble. `a-numerico.md` va primero en el orden total (`§20.7`), así que
/// es el documento que el criterio de determinismo de E26-H08 obliga a nombrar.
fn ws_afijo_heterogeneo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a-numerico.md",
        "---\npriority: 3\nstatus: active\ntags:\n  - uno\n---\n\n# Numérico\n",
    );
    write(
        dir.path(),
        "b-numerico.md",
        "---\npriority: 5\nstatus: draft\n---\n\n# Otro numérico\n",
    );
    write(
        dir.path(),
        "z-textual.md",
        "---\npriority: \"3-alta\"\nstatus: activo\n---\n\n# Textual\n",
    );
    dir
}

/// **E29-H04** · Criterio `starts_with_sobre_numero_es_type_error` (por el wire):
/// **Dado** un workspace con documentos cuyo `priority` es un **número**, **Cuando** se busca con
/// `where: "priority starts_with \"3\""`, **Entonces** la respuesta es un error `INVALID_SCHEMA` cuyo
/// mensaje nombra el campo, el operador y el tipo encontrado.
///
/// Es el caso G1-20 tal como se observó: por `knowledge_search`, con la respuesta recortada como
/// única señal (y ninguna señal, por tanto).
#[test]
fn starts_with_sobre_numero_es_type_error() {
    let dir = ws_afijo_heterogeneo();
    let linea = linea_search(1, AFIJO_SOBRE_NUMERO, None);
    let resp = roundtrip(dir.path(), &[linea.as_str()], 1);

    let err = error_de(&resp[0]).unwrap_or_else(|| {
        panic!(
            "`{AFIJO_SOBRE_NUMERO}` sobre documentos con `priority` NUMÉRICO no es una consulta \
             respondible: `starts_with` es un operador de TEXTO y el lenguaje no coerce (§20.8). Hoy \
             `eval_afijo` devuelve `false` para todo campo no-string, así que los documentos \
             numéricos se excluyen en silencio y la respuesta es la lista de los que casualmente \
             tenían `priority` textual — el caso G1-20, donde 7 documentos con `priority: 3` \
             desaparecieron sin un aviso.\nRespuesta recibida: {}",
            resp[0]
        )
    });
    juzga_error_de_afijo(
        &err,
        "priority",
        "starts_with",
        GRAFIAS_NUMBER,
        "knowledge_search",
    );
    assert!(
        err.contains("a-numerico.md"),
        "…y debe nombrar el PRIMER documento del orden total que yerra (`a-numerico.md`), como todo \
         type error desde E26-H08: «{err}»"
    );
}

/// **E29-H04** · Criterio `ends_with_sobre_numero_es_type_error` (por el wire):
/// **Dado** ese mismo workspace, **Cuando** se busca con `ends_with` sobre el mismo campo,
/// **Entonces** el mismo error.
#[test]
fn ends_with_sobre_numero_es_type_error() {
    let dir = ws_afijo_heterogeneo();
    let linea = linea_search(1, "priority ends_with \"3\"", None);
    let resp = roundtrip(dir.path(), &[linea.as_str()], 1);

    let err = error_de(&resp[0]).unwrap_or_else(|| {
        panic!(
            "`priority ends_with \"3\"` debe fallar igual que su gemelo `starts_with`: comparten \
             `eval_afijo`, y arreglar uno solo dejaría el hueco abierto por la mitad.\nRespuesta: {}",
            resp[0]
        )
    });
    juzga_error_de_afijo(
        &err,
        "priority",
        "ends_with",
        GRAFIAS_NUMBER,
        "knowledge_search (ends_with)",
    );
}

/// **E29-H04** · Criterio `starts_with_sobre_lista_es_type_error` (por el wire):
/// **Dado** un documento cuyo campo `tags` es una **lista**, **Cuando** se evalúa
/// `tags starts_with "x"`, **Entonces** es type error (no `false`).
///
/// La lista es la familia no-string más frecuente en un frontmatter real después del número, y la
/// más tentadora para una coerción («¿y si comparo el primer elemento?»): el mensaje debe decir
/// `list`, no `string`, o el agente corregirá lo que no es.
#[test]
fn starts_with_sobre_lista_es_type_error() {
    let dir = ws_afijo_heterogeneo();
    let linea = linea_search(1, "tags starts_with \"uno\"", None);
    let resp = roundtrip(dir.path(), &[linea.as_str()], 1);

    let err = error_de(&resp[0]).unwrap_or_else(|| {
        panic!(
            "`tags starts_with \"uno\"` sobre `tags: [uno, dos]` debe ser type error: una lista no \
             tiene prefijo de texto, y que su primer elemento sí lo tenga es justo la coerción que \
             §20.8 prohíbe.\nRespuesta: {}",
            resp[0]
        )
    });
    juzga_error_de_afijo(
        &err,
        "tags",
        "starts_with",
        GRAFIAS_LIST,
        "knowledge_search (lista)",
    );
}

/// **E29-H04** · Criterio `selection_con_type_error_de_afijo_aborta_el_plan`:
/// **Dado** un `change_plan` con `selection.where` que produce el type error, **Cuando** se
/// planifica, **Entonces** el plan **aborta** con `INVALID_SCHEMA` y **no se expande a ninguna
/// operación** (coherencia con E26-H08).
///
/// Es la mitad cara del defecto: hoy `change_plan` con esta selección devuelve un plan con
/// `normalizedOperations: []`, `impact.affectedCount: 0` y **`canApply: true`** — un plan vacío
/// presentado como legítimo, que el agente puede aplicar creyendo que tocó los 7 documentos que
/// buscaba. Y en el fixture heterogéneo es peor que vacío: planifica sobre `z-textual.md`, el único
/// que casualmente casó.
///
/// El test aserta además la **igualdad exacta** con el error de `knowledge_search`: las dos tools
/// comparten el lenguaje (`§20.10`), así que deben compartir veredicto **y** redacción, como ya
/// exige `misma_consulta_mismo_error_en_search_y_en_plan` para el orden.
#[test]
fn selection_con_type_error_de_afijo_aborta_el_plan() {
    let dir = ws_afijo_heterogeneo();
    let l_search = linea_search(1, AFIJO_SOBRE_NUMERO, None);
    let l_plan = linea_plan(2, AFIJO_SOBRE_NUMERO);
    let resp = roundtrip(dir.path(), &[l_search.as_str(), l_plan.as_str()], 2);

    let e_plan = error_de(&resp[1]).unwrap_or_else(|| {
        let sc = &resp[1]["result"]["structuredContent"];
        panic!(
            "`change_plan` con `selection.where: {AFIJO_SOBRE_NUMERO}` debe ABORTAR: hoy \
             `expand_selection` se salta en silencio todo documento cuya evaluación no sea \
             `Ok(true)`, así que planifica sobre el subconjunto que casualmente casó y lo presenta \
             como un plan legítimo (`canApply: {}`, `affectedCount: {}`). Un plan que afecta a menos \
             ficheros de los que el agente seleccionó es la versión cara de la respuesta \
             silenciosamente equivocada.\nRespuesta: {}",
            sc["canApply"], sc["impact"]["affectedCount"], resp[1]
        )
    });
    juzga_error_de_afijo(
        &e_plan,
        "priority",
        "starts_with",
        GRAFIAS_NUMBER,
        "change_plan",
    );

    let e_search = error_de(&resp[0]).unwrap_or_else(|| {
        panic!(
            "`knowledge_search` con la misma consulta debe fallar también (ver \
             `starts_with_sobre_numero_es_type_error`): {}",
            resp[0]
        )
    });
    assert_eq!(
        e_search, e_plan,
        "el MISMO `where` sobre el MISMO workspace debe dar el MISMO código y el MISMO mensaje por \
         las dos tools que aceptan el lenguaje (§20.10, invariante #3)"
    );
}

/// **E29-H04** · Control anti-vacuo: `starts_with` sobre un campo **string** sigue casando, y un
/// campo **ausente** sigue excluyendo sin error.
///
/// Los dos criterios anti-vacuo de la historia juntos, por el wire, que es donde importa que el
/// operador siga siendo usable: el arreglo no puede consistir en hacer ilegal `starts_with`, ni en
/// convertir en error toda consulta sobre un frontmatter heterogéneo (que es la norma: la mitad de
/// los documentos de cualquier base real no tienen la clave por la que se pregunta).
///
/// **Verde hoy**, y debe seguir verde.
#[test]
fn starts_with_sobre_string_y_campo_ausente_siguen_funcionando() {
    let dir = ws_afijo_heterogeneo();
    let l_string = linea_search(1, "status starts_with \"act\"", None);
    let l_ausente = linea_search(2, "inexistente starts_with \"x\"", None);
    let l_no_casa = linea_search(3, "status starts_with \"zzz\"", None);
    let resp = roundtrip(
        dir.path(),
        &[l_string.as_str(), l_ausente.as_str(), l_no_casa.as_str()],
        3,
    );

    assert_eq!(
        error_de(&resp[0]),
        None,
        "`status starts_with \"act\"` es una comparación perfectamente tipada (string vs string): no \
         puede convertirse en error"
    );
    let mut casan = search_paths(&resp[0]);
    casan.sort();
    assert_eq!(
        casan,
        vec!["a-numerico.md".to_string(), "z-textual.md".to_string()],
        "…y sigue casando los documentos cuyo `status` empieza por «act» (`active` y `activo`), que \
         es la prueba de que el operador conserva su trabajo"
    );

    assert_eq!(
        error_de(&resp[1]),
        None,
        "un campo que NINGÚN documento tiene no es un type error: la ausencia cortocircuita antes de \
         mirar tipos (E19-H01) y ese contrato no cambia"
    );
    assert!(
        search_paths(&resp[1]).is_empty(),
        "…y su respuesta es la lista vacía: {:?}",
        search_paths(&resp[1])
    );

    assert_eq!(
        error_de(&resp[2]),
        None,
        "y un `false` legítimo —string que no empieza por ese prefijo— sigue siendo ausencia de \
         resultados, no un fallo"
    );
    assert!(
        search_paths(&resp[2]).is_empty(),
        "…con su lista vacía: {:?}",
        search_paths(&resp[2])
    );
}

// ---------------------------------------------------------------------------
// E26-H09 — `metadata_inspect` habla el mismo dialecto de dot-paths que la consulta
//
// `App::metadata_inspect` normaliza su `field` con `FieldPath::parse`, mientras `where`, `filter` y
// `has`/`missing` pasan todos por `core::parse::build_field_path` (E24-H07/H08). Dos dialectos para
// el mismo texto, con tres consecuencias que estos tests reproducen por el wire:
//   · `field: "frontmatter.graph.backlinks"` —la sintaxis que el propio mensaje de error del parser
//     recomienda— busca una clave de primer nivel llamada `frontmatter` y devuelve `presentIn: 0`:
//     silenciosamente equivocado sobre un dato que SÍ existe;
//   · `field: "graph.backlinks"` inspecciona la clave del frontmatter, mientras el mismo texto en un
//     `where` consulta el GRAFO: el mismo dot-path significa dos cosas según la tool;
//   · `field: "frontmatter.status"` (la abreviatura legal del lenguaje) devuelve `presentIn: 0`
//     sobre una base llena de `status`.
//
// Lo que la historia decide, y estos tests clavan: `metadata_inspect` hereda las TRES reglas del
// lenguaje (abreviatura, anclaje y rechazo bajo namespace reservado) y, además, un namespace
// reservado VÁLIDO no es inspeccionable —`metadata_inspect` describe metadata, y una propiedad
// calculada no vive en ningún frontmatter—, con un mensaje que dice por dónde sí (`graph_query` o
// el anclaje `frontmatter.`).
// ---------------------------------------------------------------------------

/// Workspace con una clave de frontmatter que **colisiona** con un namespace reservado
/// (`graph.backlinks`, con el valor 7) más un `status` normal en 2 de los 3 documentos.
///
/// Discriminante por diseño: el 7 del frontmatter no coincide con ningún backlink real del grafo
/// (los tres documentos están aislados, 0 backlinks), así que una respuesta que venga del grafo no
/// puede confundirse con una que venga del frontmatter.
fn ws_reservado_en_frontmatter() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "alfa.md",
        "---\nstatus: draft\ngraph:\n  backlinks: 7\ndocument:\n  path: falso.md\n---\n\n# Alfa\n",
    );
    write(
        dir.path(),
        "bravo.md",
        "---\nstatus: draft\n---\n\n# Bravo\n",
    );
    write(dir.path(), "charlie.md", "# Charlie\n\nsin frontmatter.\n");
    dir
}

/// La llamada JSON-RPC a `metadata_inspect` en modo `field`.
fn linea_inspect(id: u32, field: &str) -> String {
    linea_call(
        id,
        "metadata_inspect",
        serde_json::json!({"mode": "field", "field": field}),
    )
}

/// **E26-H09** · Criterio `anclaje_frontmatter_alcanza_la_clave_reservada`:
/// **Dado** un documento con frontmatter `graph: {backlinks: 7}`, **Cuando** se llama a
/// `metadata_inspect{mode:"field", field:"frontmatter.graph.backlinks"}`, **Entonces** devuelve
/// `presentIn: 1` con el valor `7`.
///
/// Hoy devuelve `presentIn: 0`: `FieldPath::parse` no conoce el anclaje de E24-H08, así que el path
/// se resuelve como la clave literal `frontmatter` → `graph` → `backlinks`, que no existe. Es la
/// misma clase de defecto —una respuesta silenciosamente equivocada— que E24-H08 retiró del
/// lenguaje de consulta, sobreviviendo en la tool que existe para descubrir la base.
#[test]
fn anclaje_frontmatter_alcanza_la_clave_reservada() {
    let dir = ws_reservado_en_frontmatter();
    let resp = roundtrip(
        dir.path(),
        &[linea_inspect(1, "frontmatter.graph.backlinks").as_str()],
        1,
    );
    assert_eq!(
        error_de(&resp[0]),
        None,
        "`frontmatter.` es la sintaxis que el propio parser recomienda para alcanzar una clave \
         homónima de un namespace: no puede fallar"
    );
    let sc = &resp[0]["result"]["structuredContent"];

    assert_eq!(
        sc["presentIn"].as_u64(),
        Some(1),
        "`frontmatter.graph.backlinks` debe alcanzar la clave del USUARIO —el `backlinks: 7` de \
         `alfa.md`—, no una clave de primer nivel llamada literalmente `frontmatter`. Hasta v0.4.0 \
         esto devolvía `presentIn: 0` sobre un dato que existe: {resp:?}"
    );
    assert_eq!(
        sc["missingIn"].as_u64(),
        Some(2),
        "…y los otros 2 documentos siguen contando como ausencia (presentIn + missingIn == 3): \
         {resp:?}"
    );

    let values = sc["values"].as_array().unwrap_or_else(|| {
        panic!("la inspección debe traer `values` (array de {{value, count}}): {resp:?}")
    });
    assert_eq!(
        values,
        &vec![serde_json::json!({"value": 7, "count": 1})],
        "…con el valor 7 en su tipo JSON natural (número, sin coerción) y conteo 1: es el dato que \
         hoy es inalcanzable por esta tool: {resp:?}"
    );
    assert_eq!(
        sc["inferredTypes"]["number"].as_u64(),
        Some(1),
        "…y su tipo observado es `number`: {resp:?}"
    );
}

/// **E26-H09** · Criterio `namespace_reservado_no_es_inspeccionable`:
/// **Dado** ese mismo documento, **Cuando** se llama con `field:"graph.backlinks"`, **Entonces**
/// falla con `INVALID_SCHEMA` y el mensaje apunta al anclaje y a `graph_query`.
///
/// Hoy devuelve una inspección: la de la clave del frontmatter. O sea que el MISMO texto significa
/// «el grafo» en `where` y «mi clave `graph`» en `metadata_inspect`. Se rechaza (y no se reinterpreta
/// como el grafo) porque `metadata_inspect` describe **metadata**: una propiedad calculada no vive
/// en ningún frontmatter y no tiene `presentIn`/`missingIn` que describir.
///
/// Tres casos, uno por regla: el namespace reservado VÁLIDO (`graph.backlinks`, `document.path`) y
/// la propiedad DESCONOCIDA bajo namespace reservado (`graph.backlink`, con typo), que E24-H07 ya
/// declaró error en el lenguaje y que hoy esta tool contesta con `presentIn: 0`.
#[test]
fn namespace_reservado_no_es_inspeccionable() {
    let dir = ws_reservado_en_frontmatter();
    let campos = ["graph.backlinks", "document.path", "graph.backlink"];
    let lineas: Vec<String> = campos
        .iter()
        .enumerate()
        .map(|(i, f)| linea_inspect(i as u32 + 1, f))
        .collect();
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip(dir.path(), &refs, campos.len());

    for (i, campo) in campos.iter().enumerate() {
        let err = error_de(&resp[i]).unwrap_or_else(|| {
            panic!(
                "`field: \"{campo}\"` no es inspeccionable: `{campo}` nombra una propiedad \
                 CALCULADA (o una que no existe bajo un namespace reservado), y `metadata_inspect` \
                 describe metadata de frontmatter. Hasta v0.4.0 esta llamada devolvía una \
                 inspección —la de la clave homónima del usuario, o `presentIn: 0`—, así que el \
                 mismo dot-path significaba una cosa aquí y otra en `where`.\nRespuesta recibida: {}",
                resp[i]
            )
        });
        assert_eq!(
            codigo_de(&err),
            "INVALID_SCHEMA",
            "el `field` pedido no es un campo de metadata: es un error de entrada de la tool \
             («{campo}»): «{err}»"
        );
        let (_, mensaje) = codigo_y_mensaje(&err).unwrap_or_else(|| {
            panic!("debe emitir «CÓDIGO: mensaje» (E26-H07) para «{campo}»: «{err}»")
        });
        // La forma ANCLADA COMPLETA, no la palabra «frontmatter» suelta: el mensaje habla de
        // frontmatter varias veces (explicando que las propiedades calculadas no viven en él), así
        // que una aserción por token la satisface de rebote y sobreviviría a borrar la salida. Lo
        // que el criterio exige es que el mensaje deletree el texto que el agente tiene que
        // TECLEAR. Se comprueba por subcadena y no con `menciona` porque el anclaje lleva puntos, y
        // el punto es separador de tokens.
        let anclado = format!("frontmatter.{campo}");
        assert!(
            mensaje.contains(&anclado),
            "…y el mensaje debe deletrear la salida —el anclaje «{anclado}»—, no limitarse a \
             mencionar el frontmatter: es la mitad del diagnóstico que convierte el rechazo en una \
             instrucción ejecutable: «{err}»"
        );
    }

    // El caso del GRAFO tiene además una segunda salida que el mensaje debe nombrar: la tool que sí
    // responde por las propiedades calculadas.
    let err_grafo = error_de(&resp[0]).expect("caso `graph.backlinks`");
    assert!(
        menciona(&err_grafo.to_lowercase(), "graph_query"),
        "…y para un namespace `graph.*` válido, el mensaje debe remitir a `graph_query`: el agente \
         que preguntó por los backlinks reales tiene que salir de aquí sabiendo dónde \
         preguntarlos: «{err_grafo}»"
    );

    // Control anti-vacuo: el rechazo es del NAMESPACE, no de todo lo que lleve puntos. Un path
    // anidado normal se sigue inspeccionando.
    let ok = roundtrip(
        dir.path(),
        &[linea_inspect(1, "frontmatter.graph.backlinks").as_str()],
        1,
    );
    assert_eq!(
        error_de(&ok[0]),
        None,
        "el anclaje explícito sigue siendo la vía legal a esa misma clave: {:?}",
        ok[0]
    );
}

/// **E26-H09** · Criterio `la_abreviatura_vale_tambien_en_metadata_inspect`:
/// **Dado** un documento con `status: draft`, **Cuando** se llama con `field:"frontmatter.status"` y
/// con `field:"status"`, **Entonces** las dos respuestas son **idénticas**.
///
/// Hoy la primera devuelve `presentIn: 0` (busca la clave literal `frontmatter`). La abreviatura es
/// legal en `where`/`filter`/`has` desde E19-H02, y el `include` de `knowledge_search` incluso la
/// EXIGE (`frontmatter.status`), así que un agente que use ese mismo texto contra
/// `metadata_inspect` obtiene hoy una respuesta vacía sin un solo aviso.
///
/// La igualdad se asevera sobre el `structuredContent` **entero**, lo que fija también que el
/// `field` que ecoa la respuesta sea el path NORMALIZADO (el mismo para las dos entradas) y no el
/// texto tal cual se tecleó: si el eco variase, dos respuestas «equivalentes» no serían idénticas y
/// el agente no podría cotejarlas.
#[test]
fn la_abreviatura_vale_tambien_en_metadata_inspect() {
    let dir = ws_reservado_en_frontmatter();
    let resp = roundtrip(
        dir.path(),
        &[
            linea_inspect(1, "frontmatter.status").as_str(),
            linea_inspect(2, "status").as_str(),
        ],
        2,
    );
    assert_eq!(
        error_de(&resp[0]),
        None,
        "`frontmatter.status` es la abreviatura legal del lenguaje desde E19-H02: {:?}",
        resp[0]
    );
    let anclado = &resp[0]["result"]["structuredContent"];
    let desnudo = &resp[1]["result"]["structuredContent"];

    assert_eq!(
        anclado, desnudo,
        "`frontmatter.status` y `status` son el MISMO campo en el lenguaje de consulta, así que \
         `metadata_inspect` debe responder exactamente lo mismo. Hasta v0.4.0 la primera forma \
         devolvía `presentIn: 0`"
    );

    // No vacuo: dos respuestas vacías o dos errores también serían «idénticas».
    assert_eq!(
        desnudo["presentIn"].as_u64(),
        Some(2),
        "`status` está en 2 de los 3 documentos (el tercero no tiene frontmatter): {resp:?}"
    );
    assert_eq!(
        desnudo["values"],
        serde_json::json!([{"value": "draft", "count": 2}]),
        "…con `draft` como único valor del vocabulario: {resp:?}"
    );
}

/// **E26-H09** · Criterio `dot_path_invalido_sigue_rechazandose` (control anti-vacuo):
/// **Dado** un `field` con un dot-path inválido (`"a..b"`, `"service."`), **Cuando** se llama,
/// **Entonces** sigue siendo `INVALID_SCHEMA`.
///
/// Cambiar de normalizador no puede abrir la puerta a paths que hoy se rechazan: un segmento vacío
/// no construye un `FieldPath` en NINGUNO de los dos dialectos. Este test pasa ya hoy, y está para
/// que el arreglo no consista en dejar de validar.
#[test]
fn dot_path_invalido_sigue_rechazandose() {
    let dir = ws_reservado_en_frontmatter();
    let campos = ["a..b", "service.", ".status", ""];
    let lineas: Vec<String> = campos
        .iter()
        .enumerate()
        .map(|(i, f)| linea_inspect(i as u32 + 1, f))
        .collect();
    let refs: Vec<&str> = lineas.iter().map(String::as_str).collect();
    let resp = roundtrip(dir.path(), &refs, campos.len());

    for (i, campo) in campos.iter().enumerate() {
        let err = error_de(&resp[i]).unwrap_or_else(|| {
            panic!(
                "`field: \"{campo}\"` tiene un segmento vacío y no es un dot-path válido: {}",
                resp[i]
            )
        });
        assert_eq!(
            codigo_de(&err),
            "INVALID_SCHEMA",
            "un dot-path malformado es un error de entrada de la tool («{campo}»): «{err}»"
        );
        assert!(
            codigo_y_mensaje(&err).is_some(),
            "…y sigue viniendo con mensaje (E26-H07), no como código pelado: «{err}»"
        );
    }
}

/// **E26-H09** — el borde que destapó la revisión: una clave de PRIMER NIVEL llamada literalmente
/// `frontmatter`.
///
/// El catálogo la anuncia con su nombre literal (`frontmatter.status`), porque eso es lo que `walk`
/// emite y anclarla no la arreglaría; pero el lenguaje lee ese mismo texto como el **anclaje**
/// (E24-H08), así que `mode:"field"` resolvería la clave `status` del usuario —que aquí no existe—
/// y contestaría `presentIn: 0`: otra vez una respuesta silenciosamente equivocada sobre un dato
/// que sí está en el disco. La tool lo dice en voz alta.
///
/// Y el rechazo no es un callejón sin salida: el `include` de `knowledge_search` exige el prefijo
/// `frontmatter.` y parsea el sufijo **literalmente**, así que `frontmatter.frontmatter.status` sí
/// lee el valor. Este test comprueba que la salida que el mensaje promete **funciona de verdad**
/// (una instrucción que no se ejecuta es peor que ninguna).
///
/// Segunda mitad, para que el ruido no se coma la señal: cuando el anclaje **sí** resuelve —hay un
/// `status` de verdad en la base—, manda la resolución anclada y la inspección es normal. La
/// ambigüedad solo se reporta en el caso vacío, que es el único en el que la respuesta sería
/// engañosa.
#[test]
fn clave_frontmatter_literal_colisiona_con_ruido() {
    // --- (A) La clave literal a solas: el anclaje no resuelve nada, así que la tool lo reporta ---
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "raro.md",
        "---\nfrontmatter:\n  status: raro\n---\n\n# Raro\n",
    );
    write(dir.path(), "otro.md", "# Otro\n\nsin frontmatter.\n");

    let resp = roundtrip(
        dir.path(),
        &[
            linea_call(
                1,
                "metadata_inspect",
                serde_json::json!({"mode": "catalog"}),
            )
            .as_str(),
            linea_inspect(2, "frontmatter.status").as_str(),
            linea_call(
                3,
                "knowledge_search",
                serde_json::json!({"text": "", "include": ["frontmatter.frontmatter.status"]}),
            )
            .as_str(),
        ],
        3,
    );

    // 1) El catálogo la anuncia con su nombre literal: el dato existe y es descubrible.
    let nombres: Vec<String> = resp[0]["result"]["structuredContent"]["fields"]
        .as_array()
        .unwrap_or_else(|| panic!("el catálogo devuelve `fields`: {:?}", resp[0]))
        .iter()
        .filter_map(|f| f["name"].as_str().map(str::to_string))
        .collect();
    assert!(
        nombres.iter().any(|n| n == "frontmatter.status"),
        "el catálogo anuncia la clave literal tal cual (`walk` la emite así, y anclarla no la haría \
         direccionable): {nombres:?}"
    );

    // 2) Inspeccionarla por ese nombre NO puede contestar `presentIn: 0`: es el defecto que la
    //    épica retira. Se reporta la ambigüedad, con la salida real.
    let err = error_de(&resp[1]).unwrap_or_else(|| {
        panic!(
            "«frontmatter.status» es AMBIGUO en esta base: el catálogo lo anuncia (viene de una \
             clave de primer nivel llamada literalmente `frontmatter`) y el lenguaje lo lee como el \
             anclaje a una clave `status` que aquí no existe. Contestar `presentIn: 0` sería \
             silenciosamente equivocado sobre un dato que está en el disco.\nRespuesta recibida: {}",
            resp[1]
        )
    });
    assert_eq!(
        codigo_de(&err),
        "INVALID_SCHEMA",
        "la ambigüedad es del `field` que se pidió: «{err}»"
    );
    let (_, mensaje) = codigo_y_mensaje(&err)
        .unwrap_or_else(|| panic!("debe emitir «CÓDIGO: mensaje» (E26-H07): «{err}»"));
    assert!(
        menciona(&mensaje.to_lowercase(), "knowledge_search"),
        "…y el mensaje debe nombrar la tool por la que ese valor SÍ se lee (`knowledge_search`, con \
         su `include`): un rechazo sin salida deja al agente sin nada que hacer: «{err}»"
    );
    assert!(
        mensaje.contains("frontmatter.frontmatter.status"),
        "…deletreando el texto exacto que hay que teclear (el prefijo obligatorio del `include` más \
         la clave literal), no solo el nombre de la tool: «{err}»"
    );

    // 3) …y la salida que promete FUNCIONA: el `include` proyecta el valor de la clave literal.
    let hits = search_paths_values(&resp[2]);
    let raro = hits
        .iter()
        .find(|r| r["path"] == "raro.md")
        .unwrap_or_else(|| panic!("`raro.md` debe estar entre los resultados: {hits:?}"));
    assert_eq!(
        raro["frontmatter"],
        serde_json::json!({"frontmatter.status": "raro"}),
        "el `include` exige el prefijo `frontmatter.` y parsea el sufijo LITERALMENTE, así que \
         «frontmatter.frontmatter.status» lee la clave anidada bajo la clave literal `frontmatter` \
         y la proyecta con la clave pedida: {raro:?}"
    );

    // --- (B) Con un `status` de verdad en la base, manda el anclaje y no hay ambigüedad ---
    let dir2 = tempfile::tempdir().unwrap();
    write(
        dir2.path(),
        "raro.md",
        "---\nfrontmatter:\n  status: raro\n---\n\n# Raro\n",
    );
    write(
        dir2.path(),
        "normal.md",
        "---\nstatus: draft\n---\n\n# Normal\n",
    );
    let resp2 = roundtrip(
        dir2.path(),
        &[linea_inspect(1, "frontmatter.status").as_str()],
        1,
    );

    assert_eq!(
        error_de(&resp2[0]),
        None,
        "cuando el anclaje SÍ resuelve, manda él: `frontmatter.status` es la abreviatura legal de \
         `status` y no puede volverse un error por que otro documento tenga una clave literal \
         `frontmatter`. La ambigüedad solo se reporta cuando la respuesta sería engañosa: {:?}",
        resp2[0]
    );
    let sc = &resp2[0]["result"]["structuredContent"];
    assert_eq!(
        sc["presentIn"].as_u64(),
        Some(1),
        "…y la inspección es la del `status` del usuario: {resp2:?}"
    );
    assert_eq!(
        sc["values"],
        serde_json::json!([{"value": "draft", "count": 1}]),
        "…con su vocabulario real, no el de la clave literal (`raro`): {resp2:?}"
    );
}

// ---------------------------------------------------------------------------
// E26-H10 — Ninguna respuesta viaja sin cota
//
// Dos tools pueden devolver hoy una respuesta de tamaño proporcional al workspace:
//   · `graph_query` no tiene ni default ni máximo (`None => total`), así que un
//     `operation:"components"` sirve el grafo COMPLETO en una sola respuesta, y su `inputSchema`
//     declara `minimum: 1` sin `maximum` — el cliente tampoco puede protegerse;
//   · `metadata_inspect` no tiene paginación en NINGÚN modo: el catálogo emite una fila por cada
//     field path (incluidos los mapas intermedios) y `values` una entrada por cada valor escalar
//     distinto — N entradas para N documentos en un campo de alta cardinalidad.
// Es el contraste interno con `knowledge_search` (20/100) y `knowledge_check` (100/1000), que
// llevan cota y cursor desde E10 y que E24-H09 hizo cumplir de verdad.
//
// CONTRATO DE WIRE que fija esta historia (lo que el implementador debe respetar):
//   `graph_query.arguments.limit`  → integer, `minimum: 1`, `maximum: 1000`, `default: 100`,
//                                    DECLARADOS en el `inputSchema` y verificados por la fachada.
//   `metadata_inspect.arguments`   → gana `limit` (integer, min 1, máx 1000, default 100) y
//                                    `cursor` (string), en LOS DOS modos, declarados en el
//                                    `inputSchema` (que es `additionalProperties: false`: sin
//                                    declararlos, un cliente estricto ni siquiera podría enviarlos).
//   `metadata_inspect` structuredContent:
//        mode "catalog" → { fields: [ … ], nextCursor: string|null }
//        mode "field"   → { field, presentIn, missingIn, inferredTypes, values: [ … ],
//                           nextCursor: string|null }
//     `nextCursor` es el MISMO cursor-offset hex autosuficiente del resto de la superficie: se
//     obtiene en un proceso y se reanuda en otro fresco (mismo criterio que `search_paginacion`).
//     Los agregados (`presentIn`/`missingIn`/`inferredTypes`) se computan sobre TODO el workspace:
//     se pagina la LISTA, no la estadística.
//
// DÓNDE va la cota: en la FACHADA (`lodestar-app`), no en `core::metadata` — el core sigue puro y
// devolviendo la verdad completa (invariantes #2 y #3). Lo clava, desde el otro lado,
// `crates/lodestar-core/tests/metadata.rs::el_core_no_pagina_la_verdad_completa`.
//
// FASE ROJA (por qué falla hoy cada test):
//   · `graph_query_respeta_su_maximo`: `limit_validado` se invoca con `u64::MAX`, así que `limit:
//     5000` se ACEPTA; y el `inputSchema` no declara ni `maximum` ni `default`.
//   · `metadata_inspect_field_pagina`/`metadata_inspect_catalog_pagina`: el despachador ni lee
//     `limit`/`cursor`, así que la respuesta trae las 150/152 entradas y no hay `nextCursor`.
//   · `paginar_no_pierde_ni_duplica`/`el_cursor_es_autosuficiente`: sin default no hay más que una
//     página que recorrer ni cursor con el que reanudar.
//   · `la_estadistica_no_se_pagina`: el `limit` se ignora, así que `values` viene entero.
// El caso GRANDE (~1.000 documentos) vive en `escala_wire.rs::graph_query_tiene_default`, que es
// donde está el arnés de proceso real de E24-H16.
// ---------------------------------------------------------------------------

/// Documentos del workspace de cotas. Por encima de 100 (el default que fija la historia) y muy por
/// debajo de 1000 (el máximo), para que una página por defecto trunque y una página al máximo
/// contenga el resultado ENTERO — que es la referencia contra la que se compara el recorrido
/// completo.
const DOCS_COTA: usize = 150;

/// Campos del catálogo del workspace de cotas: un `campoNNN` por documento más `status` y `uid`.
const CAMPOS_COTA: usize = DOCS_COTA + 2;

/// **E26-H10** — Workspace de 150 documentos que fuerza las tres cotas a la vez:
///   · **grafo**: 150 nodos (cada nota enlaza a la siguiente, en ciclo → ninguna arista colgante y
///     ningún nodo fantasma, así que `components` sirve exactamente 150 nodos);
///   · **catálogo**: 152 field paths (`campo000`…`campo149` + `status` + `uid`);
///   · **vocabulario**: `uid` es de **alta cardinalidad** — un valor distinto por documento, que es
///     el caso que la historia nombra (un `id`, una fecha, un `owner`).
///
/// Deterministas por índice, así que los tres órdenes totales (nodos por `id`, campos por field
/// path, valores por conteo→texto) son reproducibles entre procesos.
fn ws_cota() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..DOCS_COTA {
        let siguiente = format!("n{:03}.md", (i + 1) % DOCS_COTA);
        write(
            dir.path(),
            &format!("n{i:03}.md"),
            &format!(
                "---\ncampo{i:03}: {i}\nstatus: draft\nuid: u{i:03}\n---\n\n# Nota {i}\n\n[siguiente]({siguiente})\n"
            ),
        );
    }
    dir
}

/// El `structuredContent` de una respuesta que **debe** haber tenido éxito (si falló, el mensaje
/// del panic lleva el error de la tool, no un `null` mudo).
fn sc_ok<'a>(resp: &'a serde_json::Value, que: &str) -> &'a serde_json::Value {
    if let Some(err) = error_de(resp) {
        panic!("«{que}» no puede fallar en este test: «{err}»");
    }
    &resp["result"]["structuredContent"]
}

/// Los elementos de la lista `clave` del `structuredContent` (nodes/fields/values), tal cual.
fn lista(sc: &serde_json::Value, clave: &str) -> Vec<serde_json::Value> {
    sc[clave]
        .as_array()
        .unwrap_or_else(|| panic!("el structuredContent debe traer «{clave}» (array): {sc}"))
        .clone()
}

/// El `nextCursor` de una respuesta paginada: `Some` si es un string no vacío, `None` si es nulo o
/// ausente (agotado).
fn cursor_de(sc: &serde_json::Value) -> Option<String> {
    match sc["nextCursor"].as_str() {
        Some(c) if !c.is_empty() => Some(c.to_string()),
        Some(_) => panic!("un `nextCursor` presente no puede ser la cadena vacía: {sc}"),
        None => None,
    }
}

/// Recorre **todas** las páginas de una tool paginada siguiendo su `nextCursor`, y devuelve la
/// concatenación de la lista `clave` más el número de páginas.
///
/// Cada página se pide en un **proceso fresco** (`roundtrip` arranca y termina el servidor), así que
/// el recorrido solo funciona si el cursor es autosuficiente — es la misma propiedad que
/// `search_paginacion` fija para `knowledge_search`.
fn recorre_paginas(
    dir: &std::path::Path,
    tool: &str,
    args: &serde_json::Value,
    clave: &str,
) -> (Vec<serde_json::Value>, usize) {
    let mut acumulado: Vec<serde_json::Value> = Vec::new();
    let mut cursor: Option<String> = None;
    let mut paginas = 0usize;
    loop {
        let mut a = args.clone();
        if let Some(c) = &cursor {
            a["cursor"] = serde_json::Value::String(c.clone());
        }
        let resp = roundtrip(dir, &[linea_call(1, tool, a).as_str()], 1);
        let sc = sc_ok(&resp[0], tool).clone();
        acumulado.extend(lista(&sc, clave));
        paginas += 1;
        assert!(
            paginas <= 20,
            "el recorrido de «{tool}» no termina: con {DOCS_COTA} documentos y el default de 100 \
             bastan 2 páginas. O el cursor no avanza, o la cota no acota."
        );
        match cursor_de(&sc) {
            Some(c) => cursor = Some(c),
            None => break,
        }
    }
    (acumulado, paginas)
}

/// La tool `nombre` tal como la declara `tools/list` en `resp` (que debe ser la respuesta a un
/// `tools/list`).
fn tool_declarada(resp: &serde_json::Value, nombre: &str) -> serde_json::Value {
    resp["result"]["tools"]
        .as_array()
        .unwrap_or_else(|| panic!("tools/list debe devolver `tools`: {resp}"))
        .iter()
        .find(|t| t["name"] == nombre)
        .unwrap_or_else(|| panic!("`tools/list` debe declarar «{nombre}»: {resp}"))
        .clone()
}

/// **E26-H10** · Criterio `graph_query_respeta_su_maximo`:
/// **Dado** `graph_query` con `limit: 5000`, **Cuando** se llama, **Entonces** falla con
/// `INVALID_SCHEMA` por exceder el máximo declarado.
///
/// Dos mitades inseparables: el `inputSchema` **declara** `default: 100` y `maximum: 1000` (sin la
/// declaración el cliente no puede protegerse, que es la mitad del defecto U5), y la fachada lo
/// **verifica** (`limit_validado`, hoy invocado con `u64::MAX`). El control anti-vacuo es el propio
/// máximo: `limit: 1000` tiene que seguir aceptándose, o «acotar» habría degenerado en «rechazar».
#[test]
fn graph_query_respeta_su_maximo() {
    let dir = ws_dos_docs();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":0,"method":"tools/list"}"#,
            linea_call(
                1,
                "graph_query",
                serde_json::json!({"operation": "components", "limit": 5000}),
            )
            .as_str(),
            linea_call(
                2,
                "graph_query",
                serde_json::json!({"operation": "components", "limit": 1000}),
            )
            .as_str(),
        ],
        3,
    );

    // (1) La VERIFICACIÓN: el máximo se hace cumplir (el criterio propiamente dicho).
    let err = error_de(&resp[1]).unwrap_or_else(|| {
        panic!(
            "`limit: 5000` excede el máximo declarado (1000) y debe RECHAZARSE. Hasta v0.4.0 \
             `limit_validado` se invocaba con `u64::MAX` para esta tool, así que cualquier valor \
             pasaba.\nRespuesta recibida: {}",
            resp[1]
        )
    });
    assert_eq!(
        codigo_de(&err),
        "INVALID_SCHEMA",
        "un `limit` fuera del rango declarado es entrada inválida, con el mismo código que en \
         `knowledge_search`/`knowledge_check` (E24-H09): «{err}»"
    );
    let (_, mensaje) = codigo_y_mensaje(&err)
        .unwrap_or_else(|| panic!("debe emitir «CÓDIGO: mensaje» (E26-H07): «{err}»"));
    assert!(
        mensaje.contains("1000"),
        "…y el mensaje debe deletrear el máximo excedido (1000), que es lo que el agente necesita \
         para corregir su llamada: «{err}»"
    );

    // (2) Control anti-vacuo: el máximo declarado SÍ se acepta (la cota no puede ser un «no» a todo).
    assert_eq!(
        error_de(&resp[2]),
        None,
        "`limit: 1000` es exactamente el máximo declarado y debe aceptarse: {}",
        resp[2]
    );

    // (3) La DECLARACIÓN: un agente descubre la cota leyendo el schema, no chocando con ella.
    let gq = tool_declarada(&resp[0], "graph_query");
    let limit = &gq["inputSchema"]["properties"]["limit"];
    assert_eq!(
        limit["maximum"].as_u64(),
        Some(1000),
        "el `inputSchema` de `graph_query.limit` debe declarar `maximum: 1000`. Hasta v0.4.0 \
         declaraba `minimum: 1` y NINGÚN máximo, así que ni el cliente podía protegerse de una \
         respuesta del tamaño del workspace: {gq}"
    );
    assert_eq!(
        limit["default"].as_u64(),
        Some(100),
        "…y `default: 100`, que es la cota que se aplica cuando el parámetro no viene: {gq}"
    );
    assert_eq!(
        limit["minimum"].as_u64(),
        Some(1),
        "…conservando el `minimum: 1` que ya declaraba (E24-H09): {gq}"
    );
}

/// **E26-H10** · Criterio `metadata_inspect_field_pagina`:
/// **Dado** un campo de alta cardinalidad (un valor distinto por documento), **Cuando** se llama a
/// `metadata_inspect{mode:"field"}` **sin** `limit`, **Entonces** `values` trae como mucho 100
/// entradas y un `nextCursor`.
///
/// Hoy trae las 150: `metadata_inspect` es la única de las 10 tools sin `limit` ni `cursor`, y un
/// `uid`/`owner`/fecha rinde una entrada por documento.
///
/// El control anti-vacuo es la segunda llamada: con el máximo declarado (`limit: 1000`) el
/// vocabulario ENTERO sigue estando disponible en una sola respuesta y `nextCursor` es nulo. Acotar
/// no puede consistir en dejar de calcular la cola.
#[test]
fn metadata_inspect_field_pagina() {
    let dir = ws_cota();
    let args = serde_json::json!({"mode": "field", "field": "uid"});
    let mut args_max = args.clone();
    args_max["limit"] = serde_json::json!(1000);

    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":0,"method":"tools/list"}"#,
            linea_call(1, "metadata_inspect", args).as_str(),
            linea_call(2, "metadata_inspect", args_max).as_str(),
        ],
        3,
    );

    // (1) La página por DEFECTO: como mucho 100 valores y un cursor con el que seguir.
    let sc = sc_ok(&resp[1], "metadata_inspect(field:uid)");
    let values = lista(sc, "values");
    assert_eq!(
        values.len(),
        100,
        "sin `limit`, `values` debe traer la página por defecto (100 entradas) sobre un campo de \
         {DOCS_COTA} valores distintos. Hasta v0.4.0 traía los {DOCS_COTA}: una respuesta de \
         tamaño proporcional al workspace, que es el defecto U5: {}",
        resp[1]
    );
    let cursor = cursor_de(sc).unwrap_or_else(|| {
        panic!(
            "…y con {DOCS_COTA} valores y una página de 100 debe venir un `nextCursor` no vacío \
             con el que recorrer el resto: {}",
            resp[1]
        )
    });
    assert!(!cursor.is_empty());

    // (2) Control anti-vacuo: con el máximo, el vocabulario ENTERO sigue disponible.
    let sc_max = sc_ok(&resp[2], "metadata_inspect(field:uid, limit:1000)");
    let todos = lista(sc_max, "values");
    assert_eq!(
        todos.len(),
        DOCS_COTA,
        "con `limit: 1000` (el máximo) el vocabulario completo cabe en una respuesta: acotar no \
         puede consistir en dejar de calcular la cola: {}",
        resp[2]
    );
    assert_eq!(
        cursor_de(sc_max),
        None,
        "…y ahí `nextCursor` debe ser nulo (no queda nada por recorrer): {}",
        resp[2]
    );
    assert_eq!(
        values,
        todos[..100].to_vec(),
        "…y la página por defecto debe ser el PREFIJO de ese orden total, no una muestra: {}",
        resp[1]
    );

    // (3) La DECLARACIÓN. El `inputSchema` de esta tool es `additionalProperties: false`: sin
    //     declarar `limit`/`cursor`, un cliente estricto ni siquiera podría enviarlos.
    let mi = tool_declarada(&resp[0], "metadata_inspect");
    let props = &mi["inputSchema"]["properties"];
    assert_eq!(
        props["limit"]["maximum"].as_u64(),
        Some(1000),
        "el `inputSchema` de `metadata_inspect` debe declarar `limit` con `maximum: 1000` \
         (por analogía con `knowledge_check`, la otra tool que enumera un catálogo): {mi}"
    );
    assert_eq!(
        props["limit"]["default"].as_u64(),
        Some(100),
        "…con `default: 100`: {mi}"
    );
    assert_eq!(
        props["limit"]["minimum"].as_u64(),
        Some(1),
        "…y `minimum: 1`, como el resto de la superficie: {mi}"
    );
    assert_eq!(
        props["cursor"]["type"].as_str(),
        Some("string"),
        "…y debe declarar `cursor` (string): es el parámetro con el que se recorre lo que la cota \
         dejó fuera, y sin declararlo la paginación es inalcanzable para un cliente estricto: {mi}"
    );
    assert!(
        mi["outputSchema"].to_string().contains("nextCursor"),
        "…y el `outputSchema` declarado debe describir el `nextCursor` que ahora viaja en la \
         respuesta (`schemars::schema_for!(MetadataInspection)`, ACTUALIZADO por esta historia): un \
         cursor que el schema no menciona es un cursor que el agente no sabe que existe: {mi}"
    );
}

/// **E26-H10** · Criterio `metadata_inspect_catalog_pagina`:
/// **Dado** un workspace con muchos field paths, **Cuando** se llama a `mode:"catalog"` sin
/// `limit`, **Entonces** `fields` trae como mucho 100 entradas y un `nextCursor`.
///
/// El catálogo emite una fila por **cada** field path que aparece en algún documento —incluidos los
/// mapas intermedios—, así que su tamaño crece con las convenciones de la base. Aquí son 152
/// (`campo000`…`campo149` + `status` + `uid`).
///
/// Control anti-vacuo: con `limit: 1000` el catálogo entero sigue cabiendo en una respuesta, y la
/// página por defecto es su prefijo.
#[test]
fn metadata_inspect_catalog_pagina() {
    let dir = ws_cota();
    let resp = roundtrip(
        dir.path(),
        &[
            linea_call(
                1,
                "metadata_inspect",
                serde_json::json!({"mode": "catalog"}),
            )
            .as_str(),
            linea_call(
                2,
                "metadata_inspect",
                serde_json::json!({"mode": "catalog", "limit": 1000}),
            )
            .as_str(),
        ],
        2,
    );

    let sc = sc_ok(&resp[0], "metadata_inspect(catalog)");
    let fields = lista(sc, "fields");
    assert_eq!(
        fields.len(),
        100,
        "sin `limit`, el catálogo debe traer la página por defecto (100 campos) de los \
         {CAMPOS_COTA} del workspace. Hasta v0.4.0 los traía todos: {}",
        resp[0]
    );
    let cursor = cursor_de(sc).unwrap_or_else(|| {
        panic!(
            "…y con {CAMPOS_COTA} campos y una página de 100 debe venir un `nextCursor`: {}",
            resp[0]
        )
    });
    assert!(!cursor.is_empty());

    let sc_max = sc_ok(&resp[1], "metadata_inspect(catalog, limit:1000)");
    let todos = lista(sc_max, "fields");
    assert_eq!(
        todos.len(),
        CAMPOS_COTA,
        "con `limit: 1000` el catálogo ENTERO sigue cabiendo en una respuesta: la cota acota la \
         página, no el cómputo: {}",
        resp[1]
    );
    assert_eq!(
        cursor_de(sc_max),
        None,
        "…y ahí `nextCursor` es nulo: {}",
        resp[1]
    );
    assert_eq!(
        fields,
        todos[..100].to_vec(),
        "…y la página por defecto es el PREFIJO del orden total del catálogo (por field path), no \
         una selección arbitraria: {}",
        resp[0]
    );
}

/// **E26-H10** · Criterio `paginar_no_pierde_ni_duplica` (control anti-vacuo CLAVE):
/// **Dado** un recorrido completo por cursor en cualquiera de los tres casos, **Cuando** se
/// concatenan las páginas, **Entonces** el resultado es **exactamente** el que devolvía v0.4.0 sin
/// paginar, sin repeticiones ni huecos.
///
/// La cota no puede consistir en tirar datos. Se comprueba en los **tres** casos que la historia
/// acota —`graph_query{components}`, `metadata_inspect{catalog}` y `metadata_inspect{field}`—
/// contra la respuesta completa, que aquí se obtiene con el `limit` **máximo** (1000 > 152 > 150):
/// esa llamada devuelve hoy y mañana exactamente lo mismo, así que es una referencia válida a
/// ambos lados del cambio.
///
/// El recorrido exige **más de una página** en los tres: si una sola página lo cubriera todo, la
/// concatenación coincidiría trivialmente y el criterio sería vacuo — que es justo lo que pasa hoy
/// (sin default no hay segunda página que recorrer).
///
/// De paso se asevera el **orden total estable por id** de `graph_query`, del que depende que un
/// cursor-offset no pierda ni duplique nada.
///
/// **Límite deliberado del criterio en el caso del grafo**: lo que se compara es el conjunto
/// ordenado de **nodos**. Las `edges` se acotan a los nodos de cada página (comportamiento vigente
/// y documentado: `App::graph_query` nunca sirve una arista colgando de un nodo que la página dejó
/// fuera), así que su concatenación NO reconstruye el conjunto completo de aristas y no se
/// asevera.
#[test]
fn paginar_no_pierde_ni_duplica() {
    let dir = ws_cota();

    let casos: [(&str, serde_json::Value, &str, usize); 3] = [
        (
            "graph_query",
            serde_json::json!({"operation": "components"}),
            "nodes",
            DOCS_COTA,
        ),
        (
            "metadata_inspect",
            serde_json::json!({"mode": "catalog"}),
            "fields",
            CAMPOS_COTA,
        ),
        (
            "metadata_inspect",
            serde_json::json!({"mode": "field", "field": "uid"}),
            "values",
            DOCS_COTA,
        ),
    ];

    for (tool, args, clave, total) in casos {
        // La verdad completa: una sola página con el `limit` máximo.
        let mut args_max = args.clone();
        args_max["limit"] = serde_json::json!(1000);
        let full_resp = roundtrip(dir.path(), &[linea_call(1, tool, args_max).as_str()], 1);
        let sc_full = sc_ok(&full_resp[0], tool);
        let completo = lista(sc_full, clave);
        assert_eq!(
            completo.len(),
            total,
            "precondición del caso «{tool}/{clave}»: con el `limit` máximo debe verse el resultado \
             entero ({total} entradas): {}",
            full_resp[0]
        );
        assert_eq!(
            cursor_de(sc_full),
            None,
            "…y sin nada pendiente, `nextCursor` nulo: {}",
            full_resp[0]
        );

        // El recorrido completo por cursor, página a página y proceso a proceso.
        let (recorrido, paginas) = recorre_paginas(dir.path(), tool, &args, clave);
        assert!(
            paginas >= 2,
            "el recorrido de «{tool}/{clave}» se agotó en {paginas} página(s) sobre {total} \
             entradas: sin la cota por defecto (100) no hay nada que recorrer y este criterio sería \
             vacuo. Es exactamente lo que pasa hasta v0.4.0."
        );
        assert_eq!(
            recorrido, completo,
            "la concatenación de las {paginas} páginas de «{tool}/{clave}» debe ser EXACTAMENTE el \
             resultado sin paginar —mismo contenido y mismo orden—: la cota acota el payload, no el \
             resultado"
        );

        // Sin repeticiones: la concatenación tiene tantos elementos distintos como longitud.
        let distintos: std::collections::BTreeSet<String> =
            recorrido.iter().map(ToString::to_string).collect();
        assert_eq!(
            distintos.len(),
            recorrido.len(),
            "…y ninguna entrada puede aparecer en dos páginas de «{tool}/{clave}»"
        );
    }

    // Orden total estable por `id` en el grafo: es la premisa del cursor-offset.
    let full = roundtrip(
        dir.path(),
        &[linea_call(
            1,
            "graph_query",
            serde_json::json!({"operation": "components", "limit": 1000}),
        )
        .as_str()],
        1,
    );
    let servidos: Vec<String> = graph_nodes(&full[0])
        .iter()
        .map(|n| {
            n["id"]
                .as_str()
                .unwrap_or_else(|| panic!("cada nodo lleva un `id` string: {n}"))
                .to_string()
        })
        .collect();
    let mut ordenados = servidos.clone();
    ordenados.sort();
    assert_eq!(
        servidos, ordenados,
        "`graph_query` debe servir sus nodos en orden total estable por `id`: un cursor-offset \
         sobre un orden inestable perdería y duplicaría entradas entre páginas"
    );
    assert_eq!(
        servidos.len(),
        DOCS_COTA,
        "…sobre los {DOCS_COTA} nodos del grafo"
    );
}

/// **E26-H10** · Criterio `el_cursor_es_autosuficiente`:
/// **Dado** un cursor obtenido en un proceso y usado en otro **fresco**, **Cuando** se reanuda,
/// **Entonces** continúa idéntico.
///
/// Mismo criterio que `search_paginacion` fija para `knowledge_search`: el cursor es un offset hex
/// opaco, no un handle atado al estado de una sesión. `roundtrip` arranca y termina un servidor por
/// llamada, así que las tres páginas de este test salen de tres procesos distintos.
#[test]
fn el_cursor_es_autosuficiente() {
    let dir = ws_cota();

    let casos: [(&str, serde_json::Value, &str, usize); 3] = [
        (
            "graph_query",
            serde_json::json!({"operation": "components"}),
            "nodes",
            DOCS_COTA,
        ),
        (
            "metadata_inspect",
            serde_json::json!({"mode": "catalog"}),
            "fields",
            CAMPOS_COTA,
        ),
        (
            "metadata_inspect",
            serde_json::json!({"mode": "field", "field": "uid"}),
            "values",
            DOCS_COTA,
        ),
    ];

    for (tool, args, clave, total) in casos {
        // Proceso 1: la primera página y su cursor.
        let p1 = roundtrip(dir.path(), &[linea_call(1, tool, args.clone()).as_str()], 1);
        let sc1 = sc_ok(&p1[0], tool);
        let pagina1 = lista(sc1, clave);
        let cursor = cursor_de(sc1).unwrap_or_else(|| {
            panic!(
                "«{tool}/{clave}» debe entregar un `nextCursor` en su primera página ({total} \
                 entradas, cota por defecto 100): {}",
                p1[0]
            )
        });

        // Proceso 2 (FRESCO): se reanuda con ese cursor.
        let mut args2 = args.clone();
        args2["cursor"] = serde_json::Value::String(cursor.clone());
        let p2 = roundtrip(dir.path(), &[linea_call(1, tool, args2).as_str()], 1);
        let sc2 = sc_ok(&p2[0], tool);
        let pagina2 = lista(sc2, clave);

        // Proceso 3 (FRESCO): la referencia completa, con el `limit` máximo.
        let mut args_max = args.clone();
        args_max["limit"] = serde_json::json!(1000);
        let full = roundtrip(dir.path(), &[linea_call(1, tool, args_max).as_str()], 1);
        let completo = lista(sc_ok(&full[0], tool), clave);

        assert_eq!(
            pagina2,
            completo[pagina1.len()..].to_vec(),
            "el cursor de «{tool}/{clave}» se emitió en un proceso y se consumió en otro FRESCO: \
             debe reanudar exactamente donde acabó la página anterior (offset autosuficiente, no un \
             handle de sesión)"
        );
        assert_eq!(
            pagina1.len() + pagina2.len(),
            total,
            "…y entre las dos páginas debe estar el resultado entero de «{tool}/{clave}»"
        );
        assert_eq!(
            cursor_de(sc2),
            None,
            "…y la segunda página agota el recorrido, así que su `nextCursor` es nulo: {}",
            p2[0]
        );
    }
}

/// **E26-H10** · Criterio `la_estadistica_no_se_pagina`:
/// **Dado** los conteos agregados de `metadata_inspect`, **Cuando** se pide una página, **Entonces**
/// `presentIn`/`missingIn` siguen refiriéndose a **todo** el workspace.
///
/// Lo que se pagina es la **lista**, no la estadística: un agente que pide 5 valores para orientarse
/// no puede recibir a cambio un `presentIn: 5` sobre un campo que está en 150 documentos —sería una
/// respuesta silenciosamente equivocada, la familia de defecto que esta épica cierra—.
///
/// Se comprueba en los dos modos: en `field` sobre `presentIn`/`missingIn`/`inferredTypes`, y en
/// `catalog` sobre el `presentIn` de las filas servidas (que se computa sobre el workspace entero,
/// no sobre la página).
///
/// **El caso del catálogo se mide donde discrimina**. Las filas de la primera página son todas
/// `campoNNN` con `presentIn: 1`, y un `1` vale lo mismo contado sobre el workspace que sobre la
/// página: aseverarlo ahí no distingue una implementación correcta de una que recortara la
/// estadística al subconjunto servido. Por eso la presencia se asevera en la página que contiene
/// `status` y `uid`, presentes en los **150** documentos: un número mayor que el tamaño de su
/// propia página (52 filas), así que cualquier `min(presentIn, filas_de_la_página)` se delata.
#[test]
fn la_estadistica_no_se_pagina() {
    let dir = ws_cota();
    let resp = roundtrip(
        dir.path(),
        &[
            linea_call(
                1,
                "metadata_inspect",
                serde_json::json!({"mode": "field", "field": "uid", "limit": 5}),
            )
            .as_str(),
            linea_call(
                2,
                "metadata_inspect",
                serde_json::json!({"mode": "field", "field": "uid", "limit": 1000}),
            )
            .as_str(),
            linea_call(
                3,
                "metadata_inspect",
                serde_json::json!({"mode": "catalog", "limit": 5}),
            )
            .as_str(),
            linea_call(
                4,
                "metadata_inspect",
                serde_json::json!({"mode": "catalog"}),
            )
            .as_str(),
        ],
        4,
    );

    let pagina = sc_ok(&resp[0], "metadata_inspect(field:uid, limit:5)");
    let completa = sc_ok(&resp[1], "metadata_inspect(field:uid, limit:1000)");

    // La lista SÍ se acota (si no, el criterio no tendría de qué hablar).
    assert_eq!(
        lista(pagina, "values").len(),
        5,
        "con `limit: 5` deben viajar 5 valores: {}",
        resp[0]
    );

    // …y la estadística NO.
    assert_eq!(
        pagina["presentIn"].as_u64(),
        Some(DOCS_COTA as u64),
        "`presentIn` describe TODO el workspace ({DOCS_COTA} documentos con `uid`), no la página \
         de 5 valores: se pagina la lista, no la estadística: {}",
        resp[0]
    );
    assert_eq!(
        pagina["missingIn"].as_u64(),
        Some(0),
        "…y `missingIn` sigue siendo el resto del workspace (0): {}",
        resp[0]
    );
    assert_eq!(
        pagina["inferredTypes"], completa["inferredTypes"],
        "…y los tipos observados son los de todo el workspace, idénticos a los de la respuesta sin \
         paginar: {}",
        resp[0]
    );
    assert_eq!(
        pagina["presentIn"], completa["presentIn"],
        "…dicho de otro modo: los agregados de una página y los de la respuesta completa coinciden"
    );

    // --- Modo catálogo -------------------------------------------------------------------------
    // (a) La lista SÍ se acota también aquí, y las filas de esta primera página son las de
    //     presencia BAJA (`campoNNN`, 1 documento cada uno): sirve de control de que la estadística
    //     no es una constante, pero NO discrimina un recorte a la página (min(1, 5) == 1).
    let cat = sc_ok(&resp[2], "metadata_inspect(catalog, limit:5)");
    let filas = lista(cat, "fields");
    assert_eq!(
        filas.len(),
        5,
        "con `limit: 5` viajan 5 campos: {}",
        resp[2]
    );
    for f in &filas {
        assert_eq!(
            f["presentIn"].as_u64(),
            Some(1),
            "cada `campoNNN` está en exactamente 1 de los {DOCS_COTA} documentos: {f}"
        );
    }

    // (b) Donde el criterio muerde: la página que contiene las filas de presencia ALTA. El orden
    //     del catálogo es por field path (`campo000`…`campo149` < `status` < `uid`), así que
    //     `status`/`uid` caen en la SEGUNDA página del recorrido por defecto.
    let cat1 = sc_ok(&resp[3], "metadata_inspect(catalog)");
    let cursor = cursor_de(cat1).unwrap_or_else(|| {
        panic!(
            "el catálogo de {CAMPOS_COTA} campos debe entregar un `nextCursor` en su primera \
             página: {}",
            resp[3]
        )
    });
    let p2 = roundtrip(
        dir.path(),
        &[linea_call(
            5,
            "metadata_inspect",
            serde_json::json!({"mode": "catalog", "cursor": cursor}),
        )
        .as_str()],
        1,
    );
    let filas2 = lista(sc_ok(&p2[0], "metadata_inspect(catalog, cursor)"), "fields");
    assert_eq!(
        filas2.len(),
        CAMPOS_COTA - 100,
        "la segunda página del catálogo trae los {} campos restantes: {}",
        CAMPOS_COTA - 100,
        p2[0]
    );
    for nombre in ["status", "uid"] {
        let fila = filas2
            .iter()
            .find(|f| f["name"] == nombre)
            .unwrap_or_else(|| {
                panic!(
                    "precondición: «{nombre}» (presente en los {DOCS_COTA} documentos) tiene que \
                     caer en esta página, o el criterio no discriminaría: {}",
                    p2[0]
                )
            });
        assert_eq!(
            fila["presentIn"].as_u64(),
            Some(DOCS_COTA as u64),
            "«{nombre}» está en los {DOCS_COTA} documentos del workspace, y ese conteo NO puede \
             encogerse a las {} filas de la página que lo trae: se pagina la lista, no la \
             estadística: {fila}",
            filas2.len()
        );
    }
}

// ===========================================================================
// E28-H02 — `create`/`move` sobre un destino ocupado quedan bloqueados por un guard de colisión.
//
// Defecto A-05 del testbench homelab (caso G1-11, repro literal en
// `docs/qa/testbench/batches/verify_G1-11.json`; ficha en
// `decisiones/23-hallazgos-testbench-homelab.md`): sobre un workspace con `notas/existente.md`, un
// `change_plan` con `{"op":"create","path":"notas/existente.md"}` devuelve `canApply:true` sin un
// solo diagnóstico de colisión — aplicado, sobrescribe el documento. Simétricamente, un `move` cuyo
// `to` ya está ocupado publica encima de él. La dirección contraria (tocar algo que NO existe) sí
// tiene guard desde siempre: `DOCUMENT_NOT_FOUND`.
//
// Causa raíz (verificada en el código): `crates/lodestar-core/src/plan.rs`, `normalize_create`
// descarta el `DocumentSet` como `_workspace`; `normalize_move` nunca consulta `doc_set.files()`
// para el destino.
//
// LO QUE FIJAN ESTOS TESTS es la superficie de WIRE: una colisión deja el plan NO APLICABLE y expone
// el código estable nuevo `DOCUMENT_ALREADY_EXISTS` (fila 17 del catálogo `ErrorCode`, «Delta de
// contrato propuesto» de la historia) nombrando el path colisionado. Son deliberadamente agnósticos
// a CÓMO se materialice esa no-aplicabilidad —error de ejecución de la tool (`isError:true`, la ruta
// que toma hoy `DOCUMENT_NOT_FOUND` y la que sugiere el «Alcance»: `Err(CoreError)` en la
// normalización) o plan con `canApply:false` y el código en el diagnóstico—, porque la historia
// admite ambas lecturas; lo que NO admiten es un plan aplicable.
//
// Como `ErrorCode::DocumentAlreadyExists` todavía no existe, el código se asevera por su
// representación de WIRE (la cadena que ve el agente), no por la variante Rust: así estos tests
// compilan hoy y fallan por ASERCIÓN.
//
// ROJO esperado HOY: por ASERCIÓN. `change_plan` devuelve `canApply:true` sin `isError`.
// ===========================================================================

/// Workspace del criterio: dos documentos existentes bajo `notas/` —`existente.md` (el destino
/// ocupado, con un contenido reconocible que ninguna operación debe pisar) y `origen.md` (el
/// documento a mover)—. Sin `index.md`: el modelo es universal (`§20`), ningún fichero es especial.
fn workspace_dos_notas() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "notas/existente.md",
        "---\ntitle: Existente\n---\n\n# Existente\n\ncontenido que no se debe pisar\n",
    );
    write(
        dir.path(),
        "notas/origen.md",
        "---\ntitle: Origen\n---\n\n# Origen\n\ncuerpo del origen\n",
    );
    dir
}

/// El código de wire (fila 17 del catálogo `ErrorCode`, E28-H02) de una colisión de destino.
const COLISION: &str = "DOCUMENT_ALREADY_EXISTS";

/// Asevera que la respuesta de `change_plan` deja el cambio **no aplicable por colisión**: el plan
/// no puede aplicarse (o no llega a existir) y la respuesta expone el código `DOCUMENT_ALREADY_EXISTS`
/// nombrando `path`. Acepta las dos formas admisibles (error de ejecución de la tool, o plan con
/// `canApply:false`) y rechaza la única inadmisible: un plan aplicable.
fn asevera_colision(resp: &serde_json::Value, path: &str) {
    let texto = resp.to_string();

    // Nunca un error de PROTOCOLO: una colisión es un resultado de la operación, no un fallo de
    // transporte (mismo patrón que `DOCUMENT_NOT_FOUND`/`REVISION_CONFLICT`).
    assert!(
        resp["error"].is_null(),
        "una colisión de destino NO debe ser un error JSON-RPC de transporte: {resp}"
    );

    // El cambio no puede quedar aplicable: o la tool falla, o el plan sale con `canApply:false`.
    let es_error = resp["result"]["isError"].as_bool() == Some(true);
    let can_apply = resp["result"]["structuredContent"]["canApply"].as_bool();
    assert!(
        es_error || can_apply == Some(false),
        "planificar sobre el destino ocupado «{path}» NO puede dar un plan aplicable: o la tool \
         falla con {COLISION}, o el plan sale con `canApply:false`. Respuesta: {resp}"
    );

    // Y el agente debe poder ramificar por el código estable, con el path nombrado.
    assert!(
        texto.contains(COLISION),
        "la respuesta debe exponer el código estable «{COLISION}» (fila 17 del catálogo, simétrica \
         de DOCUMENT_NOT_FOUND): {resp}"
    );
    assert!(
        texto.contains(path),
        "el diagnóstico de colisión debe NOMBRAR el path ocupado «{path}», igual que \
         DOCUMENT_NOT_FOUND nombra el ref que no resolvió: {resp}"
    );
}

/// **E28-H02** · Criterio `create_sobre_path_existente_es_document_already_exists`:
/// **Dado** un workspace con `notas/existente.md`, **Cuando** se llama a `change_plan` con
/// `{"op":"create","path":"notas/existente.md"}`, **Entonces** el cambio no queda aplicable y el
/// diagnóstico lleva `DOCUMENT_ALREADY_EXISTS` nombrando el path.
///
/// Es el paso 1 literal del repro `verify_G1-11.json`.
#[test]
fn create_sobre_path_existente_es_document_already_exists() {
    let dir = workspace_dos_notas();
    let antes = snapshot_md(dir.path());

    let ops = serde_json::json!([
        { "op": "create", "path": "notas/existente.md",
          "frontmatter": { "title": "Pisado" }, "body": "x\n" },
    ]);
    let resp = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );

    asevera_colision(&resp[0], "notas/existente.md");

    // Planificar nunca escribe (invariante #1): el documento existente sigue intacto byte a byte.
    assert_eq!(
        snapshot_md(dir.path()),
        antes,
        "`change_plan` no escribe: el conocimiento en disco debe quedar idéntico: {resp:?}"
    );
}

/// **E28-H02** · Criterio `move_a_destino_ocupado_es_document_already_exists`:
/// **Dado** un workspace con `notas/existente.md` y `notas/origen.md`, **Cuando** se llama a
/// `change_plan` con `{"op":"move","from":"notas/origen.md","to":"notas/existente.md"}`, **Entonces**
/// el mismo veredicto y el mismo código, nombrando el DESTINO.
///
/// Es el paso 2 del repro `verify_G1-11.json` (allí, `guias/tmux.md → README.md`).
#[test]
fn move_a_destino_ocupado_es_document_already_exists() {
    let dir = workspace_dos_notas();
    let antes = snapshot_md(dir.path());

    let ops = serde_json::json!([
        { "op": "move", "from": "notas/origen.md", "to": "notas/existente.md",
          "rewriteInboundLinks": true },
    ]);
    let resp = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );

    asevera_colision(&resp[0], "notas/existente.md");

    assert_eq!(
        snapshot_md(dir.path()),
        antes,
        "`change_plan` no escribe: el conocimiento en disco debe quedar idéntico: {resp:?}"
    );
}

/// **E28-H02** · Criterio `create_sobre_path_libre_sigue_funcionando` (control anti-vacuo):
/// **Dado** un workspace **sin** `notas/nueva.md`, **Cuando** se planifica su `create`, **Entonces**
/// el plan sigue siendo aplicable (`canApply:true`) y sin rastro del código de colisión.
///
/// Sin este control, el guard podría implementarse rechazándolo todo.
#[test]
fn create_sobre_path_libre_sigue_funcionando() {
    let dir = workspace_dos_notas();
    assert!(
        !dir.path().join("notas/nueva.md").exists(),
        "precondición: `notas/nueva.md` debe estar LIBRE"
    );

    let ops = serde_json::json!([
        { "op": "create", "path": "notas/nueva.md", "body": "# Nueva\n\ncuerpo nuevo\n" },
    ]);
    let resp = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );

    let sc = plan_sc(&resp[0]);
    assert_eq!(
        sc["canApply"],
        serde_json::Value::Bool(true),
        "un `create` sobre un path libre debe seguir dando un plan aplicable: {resp:?}"
    );
    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "un `create` sobre un path libre no debe dar isError: {resp:?}"
    );
    assert!(
        !resp[0].to_string().contains(COLISION),
        "un destino libre no es una colisión: {COLISION} no debe aparecer: {resp:?}"
    );
}

/// **E28-H02** · Criterio `move_a_destino_libre_sigue_funcionando` (control anti-vacuo):
/// **Dado** `notas/origen.md` y **sin** `notas/destino.md`, **Cuando** se planifica el `move`,
/// **Entonces** el plan sigue siendo aplicable y emite su `Move`.
#[test]
fn move_a_destino_libre_sigue_funcionando() {
    let dir = workspace_dos_notas();
    assert!(
        !dir.path().join("notas/destino.md").exists(),
        "precondición: `notas/destino.md` debe estar LIBRE"
    );

    let ops = serde_json::json!([
        { "op": "move", "from": "notas/origen.md", "to": "notas/destino.md",
          "rewriteInboundLinks": true },
    ]);
    let resp = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );

    let sc = plan_sc(&resp[0]);
    assert_eq!(
        sc["canApply"],
        serde_json::Value::Bool(true),
        "un `move` hacia un destino libre debe seguir dando un plan aplicable: {resp:?}"
    );
    assert!(
        !resp[0].to_string().contains(COLISION),
        "un destino libre no es una colisión: {COLISION} no debe aparecer: {resp:?}"
    );
    let normalized = sc["normalizedOperations"]
        .as_array()
        .unwrap_or_else(|| panic!("el plan debe traer `normalizedOperations`: {resp:?}"));
    assert!(
        normalized
            .iter()
            .any(|op| op["op"] == "move" && op["to"] == "notas/destino.md"),
        "el plan debe incluir el `move` hacia el destino libre: {resp:?}"
    );
}

/// **E28-H02** · Criterio `apply_de_plan_con_colision_rechaza_sin_tocar_disco`:
/// **Dado** un plan con una colisión de `create`, **Cuando** se intenta aplicar, **Entonces** la
/// llamada se rechaza y el documento existente sigue **exactamente** como estaba en disco.
///
/// El test cubre las dos formas que la historia admite para la no-aplicabilidad:
///   - si `change_plan` NO llega a producir un `changeSetId` (la colisión aborta la normalización),
///     no hay nada que aplicar y se exige que tampoco quedara un plan persistido en runtime;
///   - si sí lo produce (plan con `canApply:false`), se llama a `change_apply` con él y se exige que
///     el apply lo rechace (`isError:true`).
///
/// En ambos casos la aserción que de verdad importa es la misma: **el `.md` en disco no cambió**.
#[test]
fn apply_de_plan_con_colision_rechaza_sin_tocar_disco() {
    let dir = workspace_dos_notas();
    let antes = snapshot_md(dir.path());
    let contenido_original =
        std::fs::read_to_string(dir.path().join("notas/existente.md")).unwrap();

    let ops = serde_json::json!([
        { "op": "create", "path": "notas/existente.md",
          "frontmatter": { "title": "Pisado" }, "body": "contenido intruso\n" },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );

    // El plan no puede quedar aplicable (mismo veredicto que el criterio 1).
    asevera_colision(&plan[0], "notas/existente.md");

    match plan[0]["result"]["structuredContent"]["changeSetId"].as_str() {
        // (a) Hay plan (con `canApply:false`): el apply debe rechazarlo.
        Some(id) => {
            let apply = roundtrip(dir.path(), &[change_apply_line(id, None).as_str()], 1);
            assert_eq!(
                apply[0]["result"]["isError"],
                serde_json::Value::Bool(true),
                "aplicar un plan con una colisión de destino debe ser RECHAZADO: {apply:?}"
            );
            assert!(
                apply[0]["result"]["structuredContent"]["applied"].as_bool() != Some(true),
                "un plan con colisión nunca puede reportar `applied:true`: {apply:?}"
            );
        }
        // (b) No hay plan: la colisión abortó la normalización. Tampoco debe quedar plan persistido
        //     en runtime que alguien pudiera aplicar después.
        None => {
            let plans = dir.path().join(".lodestar").join("runtime").join("plans");
            let persistidos: Vec<_> = std::fs::read_dir(&plans)
                .map(|it| {
                    it.filter_map(|e| e.ok().map(|e| e.path()))
                        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
                        .collect()
                })
                .unwrap_or_default();
            assert!(
                persistidos.is_empty(),
                "una colisión no debe dejar un plan persistido y aplicable en runtime: {persistidos:?}"
            );
        }
    }

    // Lo nuclear: el conocimiento en disco quedó intacto (invariante #1).
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notas/existente.md")).unwrap(),
        contenido_original,
        "el documento existente NO puede haber sido pisado por la operación colisionada"
    );
    assert_eq!(
        snapshot_md(dir.path()),
        antes,
        "ningún `.md` del workspace puede haber cambiado por un plan colisionado"
    );
}

/// **E28-H02** · Criterio `move_a_si_mismo_no_es_colision`:
/// **Dado** un `move` con `from == to` sobre un documento existente, **Cuando** se planifica,
/// **Entonces** NO es una colisión — el destino coincide consigo mismo, no con otro documento.
///
/// Comportamiento que fija esta fase roja (el que el motor tiene HOY, verificado ejecutando los
/// binarios antes de escribir el test, y que el guard no debe romper): **no-op válido**. El plan sale
/// aplicable, `change_apply` responde `applied:true` con `changedPaths` vacío y el `.md` queda
/// intacto byte a byte.
#[test]
fn move_a_si_mismo_no_es_colision() {
    let dir = workspace_dos_notas();
    let original = std::fs::read_to_string(dir.path().join("notas/origen.md")).unwrap();

    let ops = serde_json::json!([
        { "op": "move", "from": "notas/origen.md", "to": "notas/origen.md",
          "rewriteInboundLinks": true },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );

    // (1) No se confunde con una colisión de destino.
    assert!(
        !plan[0].to_string().contains(COLISION),
        "un `move` a sí mismo NO es una colisión de destino ({COLISION} no debe aparecer): {plan:?}"
    );
    let sc = plan_sc(&plan[0]);
    assert_eq!(
        sc["canApply"],
        serde_json::Value::Bool(true),
        "un `move` con `from == to` es un no-op válido, no un plan rechazado: {plan:?}"
    );

    // (2) Y aplicarlo es inocuo: el documento sigue en su sitio, byte a byte.
    let id = plan_change_set_id(&plan[0]);
    let apply = roundtrip(dir.path(), &[change_apply_line(&id, None).as_str()], 1);
    let asc = apply_sc(&apply[0]);
    assert_eq!(
        asc["applied"],
        serde_json::Value::Bool(true),
        "aplicar el no-op debe tener éxito: {apply:?}"
    );
    assert_eq!(
        asc["changedPaths"],
        serde_json::json!([]),
        "un `move` a sí mismo no cambia ningún path: {apply:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notas/origen.md")).unwrap(),
        original,
        "el documento del `move` a sí mismo debe sobrevivir intacto byte a byte"
    );
}

// ===========================================================================
// E28-H04 — la normalización de un plan juzga cada operación contra el ESTADO ACUMULADO por las
// operaciones ANTERIORES del mismo plan, no contra el `DocumentSet` con el que empezó.
//
// Bloqueante de E28-H02 verificado por juez ciego ejecutando el binario por JSON-RPC
// (`requirements/epica-28-defectos-destructivos-testbench.md`, «Adenda correctiva»). Los guards que
// H02 puso en `normalize_create`/`normalize_move` comparan contra el `DocumentSet` **inicial** —el
// bucle de `App::change_plan_uncounted` pasa el mismo `&doc_set` en cada iteración—, que deja de ser
// cierto en cuanto el plan tiene más de una operación tocando paths relacionados.
//
// Los CINCO escenarios de abajo se reprodujeron uno a uno contra el binario real antes de escribir
// los tests. Estado de HOY:
//
//   (i) FALSOS NEGATIVOS DESTRUCTIVOS — `canApply:true`, `risk: low`, sin un solo diagnóstico:
//       · `[move a→final, move b→final]` → aplica y deja SOLO `final.md` con el cuerpo de `b`:
//         el documento `a` desaparece del workspace.
//       · `[create x, move b→x]`        → aplica y deja `x.md` con el cuerpo de `b`: el `create`
//         del propio plan queda pisado.
//       · `[create x, create x]`        → aplica y gana el SEGUNDO en silencio.
//
//   (ii) FALSOS POSITIVOS — regresión respecto al commit padre de H02 (`85af8b9`, verificado
//        ejecutando su binario): dos idiomas legítimos que allí aplicaban y hoy responden
//        `isError:true` con `DOCUMENT_ALREADY_EXISTS`:
//       · `[delete x, create x]`   → antes dejaba `x.md` = «# X recreado\n».
//       · `[move A→B, create A]`   → antes dejaba `B.md` con el original y `A.md` = «# A stub\n».
//
// Los tres de (i) fallan HOY porque el plan sale APLICABLE; los dos de (ii) fallan HOY porque el
// plan se RECHAZA. Los cinco se juzgan por la superficie de wire (respuesta JSON-RPC + disco), no
// por símbolos Rust, así que no dependen de la forma interna que elija el implementador —la
// historia deja abiertas dos (`DocumentSet` hipotético recalculado vs. conjunto de paths aparte)—.
//
// El control anti-vacuo de la colisión contra DISCO de una sola operación son los tests de H02 de
// más arriba (`create_sobre_path_existente_es_document_already_exists`,
// `move_a_destino_ocupado_es_document_already_exists`), que siguen verdes sin tocarse.
// ===========================================================================

/// Workspace de los escenarios intra-plan: `a.md` y `b.md` con cuerpos distinguibles, y **sin**
/// `final.md`/`x.md`, que son los destinos que las operaciones del plan van ocupando.
fn workspace_a_y_b() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "a.md",
        "---\ntitle: A\n---\n\n# A\n\ncuerpo de a\n",
    );
    write(
        dir.path(),
        "b.md",
        "---\ntitle: B\n---\n\n# B\n\ncuerpo de b\n",
    );
    dir
}

/// Planifica `ops` y, si el plan sale aplicable, lo aplica en la misma sesión. Devuelve
/// `(respuesta_del_plan, respuesta_del_apply_si_la_hubo)`.
///
/// Los escenarios de (i) necesitan esto para poder aseverar que HOY el defecto llega hasta el disco;
/// los de (ii) lo necesitan para verificar el disco final, que es el criterio de la historia.
fn plan_y_apply(
    dir: &std::path::Path,
    ops: serde_json::Value,
) -> (serde_json::Value, Option<serde_json::Value>) {
    let plan = roundtrip(
        dir,
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );
    let id = plan[0]["result"]["structuredContent"]["changeSetId"]
        .as_str()
        .map(str::to_string);
    let Some(id) = id else {
        return (plan[0].clone(), None);
    };
    let apply = roundtrip(dir, &[change_apply_line(&id, None).as_str()], 1);
    (plan[0].clone(), Some(apply[0].clone()))
}

/// **E28-H04** · Criterio `dos_moves_al_mismo_destino_en_el_mismo_plan_es_document_already_exists`:
/// **Dado** un workspace con `a.md` y `b.md` y **sin** `final.md`, **Cuando** se planifica
/// `[{move a→final}, {move b→final}]`, **Entonces** el plan NO queda aplicable y el diagnóstico lleva
/// `DOCUMENT_ALREADY_EXISTS` nombrando `final.md`.
///
/// Es el falso negativo más grave de los tres: hoy el plan sale con `risk: low` y, aplicado, deja en
/// disco un único `final.md` con el cuerpo de `b` — el documento `a` desaparece del workspace sin
/// que nada lo señale (invariante #1: la única fuente de verdad, destruida en silencio).
#[test]
fn dos_moves_al_mismo_destino_en_el_mismo_plan_es_document_already_exists() {
    let dir = workspace_a_y_b();
    let antes = snapshot_md(dir.path());

    let ops = serde_json::json!([
        { "op": "move", "from": "a.md", "to": "final.md", "rewriteInboundLinks": true },
        { "op": "move", "from": "b.md", "to": "final.md", "rewriteInboundLinks": true },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );

    asevera_colision(&plan[0], "final.md");
    assert_eq!(
        snapshot_md(dir.path()),
        antes,
        "`change_plan` no escribe: el conocimiento en disco debe quedar idéntico: {plan:?}"
    );
}

/// **E28-H04** · Criterio `create_seguido_de_move_al_mismo_path_es_document_already_exists`:
/// **Dado** un workspace con `b.md` y **sin** `x.md`, **Cuando** se planifica
/// `[{create x}, {move b→x}]`, **Entonces** el plan NO queda aplicable y el diagnóstico lleva
/// `DOCUMENT_ALREADY_EXISTS` nombrando `x.md`.
///
/// Hoy el `move` se normaliza contra el `DocumentSet` original —donde `x.md` no existe— y pisa en
/// silencio el `create` del propio plan: aplicado, `x.md` acaba con el cuerpo de `b`.
#[test]
fn create_seguido_de_move_al_mismo_path_es_document_already_exists() {
    let dir = workspace_a_y_b();
    let antes = snapshot_md(dir.path());

    let ops = serde_json::json!([
        { "op": "create", "path": "x.md", "body": "# X nuevo\n\ncuerpo del create\n" },
        { "op": "move", "from": "b.md", "to": "x.md", "rewriteInboundLinks": true },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );

    asevera_colision(&plan[0], "x.md");
    assert_eq!(
        snapshot_md(dir.path()),
        antes,
        "`change_plan` no escribe: el conocimiento en disco debe quedar idéntico: {plan:?}"
    );
}

/// **E28-H04** · Criterio `dos_creates_al_mismo_path_en_el_mismo_plan_es_document_already_exists`:
/// **Dado** un workspace **sin** `x.md`, **Cuando** se planifica `[{create x}, {create x}]`,
/// **Entonces** el plan NO queda aplicable y el diagnóstico lleva `DOCUMENT_ALREADY_EXISTS`
/// nombrando `x.md`.
///
/// Hoy los dos `create` se normalizan contra el mismo `DocumentSet` original, ninguno ve al otro, y
/// al aplicar gana el segundo: el agente que envió dos cuerpos distintos no recibe ninguna señal de
/// que uno se descartó.
#[test]
fn dos_creates_al_mismo_path_en_el_mismo_plan_es_document_already_exists() {
    let dir = workspace_a_y_b();
    let antes = snapshot_md(dir.path());

    let ops = serde_json::json!([
        { "op": "create", "path": "x.md", "body": "# Primero\n" },
        { "op": "create", "path": "x.md", "body": "# Segundo\n" },
    ]);
    let plan = roundtrip(
        dir.path(),
        &[change_plan_line(None, ops, policy_permisiva()).as_str()],
        1,
    );

    asevera_colision(&plan[0], "x.md");
    assert_eq!(
        snapshot_md(dir.path()),
        antes,
        "`change_plan` no escribe: el conocimiento en disco debe quedar idéntico: {plan:?}"
    );
}

/// **E28-H04** · Criterio `delete_seguido_de_create_del_mismo_path_aplica`:
/// **Dado** un workspace con `x.md`, **Cuando** se planifica `[{delete x}, {create x}]`,
/// **Entonces** el plan es aplicable y `change_apply` deja `x.md` en disco con el cuerpo del
/// `create`.
///
/// Idioma legítimo —recrear un documento borrado dentro del mismo plan— que funcionaba en el commit
/// padre de H02 (`85af8b9`, verificado ejecutando su binario: dejaba `x.md` = «# X recreado\n») y que
/// hoy responde `isError:true` con `DOCUMENT_ALREADY_EXISTS`, porque el `create` ve `x.md` todavía
/// presente en el `DocumentSet` inicial: el `delete` que lo precede en el propio plan no se refleja.
#[test]
fn delete_seguido_de_create_del_mismo_path_aplica() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "x.md",
        "---\ntitle: X\n---\n\n# X\n\ncuerpo original de x\n",
    );

    let ops = serde_json::json!([
        { "op": "delete", "ref": { "path": "x.md" }, "inboundLinksPolicy": "remove_links" },
        { "op": "create", "path": "x.md", "body": "# X recreado\n" },
    ]);
    let (plan, apply) = plan_y_apply(dir.path(), ops);

    // (1) El plan vuelve a ser aplicable: liberar y reocupar NO es una colisión.
    assert!(
        !plan.to_string().contains(COLISION),
        "`[delete x, create x]` LIBERA el path antes de reocuparlo: es el idioma legítimo que \
         funcionaba antes de E28-H02, no una colisión ({COLISION} no debe aparecer): {plan:?}"
    );
    let sc = plan_sc(&plan);
    assert_eq!(
        sc["canApply"],
        serde_json::Value::Bool(true),
        "el plan de recrear un documento borrado en el mismo change set debe ser aplicable: {plan:?}"
    );

    // (2) Y aplicarlo deja el disco como lo dejaba antes de H02: el cuerpo del SEGUNDO `create`.
    let apply =
        apply.unwrap_or_else(|| panic!("el plan aplicable debe traer `changeSetId`: {plan:?}"));
    let asc = apply_sc(&apply);
    assert_eq!(
        asc["applied"],
        serde_json::Value::Bool(true),
        "aplicar `[delete x, create x]` debe tener éxito: {apply:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("x.md")).unwrap(),
        "# X recreado\n",
        "el disco final debe llevar el cuerpo del `create` que reocupó el path, exactamente como \
         antes de E28-H02 (sin frontmatter: el `create` no pidió ninguno, `§20.2` invariante 3)"
    );
}

/// **E28-H04** · Criterio `move_seguido_de_create_del_path_liberado_aplica`:
/// **Dado** un workspace con `A.md` y **sin** `B.md`, **Cuando** se planifica
/// `[{move A→B}, {create A}]`, **Entonces** el plan es aplicable y `change_apply` deja `B.md` con el
/// contenido movido y `A.md` con el del `create`.
///
/// El otro idioma legítimo que H02 regresionó: liberar un path moviéndolo y reutilizarlo en el mismo
/// plan (el patrón «archivo esto y dejo un stub en su sitio»). En el commit padre de H02 dejaba
/// `B.md` con el original y `A.md` = «# A stub\n»; hoy responde `DOCUMENT_ALREADY_EXISTS` sobre
/// `A.md` porque el `DocumentSet` inicial todavía lo tiene ocupado por sí mismo.
#[test]
fn move_seguido_de_create_del_path_liberado_aplica() {
    let dir = tempfile::tempdir().unwrap();
    let original = "---\ntitle: A\n---\n\n# A\n\ncuerpo de a\n";
    write(dir.path(), "A.md", original);

    let ops = serde_json::json!([
        { "op": "move", "from": "A.md", "to": "B.md", "rewriteInboundLinks": true },
        { "op": "create", "path": "A.md", "body": "# A stub\n" },
    ]);
    let (plan, apply) = plan_y_apply(dir.path(), ops);

    // (1) El `move` libera `A.md`, así que el `create` posterior lo reocupa legítimamente.
    assert!(
        !plan.to_string().contains(COLISION),
        "`[move A→B, create A]` libera `A.md` con el propio `move`: reocuparlo después no es una \
         colisión ({COLISION} no debe aparecer): {plan:?}"
    );
    let sc = plan_sc(&plan);
    assert_eq!(
        sc["canApply"],
        serde_json::Value::Bool(true),
        "el plan de mover un documento y dejar un stub en su path debe ser aplicable: {plan:?}"
    );

    // (2) Disco final: el original viajó a `B.md` y `A.md` quedó con el stub.
    let apply =
        apply.unwrap_or_else(|| panic!("el plan aplicable debe traer `changeSetId`: {plan:?}"));
    let asc = apply_sc(&apply);
    assert_eq!(
        asc["applied"],
        serde_json::Value::Bool(true),
        "aplicar `[move A→B, create A]` debe tener éxito: {apply:?}"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("B.md")).unwrap(),
        original,
        "el destino del `move` debe llevar el documento ORIGINAL byte a byte, no el stub"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("A.md")).unwrap(),
        "# A stub\n",
        "el path liberado debe quedar con el cuerpo del `create` que lo reocupó"
    );
}

// ---------------------------------------------------------------------------
// E29-H02 — Una `policy` PARCIAL en `change_plan` respeta el `Default` que el contrato promete
// (`requirements/epica-29-honestidad-superficie.md`, `decisiones §19(b)`).
//
// Síntoma: `{"policy": {"requireValidResult": false}}` → `INVALID_SCHEMA: … missing field
// allowWarnings`, pese a que `contracts/mcp.yml`/`inputSchema` declaran los dos campos opcionales
// con default. Omitir `policy` ENTERA sí funciona hoy; el caso roto es el intermedio. Causa raíz:
// `PlanPolicy` (`crates/lodestar-core/src/plan.rs:271`) deriva `Deserialize` sin `#[serde(default)]`
// por campo. Rojo esperado: `INVALID_SCHEMA` (isError) donde debería salir un plan.
// ---------------------------------------------------------------------------

/// Criterio `policy_parcial_toma_el_default_del_campo_omitido`: **Dado** un workspace válido,
/// **Cuando** se llama a `change_plan` con `policy: {"requireValidResult": false}` (sin
/// `allowWarnings`), **Entonces** el plan se computa (sin `INVALID_SCHEMA`) y `canApply` se evalúa
/// con `allowWarnings = true` (el default) — aseverado por el EFECTO observable: sobre un
/// resultado con warnings pero sin errores, `canApply` es `true` (si `allowWarnings` hubiera caído
/// a `false` por error de deserialización, sería `false`).
#[test]
fn policy_parcial_toma_el_default_del_campo_omitido() {
    let dir = workspace_cinco_relacionados();
    // Enlace a un fichero de proyecto (no-`.md`) inexistente → `LINK-TARGET-MISSING`/Warn
    // (`missingWorkspaceFiles: warning`, `§20.9`): el resultado hipotético queda con >=1 warning y
    // 0 errores, así que la rama `allowWarnings` de `can_apply` es la que decide `canApply`.
    write(
        dir.path(),
        "a.md",
        "---\ntype: Concept\ntitle: A\ndescription: nodo a del cluster\n---\n\n# A\n\n[Siguiente](b.md)\n\n[guía](guia.pdf)\n",
    );
    let ops = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "d.md" },
          "patch": { "description": "d actualizada por el plan" } },
    ]);
    // Solo `requireValidResult`: `allowWarnings` queda OMITIDO — el caso roto de la historia.
    let line = change_plan_line(
        None,
        ops,
        serde_json::json!({ "requireValidResult": false }),
    );
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);

    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "una `policy` PARCIAL (solo `requireValidResult`) no debe dar isError/INVALID_SCHEMA: {resp:?}"
    );
    let sc = plan_sc(&resp[0]);

    // Precondición del fixture: el resultado hipotético tiene >=1 warning y 0 errores (si no, el
    // criterio de `allowWarnings` quedaría vacuo: `canApply` saldría `true` por `requireValidResult`
    // ya satisfecho, sin ejercitar el campo omitido).
    let warnings = sc["diagnosticsAfter"]["warnings"]
        .as_u64()
        .unwrap_or_else(|| {
            panic!("change_plan debe devolver diagnosticsAfter.warnings (u64): {sc:?}")
        });
    let errors = sc["diagnosticsAfter"]["errors"]
        .as_u64()
        .unwrap_or_else(|| {
            panic!("change_plan debe devolver diagnosticsAfter.errors (u64): {sc:?}")
        });
    assert!(
        warnings >= 1 && errors == 0,
        "precondición del fixture: el resultado hipotético debe tener >=1 warning y 0 errores \
         para que el criterio ejercite `allowWarnings`, no `requireValidResult`; diagnosticsAfter = {:?}",
        sc["diagnosticsAfter"]
    );

    // `allowWarnings` omitido ⇒ default `true` ⇒ los warnings NO bloquean `canApply`.
    assert_eq!(
        sc["canApply"],
        serde_json::Value::Bool(true),
        "con `allowWarnings` OMITIDO (debe tomar el default `true`) y solo warnings (sin errores), \
         `canApply` debe ser `true`; si el campo omitido hubiera caído a `false` por un fallo de \
         deserialización, `canApply` sería `false`: {sc:?}"
    );
}

/// Criterio `policy_parcial_respeta_el_campo_enviado`: el caso simétrico — **Dado** un workspace
/// cuyo resultado simulado tiene warnings, **Cuando** se llama con
/// `policy: {"allowWarnings": false}` (sin `requireValidResult`), **Entonces** `canApply` es
/// `false` (el campo ENVIADO se respeta) y `requireValidResult` vale `true` por defecto — el plan
/// se sigue computando (sin `INVALID_SCHEMA`; `canApply:false` es un veredicto, no un fallo de la
/// tool).
#[test]
fn policy_parcial_respeta_el_campo_enviado() {
    let dir = workspace_cinco_relacionados();
    write(
        dir.path(),
        "a.md",
        "---\ntype: Concept\ntitle: A\ndescription: nodo a del cluster\n---\n\n# A\n\n[Siguiente](b.md)\n\n[guía](guia.pdf)\n",
    );
    let ops = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "d.md" },
          "patch": { "description": "d actualizada por el plan" } },
    ]);
    // Solo `allowWarnings`: `requireValidResult` queda OMITIDO (debe tomar el default `true`, que
    // en este fixture es irrelevante porque no hay errores — lo que decide aquí es `allowWarnings`).
    let line = change_plan_line(None, ops, serde_json::json!({ "allowWarnings": false }));
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);

    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "una `policy` PARCIAL (solo `allowWarnings`) no debe dar isError/INVALID_SCHEMA: {resp:?}"
    );
    let sc = plan_sc(&resp[0]);

    let warnings = sc["diagnosticsAfter"]["warnings"]
        .as_u64()
        .unwrap_or_else(|| {
            panic!("change_plan debe devolver diagnosticsAfter.warnings (u64): {sc:?}")
        });
    assert!(
        warnings >= 1,
        "precondición del fixture: el resultado hipotético debe tener >=1 warning para que el \
         criterio no sea vacuo; diagnosticsAfter = {:?}",
        sc["diagnosticsAfter"]
    );

    // `allowWarnings:false` ENVIADO ⇒ se respeta ⇒ el warning bloquea `canApply`.
    assert_eq!(
        sc["canApply"],
        serde_json::Value::Bool(false),
        "con `allowWarnings:false` ENVIADO y al menos un warning, `canApply` debe ser `false` \
         (el campo enviado se respeta, no se pisa por el default del campo omitido): {sc:?}"
    );
}

/// Criterio `policy_vacia_equivale_a_omitirla`: **Dado** `policy: {}` (objeto vacío), **Cuando** se
/// planifica, **Entonces** equivale a omitir `policy` entera — mismo `canApply` sobre el MISMO
/// `operations`/workspace. Control de forma: `policy: {}` no debe dar `INVALID_SCHEMA` ni un
/// veredicto distinto al de omitir la clave.
#[test]
fn policy_vacia_equivale_a_omitirla() {
    let dir = workspace_cinco_relacionados();

    let linea_vacia = change_plan_line(None, cinco_operaciones(), serde_json::json!({}));
    let resp_vacia = roundtrip(dir.path(), &[linea_vacia.as_str()], 1);
    assert!(
        resp_vacia[0]["result"]["isError"].as_bool() != Some(true),
        "`policy: {{}}` no debe dar isError/INVALID_SCHEMA: {resp_vacia:?}"
    );
    let sc_vacia = plan_sc(&resp_vacia[0]);

    // Omitir `policy` ENTERA: sin la clave en `arguments` (no `change_plan_line`, que siempre la
    // incluye) — construida a mano para que la ausencia de la clave sea literal.
    let args_omitida = serde_json::json!({ "operations": cinco_operaciones() });
    let linea_omitida = serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": "change_plan", "arguments": args_omitida }
    })
    .to_string();
    let resp_omitida = roundtrip(dir.path(), &[linea_omitida.as_str()], 1);
    assert!(
        resp_omitida[0]["result"]["isError"].as_bool() != Some(true),
        "omitir `policy` entera debe seguir funcionando (control anti-vacuo): {resp_omitida:?}"
    );
    let sc_omitida = plan_sc(&resp_omitida[0]);

    assert_eq!(
        sc_vacia["canApply"], sc_omitida["canApply"],
        "`policy: {{}}` debe producir el MISMO `canApply` que omitir `policy` entera: \
         vacía = {:?}, omitida = {:?}",
        sc_vacia["canApply"], sc_omitida["canApply"]
    );
}

/// Control anti-vacuo (`policy` completa sigue igual): **Dado** un workspace válido, **Cuando** se
/// llama a `change_plan` con `policy: {"requireValidResult": false, "allowWarnings": true}` (los
/// DOS campos presentes — el camino que ya funcionaba hoy), **Entonces** el plan se computa y el
/// arreglo del campo omitido no debe alterar este caso.
#[test]
fn policy_completa_sigue_funcionando_igual() {
    let dir = workspace_cinco_relacionados();
    let line = change_plan_line(None, cinco_operaciones(), policy_permisiva());
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);

    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "una `policy` COMPLETA no debe verse afectada por el arreglo del campo omitido: {resp:?}"
    );
    let sc = plan_sc(&resp[0]);
    assert!(
        sc["changeSetId"].as_str().is_some_and(|s| !s.is_empty()),
        "una `policy` completa debe seguir produciendo un plan con `changeSetId`: {resp:?}"
    );
}

/// Control anti-vacuo (clave DESCONOCIDA en `policy`): historia del criterio — E29-H02 (L251-253
/// de su spec) declaró EXPLÍCITAMENTE que este caso no era suyo y lo delegó en E29-H08 («rechazo
/// estricto» si esa historia ya estaba integrada). En su primera versión, este test fijaba el
/// comportamiento TOLERADO-provisional de entonces (sin `deny_unknown_fields`, serde ignoraba la
/// clave y el plan se computaba igual). **E29-H08 ya está integrada** y volvió ese comportamiento
/// en rechazo: el wire estricto de parámetros no declarados alcanza también a las claves de
/// `policy`, así que una `policy` con `"strictMode"` (que no existe en `PlanPolicy`) es HOY
/// `INVALID_SCHEMA`, nombrando la clave sobrante y las declaradas.
///
/// **Dado** una `policy` con los DOS campos reconocidos presentes (para que este test sea
/// independiente del arreglo del campo omitido: aísla exclusivamente el efecto de la clave
/// desconocida) MÁS una clave que no existe en `PlanPolicy` (`"strictMode"`), **Cuando** se
/// planifica, **Entonces** la tool RECHAZA con `INVALID_SCHEMA` y el mensaje nombra `strictMode`.
#[test]
fn policy_con_clave_desconocida_se_rechaza_desde_h08() {
    let dir = workspace_cinco_relacionados();
    let ops = serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "d.md" },
          "patch": { "description": "d actualizada por el plan" } },
    ]);
    let line = change_plan_line(
        None,
        ops,
        serde_json::json!({
            "requireValidResult": false, "allowWarnings": true, "strictMode": true
        }),
    );
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);

    assert_eq!(
        resp[0]["result"]["isError"],
        serde_json::Value::Bool(true),
        "desde E29-H08, una clave desconocida en `policy` debe RECHAZARSE (isError), no \
         ignorarse: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("INVALID_SCHEMA"),
        "el rechazo debe exponer el código estable «INVALID_SCHEMA»: {resp:?}"
    );
    assert!(
        texto.contains("strictMode"),
        "el mensaje debe nombrar la clave sobrante «strictMode» (no un rechazo genérico): {resp:?}"
    );
}

/// PIN del default `requireValidResult: true` (`PlanPolicy::default()`, `plan.rs:288`), que el
/// mutation testing del juez ciego encontró SIN cubrir: mutar esa línea a `false` deja el
/// workspace entero en verde porque ningún test existente ejercita la rama `requireValidResult`
/// del campo OMITIDO con un resultado NO conforme. Este test nace VERDE (no es la fase roja de
/// E29-H02: el defecto que arregla la historia es que la deserialización parcial no fallara con
/// `INVALID_SCHEMA`, no el valor del default) — su función es de PIN, para que una regresión
/// futura del `Default` (p. ej. a `false`) rompa la suite.
///
/// **Dado** un workspace cuyo resultado simulado tiene al menos un error de validación, **Cuando**
/// se llama a `change_plan` con `policy: {"allowWarnings": false}` (`requireValidResult` OMITIDO),
/// **Entonces** `canApply` es `false` **porque** la rama `requireValidResult` (default `true`) lo
/// bloquea — no la rama `allowWarnings`, que en este fixture no dice nada sobre errores. Se
/// asevera la PRECONDICIÓN del fixture (`errors >= 1`) para que el test no pueda salvarse por otra
/// rama: si el fixture no tuviera errores, un `requireValidResult` mutado a `false` seguiría dando
/// `canApply` indeterminado por la rama de warnings, y el pin no pincharía nada.
#[test]
fn policy_parcial_sin_require_valid_result_bloquea_por_el_default_true_con_resultado_no_conforme() {
    let dir = workspace_cinco_relacionados();
    // `d.md` enlaza a un `.md` inexistente → `LINK-TARGET-MISSING`/Err (mismo patrón que
    // `plan_no_conforme_rechaza` de `crates/lodestar-core/tests/core.rs`): el resultado hipotético
    // queda NO conforme, sin depender de ningún warning.
    let ops = serde_json::json!([
        { "op": "replace_body", "ref": { "path": "d.md" },
          "body": "# D\n\n[roto](no-existe.md)\n" },
    ]);
    // Solo `allowWarnings`: `requireValidResult` queda OMITIDO — debe tomar el default `true`.
    let line = change_plan_line(None, ops, serde_json::json!({ "allowWarnings": false }));
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);

    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "una `policy` PARCIAL (solo `allowWarnings`) no debe dar isError/INVALID_SCHEMA: {resp:?}"
    );
    let sc = plan_sc(&resp[0]);

    // Precondición del fixture: el resultado hipotético tiene >=1 error. Sin esto, un
    // `requireValidResult` mutado a `false` no se distinguiría de la rama `allowWarnings`, y el
    // pin no ejercitaría lo que dice ejercitar.
    let errors = sc["diagnosticsAfter"]["errors"]
        .as_u64()
        .unwrap_or_else(|| {
            panic!("change_plan debe devolver diagnosticsAfter.errors (u64): {sc:?}")
        });
    assert!(
        errors >= 1,
        "precondición del fixture: el resultado hipotético debe tener >=1 error para que el pin \
         ejercite la rama `requireValidResult`, no la de `allowWarnings`; diagnosticsAfter = {:?}",
        sc["diagnosticsAfter"]
    );

    // `requireValidResult` OMITIDO ⇒ debe tomar el default `true` ⇒ un resultado no conforme
    // bloquea `canApply`, aunque `allowWarnings:false` no tenga ningún warning que morder.
    assert_eq!(
        sc["canApply"],
        serde_json::Value::Bool(false),
        "con `requireValidResult` OMITIDO (debe tomar el default `true`) y un resultado NO \
         conforme, `canApply` debe ser `false`; si el default fuera `false`, este resultado no \
         conforme no bloquearía nada y `canApply` saldría `true`: {sc:?}"
    );
}

// ---------------------------------------------------------------------------
// E29-H01 — Config estricta: el servidor MCP tampoco arranca con una config que no entiende
// (`requirements/epica-29-honestidad-superficie.md`, `decisiones §16(e)` + `§23/A-08`).
//
// La mitad de workspace vive en `crates/lodestar-workspace/tests/config.rs` y la de CLI en
// `crates/lodestar-cli/tests/e2e.rs`. Este test cierra el criterio de la **segunda fachada**: el
// mismo `config.yaml` que hace salir a la CLI con 3 no puede dejar al MCP sirviendo con la política
// por defecto durante toda una sesión de agente.
// ---------------------------------------------------------------------------

/// E29-H01 · Criterio `mcp_no_arranca_con_config_de_clave_desconocida`:
/// **Dado** un `.lodestar/config.yaml` con `workspace: { writeableRoots: ["notas"] }` (typo de
/// `writableRoots`), **Cuando** arranca `lodestar-mcp`, **Entonces** el proceso falla al abrir el
/// workspace en vez de servir con la política por defecto.
///
/// ## Por qué el MCP tiene su propio criterio
///
/// El daño es peor aquí que en la CLI. `lodestar check` es un one-shot y su exit code lo lee un CI;
/// el MCP **se queda vivo toda la sesión**, y una `writableRoots` descartada por un typo deja la
/// política de escritura en su default —que es *«todo el workspace es escribible»* (`Vec` vacío =
/// sin restricción, ver `WorkspaceSection::writable_roots`)—, o sea **más permisiva** que la que el
/// usuario escribió, delante de un agente que sí puede escribir. Que la CLI aprenda a rechazar no
/// implica que el MCP lo haga: son dos `main` distintos y el criterio de `§15` es explícito en que
/// el repo no puede quedarse con dos criterios opuestos.
///
/// ## Cómo se observa «no arranca»
///
/// Con el patrón documentado en `roundtrip_en`: si el servidor aborta al arrancar, el vector de
/// respuestas sale **vacío**. Por eso se assertea primero la longitud —para que el rojo se lea como
/// «el servidor arrancó cuando no debía» y no como un índice fuera de rango— y se usa `tools/list`,
/// la petición más inofensiva posible: si llega respuesta a eso, el servidor está sirviendo.
///
/// El exit code y el mensaje por stderr se comprueban aparte, lanzando el binario sin stdin: el
/// contrato de arranque del MCP es exit 3 con el motivo por stderr (`main.rs`: «no se pudo abrir el
/// workspace»), y stdout tiene que seguir siendo JSON-RPC puro —vacío, en este caso—, porque un
/// cliente que lo parsee no puede encontrarse un mensaje de error suelto.
///
/// Fase ROJA: hoy `WorkspaceConfig` no lleva `deny_unknown_fields`, así que `App::open` devuelve
/// `Ok` y el servidor responde `tools/list` con normalidad.
#[test]
fn mcp_no_arranca_con_config_de_clave_desconocida() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "alfa.md",
        "---\ntype: Nota\ntitle: Alfa\ndescription: d\n---\n\n# Alfa\n\ncuerpo\n",
    );
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "workspace:\n  writeableRoots: [\"notas\"]\n",
    );

    // --- (1) No sirve: ninguna respuesta a la petición más inofensiva ----------------
    let resp = roundtrip(
        dir.path(),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"tools/list"}"#],
        1,
    );
    assert!(
        resp.is_empty(),
        "con una clave desconocida en `.lodestar/config.yaml` el servidor NO puede arrancar: \
         serviría toda la sesión con la política por defecto —más permisiva que la que el usuario \
         escribió— delante de un agente que puede escribir. Respondió: {resp:?}"
    );

    // --- (2) …y lo hace con exit 3 y el motivo por stderr ----------------------------
    let out = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(dir.path())
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(
        out.status.code(),
        Some(3),
        "el arranque fallido del MCP es exit 3 (`main.rs`, mismo código que la puerta de CI); \
         stderr=\n{stderr}"
    );
    assert!(
        stderr.contains("writeableRoots"),
        "el mensaje debe NOMBRAR la clave rechazada: un «no se pudo abrir el workspace» a secas \
         deja al usuario sin saber qué línea del YAML borrar; stderr=\n{stderr}"
    );
    assert!(
        String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        "stdout es JSON-RPC puro también cuando el arranque falla: el motivo va por stderr; \
         stdout=\n{}",
        String::from_utf8_lossy(&out.stdout)
    );
}

// ---------------------------------------------------------------------------
// E29-H03 — `has(frontmatter)` responde la verdad POR EL WIRE.
// `requirements/epica-29-honestidad-superficie.md §E29-H03` · `decisiones §19(a)` ·
// `ARCHITECTURE.md §20.8` (la promesa literal: «existencia `has(x)` `missing(x)` (incluido
// `has(frontmatter)`)») · `CLAUDE.md` invariante #3.
//
// La semántica pura la fijan los tests homónimos de `crates/lodestar-core/tests/consulta.rs`. Este
// es el caso e2e que la historia pide para que la evidencia rojo→verde sea observable **por el
// wire**, que es como se observó el hallazgo (escribiendo `docs/user/query-language.md` contra
// `examples/demo/`: `has(frontmatter)` → 0 de 10, `missing(frontmatter)` → 10 de 10, mientras
// `document.has_frontmatter = true` → 7).
//
// SÍNTOMA verificado hoy (v0.5.0) contra el binario real:
//   knowledge_search {where: "has(frontmatter)"}      -> []            (deberían ser los 2 con bloque)
//   knowledge_search {where: "missing(frontmatter)"}  -> los 3         (debería ser el 1 sin bloque)
//
// ROJO esperado HOY: por ASERCIÓN (ninguna tool nueva, ningún parámetro nuevo, ningún stub).
// ---------------------------------------------------------------------------

/// Workspace del criterio e2e: 2 documentos **con** bloque de frontmatter y 1 **sin** él, con
/// cuerpos deliberadamente parecidos para que solo la presencia del bloque los separe. Los dos con
/// bloque no comparten ninguna clave (`status` vs `owner`): así el veredicto de `has(frontmatter)`
/// no puede confundirse con el de `has(<una clave concreta>)`.
fn workspace_con_y_sin_frontmatter() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "con-claves.md",
        "---\nstatus: accepted\n---\n\n# Con claves\n\ntexto corriente.\n",
    );
    write(
        dir.path(),
        "con-otra-clave.md",
        "---\nowner: ana\n---\n\n# Con otra clave\n\ntexto corriente.\n",
    );
    write(
        dir.path(),
        "sin-bloque.md",
        "# Sin bloque\n\ntexto corriente.\n",
    );
    dir
}

/// E29-H03 · Criterio `has_frontmatter_pelado_casa_los_documentos_con_frontmatter` (por el wire):
/// Dado un workspace con 3 documentos, 2 con bloque de frontmatter y 1 sin él, Cuando se llama a
/// `knowledge_search` con `where: "has(frontmatter)"`, Entonces devuelve exactamente los 2 con
/// bloque; con `missing(frontmatter)`, exactamente el 1 sin bloque; y los dos conjuntos coinciden
/// con los de `document.has_frontmatter` (invariante #3: una sola verdad computada, también por el
/// wire).
#[test]
fn has_frontmatter_por_el_wire_casa_los_documentos_con_frontmatter() {
    let dir = workspace_con_y_sin_frontmatter();
    let resp = roundtrip(
        dir.path(),
        &[
            ks_call(serde_json::json!({ "where": "has(frontmatter)" })).as_str(),
            ks_call(serde_json::json!({ "where": "missing(frontmatter)" })).as_str(),
            ks_call(serde_json::json!({ "where": "document.has_frontmatter = true" })).as_str(),
            ks_call(serde_json::json!({ "where": "document.has_frontmatter = false" })).as_str(),
            ks_call(serde_json::json!({ "filter": { "has": { "field": "frontmatter" } } }))
                .as_str(),
        ],
        5,
    );
    let conjunto = |i: usize| -> std::collections::BTreeSet<String> {
        search_paths(&resp[i]).into_iter().collect()
    };
    let con_bloque: std::collections::BTreeSet<String> = ["con-claves.md", "con-otra-clave.md"]
        .into_iter()
        .map(String::from)
        .collect();
    let sin_bloque: std::collections::BTreeSet<String> =
        ["sin-bloque.md"].into_iter().map(String::from).collect();
    // Resumen legible de las 5 consultas para los mensajes de fallo: el `resp` crudo son 5
    // respuestas JSON-RPC completas (con snippets y revisiones) y esconde el dato que importa.
    let resumen = format!(
        "has(frontmatter)={:?} · missing(frontmatter)={:?} · document.has_frontmatter=true{:?} \
         · =false{:?} · filter{{has:frontmatter}}={:?}",
        conjunto(0),
        conjunto(1),
        conjunto(2),
        conjunto(3),
        conjunto(4)
    );

    // Guarda de no vacuidad: el camino LARGO ya responde bien hoy por el wire, y responde algo
    // distinto de «todos» y de «ninguno». Es la verdad a la que se atan los asserts de abajo.
    assert_eq!(
        conjunto(2),
        con_bloque,
        "premisa: `document.has_frontmatter = true` ya distingue hoy los 2 documentos con bloque \
         por el wire. {resumen}"
    );
    assert_eq!(
        conjunto(3),
        sin_bloque,
        "premisa simétrica: `document.has_frontmatter = false` devuelve solo el que no lo tiene. \
         {resumen}"
    );

    // (a) `has(frontmatter)` casa los que TIENEN bloque.
    assert_eq!(
        conjunto(0),
        con_bloque,
        "`has(frontmatter)` debe casar los documentos con bloque de frontmatter. Hoy devuelve [] \
         para todo workspace —el hallazgo de `§19(a)`: 0 de 10 sobre `examples/demo/`—, que es la \
         respuesta CONTRARIA a la correcta y sin ningún error que lo delate. {resumen}"
    );

    // (b) `missing(frontmatter)` casa el que NO lo tiene.
    assert_eq!(
        conjunto(1),
        sin_bloque,
        "`missing(frontmatter)` debe casar solo el documento sin bloque. Hoy casa los 3 (10 de 10 \
         sobre `examples/demo/`): es la negación de un `has` que siempre es `false`. {resumen}"
    );

    // (c) Camino corto y camino largo son el MISMO conjunto por el wire (invariante #3).
    assert_eq!(
        conjunto(0),
        conjunto(2),
        "`has(frontmatter)` y `document.has_frontmatter = true` deben devolver el mismo conjunto \
         por el wire: la presencia del bloque no se computa dos veces con dos respuestas. {resumen}"
    );
    assert_eq!(
        conjunto(1),
        conjunto(3),
        "…y `missing(frontmatter)` con `document.has_frontmatter = false`. {resumen}"
    );

    // (d) La otra puerta del wire (`filter` JSON, §20.10) responde lo mismo que `where`.
    assert_eq!(
        conjunto(4),
        conjunto(0),
        "`{{\"has\":{{\"field\":\"frontmatter\"}}}}` por `filter` debe devolver lo mismo que \
         `has(frontmatter)` por `where`: comparten `build_field_path` y el mismo `Expression`, y la \
         superficie no puede tener dos verdades según la puerta. {resumen}"
    );
}

/// E29-H03 · Control anti-vacuo por el wire: `has()` con anclaje y sufijo, con clave a secas y con
/// namespace calculado sigue respondiendo lo mismo que antes del arreglo.
///
/// El arreglo toca el camino del anclaje —el mismo que resuelve `frontmatter.<clave>`—, así que si
/// reconocer el anclaje pelado se llevara por delante el resto del operador, este test lo vería por
/// la misma puerta por la que se observó el defecto. **Verde hoy**, y debe seguirlo estando.
#[test]
fn has_con_sufijo_y_de_namespace_no_cambia_por_el_wire() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "con-claves.md",
        "---\nstatus: accepted\n---\n\n# Con claves\n\ntexto corriente.\n",
    );
    write(
        dir.path(),
        "sin-bloque.md",
        "# Sin bloque\n\ntexto corriente.\n",
    );
    let resp = roundtrip(
        dir.path(),
        &[
            ks_call(serde_json::json!({ "where": "has(frontmatter.status)" })).as_str(),
            ks_call(serde_json::json!({ "where": "has(status)" })).as_str(),
            ks_call(serde_json::json!({ "where": "has(inventada)" })).as_str(),
            ks_call(serde_json::json!({ "where": "has(graph.backlinks)" })).as_str(),
        ],
        4,
    );
    let conjunto = |i: usize| -> std::collections::BTreeSet<String> {
        search_paths(&resp[i]).into_iter().collect()
    };
    let solo_con_claves: std::collections::BTreeSet<String> =
        ["con-claves.md"].into_iter().map(String::from).collect();
    let ambos: std::collections::BTreeSet<String> = ["con-claves.md", "sin-bloque.md"]
        .into_iter()
        .map(String::from)
        .collect();
    let resumen = format!(
        "has(frontmatter.status)={:?} · has(status)={:?} · has(inventada)={:?} \
         · has(graph.backlinks)={:?}",
        conjunto(0),
        conjunto(1),
        conjunto(2),
        conjunto(3)
    );

    assert_eq!(
        conjunto(0),
        solo_con_claves,
        "`has(frontmatter.status)` (anclaje CON sufijo) sigue direccionando la CLAVE, no el bloque. \
         {resumen}"
    );
    assert_eq!(
        conjunto(1),
        solo_con_claves,
        "`has(status)` sin anclaje responde lo mismo que con él (§20.8). {resumen}"
    );
    assert!(
        conjunto(2).is_empty(),
        "`has(inventada)` sigue sin casar a nadie: una clave ausente es ausencia, no presencia. \
         {resumen}"
    );
    assert_eq!(
        conjunto(3),
        ambos,
        "`has(graph.backlinks)` sigue siendo trivialmente cierto para TODO documento, incluido el \
         que no tiene frontmatter (fuera de alcance de E29-H03). {resumen}"
    );
}

// ---------------------------------------------------------------------------
// E29-H05 — `knowledge_check` scope `paths` con un path inexistente responde `DOCUMENT_NOT_FOUND`.
// `requirements/epica-29-honestidad-superficie.md §E29-H05` · `decisiones §23/A-07` (criterio
// ratificado: `DOCUMENT_NOT_FOUND`) · `decisiones §22` (principio anti-typo) · `CLAUDE.md`
// invariante #1.
//
// SÍNTOMA (caso G1-23 del testbench homelab): `knowledge_check(scope: {kind: "paths",
// paths: ["no-existe.md"]})` devuelve 0 diagnósticos SIN error — indistinguible de «ese documento
// está impecable». Con una lista mixta (`["real.md", "typo.md"]`) el resultado es el de `real.md` a
// secas: el agente cree haber auditado dos documentos y auditó uno.
//
// CAUSA RAÍZ (`crates/lodestar-app/src/lib.rs`, `App::scope_paths`, brazo `CheckScope::Paths`):
// `Ok(paths.iter().cloned().collect())` — mete los `RelPath` en el conjunto sin comprobar que
// existan en el inventario. Los brazos `Document`/`Affected` sí resuelven con `self.resolve_ref(…)?`
// y por eso ya dan `DOCUMENT_NOT_FOUND`.
//
// ROJO esperado HOY: por ASERCIÓN (ninguna tool nueva, ningún parámetro nuevo, ningún stub — el
// brazo `Paths` ya existe, solo le falta la comprobación).
// ---------------------------------------------------------------------------

/// Workspace mínimo del criterio: un único documento real (`notas/alfa.md`), sin enlaces ni
/// frontmatter que puedan aportar diagnósticos que confundan la lectura del resultado.
fn workspace_check_paths() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "notas/alfa.md",
        "# Alfa\n\nDocumento real, sin enlaces ni frontmatter.\n",
    );
    dir
}

/// Construye la línea JSON-RPC de `knowledge_check` con `scope: {kind: "paths", paths: […]}`.
fn check_paths_call(paths: &[&str]) -> String {
    let arguments = serde_json::json!({
        "scope": { "kind": "paths", "paths": paths }
    });
    format!(
        r#"{{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{{"name":"knowledge_check","arguments":{}}}}}"#,
        arguments
    )
}

/// E29-H05 · Criterio `check_scope_paths_con_path_inexistente_falla`:
/// Dado un workspace con `notas/alfa.md`, Cuando se llama a `knowledge_check` con
/// `scope: {kind: "paths", paths: ["notas/no-existe.md"]}`, Entonces la respuesta es un error de
/// EJECUCIÓN de la tool (`isError`, no un error de protocolo JSON-RPC) con el código estable
/// `DOCUMENT_NOT_FOUND` que nombra el path — el mismo contrato que ya cumplen los scopes `document`
/// y `affected`.
#[test]
fn check_scope_paths_con_path_inexistente_falla() {
    let dir = workspace_check_paths();
    let resp = roundtrip(
        dir.path(),
        &[check_paths_call(&["notas/no-existe.md"]).as_str()],
        1,
    );

    assert_eq!(
        resp[0]["result"]["isError"], true,
        "un scope `paths` con un path inexistente debe dar isError en knowledge_check: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "un path inexistente en scope.paths NO debe ser un error de protocolo JSON-RPC: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("DOCUMENT_NOT_FOUND"),
        "el error debe exponer el código estable «DOCUMENT_NOT_FOUND»: {resp:?}"
    );
    assert!(
        texto.contains("notas/no-existe.md"),
        "el mensaje debe NOMBRAR el path que no resolvió (mismo estilo que resolve_ref): {resp:?}"
    );
}

/// E29-H05 · Criterio `check_scope_paths_falla_aunque_haya_paths_validos`:
/// Dado ese mismo workspace, Cuando el scope mezcla un path real (`notas/alfa.md`) y uno inexistente
/// (`notas/typo.md`), Entonces también falla con `DOCUMENT_NOT_FOUND` — no devuelve el informe
/// parcial del path real. El síntoma exacto del testbench: con una lista mixta, el agente cree haber
/// auditado dos documentos y auditó uno.
#[test]
fn check_scope_paths_falla_aunque_haya_paths_validos() {
    let dir = workspace_check_paths();
    let resp = roundtrip(
        dir.path(),
        &[check_paths_call(&["notas/alfa.md", "notas/typo.md"]).as_str()],
        1,
    );

    assert_eq!(
        resp[0]["result"]["isError"], true,
        "una lista mixta (un path real + uno inexistente) debe fallar entera, no devolver el \
         informe parcial del real: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("DOCUMENT_NOT_FOUND"),
        "el error debe exponer el código estable «DOCUMENT_NOT_FOUND»: {resp:?}"
    );
    assert!(
        texto.contains("notas/typo.md"),
        "el mensaje debe nombrar el path inexistente, no el real: {resp:?}"
    );
}

/// E29-H05 · Criterio `check_scope_paths_reporta_el_primer_path_inexistente`:
/// Dado un scope con DOS paths inexistentes, Cuando se llama, Entonces el mensaje nombra el
/// PRIMERO de la lista recibida (orden determinista de la lista tal cual la envió el cliente, no el
/// orden de un `BTreeSet`), para que el mensaje sea reproducible y apunte a lo que el agente escribió
/// primero. `zzz-no-existe.md` ordena DESPUÉS de `aaa-no-existe.md` en orden lexicográfico, así que
/// si el implementador reportara por orden de `BTreeSet` en vez de por orden de la lista recibida,
/// este test lo distinguiría.
#[test]
fn check_scope_paths_reporta_el_primer_path_inexistente() {
    let dir = workspace_check_paths();
    let resp = roundtrip(
        dir.path(),
        &[check_paths_call(&["zzz-no-existe.md", "aaa-no-existe.md"]).as_str()],
        1,
    );

    assert_eq!(
        resp[0]["result"]["isError"], true,
        "dos paths inexistentes deben fallar: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("zzz-no-existe.md"),
        "el mensaje debe nombrar el PRIMERO de la lista recibida («zzz-no-existe.md», pese a \
         ordenar después de «aaa-no-existe.md» en un BTreeSet): {resp:?}"
    );
    assert!(
        !texto.contains("aaa-no-existe.md"),
        "el mensaje NO debe nombrar el segundo path inexistente: solo el primero de la lista \
         recibida se reporta: {resp:?}"
    );
}

/// E29-H05 · Criterio `check_scope_paths_trata_lo_excluido_como_inexistente`:
/// Dado un documento excluido por `.lodestarignore`, Cuando se pide en `scope.paths`, Entonces es
/// `DOCUMENT_NOT_FOUND` (no está en el inventario) — el mismo criterio que ya aplica `resolve_ref`
/// para los scopes `document`/`affected`: el inventario es la única verdad de qué documentos hay.
#[test]
fn check_scope_paths_trata_lo_excluido_como_inexistente() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "notas/alfa.md",
        "# Alfa\n\nDocumento real, sin enlaces ni frontmatter.\n",
    );
    write(
        dir.path(),
        "borradores/wip.md",
        "# WIP\n\nExcluido por .lodestarignore.\n",
    );
    write(dir.path(), ".lodestarignore", "borradores/\n");

    let resp = roundtrip(
        dir.path(),
        &[check_paths_call(&["borradores/wip.md"]).as_str()],
        1,
    );

    assert_eq!(
        resp[0]["result"]["isError"], true,
        "un path excluido por .lodestarignore (fuera del inventario) debe dar DOCUMENT_NOT_FOUND, \
         no un informe vacío: {resp:?}"
    );
    let texto = resp[0].to_string();
    assert!(
        texto.contains("DOCUMENT_NOT_FOUND"),
        "el error debe exponer el código estable «DOCUMENT_NOT_FOUND»: {resp:?}"
    );
    assert!(
        texto.contains("borradores/wip.md"),
        "el mensaje debe nombrar el path excluido: {resp:?}"
    );
}

/// E29-H05 · Criterio `check_scope_paths_valido_sigue_funcionando` (control anti-vacuo):
/// Dado un scope con paths que TODOS existen, Cuando se llama, Entonces devuelve el informe (sin
/// error) exactamente como hoy — el rechazo del path inexistente no puede haberse llevado por delante
/// el caso feliz. Endurecido tras revisión del juez ciego: el fixture antiguo (documento sin
/// diagnósticos) dejaba pasar un mutante que "valida existencia pero devuelve el conjunto vacío"
/// (el scope respondería «impecable» sin haber auditado nada, indistinguible del caso feliz). Aquí
/// `roto.md` tiene un enlace roto propio (`LINK-TARGET-MISSING`), así que el criterio exige que ESE
/// diagnóstico concreto llegue en `diagnostics`, no solo que `isError` sea `false`.
#[test]
fn check_scope_paths_valido_sigue_funcionando() {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "roto.md",
        "# Roto\n\nEnlace a un documento inexistente: [falta](inexistente.md).\n",
    );
    let resp = roundtrip(dir.path(), &[check_paths_call(&["roto.md"]).as_str()], 1);

    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "un scope `paths` con paths que TODOS existen no debe fallar: {resp:?}"
    );
    let diags = check_diagnostics(&resp[0]);
    let diag = diags
        .iter()
        .find(|d| d["code"] == "LINK-TARGET-MISSING")
        .unwrap_or_else(|| {
            panic!(
                "el scope `paths: [\"roto.md\"]` debe traer el LINK-TARGET-MISSING de «roto.md»: \
                 un informe vacío pasaría por casualidad, no porque el documento se haya auditado \
                 de verdad. Diagnósticos: {diags:?}"
            )
        });
    assert!(
        diag_targets(diag).iter().any(|t| t == "roto.md"),
        "el diagnóstico debe señalar a «roto.md»: {diag:?}"
    );
    assert_eq!(
        resp[0]["result"]["structuredContent"]["valid"], false,
        "con un LINK-TARGET-MISSING de severidad Err, el informe del scope debe ser NO válido: {resp:?}"
    );
}

/// E29-H05 · Criterio `check_scope_paths_vacio_no_es_error` (control anti-vacuo del borde):
/// Dado `scope: {kind: "paths", paths: []}`, Cuando se llama, Entonces devuelve un informe vacío SIN
/// error — un scope `paths` legítimamente vacío no es lo mismo que un path que no resuelve, y el
/// rechazo de esta historia no puede confundir los dos casos.
#[test]
fn check_scope_paths_vacio_no_es_error() {
    let dir = workspace_check_paths();
    let resp = roundtrip(dir.path(), &[check_paths_call(&[]).as_str()], 1);

    assert!(
        resp[0]["result"]["isError"].as_bool() != Some(true),
        "un scope `paths` VACÍO no debe ser un error: {resp:?}"
    );
    let diagnosticos = resp[0]["result"]["structuredContent"]["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("el informe debe traer `diagnostics` (array): {resp:?}"));
    assert!(
        diagnosticos.is_empty(),
        "un scope `paths` vacío no puede aportar ningún diagnóstico: {resp:?}"
    );
    assert_eq!(
        resp[0]["result"]["structuredContent"]["valid"], true,
        "un scope vacío es trivialmente válido: {resp:?}"
    );
}

/// E29-H05 · Control anti-vacuo de la historia hermana: los scopes `document`/`affected` con una
/// referencia inexistente siguen dando `DOCUMENT_NOT_FOUND` exactamente igual que hoy — la corrección
/// del scope `paths` no puede haber tocado (ni roto) el camino que ya funcionaba.
#[test]
fn check_scope_document_y_affected_siguen_dando_document_not_found() {
    let dir = workspace_check_paths();
    let doc_call = r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_check","arguments":{"scope":{"kind":"document","ref":{"path":"notas/no-existe.md"}}}}}"#;
    let affected_call = r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"knowledge_check","arguments":{"scope":{"kind":"affected","refs":[{"path":"notas/no-existe.md"}],"depth":1}}}}"#;
    let resp = roundtrip(dir.path(), &[doc_call, affected_call], 2);

    for (i, nombre) in [(0, "document"), (1, "affected")] {
        assert_eq!(
            resp[i]["result"]["isError"], true,
            "scope `{nombre}` con una ref inexistente debe seguir dando isError: {:?}",
            resp[i]
        );
        let texto = resp[i].to_string();
        assert!(
            texto.contains("DOCUMENT_NOT_FOUND"),
            "scope `{nombre}` debe seguir exponiendo DOCUMENT_NOT_FOUND: {:?}",
            resp[i]
        );
    }
}

// ---------------------------------------------------------------------------
// E29-H06 — Un workspace vacío se distingue de un directorio equivocado, POR EL WIRE.
// `requirements/epica-29-honestidad-superficie.md §E29-H06` · `decisiones §16(f)`.
//
// SÍNTOMA: un directorio sin `.md` (o cuya `discovery.include` excluye todo) da
// `workspace_status`/`knowledge_check` sin ningún aviso — indistinguible de un repo legítimamente
// vacío en vez de un `cd` al directorio equivocado. Esta sección fija el diagnóstico
// `WORKSPACE-EMPTY` (severidad `warn`) visible en `knowledge_check(scope: workspace)` por el wire,
// SIN tumbar `valid`.
//
// PUERTA DE DECISIÓN DE ANCLAJE (declarada en la spec, resuelta en la fase roja — ver la nota
// completa en `crates/lodestar-app/tests/validacion.rs`, sección gemela E29-H06, y en
// `crates/lodestar-cli/tests/e2e.rs`): `RelPath::new("")` es inválido por diseño (invariante #6 de
// `CLAUDE.md`, único chokepoint de `RelPath`), así que anclar `WORKSPACE-EMPTY` a la raíz como
// `target` es INVIABLE. Se elige extender el indexado de `App::full_analysis` para que los
// diagnósticos sin `target` no se descarten. `knowledge_check` en cambio ya soporta un anchor sin
// target (usa `check.targets.first()...unwrap_or_default()` sobre un `Vec<(String, Check)>`, no un
// `BTreeMap<RelPath, _>`), así que estos tests SOLO exigen el efecto observable por el wire: el
// código `WORKSPACE-EMPTY` presente en `structuredContent.diagnostics`, con severidad `warn` y sin
// tumbar `valid`.
//
// ROJO esperado HOY: por ASERCIÓN (no hay productor de `WORKSPACE-EMPTY` en ninguna parte; el stub
// de `CheckCode::WorkspaceEmpty` en `lodestar-core::types` es solo la firma, sin lógica).
// ---------------------------------------------------------------------------

/// Construye la línea JSON-RPC de `knowledge_check(scope: workspace)`.
fn check_workspace_call() -> String {
    r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_check","arguments":{"scope":{"kind":"workspace"}}}}"#.to_string()
}

/// E29-H06 · Criterio `knowledge_check_en_workspace_vacio_avisa`:
/// Dado un directorio temporal SIN ningún `.md`, Cuando se llama a `knowledge_check` scope
/// `workspace`, Entonces el informe incluye el diagnóstico `WORKSPACE-EMPTY` con severidad `warn` y
/// `valid` sigue siendo `true`.
#[test]
fn knowledge_check_en_workspace_vacio_avisa() {
    let dir = tempfile::tempdir().unwrap();
    // Un fichero no-Markdown NO debe cambiar nada: el inventario de documentos sigue vacío.
    write(dir.path(), "LEEME.txt", "esto no es un documento OKF\n");

    let resp = roundtrip(dir.path(), &[check_workspace_call().as_str()], 1);
    let diags = check_diagnostics(&resp[0]);

    let workspace_empty: Vec<&serde_json::Value> = diags
        .iter()
        .filter(|d| d["code"] == "WORKSPACE-EMPTY")
        .collect();
    assert!(
        !workspace_empty.is_empty(),
        "knowledge_check(scope: workspace) sobre un directorio sin `.md` debe incluir el \
         diagnóstico WORKSPACE-EMPTY: {resp:?}"
    );
    assert!(
        workspace_empty.iter().all(|d| d["level"] == "warn"),
        "WORKSPACE-EMPTY debe ser severidad «warn»: {resp:?}"
    );
    assert_eq!(
        resp[0]["result"]["structuredContent"]["valid"],
        serde_json::Value::Bool(true),
        "un workspace vacío SIN otros diagnósticos sigue siendo `valid: true` (el aviso no bloquea \
         el veredicto): {resp:?}"
    );
}

/// E29-H06 · Criterio `workspace_con_todo_excluido_tambien_avisa` (mitad MCP): un directorio CON
/// `.md` pero cuya `discovery.include` los excluye todos también avisa — «no hay inventario», no
/// solo «no hay ficheros».
#[test]
fn mcp_workspace_con_todo_excluido_tambien_avisa() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "notas/alfa.md", "# Alfa\n\ncontenido real.\n");
    write(
        dir.path(),
        ".lodestar/config.yaml",
        "discovery:\n  include: [\"solo-esto/**/*.md\"]\n",
    );

    let resp = roundtrip(dir.path(), &[check_workspace_call().as_str()], 1);
    let diags = check_diagnostics(&resp[0]);

    assert!(
        diags.iter().any(|d| d["code"] == "WORKSPACE-EMPTY"),
        "un `discovery.include` que excluye TODO también debe avisar con WORKSPACE-EMPTY por el \
         wire: {resp:?}"
    );
}

/// E29-H06 · Criterio `workspace_con_documentos_no_avisa` (control anti-vacuo, mitad MCP): un
/// workspace con AL MENOS un documento no lleva `WORKSPACE-EMPTY` en `knowledge_check`.
#[test]
fn mcp_workspace_con_documentos_no_avisa() {
    let dir = workspace_min();
    let resp = roundtrip(dir.path(), &[check_workspace_call().as_str()], 1);
    let diags = check_diagnostics(&resp[0]);

    assert!(
        !diags.iter().any(|d| d["code"] == "WORKSPACE-EMPTY"),
        "un workspace con documentos (index.md) NO debe llevar WORKSPACE-EMPTY: {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// E29-H09 — `instructions` por perfil y `protocolVersion` no soportada.
//
// Dos defectos del mismo hallazgo (`decisiones §23/D-01`, caso G1-24 del testbench):
// (1) `SERVER_INSTRUCTIONS` es una constante única servida sin mirar el `profile`, así que bajo
//     `readonly` describe el flujo completo de 10 pasos (incluidas las 3 tools de cambio) aunque
//     `tools/list` sirva solo 7 — un agente que las siga acaba en `-32602`. El test histórico
//     `instructions_sin_vocabulario_retirado` (línea 177) SOLO ejercita `standard` vía `roundtrip()`,
//     por eso el drift bajo `readonly` no lo detectó nunca.
// (2) `protocolVersion` no soportada NO se rechaza: el brazo `initialize` (`main.rs` L200-204) la
//     descarta con `.filter(...)` y cae a `2024-11-05` como si el cliente hubiera pedido esa versión
//     — una respuesta de éxito para un handshake que no debería prosperar.
//
// Los tests de abajo son NUEVOS (no tocan `instructions_sin_vocabulario_retirado`, que sigue
// ejercitando solo `standard` y debe seguir verde tal cual): generalizan la guarda a los dos
// perfiles y fijan el rechazo de versión con el vehículo que la historia pide, `roundtrip_profile`.
// ---------------------------------------------------------------------------

/// Extrae `instructions` de la respuesta a `initialize` (posición 0 de `resp`).
fn instructions_de(resp: &[serde_json::Value]) -> String {
    resp[0]["result"]["instructions"]
        .as_str()
        .expect("initialize sirve «instructions» (string)")
        .to_lowercase()
}

/// Extrae los nombres de tool servidos por `tools/list` (posición 1 de `resp`).
fn tools_servidas_de(resp: &[serde_json::Value]) -> Vec<String> {
    resp[1]["result"]["tools"]
        .as_array()
        .expect("tools/list devuelve un array de tools")
        .iter()
        .map(|t| {
            t["name"]
                .as_str()
                .expect("cada tool tiene «name»")
                .to_string()
        })
        .collect()
}

/// E29-H09 · Criterio `instructions_readonly_nombra_solo_las_tools_servidas`:
/// Dado el servidor con `--profile readonly`, Cuando se hace `initialize` + `tools/list`, Entonces
/// el conjunto de tools nombradas en `instructions` es EXACTAMENTE el servido por `tools/list` (7):
/// ni una de menos (un flujo que se salta una tool la deja invisible) ni una de más (nombrar una
/// tool que `tools/call` va a rechazar con `-32602`).
#[test]
fn instructions_readonly_nombra_solo_las_tools_servidas() {
    let dir = workspace_min();
    let resp = roundtrip_profile(
        dir.path(),
        "readonly",
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ],
        2,
    );

    let instructions = instructions_de(&resp);
    let servidas = tools_servidas_de(&resp);
    assert_eq!(
        servidas.len(),
        7,
        "el perfil readonly debe servir 7 tools en tools/list: {servidas:?}"
    );

    for tool in &servidas {
        assert!(
            instructions.contains(tool.as_str()),
            "`{tool}` está en `tools/list` bajo readonly pero `instructions` no la nombra:\n{instructions}"
        );
    }
    // Ninguna tool NO servida por este perfil puede aparecer nombrada: seguirla es un -32602.
    let no_servidas = ["change_plan", "change_apply", "change_revert"];
    for tool in no_servidas {
        assert!(
            !servidas.iter().any(|s| s == tool),
            "sanity: `{tool}` no debería estar en tools/list bajo readonly: {servidas:?}"
        );
        assert!(
            !instructions.contains(tool),
            "bajo readonly, `instructions` nombra `{tool}`, que tools/list NO sirve: seguirla \
             acaba en -32602\n---\n{instructions}"
        );
    }
}

/// E29-H09 · Criterio `instructions_standard_sigue_coincidiendo` (control anti-vacuo): la
/// generalización de la guarda a los dos perfiles no puede romper el caso `standard`, que ya
/// funcionaba. Mismo test que `instructions_sin_vocabulario_retirado` en su mitad de conteo, pero
/// ejercitado explícitamente vía `roundtrip_profile("standard", …)` para que el vehículo sea
/// simétrico al de `readonly`.
#[test]
fn instructions_standard_sigue_coincidiendo() {
    let dir = workspace_min();
    let resp = roundtrip_profile(
        dir.path(),
        "standard",
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/list"}"#,
        ],
        2,
    );

    let instructions = instructions_de(&resp);
    let servidas = tools_servidas_de(&resp);
    assert_eq!(
        servidas.len(),
        10,
        "el perfil standard debe servir las 10 tools objetivo: {servidas:?}"
    );
    for tool in &servidas {
        assert!(
            instructions.contains(tool.as_str()),
            "`{tool}` está en `tools/list` bajo standard pero `instructions` no la nombra:\n{instructions}"
        );
    }
}

/// E29-H09 · Criterio `instructions_readonly_no_nombra_tools_de_cambio` (aserción directa del
/// síntoma reproducible, caso G1-24): bajo `readonly`, `change_apply` no aparece en el texto de
/// `instructions`. Es deliberadamente redundante con
/// `instructions_readonly_nombra_solo_las_tools_servidas` (que ya lo cubre por conjunto) porque la
/// historia lo pide como aserción propia, más legible cuando falla.
#[test]
fn instructions_readonly_no_nombra_tools_de_cambio() {
    let dir = workspace_min();
    let resp = roundtrip_profile(
        dir.path(),
        "readonly",
        &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#],
        1,
    );
    let instructions = instructions_de(&resp[..1]);
    assert!(
        !instructions.contains("change_apply"),
        "bajo readonly, `instructions` no debe mencionar `change_apply`:\n{instructions}"
    );
}

/// E29-H09 · Criterio `protocol_version_no_soportada_se_rechaza`:
/// Dado un `initialize` con `protocolVersion: "1990-01-01"`, Cuando se llama, Entonces la respuesta
/// es un error JSON-RPC `-32602` cuyo mensaje lista las tres versiones aceptadas.
///
/// Decisión de forma (delegada por la historia a la fase roja, ver spec L1004-1009): la spec MCP
/// oficial de negociación de versión (2025-06-18, sección «Version Negotiation») dice que si el
/// servidor no soporta la `protocolVersion` pedida, debe responder con la versión que SÍ soporta y
/// dejar que el CLIENTE decida cerrar la conexión — no es, en el spec base, un error JSON-RPC. Pero
/// la propia historia lo prescribe explícitamente distinto para este repo: «Forma propuesta: error
/// JSON-RPC `-32602`». Se sigue la prescripción explícita de la historia (no la negociación blanda
/// del spec base) porque coincide con el principio rector de la épica —silencio peor que error— y
/// con el patrón que el servidor YA usa para "tool no disponible"/"tool desconocida": mantener dos
/// criterios de rechazo distintos en el mismo servidor (uno blando para protocolVersion, uno duro
/// para tools) sería la clase de inconsistencia que la épica cierra en `§15`.
#[test]
fn protocol_version_no_soportada_se_rechaza() {
    let dir = workspace_min();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"1990-01-01"}}"#,
        ],
        1,
    );
    assert_eq!(
        resp[0]["error"]["code"], -32602,
        "protocolVersion no soportada debe rechazarse con -32602: {resp:?}"
    );
    let msg = resp[0]["error"]["message"]
        .as_str()
        .expect("el error de protocolVersion no soportada lleva mensaje")
        .to_lowercase();
    for version in ["2024-11-05", "2025-03-26", "2025-06-18"] {
        assert!(
            msg.contains(version),
            "el mensaje de rechazo debe listar la versión soportada «{version}»: {msg}"
        );
    }
    // Un initialize rechazado es un handshake fallido, no un error de dominio de tool: no debe
    // llevar `result` (ni siquiera con isError) y el error no es del catálogo de ErrorCode.
    assert!(
        resp[0]["result"].is_null(),
        "un initialize rechazado no debe producir result: {resp:?}"
    );
}

/// E29-H09 · Criterio `initialize_sin_version_sigue_funcionando` (control anti-vacuo: el rechazo
/// de versión no puede cerrarse de más): Dado un `initialize` SIN `protocolVersion`, Cuando se
/// llama, Entonces responde `2024-11-05` sin error — omitir no es lo mismo que pedir algo imposible.
#[test]
fn initialize_sin_version_sigue_funcionando() {
    let dir = workspace_min();
    let resp = roundtrip(
        dir.path(),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{}}"#],
        1,
    );
    assert_eq!(
        resp[0]["result"]["protocolVersion"], "2024-11-05",
        "sin protocolVersion, el servidor debe responder su versión por defecto sin error: {resp:?}"
    );
    assert!(
        resp[0]["error"].is_null(),
        "sin protocolVersion no debe haber error: {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// E30-H03 — seguimiento 10: `protocolVersion` presente pero de tipo NO string.
//
// `requirements/epica-30-higiene-escoba.md` E30-H03 punto 10 (`decisiones §23`, seguimiento sin
// numerar de los jueces ciegos de E28/E29). `E29-H09` fijó el rechazo de una `protocolVersion`
// STRING pero no soportada (arriba, `protocol_version_no_soportada_se_rechaza`); este seguimiento
// cubre el hueco distinto: `protocolVersion` presente con un valor que NO es string en absoluto
// (número, `null` explícito, objeto). Causa raíz: `main.rs` L249,
// `params.get("protocolVersion").and_then(Value::as_str)` — `.and_then` devuelve `None` tanto si
// la clave está ausente como si está presente con un tipo no-string, y el código no distingue los
// dos casos: cae al brazo de "ausente" y responde éxito con la versión por defecto. Debe
// distinguir "ausente" (éxito, ver `initialize_sin_version_sigue_funcionando`, control anti-vacuo
// que este bloque NO duplica) de "presente con tipo incorrecto" (rechazo `-32602`, mismo código
// que la versión no soportada).
// ---------------------------------------------------------------------------

/// E30-H03 (seguimiento 10) · Criterio `protocol_version_no_string_es_rechazado`:
/// Dado un `initialize` con `protocolVersion: 12345` (número), Cuando se procesa, Entonces la
/// respuesta es un error JSON-RPC `-32602` que nombra que `protocolVersion` debe ser una cadena
/// (no un éxito silencioso con la versión por defecto).
#[test]
fn protocol_version_no_string_es_rechazado() {
    let dir = workspace_min();
    let resp = roundtrip(
        dir.path(),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":12345}}"#],
        1,
    );
    assert_eq!(
        resp[0]["error"]["code"], -32602,
        "protocolVersion numérica (12345) debe rechazarse con -32602, no colar como ausente: {resp:?}"
    );
    let msg = resp[0]["error"]["message"]
        .as_str()
        .expect("el error de protocolVersion no-string lleva mensaje")
        .to_lowercase();
    assert!(
        // Paréntesis EXPLÍCITOS: en Rust `&&` liga más que `||`, así que sin ellos la expresión
        // era `(protocolversion && cadena) || string` — un mensaje que dijera «string» sin nombrar
        // el parámetro habría pasado. La intención es la conjunción: nombrar el PARÁMETRO **y**
        // decir que debe ser una cadena/string.
        msg.contains("protocolversion") && (msg.contains("cadena") || msg.contains("string")),
        "el mensaje debe nombrar que protocolVersion debe ser una cadena/string: {msg}"
    );
    assert!(
        resp[0]["result"].is_null(),
        "un initialize con protocolVersion de tipo incorrecto no debe producir result: {resp:?}"
    );
}

/// E30-H03 (seguimiento 10) · Variante `protocol_version_null_explicito_es_rechazado`: un
/// `protocolVersion: null` **explícito** (la clave está presente, con valor JSON `null`) no es lo
/// mismo que omitir la clave — sigue siendo "presente con tipo incorrecto", no "ausente".
#[test]
fn protocol_version_null_explicito_es_rechazado() {
    let dir = workspace_min();
    let resp = roundtrip(
        dir.path(),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":null}}"#],
        1,
    );
    assert_eq!(
        resp[0]["error"]["code"], -32602,
        "protocolVersion: null EXPLÍCITO debe rechazarse con -32602, distinto de omitir la clave: {resp:?}"
    );
    assert!(
        resp[0]["result"].is_null(),
        "un initialize con protocolVersion: null explícito no debe producir result: {resp:?}"
    );
}

/// E30-H03 (seguimiento 10) · Variante `protocol_version_objeto_es_rechazado`: un `protocolVersion`
/// que es un objeto JSON (tipo claramente incorrecto) también se rechaza, no solo los escalares.
#[test]
fn protocol_version_objeto_es_rechazado() {
    let dir = workspace_min();
    let resp = roundtrip(
        dir.path(),
        &[r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":{"x":1}}}"#],
        1,
    );
    assert_eq!(
        resp[0]["error"]["code"], -32602,
        "protocolVersion como objeto debe rechazarse con -32602: {resp:?}"
    );
}

// ---------------------------------------------------------------------------
// E29-H08 — El wire RECHAZA los parámetros que no declara.
// `requirements/epica-29-honestidad-superficie.md §E29-H08` · `decisiones §15` (decidido:
// **(a) ejecutar** lo que el schema declara) · `decisiones §16(e)` (el criterio gemelo para el
// disco, que E29-H01 ya aplicó: «el repo no se queda con dos criterios opuestos según si lo
// desconocido llega por el wire o por disco»).
//
// EL DEFECTO: los 10 `inputSchema` declaran `additionalProperties: false` (`tools.rs`, en `list()`
// y en cada objeto anidado `ref`/`to`/`proposedOperation`) y el servidor NO lo ejecuta: `tools::call`
// lee campo a campo con `params.get("…")` y nunca mira las claves sobrantes. Medido en la revisión de
// la v0.3.0 (sonda 4): **15 casos aceptados en silencio**, entre ellos el `sort` que E23-H11 retiró,
// un `offset` inexistente y typos como `wheres`/`filters`. Un agente que se equivoca de nombre de
// parámetro recibe la respuesta POR DEFECTO, indistinguible de una legítima.
//
// LOS DOS NIVELES DE RECHAZO, CON CRITERIO DISTINTO (alcance de la historia, L841-853):
//   1. **Nivel tool** (`tools/call` → `arguments`): partición LIMPIA. Una clave que el `inputSchema`
//      de esa tool no declara → `INVALID_SCHEMA` nombrándola. Lo mismo dentro de los objetos
//      ANIDADOS que declaran `additionalProperties: false` (`ref`, `to`, `proposedOperation`).
//   2. **Nivel operación** (`operations[]` de `change_plan` y el `operation` de la selección
//      masiva): validación por **UNIÓN**, no por partición. Se rechaza lo que no esté en la unión de
//      los 17 campos legales; NO se rechaza un campo legal para OTRA op. Es decir: un `body` en un
//      `patch_frontmatter` **se sigue ignorando**, y un `bodyy` se rechaza. Razón (`§15`): `path`/
//      `ref` son intercambiables salvo en `create` y `body` pertenece a DOS ops, así que una
//      partición estricta rechazaría lotes válidos —un agente que reutiliza la misma plantilla de
//      objeto para varias operaciones de un lote— y el `oneOf` por operación sigue sin existir.
//      Cerrar la partición por op es **decisión posterior**, declarada, no un olvido de esta fase.
//
// ROJO ESPERADO HOY: por ASERCIÓN, contra el binario real. Sondado antes de escribir estos tests —
// `knowledge_search{sort}`, `knowledge_search{wheres}`, `knowledge_get{ref:{depth}}`,
// `workspace_status{foo}` y `change_plan` con `bodyy` responden HOY los cinco con éxito y el
// parámetro descartado. No hace falta ningún stub de producción: el rechazo es comportamiento nuevo
// sobre símbolos que ya existen.
//
// REPARTO DE FICHEROS (campo Pruebas de la historia): aquí van los casos por el WIRE (uno por
// llamada aislada, que es como se observó el hallazgo); el barrido data-driven sobre las 10 tools
// (`el_schema_declarado_coincide_con_lo_aceptado`) vive en `tests/descubribilidad.rs`, que es donde
// está la guarda de la política y el arnés que lee `tools/list`; la tabla de campos legales por
// operación (`los_campos_legales_de_cada_operacion_se_aceptan`, la CONDICIÓN DE ENTRADA) vive en
// `crates/lodestar-app/tests/plan.rs`, contra `App::change_plan` directamente.
// ---------------------------------------------------------------------------

/// Construye la línea JSON-RPC de un `tools/call` arbitrario (id 1).
fn tool_call_line(nombre: &str, arguments: serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "tools/call",
        "params": { "name": nombre, "arguments": arguments }
    })
    .to_string()
}

/// El mensaje de error de EJECUCIÓN de una tool (`result.content[0].text`), o `None` si la llamada
/// no falló. `isError` distingue el fallo de tool del error de protocolo (que iría en `error`).
fn error_de_tool(resp: &serde_json::Value) -> Option<String> {
    if resp["result"]["isError"].as_bool() != Some(true) {
        return None;
    }
    Some(
        resp["result"]["content"][0]["text"]
            .as_str()
            .expect("un error de tool viaja como texto en content[0].text")
            .to_string(),
    )
}

/// Asevera que la respuesta es un rechazo `INVALID_SCHEMA` cuyo mensaje **nombra** el parámetro
/// desconocido. Nombrarlo no es cosmética: es la diferencia entre «corrige `wheres`» y «algo de tu
/// llamada está mal», que es justo el silencio que la historia cierra.
fn asevera_rechazo_nombrando(resp: &serde_json::Value, desconocido: &str, contexto: &str) {
    let msg = error_de_tool(resp).unwrap_or_else(|| {
        panic!(
            "{contexto}: el parámetro no declarado «{desconocido}» debe RECHAZARSE, no descartarse \
             en silencio (hoy la llamada responde con éxito y el parámetro ignorado): {resp:?}"
        )
    });
    assert!(
        msg.starts_with("INVALID_SCHEMA"),
        "{contexto}: el rechazo debe abrir con el código del catálogo `INVALID_SCHEMA`; fue: {msg}"
    );
    assert!(
        msg.contains(desconocido),
        "{contexto}: el mensaje debe NOMBRAR el parámetro desconocido «{desconocido}» para que el \
         agente sepa cuál corregir; fue: {msg}"
    );
}

/// Workspace mínimo con un documento de frontmatter real bajo `notas/`, sobre el que se pueden
/// ejercer tanto las tools de lectura como una op de cambio.
fn workspace_una_nota() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "notas/alfa.md",
        "---\nstatus: accepted\n---\n\n# Alfa\n\nCuerpo con la palabra lodestar.\n",
    );
    dir
}

/// E29-H08 · Criterio `parametro_retirado_se_rechaza_nombrandolo`:
/// **Dado** un `knowledge_search` con `sort: "title"` (parámetro RETIRADO en E23-H11), **Cuando** se
/// llama, **Entonces** la respuesta es `INVALID_SCHEMA` nombrando `sort`, no la lista por defecto.
///
/// Es el caso emblemático de `decisiones §15`: `sort` existió, un cliente de v0.2 lo manda de buena
/// fe, y hoy recibe resultados en un orden que NO es el que pidió, sin la menor señal. La retirada
/// de E23-H11 fue solo declarativa (fuera del `inputSchema`); esta historia la hace ejecutable.
#[test]
fn parametro_retirado_se_rechaza_nombrandolo() {
    let dir = workspace_una_nota();
    let line = tool_call_line(
        "knowledge_search",
        serde_json::json!({ "text": "", "sort": "title" }),
    );
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);
    asevera_rechazo_nombrando(&resp[0], "sort", "knowledge_search con el `sort` retirado");
}

/// E29-H08 · Criterio `typo_de_parametro_se_rechaza`:
/// **Dado** un `knowledge_search` con `wheres` (typo de `where`), **Cuando** se llama, **Entonces**
/// `INVALID_SCHEMA` nombrando `wheres`.
///
/// El typo es peor que el parámetro retirado: hoy `wheres` se descarta y la búsqueda devuelve TODOS
/// los documentos, que es una respuesta plausible —el agente cree que su consulta no filtró nada— en
/// vez de una vacía que le habría hecho sospechar.
#[test]
fn typo_de_parametro_se_rechaza() {
    let dir = workspace_una_nota();
    let line = tool_call_line(
        "knowledge_search",
        serde_json::json!({ "wheres": "status = \"accepted\"" }),
    );
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);
    asevera_rechazo_nombrando(&resp[0], "wheres", "knowledge_search con el typo `wheres`");
}

/// E29-H08 · Criterio `typo_de_parametro_se_rechaza`, segunda mitad — el rechazo alcanza a las tools
/// de CAMBIO, no solo a las de lectura.
///
/// La historia pide typos «en varias tools representativas (lectura y cambio)». `change_apply` es la
/// más peligrosa de las tres: un `changeSetID` (con la `D` mayúscula, un typo verosímil de
/// `changeSetId`) hoy hace que el parámetro obligatorio parezca ausente, así que el agente recibe
/// «falta el parámetro obligatorio «changeSetId»» sin la menor pista de que sí lo mandó, escrito de
/// otra forma. Tras la historia debe decírsele que `changeSetID` no existe.
#[test]
fn typo_de_parametro_en_tool_de_cambio_se_rechaza() {
    let dir = workspace_una_nota();
    let line = tool_call_line(
        "change_apply",
        serde_json::json!({ "changeSetID": "changeset:0000", "planId": "x" }),
    );
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);
    asevera_rechazo_nombrando(
        &resp[0],
        "changeSetID",
        "change_apply con el typo `changeSetID`",
    );
}

/// E29-H08 · Criterio `clave_desconocida_en_objeto_anidado_se_rechaza`:
/// **Dado** un `knowledge_get` con `ref: {path: "…", depth: 2}` (clave desconocida en el objeto
/// **anidado**), **Cuando** se llama, **Entonces** también se rechaza.
///
/// El objeto `ref` declara su propio `additionalProperties: false` en el schema (`tools.rs` L117),
/// así que la promesa incumplida es la misma un nivel más abajo. Hoy `serde_json::from_value` sobre
/// `DocumentRef` ignora los campos sobrantes y el `depth` desaparece —lo que importa porque `depth`
/// SÍ existe en otras tools (`graph_query`, `impact_analyze`): un agente puede creer legítimamente
/// que aquí también hace algo.
#[test]
fn clave_desconocida_en_objeto_anidado_se_rechaza() {
    let dir = workspace_una_nota();
    let line = tool_call_line(
        "knowledge_get",
        serde_json::json!({ "ref": { "path": "notas/alfa.md", "depth": 2 } }),
    );
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);
    asevera_rechazo_nombrando(
        &resp[0],
        "depth",
        "knowledge_get con `depth` dentro del objeto anidado `ref`",
    );
}

/// E29-H08 · Criterio `tool_sin_parametros_rechaza_cualquier_argumento`:
/// **Dado** un `workspace_status` (cuyo schema es el objeto VACÍO con `additionalProperties: false`),
/// **Cuando** se llama con `{"foo": 1}`, **Entonces** se rechaza.
///
/// Es el borde del criterio: una tool sin parámetros declara la lista vacía, y por unión eso
/// significa que CUALQUIER clave sobra. Sin este caso, una implementación que solo mire tools con
/// `properties` no vacío pasaría los demás tests.
#[test]
fn tool_sin_parametros_rechaza_cualquier_argumento() {
    let dir = workspace_una_nota();
    let line = tool_call_line("workspace_status", serde_json::json!({ "foo": 1 }));
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);
    asevera_rechazo_nombrando(
        &resp[0],
        "foo",
        "workspace_status (schema de objeto vacío) con un argumento inventado",
    );
}

/// E29-H08 · **Remate del juez** — el rechazo de nivel operación DESCIENDE a los sub-objetos:
/// **Dado** un `change_plan` con una operación cuyo `ref` lleva una clave que el objeto `ref` no
/// declara, **Cuando** se planifica, **Entonces** `INVALID_SCHEMA` nombrándola con su contexto.
///
/// El hueco que cierra: el nivel operación validaba las claves de la op **en su nivel raíz** y no
/// bajaba a los objetos anidados, así que
/// `{"op":"patch_frontmatter","ref":{"path":"…","parametroQueNoExiste":1}}` pasaba en silencio. Eso
/// dejaba al servidor con **dos criterios opuestos para el mismo objeto `ref`** según por dónde
/// entrara: por `knowledge_get` se rechazaba (`clave_desconocida_en_objeto_anidado_se_rechaza`) y
/// por una operación de `change_plan` se tragaba. La asimetría es justo la forma que `decisiones
/// §15` prohíbe —«el repo no se queda con dos criterios opuestos según por dónde llegue lo
/// desconocido»— y es peor aquí que en la lectura, porque el `ref` de una operación identifica el
/// documento que se va a **escribir**.
///
/// El mensaje debe llevar **contexto**, no solo el nombre: `ref` está anidado, así que decir
/// «`parametroQueNoExiste` no es un parámetro declarado» a secas obliga al agente a adivinar en
/// cuál de los objetos de su lote está el typo. Se exige que el error cite también `ref`.
#[test]
fn clave_desconocida_en_ref_de_una_operacion_se_rechaza() {
    let dir = workspace_una_nota();
    let line = tool_call_line(
        "change_plan",
        serde_json::json!({
            "operations": [
                { "op": "patch_frontmatter",
                  "ref": { "path": "notas/alfa.md", "parametroQueNoExiste": 1 },
                  "patch": { "status": "review" } }
            ]
        }),
    );
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);
    asevera_rechazo_nombrando(
        &resp[0],
        "parametroQueNoExiste",
        "change_plan con una clave desconocida DENTRO del `ref` de una operación",
    );
    let msg = error_de_tool(&resp[0]).expect("ya aseverado como rechazo justo arriba");
    assert!(
        msg.contains("ref"),
        "el mensaje debe situar la clave desconocida en el objeto `ref` de la operación: sin el \
         contexto, un agente con un lote de 20 ops no sabe cuál corregir; fue: {msg}"
    );
}

/// E29-H08 · **Remate del juez, control anti-vacuo**: el descenso a los sub-objetos de una
/// operación no puede cerrarse de más. Dos propiedades en el mismo test, porque las dos son la
/// misma pregunta —«¿qué sub-objetos son cerrados y cuáles abiertos?»— y separarlas invitaría a
/// arreglar una y romper la otra:
///
/// 1. **`ref` legal sigue funcionando**: `{"ref": {"path": "…"}}` dentro de una operación se acepta,
///    igual que antes del remate. Es la mitad que un descenso demasiado celoso rompería.
/// 2. **El merge-patch de `patch_frontmatter` sigue EXENTO**: las claves de `patch` son el
///    frontmatter **del usuario** (`§20.2` invariante 3: YAML arbitrario, ninguna clave tiene
///    semántica impuesta), así que son abiertas POR DEFINICIÓN y no se pueden validar contra
///    ninguna lista. Un descenso que tratara `patch` como un objeto cerrado haría imposible
///    escribir cualquier campo de frontmatter nuevo — sería el peor daño colateral posible de esta
///    historia, y por eso se fija con un patch de claves deliberadamente inventadas.
///
/// Lo mismo aplica a `frontmatter` en `create`, que se comprueba en el mismo barrido por ser el
/// otro sub-objeto de contenido arbitrario de la tabla de campos legales.
#[test]
fn los_subobjetos_abiertos_de_una_operacion_siguen_aceptando_claves_arbitrarias() {
    let dir = workspace_una_nota();

    // (1) `ref` legal dentro de una operación: sigue planificando.
    let legal = tool_call_line(
        "change_plan",
        serde_json::json!({
            "operations": [
                { "op": "patch_frontmatter", "ref": { "path": "notas/alfa.md" },
                  "patch": { "status": "review" } }
            ]
        }),
    );
    let resp = roundtrip(dir.path(), &[legal.as_str()], 1);
    assert!(
        error_de_tool(&resp[0]).is_none(),
        "un `ref` con solo `path` (su única clave declarada) debe seguir aceptándose dentro de una \
         operación: el descenso a los sub-objetos no puede tocar lo declarado: {:?}",
        resp[0]
    );

    // (2) El merge-patch y el `frontmatter` de `create`: claves ARBITRARIAS del usuario.
    let arbitrarias = tool_call_line(
        "change_plan",
        serde_json::json!({
            "operations": [
                { "op": "patch_frontmatter", "path": "notas/alfa.md",
                  "patch": { "claveQueSoloExisteEnEsteWorkspace": "x",
                             "otraInventada": 1, "anidada": { "profunda": true } } },
                { "op": "create", "path": "notas/nuevo.md",
                  "frontmatter": { "campoDelUsuario": "y", "sonar.projectKey": "z" },
                  "body": "# Nuevo\n" }
            ]
        }),
    );
    let resp = roundtrip(dir.path(), &[arbitrarias.as_str()], 1);
    assert!(
        error_de_tool(&resp[0]).is_none(),
        "las claves de `patch` y de `frontmatter` son el YAML ARBITRARIO del usuario (`§20.2` \
         invariante 3): son abiertas por definición y NO se validan contra ninguna lista. \
         Cerrarlas haría imposible escribir un campo de frontmatter nuevo, que es el peor daño \
         colateral posible de esta historia: {:?}",
        resp[0]
    );
    let sc = plan_sc(&resp[0]);
    assert_eq!(
        sc["normalizedOperations"]
            .as_array()
            .expect("el plan lleva `normalizedOperations`")
            .len(),
        2,
        "y las dos operaciones deben normalizarse: {sc}"
    );
}

/// E29-H08 · Criterio `campo_inexistente_en_una_operacion_se_rechaza`:
/// **Dado** un `change_plan` con una operación que lleva `bodyy` (typo, NO está en la unión de los
/// 17 campos legales), **Cuando** se planifica, **Entonces** `INVALID_SCHEMA` nombrando `bodyy`.
///
/// Este es el nivel 2 (operación) en su mitad de RECHAZO. La op elegida es `replace_body`, donde
/// `body` sí es legal: así el typo es de verdad un typo —el agente quería `body`— y el rechazo no se
/// puede confundir con «campo de otra op». Hoy el plan se computa con el cuerpo SIN sustituir y
/// `canApply: true`: el agente cree haber reemplazado el cuerpo y no reemplazó nada.
#[test]
fn campo_inexistente_en_una_operacion_se_rechaza() {
    let dir = workspace_una_nota();
    let line = tool_call_line(
        "change_plan",
        serde_json::json!({
            "operations": [
                { "op": "replace_body", "path": "notas/alfa.md",
                  "bodyy": "# Alfa\n\nCuerpo nuevo.\n" }
            ]
        }),
    );
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);
    asevera_rechazo_nombrando(
        &resp[0],
        "bodyy",
        "change_plan con el typo `bodyy` en una operación",
    );
}

/// E29-H08 · Criterio `campo_inexistente_en_una_operacion_se_rechaza`, mitad de la SELECCIÓN MASIVA:
/// el mismo criterio de unión rige dentro del objeto `operation` de la selección masiva, que es la
/// otra forma de wire por la que llegan parámetros de operación (`§20.11`).
///
/// La historia declara los dos vehículos en el mismo criterio («`operations[]` de `change_plan`, y
/// el objeto `operation` de la selección masiva»). Se separa en un test propio porque el camino de
/// código es OTRO —`expand_selection`/`single_operation`, no `normalize_raw_op` sobre un array—, y
/// una implementación que solo mire `operations[]` dejaría esta puerta abierta.
#[test]
fn campo_inexistente_en_la_seleccion_masiva_se_rechaza() {
    let dir = workspace_una_nota();
    let line = tool_call_line(
        "change_plan",
        serde_json::json!({
            "selection": { "where": "status = \"accepted\"" },
            "operation": { "replace_text": { "find": "lodestar", "replace": "Lodestar",
                                             "expectedOcurrences": 1 } }
        }),
    );
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);
    asevera_rechazo_nombrando(
        &resp[0],
        "expectedOcurrences",
        "change_plan (selección masiva) con el typo `expectedOcurrences`",
    );
}

/// E29-H08 · Criterio `campo_legal_de_otra_operacion_se_sigue_ignorando` (**la excepción
/// declarada**, y el control anti-vacuo más importante de la historia):
/// **Dado** un `change_plan` con una operación `patch_frontmatter` que además lleva `body` (campo
/// legal de OTRA op), **Cuando** se planifica, **Entonces** se acepta y `body` se ignora, como hoy.
///
/// La decisión de diseño ratificada es **validar por UNIÓN, no por partición**: `path`/`ref` son
/// intercambiables salvo en `create` y `body` pertenece a DOS ops, así que una partición estricta
/// rompería un lote perfectamente válido en el que un agente reutiliza la misma plantilla de objeto
/// para varias operaciones. Cerrar la partición por op es decisión POSTERIOR.
///
/// Este test nace **VERDE** (fija el comportamiento actual para que la historia no lo cambie por
/// accidente) y debe seguir verde después: es la mitad del criterio que impide que el rechazo se
/// cierre de más. Se asevera además que el `body` de verdad NO se aplicó —el diff no toca el
/// cuerpo—, para que «se ignora» sea una afirmación verificada y no una tautología del `isError`.
#[test]
fn campo_legal_de_otra_operacion_se_sigue_ignorando() {
    let dir = workspace_una_nota();
    let line = tool_call_line(
        "change_plan",
        serde_json::json!({
            "operations": [
                { "op": "patch_frontmatter", "path": "notas/alfa.md",
                  "patch": { "status": "review" },
                  "body": "# CUERPO QUE NO DEBE APLICARSE\n" }
            ]
        }),
    );
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);

    assert!(
        error_de_tool(&resp[0]).is_none(),
        "un campo legal de OTRA op (`body` en un `patch_frontmatter`) debe SEGUIR ignorándose: la \
         validación es por UNIÓN de los 17 campos legales, no por partición por op — rechazarlo \
         rompería los lotes que reutilizan una plantilla de objeto (`decisiones §15`): {:?}",
        resp[0]
    );
    let sc = plan_sc(&resp[0]);
    assert!(
        sc["changeSetId"].as_str().is_some_and(|s| !s.is_empty()),
        "y el plan debe producirse igual que hoy: {:?}",
        resp[0]
    );
    let ops = sc["normalizedOperations"]
        .as_array()
        .expect("el plan lleva `normalizedOperations`");
    assert_eq!(
        ops.len(),
        1,
        "el `body` ignorado no puede generar una operación extra: {ops:?}"
    );
    assert!(
        !serde_json::to_string(sc)
            .expect("el plan serializa")
            .contains("CUERPO QUE NO DEBE APLICARSE"),
        "«ignorado» significa que el `body` no llega al resultado: si apareciera en el plan, no se \
         estaría ignorando sino aplicando. Plan: {sc}"
    );
}

/// E29-H08 · Criterio `la_seleccion_masiva_sigue_funcionando` (control anti-vacuo del caso donde
/// `params` viaja ENTERO):
/// **Dado** un `change_plan` en forma de selección masiva, **Cuando** lleva `selection` + `operation`
/// + `policy` (todo legal), **Entonces** se planifica como hoy.
///
/// Es el control que protege la trampa señalada por la propia historia (L854-859): con `selection`,
/// `tools.rs:468-472` pasa `params.clone()` **entero** a `App::change_plan`, así que el objeto de
/// argumentos de la tool y el de la selección masiva son el MISMO. Una implementación que valide el
/// nivel operación sobre ese objeto entero vería `selection`/`operation`/`policy` como «campos de
/// operación desconocidos» y rompería la selección masiva completa. Nace VERDE y debe seguir verde.
#[test]
fn la_seleccion_masiva_sigue_funcionando() {
    let dir = workspace_una_nota();
    let line = tool_call_line(
        "change_plan",
        serde_json::json!({
            "selection": { "where": "status = \"accepted\"" },
            "operation": { "patch_frontmatter": { "status": "review" } },
            "policy": { "requireValidResult": false, "allowWarnings": true }
        }),
    );
    let resp = roundtrip(dir.path(), &[line.as_str()], 1);

    assert!(
        error_de_tool(&resp[0]).is_none(),
        "la selección masiva con `selection`+`operation`+`policy` (todo LEGAL) debe seguir \
         planificándose: con `selection`, el objeto de argumentos de la tool viaja entero como \
         `raw_ops`, y confundir sus claves con campos de operación rompería la forma masiva \
         entera: {:?}",
        resp[0]
    );
    let sc = plan_sc(&resp[0]);
    let ops = sc["normalizedOperations"]
        .as_array()
        .expect("el plan lleva `normalizedOperations`");
    assert_eq!(
        ops.len(),
        1,
        "la selección debe expandirse sobre el único documento que casa `status = accepted`: {sc}"
    );
}

/// E29-H08 · Control anti-vacuo de los parámetros OPCIONALES legítimos (`cursor`/`limit` y compañía):
/// **Dado** llamadas que usan TODOS los parámetros opcionales que su `inputSchema` declara,
/// **Cuando** se llaman, **Entonces** siguen funcionando exactamente como hoy.
///
/// La historia lo pide explícitamente («cursor/limit y campos opcionales legítimos intactos»). Sin
/// él, una implementación que rechazara todo lo que no sea obligatorio pasaría los seis tests de
/// rechazo de arriba y rompería la paginación de la superficie entera. Se ejercitan las tres tools
/// paginadas + una llamada con `cursor` REAL obtenido de una respuesta previa, que es el uso que un
/// agente hace de verdad.
#[test]
fn parametros_opcionales_legitimos_siguen_funcionando() {
    let dir = workspace_una_nota();
    write(
        dir.path(),
        "notas/beta.md",
        "---\nstatus: draft\n---\n\n# Beta\n\nCuerpo con la palabra lodestar.\n",
    );

    let llamadas = [
        (
            "knowledge_search",
            serde_json::json!({ "text": "lodestar", "where": "status = \"accepted\"",
                                "filter": { "field": "frontmatter.status", "operator": "equals",
                                            "value": "accepted" },
                                "include": ["frontmatter.status"], "limit": 1 }),
        ),
        (
            "knowledge_check",
            serde_json::json!({ "scope": { "kind": "workspace" }, "minimumSeverity": "info",
                                "includeSuggestedFixes": false, "limit": 5 }),
        ),
        (
            "metadata_inspect",
            serde_json::json!({ "mode": "catalog", "limit": 5 }),
        ),
        (
            "graph_query",
            serde_json::json!({ "operation": "neighborhood", "ref": { "path": "notas/alfa.md" },
                                "depth": 1, "direction": "both", "limit": 5 }),
        ),
    ];
    for (tool, args) in llamadas {
        let line = tool_call_line(tool, args.clone());
        let resp = roundtrip(dir.path(), &[line.as_str()], 1);
        assert!(
            error_de_tool(&resp[0]).is_none(),
            "«{tool}» con SOLO parámetros declarados ({args}) debe seguir funcionando: el rechazo \
             de lo no declarado no puede tocar lo declarado-y-opcional: {:?}",
            resp[0]
        );
    }

    // Y el `cursor` de verdad: se pagina de una respuesta a la siguiente, que es donde un rechazo
    // demasiado celoso rompería la paginación sin que ningún caso sintético lo notara.
    let primera = roundtrip(
        dir.path(),
        &[tool_call_line(
            "knowledge_search",
            serde_json::json!({ "text": "", "limit": 1 }),
        )
        .as_str()],
        1,
    );
    let cursor = primera[0]["result"]["structuredContent"]["nextCursor"]
        .as_str()
        .expect("con 2 documentos y limit 1 debe haber `nextCursor`")
        .to_string();
    let segunda = roundtrip(
        dir.path(),
        &[tool_call_line(
            "knowledge_search",
            serde_json::json!({ "text": "", "limit": 1, "cursor": cursor }),
        )
        .as_str()],
        1,
    );
    assert!(
        error_de_tool(&segunda[0]).is_none(),
        "la segunda página con el `cursor` devuelto por la primera debe servirse igual que hoy: {:?}",
        segunda[0]
    );
}

// ---------------------------------------------------------------------------
// E30-H01 — Cursores estrictos: malformado o ajeno a la tool es `INVALID_SCHEMA`
//
// Defecto (verificado por el wire antes de escribir estos tests, no supuesto):
//   · A-02 (ROB-05): `decode_cursor` (`lodestar-app/src/lib.rs:3987`) hace
//     `usize::from_str_radix(cursor, 16).unwrap_or(0)`. Un `cursor: "zzz-no-hex"` cae a offset 0 y
//     la tool devuelve la PRIMERA página con `isError` ausente: indistinguible de un cliente que
//     omitió el parámetro a propósito.
//   · A-03 (ROB-06): las cuatro tools paginadas comparten `pagina()`/`encode_cursor()`/
//     `decode_cursor()` y el cursor es un offset hex DESNUDO, sin marca de origen. Comprobado por el
//     wire: el `nextCursor: "2"` que emite `knowledge_check` lo acepta `knowledge_search` y sirve
//     una página «válida» en forma y ajena en significado.
//
// COMPORTAMIENTO que fijan estos tests (lo observable; el encoding interno lo elige la fase verde):
//   1. Un cursor que no decodifica → `INVALID_SCHEMA` nombrando `cursor` y el valor recibido.
//   2. Un cursor emitido por OTRA tool → `INVALID_SCHEMA` nombrando que no pertenece a esta tool.
//   3. Un cursor emitido por otro MODO de `metadata_inspect` → también rechazado (ver la decisión
//      de abajo).
//   4. El camino feliz no se toca: un cursor obtenido de una respuesta REAL de la misma tool con
//      los MISMOS parámetros sigue paginando, y el recorrido completo hasta `nextCursor: null`
//      reconstruye exactamente el resultado sin paginar.
//
// DECISIÓN DE LA FASE ROJA sobre la identidad de consulta (el alcance pide decidirlo y dejarlo
// escrito, no dejarlo a interpretación):
//   · **La identidad se ata a (tool, contexto de listado)**, entendiendo por contexto el que
//     determina QUÉ lista se pagina y en qué orden total cuando la propia tool tiene más de una:
//     el `mode` (y el `field` en mode «field») de `metadata_inspect`. Ese caso es OBLIGATORIO
//     porque el criterio 3 de la historia lo exige con test, y porque las dos listas de
//     `metadata_inspect` tienen órdenes totales distintos: un offset del catálogo sobre el
//     vocabulario de un campo es exactamente el mismo defecto que A-03 dentro de una sola tool.
//   · **NO se ata al criterio de selección** (`text`/`where`/`filter` de `knowledge_search`,
//     `scope`/`minimumSeverity` de `knowledge_check`, `ref`/`depth`/`direction` de `graph_query`).
//     Razones: (a) el criterio de la historia solo lo exige para el par catalog/field; (b) atarlo a
//     los parámetros de selección obliga a hashear una entrada de forma libre —y el `filter`/`where`
//     admiten formas equivalentes que hashearían distinto, convirtiendo en `INVALID_SCHEMA`
//     paginaciones legítimas—; (c) ese endurecimiento cambia lo que hoy es una respuesta
//     desalineada, no silenciosamente errónea en la misma medida (el orden total sí depende de la
//     consulta, pero el cliente que cambia la consulta a mitad de recorrido lo hace a sabiendas).
//     Queda como **hallazgo de seguimiento declarado**, no como hueco descubierto por accidente:
//     `cursor_de_otra_consulta_de_la_misma_tool_es_hallazgo_de_seguimiento` lo deja escrito
//     ejerciendo el comportamiento que esta historia SÍ garantiza (sigue paginando, sin romper).
//   · **`cursor: ""` (cadena vacía) cuenta como AUSENTE, no como malformado.** Es lo que hace hoy y
//     lo que `descubribilidad.rs::el_schema_declarado_coincide_con_lo_aceptado` da por bueno al
//     mandar `json!("")` como valor de ejemplo del parámetro declarado; convertirlo en error
//     rompería esa guarda por una razón ajena a este defecto. Lo fija
//     `cursor_vacio_cuenta_como_ausente`.
//
// TESTS PREEXISTENTES QUE FIJABAN LA TOLERANCIA: **ninguno**. Se revisó toda la suite
// (`mcp.rs`, `escala_wire.rs`, `descubribilidad.rs`, `lodestar-app/tests/`): todos los tests de
// paginación —`search_paginacion`, `el_cursor_es_autosuficiente`, `paginar_no_pierde_ni_duplica`,
// `graph_query_tiene_default`, `recorre_paginas`— usan cursores OBTENIDOS de respuestas reales,
// nunca fabricados a mano, así que ninguno depende de que un cursor basura caiga a offset 0. El
// único cursor sintético de la suite es el `json!("")` de `descubribilidad.rs`, cubierto por la
// decisión de arriba. No se reescribe ni se toca ningún test existente.
// ---------------------------------------------------------------------------

/// Un cursor sintáctico**mente** basura: ni hex, ni nada que ninguna codificación razonable emita.
/// Es el literal que `decisiones §23/A-02` (ROB-05) reporta.
const CURSOR_BASURA: &str = "zzz-no-hex";

/// Las cuatro tools paginadas con unos argumentos que hoy tienen éxito, más el nombre de la lista
/// que paginan. `metadata_inspect` aparece una vez por modo: son dos listas con órdenes totales
/// distintos, y el criterio 3 de la historia las trata como contextos separados.
fn casos_paginados() -> Vec<(&'static str, serde_json::Value, &'static str)> {
    vec![
        (
            "knowledge_search",
            serde_json::json!({"text": "nota", "limit": 20}),
            "results",
        ),
        (
            "knowledge_check",
            serde_json::json!({"scope": {"kind": "workspace"}, "limit": 20}),
            "diagnostics",
        ),
        (
            "metadata_inspect",
            serde_json::json!({"mode": "catalog", "limit": 20}),
            "fields",
        ),
        (
            "metadata_inspect",
            serde_json::json!({"mode": "field", "field": "uid", "limit": 20}),
            "values",
        ),
        (
            "graph_query",
            serde_json::json!({"operation": "components", "limit": 20}),
            "nodes",
        ),
    ]
}

/// **E30-H01** · Criterio `cursor_malformado_es_invalid_schema` (A-02 / ROB-05):
/// **Dado** una llamada con `cursor: "zzz-no-hex"`, **Cuando** se ejecuta, **Entonces** la respuesta
/// es `INVALID_SCHEMA` nombrando el cursor recibido como no decodificable — **no** una página desde
/// el offset 0.
///
/// Se ejerce en **las cuatro** tools paginadas (cinco llamadas, porque `metadata_inspect` tiene dos
/// modos): el defecto vive en `decode_cursor`, que las cinco comparten, así que un arreglo que solo
/// endureciera `knowledge_search` dejaría el mismo agujero en las otras.
///
/// El anti-vacuo está dentro del propio test: la MISMA llamada **sin** `cursor` tiene que seguir
/// devolviendo su página, o «rechazar el cursor basura» habría degenerado en «rechazar la tool».
#[test]
fn cursor_malformado_es_invalid_schema() {
    // `ws_cota_rota` y no `ws_cota`: este último es un ciclo perfecto sin enlaces colgantes, así que
    // `knowledge_check` sirve CERO diagnósticos sobre él y la precondición de abajo («sin `cursor`
    // esta llamada debe traer resultados») es insatisfacible para esa tool, con arreglo o sin él.
    let dir = ws_cota_rota();

    for (tool, args, clave) in casos_paginados() {
        // (1) Control: sin `cursor`, la llamada tiene éxito y trae su lista (precondición del caso).
        let ok = roundtrip(dir.path(), &[linea_call(1, tool, args.clone()).as_str()], 1);
        let sc_ok_ = sc_ok(&ok[0], tool);
        assert!(
            !lista(sc_ok_, clave).is_empty(),
            "precondición de «{tool}/{clave}»: sin `cursor` esta llamada debe traer resultados, o \
             el caso no discrimina nada: {}",
            ok[0]
        );

        // (2) El criterio: con el cursor basura, error.
        let mut args_basura = args.clone();
        args_basura["cursor"] = serde_json::json!(CURSOR_BASURA);
        let resp = roundtrip(dir.path(), &[linea_call(2, tool, args_basura).as_str()], 1);
        let err = error_de(&resp[0]).unwrap_or_else(|| {
            panic!(
                "«{tool}/{clave}» con `cursor: \"{CURSOR_BASURA}\"` debe RECHAZAR la llamada. Hasta \
                 v0.5.0 `decode_cursor` hacía `usize::from_str_radix(cursor, 16).unwrap_or(0)`, así \
                 que cualquier basura se reinterpretaba como «empieza desde el principio» y el \
                 agente recibía la primera página creyendo que había avanzado (ROB-05, \
                 `decisiones §23/A-02`).\nRespuesta recibida: {}",
                resp[0]
            )
        });
        assert_eq!(
            codigo_de(&err),
            "INVALID_SCHEMA",
            "un cursor que no decodifica es entrada inválida del agente, con el mismo código que \
             el resto de la validación de parámetros de «{tool}» (E24-H09/E26-H07): «{err}»"
        );
        let (_, mensaje) = codigo_y_mensaje(&err).unwrap_or_else(|| {
            panic!("«{tool}» debe emitir «CÓDIGO: mensaje» (E26-H07), no el código pelado: «{err}»")
        });
        assert!(
            menciona(mensaje, "cursor"),
            "…y el mensaje debe NOMBRAR el parámetro `cursor`: es lo que el agente necesita para \
             saber cuál de sus argumentos corregir (mismo criterio que el resto del catálogo de \
             type errors): «{err}»"
        );
        assert!(
            mensaje.contains(CURSOR_BASURA),
            "…y deletrear el valor recibido («{CURSOR_BASURA}»), o el agente no puede distinguir \
             «mandé un cursor que no vale» de «esta tool no acepta cursor»: «{err}»"
        );
    }
}

/// **E30-H01** · Regresión de robustez encontrada por un juez ciego EJECUTANDO (no leyendo) el
/// arreglo de este mismo criterio: un cursor con un carácter multibyte hacía `panic!` —
/// `byte index 2 is not a char boundary` — dentro de `decode_cursor_firmado`, y ese panic tumbaba el
/// proceso `lodestar-mcp` entero (`rc=101`), matando la sesión JSON-RPC completa en vez de servir un
/// error. Causa: el troceo hex de `cuerpo` indexaba por posición de **carácter** de un `&str` que
/// podía contener bytes UTF-8 de más de un byte, así que el corte `cuerpo[i*2..i*2+2]` caía a media
/// secuencia multibyte. El arreglo (ya en el árbol, sin commitear) antepone un guard
/// `is_ascii_hexdigit` sobre `cuerpo` y `firma` antes de trocear, y trocea por
/// `as_bytes().chunks_exact(2)` en vez de por índice de carácter.
///
/// **Dado** un cursor con caracteres no-ASCII de distinto ancho en UTF-8, **Cuando** se manda a
/// cualquiera de las cuatro tools paginadas, **Entonces** la respuesta es `INVALID_SCHEMA` (un error
/// SERVIDO, nunca un panic) — y, crítico, **la sesión JSON-RPC sobrevive**: una llamada posterior en
/// la MISMA tubería stdin/stdout recibe respuesta. Es lo único que distingue «error servido» de
/// «proceso muerto», y ningún test anterior de la sección lo comprobaba (todos abren un proceso,
/// mandan una sola llamada mala y lo cierran, así que un panic que mata el proceso justo después de
/// escribir el mensaje de error habría pasado desapercibido).
///
/// Tres cuerpos de cursor, elegidos por cómo desalinean (o no) el troceo de 2 bytes:
///   · `"🔥.807e307a"` — emoji, 4 bytes UTF-8: el caso que el juez reprodujo.
///   · `"中中.deadbeef"` — CJK, 3 bytes cada carácter (6 en total): mismo género de fallo con otro
///     ancho, para no fijar la regresión a un solo tamaño de carácter.
///   · `"ññ.deadbeef"` — contraste documentado a propósito: `ñ` ocupa 2 bytes en UTF-8, así que
///     `chunks_exact(2)` cae ALINEADO con la frontera de carácter y este caso en concreto no
///     reventaba ni antes del arreglo (el guard ASCII lo rechaza igual, pero por una vía distinta:
///     sin él, este cursor en concreto habría producido un byte inválido en vez de un panic de
///     frontera — dos síntomas del mismo defecto de fondo, «el cuerpo puede no ser ASCII»).
#[test]
fn cursor_no_ascii_no_tumba_el_servidor() {
    let dir = ws_cota_rota();

    const CURSORES_NO_ASCII: &[&str] = &["🔥.807e307a", "中中.deadbeef", "ññ.deadbeef"];

    for (tool, args, clave) in casos_paginados() {
        for cursor in CURSORES_NO_ASCII {
            let mut args_malos = args.clone();
            args_malos["cursor"] = serde_json::json!(cursor);

            // Dos líneas en la MISMA sesión: la llamada con el cursor no-ASCII y, a continuación,
            // una llamada inocua. Si el proceso hubiera hecho panic al procesar la primera, la
            // segunda respuesta no llegaría nunca y `roundtrip` se quedaría sin las 2 esperadas
            // (bloqueado hasta EOF de un `stdout` ya cerrado, o devolviendo menos de las pedidas).
            let resp = roundtrip(
                dir.path(),
                &[
                    linea_call(1, tool, args_malos).as_str(),
                    linea_call(2, "workspace_status", serde_json::json!({})).as_str(),
                ],
                2,
            );
            assert_eq!(
                resp.len(),
                2,
                "«{tool}/{clave}» con `cursor: \"{cursor}\"`: la sesión debe sobrevivir y responder \
                 a AMBAS llamadas. Si solo llega 1 respuesta (o 0), el proceso murió al procesar el \
                 cursor no-ASCII — exactamente el `panic!` (`byte index 2 is not a char boundary`, \
                 rc=101) que un juez ciego de robustez encontró EJECUTANDO este caso, no leyendo el \
                 código: {resp:?}"
            );

            let err = error_de(&resp[0]).unwrap_or_else(|| {
                panic!(
                    "«{tool}/{clave}» con `cursor: \"{cursor}\"` debe RECHAZAR la llamada con un \
                     error servido, no aceptarla ni (peor) hacer panic: {}",
                    resp[0]
                )
            });
            assert_eq!(
                codigo_de(&err),
                "INVALID_SCHEMA",
                "un cursor no-ASCII es tan malformado como el hex basura de \
                 `cursor_malformado_es_invalid_schema`: mismo código: «{err}»"
            );

            // La segunda respuesta (`workspace_status`) es la prueba crítica de la sesión viva: debe
            // tener éxito con normalidad, como si la llamada anterior nunca hubiera pasado de ser un
            // error de cliente.
            assert_eq!(
                error_de(&resp[1]),
                None,
                "la sesión debe seguir sirviendo tras el cursor no-ASCII: `workspace_status` \
                 inmediatamente después debe tener éxito, no arrastrar ningún estado roto: {}",
                resp[1]
            );
        }
    }
}

/// **E30-H01** · Criterio `cursor_de_otra_tool_es_invalid_schema` (A-03 / ROB-06):
/// **Dado** un `nextCursor` devuelto por una llamada **real** a `graph_query`, **Cuando** se pasa
/// ese mismo valor como `cursor` a `knowledge_search`, **Entonces** la respuesta es `INVALID_SCHEMA`
/// nombrando que el cursor no pertenece a esta tool — **no** una página «válida» de
/// `knowledge_search`.
///
/// El cursor se **obtiene de una respuesta real** (nunca se fabrica): es lo que hace que el caso sea
/// el defecto reportado y no una variante de «cursor malformado». Hoy `graph_query` emite `"64"`
/// (100 en hex) y `knowledge_search` lo acepta como offset propio.
///
/// Se cruzan las dos direcciones —`graph_query`→`knowledge_search` (el caso literal de la ficha) y
/// `knowledge_check`→`metadata_inspect`— para que el arreglo no pueda consistir en un caso especial
/// de un par concreto de tools.
#[test]
fn cursor_de_otra_tool_es_invalid_schema() {
    // `ws_cota_rota` y no `ws_cota`: `knowledge_check` es una de las tools EMISORAS de este cruce y
    // solo emite `nextCursor` si hay más diagnósticos que la página; sobre el ciclo perfecto de
    // `ws_cota` no hay ni uno, así que no habría cursor real que cruzar.
    let dir = ws_cota_rota();

    // Emisor y receptor de cada cruce: (tool emisora, args emisores, tool receptora, args
    // receptores, lista del receptor).
    let cruces: [(&str, serde_json::Value, &str, serde_json::Value, &str); 2] = [
        (
            "graph_query",
            serde_json::json!({"operation": "components"}),
            "knowledge_search",
            serde_json::json!({"text": "nota", "limit": 20}),
            "results",
        ),
        (
            "knowledge_check",
            serde_json::json!({"scope": {"kind": "workspace"}, "limit": 20}),
            "metadata_inspect",
            serde_json::json!({"mode": "catalog", "limit": 20}),
            "fields",
        ),
    ];

    for (emisora, args_emisora, receptora, args_receptora, clave) in cruces {
        // (1) El cursor AJENO, obtenido de una respuesta real de la tool emisora.
        let p1 = roundtrip(
            dir.path(),
            &[linea_call(1, emisora, args_emisora).as_str()],
            1,
        );
        let ajeno = cursor_de(sc_ok(&p1[0], emisora)).unwrap_or_else(|| {
            panic!(
                "precondición: «{emisora}» debe emitir un `nextCursor` real que cruzar a \
                 «{receptora}» (el caso exige un cursor OBTENIDO, no fabricado): {}",
                p1[0]
            )
        });

        // (2) Control anti-vacuo: en su propia tool, ese mismo cursor SÍ vale. Sin esto, un arreglo
        //     que rechazara todos los cursores pasaría este test.
        let mut args_propios = args_receptora.clone();
        let p_propia = roundtrip(
            dir.path(),
            &[linea_call(2, emisora, {
                let mut a = match emisora {
                    "graph_query" => serde_json::json!({"operation": "components"}),
                    _ => serde_json::json!({"scope": {"kind": "workspace"}, "limit": 20}),
                };
                a["cursor"] = serde_json::Value::String(ajeno.clone());
                a
            })
            .as_str()],
            1,
        );
        assert_eq!(
            error_de(&p_propia[0]),
            None,
            "control anti-vacuo: el cursor que «{emisora}» emitió debe seguir sirviéndole a \
             «{emisora}». Si también lo rechaza, el arreglo rompió la paginación en vez de \
             firmarla: {}",
            p_propia[0]
        );

        // (3) El criterio: la MISMA cadena, en otra tool, se rechaza.
        args_propios["cursor"] = serde_json::Value::String(ajeno.clone());
        let resp = roundtrip(
            dir.path(),
            &[linea_call(3, receptora, args_propios).as_str()],
            1,
        );
        let err = error_de(&resp[0]).unwrap_or_else(|| {
            panic!(
                "el `nextCursor` «{ajeno}» lo emitió «{emisora}»; «{receptora}» debe RECHAZARLO. \
                 Hasta v0.5.0 el cursor era un offset hex desnudo compartido por las cuatro tools \
                 paginadas, así que «{receptora}» lo aceptaba y servía una página válida en forma y \
                 ajena en significado: el agente cree haber avanzado en la consulta que pidió y ve \
                 un fragmento de otro resultado (ROB-06, `decisiones §23/A-03`).\nRespuesta \
                 recibida: {}",
                resp[0]
            )
        });
        assert_eq!(
            codigo_de(&err),
            "INVALID_SCHEMA",
            "un cursor de otra tool es entrada inválida, con el mismo código que el malformado: \
             «{err}»"
        );
        let (_, mensaje) = codigo_y_mensaje(&err)
            .unwrap_or_else(|| panic!("debe emitir «CÓDIGO: mensaje» (E26-H07): «{err}»"));
        assert!(
            menciona(mensaje, "cursor"),
            "…nombrando el parámetro `cursor`: «{err}»"
        );
        assert!(
            menciona(mensaje, receptora) || menciona(mensaje, emisora),
            "…y nombrando de qué tool es el cursor frente a cuál lo esperaba («{emisora}» / \
             «{receptora}»): sin eso el mensaje es indistinguible del de un cursor basura, y el \
             agente no sabe que lo que hizo fue mezclar dos paginaciones: «{err}»"
        );

        // (4) …y el rechazo no puede haber sido «esta tool no pagina»: sin cursor sigue sirviendo.
        let limpia = roundtrip(
            dir.path(),
            &[linea_call(4, receptora, args_receptora).as_str()],
            1,
        );
        assert!(
            !lista(sc_ok(&limpia[0], receptora), clave).is_empty(),
            "control anti-vacuo: «{receptora}» sin `cursor` debe seguir sirviendo su lista: {}",
            limpia[0]
        );
    }
}

/// **E30-H01** · Criterio `cursor_de_otro_modo_de_la_misma_tool_segun_decision_de_fase_roja`:
/// **Dado** un `nextCursor` devuelto por `metadata_inspect` en modo `catalog`, **Cuando** se pasa a
/// `metadata_inspect` en modo `field`, **Entonces** también es rechazado.
///
/// **La decisión de la fase roja** (declarada arriba, en la cabecera de la sección): la identidad
/// del cursor se ata a la **tool y al contexto de listado** — para `metadata_inspect`, su `mode` y,
/// en mode «field», el `field`. Es del mismo género que A-03 **dentro** de una sola tool: el
/// catálogo se ordena por field path y el vocabulario por conteo→texto, así que un offset del
/// primero aplicado al segundo apunta a una entrada arbitraria de otra lista. Rechazarlo es la misma
/// garantía, no una nueva.
///
/// Se ejercen las dos direcciones (catalog→field y field→catalog) y, además, el cruce entre **dos
/// campos distintos** del mismo modo `field`: son dos vocabularios distintos con órdenes totales
/// distintos, así que la identidad no puede quedarse en el `mode` y olvidar el `field`.
///
/// Control anti-vacuo: cada cursor sigue valiendo en su propio modo/campo.
#[test]
fn cursor_de_otro_modo_de_la_misma_tool_segun_decision_de_fase_roja() {
    let dir = ws_cota();

    // Contextos de listado de `metadata_inspect` que este test considera DISTINTOS entre sí.
    let contextos: [(&str, serde_json::Value, &str); 3] = [
        ("catalog", serde_json::json!({"mode": "catalog"}), "fields"),
        (
            "field:uid",
            serde_json::json!({"mode": "field", "field": "uid"}),
            "values",
        ),
        (
            "field:campo000",
            serde_json::json!({"mode": "field", "field": "campo000"}),
            "values",
        ),
    ];

    // Los dos primeros contextos tienen más entradas que la página por defecto (152 campos y 150
    // valores de `uid`), así que emiten cursor; `campo000` solo tiene 1 valor y sirve de RECEPTOR.
    for (i, (nombre_emisor, args_emisor, _)) in contextos.iter().enumerate().take(2) {
        let p1 = roundtrip(
            dir.path(),
            &[linea_call(1, "metadata_inspect", args_emisor.clone()).as_str()],
            1,
        );
        let cursor = cursor_de(sc_ok(&p1[0], "metadata_inspect")).unwrap_or_else(|| {
            panic!(
                "precondición: el contexto «{nombre_emisor}» debe emitir un `nextCursor` real \
                 (>100 entradas con la cota por defecto): {}",
                p1[0]
            )
        });

        // Control anti-vacuo: en SU contexto, ese cursor sigue paginando.
        let mut propios = args_emisor.clone();
        propios["cursor"] = serde_json::Value::String(cursor.clone());
        let propia = roundtrip(
            dir.path(),
            &[linea_call(2, "metadata_inspect", propios).as_str()],
            1,
        );
        assert_eq!(
            error_de(&propia[0]),
            None,
            "el cursor de «{nombre_emisor}» debe seguir sirviendo en «{nombre_emisor}»: firmar el \
             origen no puede consistir en rechazar también el camino feliz: {}",
            propia[0]
        );

        // El criterio: el mismo cursor en CUALQUIER otro contexto de la misma tool se rechaza.
        for (j, (nombre_receptor, args_receptor, clave)) in contextos.iter().enumerate() {
            if i == j {
                continue;
            }
            let mut ajenos = args_receptor.clone();
            ajenos["cursor"] = serde_json::Value::String(cursor.clone());
            let resp = roundtrip(
                dir.path(),
                &[linea_call(3, "metadata_inspect", ajenos).as_str()],
                1,
            );
            let err = error_de(&resp[0]).unwrap_or_else(|| {
                panic!(
                    "el cursor «{cursor}» lo emitió `metadata_inspect` en el contexto \
                     «{nombre_emisor}»; en «{nombre_receptor}» debe RECHAZARSE: son dos listas con \
                     órdenes totales distintos (catálogo por field path, vocabulario por \
                     conteo→texto, y un vocabulario por campo), así que reinterpretar el offset \
                     apunta a una entrada arbitraria de otra lista — el mismo defecto que A-03, \
                     dentro de una sola tool.\nRespuesta recibida ({clave}): {}",
                    resp[0]
                )
            });
            assert_eq!(
                codigo_de(&err),
                "INVALID_SCHEMA",
                "…con el mismo código que los otros dos casos de cursor ajeno: «{err}»"
            );
            let (_, mensaje) = codigo_y_mensaje(&err)
                .unwrap_or_else(|| panic!("debe emitir «CÓDIGO: mensaje» (E26-H07): «{err}»"));
            assert!(
                menciona(mensaje, "cursor"),
                "…nombrando el parámetro `cursor`: «{err}»"
            );
        }
    }
}

/// **E30-H01** · Criterio `cursor_legitimo_de_la_misma_tool_sigue_paginando` (ANTI-VACUO clave):
/// **Dado** un workspace con más documentos que el `limit` de página, **Cuando** se pide la primera
/// página de `knowledge_search`, se toma su `nextCursor` y se usa **ese** valor exacto en una
/// segunda llamada con los **mismos** parámetros, **Entonces** la segunda página se sirve
/// correctamente y sin solaparse con la primera.
///
/// El cursor se obtiene de una respuesta **real** (nunca fabricado a mano): es lo que impide que el
/// roundtrip sea vacuo. Y cada página sale de un proceso FRESCO (`roundtrip` arranca y termina el
/// servidor), así que el criterio incluye que el cursor firmado siga siendo **autosuficiente** —
/// la propiedad que `encode_cursor` ya declara y que la historia manda conservar: nada de estado de
/// sesión.
#[test]
fn cursor_legitimo_de_la_misma_tool_sigue_paginando() {
    let dir = workspace_cincuenta();
    let args = serde_json::json!({"text": "paginacion", "limit": 20});

    // Página 1 (sin cursor) y su `nextCursor` REAL.
    let p1 = roundtrip(
        dir.path(),
        &[linea_call(1, "knowledge_search", args.clone()).as_str()],
        1,
    );
    let sc1 = sc_ok(&p1[0], "knowledge_search");
    let pagina1 = search_paths(&p1[0]);
    assert_eq!(
        pagina1.len(),
        20,
        "precondición: con 50 documentos y `limit: 20` la primera página trae 20: {}",
        p1[0]
    );
    let cursor = cursor_de(sc1).unwrap_or_else(|| {
        panic!(
            "precondición: con 50 > 20 resultados debe venir un `nextCursor` real que reusar: {}",
            p1[0]
        )
    });

    // Página 2, con ESE valor exacto y los MISMOS parámetros, en un proceso fresco.
    let mut args2 = args.clone();
    args2["cursor"] = serde_json::Value::String(cursor.clone());
    let p2 = roundtrip(
        dir.path(),
        &[linea_call(1, "knowledge_search", args2).as_str()],
        1,
    );
    assert_eq!(
        error_de(&p2[0]),
        None,
        "el `nextCursor` que la propia `knowledge_search` acaba de emitir, reusado con los MISMOS \
         parámetros, debe seguir paginando: endurecer el cursor no puede romper el camino feliz \
         que todo cliente que pagina ya usa: {}",
        p2[0]
    );
    let pagina2 = search_paths(&p2[0]);
    assert_eq!(pagina2.len(), 20, "…y traer los 20 siguientes: {}", p2[0]);

    // Sin solapamiento entre las dos páginas.
    let en_p1: std::collections::BTreeSet<&String> = pagina1.iter().collect();
    for path in &pagina2 {
        assert!(
            !en_p1.contains(path),
            "«{path}» aparece en las dos páginas: el cursor legítimo debe CONTINUAR, no reiniciar \
             (reiniciar es exactamente lo que hacía un cursor no decodificable hasta v0.5.0)"
        );
    }
}

/// **E30-H01** · Criterio del recorrido completo (ANTI-VACUO obligatorio de la historia):
/// **Dado** el recorrido completo por cursor (primera página → cursor → … hasta `nextCursor: null`),
/// **Cuando** se concatenan todas las páginas, **Entonces** el conjunto coincide exactamente con el
/// que produce la tool sin paginar — mismo orden total, sin duplicados ni huecos — en **cada una** de
/// las cuatro tools paginadas.
///
/// Es el par natural de los tres rechazos: garantiza que la firma de origen no rompió lo único que
/// la paginación tenía que seguir haciendo. Todos los cursores del recorrido salen de respuestas
/// reales (`recorre_paginas` sigue el `nextCursor` de cada página) y cada página se pide en un
/// proceso fresco.
///
/// `knowledge_check` es la que faltaba en el arnés de E26-H10 (`paginar_no_pierde_ni_duplica` cubre
/// `graph_query` y los dos modos de `metadata_inspect`): aquí entra con un `limit` pequeño sobre un
/// workspace con un diagnóstico por documento, que es lo que fuerza varias páginas sin necesitar
/// mil documentos.
#[test]
fn recorrido_completo_por_cursor_legitimo_en_las_cuatro_tools() {
    let dir = ws_cota_rota();

    // (tool, args de la página acotada, args de la referencia completa, lista)
    let casos: [(&str, serde_json::Value, serde_json::Value, &str); 4] = [
        (
            "knowledge_search",
            serde_json::json!({"text": TOKEN_BUSCABLE, "limit": 20}),
            serde_json::json!({"text": TOKEN_BUSCABLE, "limit": 100}),
            "results",
        ),
        (
            "knowledge_check",
            serde_json::json!({"scope": {"kind": "workspace"}, "limit": 20}),
            serde_json::json!({"scope": {"kind": "workspace"}, "limit": 1000}),
            "diagnostics",
        ),
        (
            "metadata_inspect",
            serde_json::json!({"mode": "field", "field": "uid", "limit": 20}),
            serde_json::json!({"mode": "field", "field": "uid", "limit": 1000}),
            "values",
        ),
        (
            "graph_query",
            serde_json::json!({"operation": "components", "limit": 20}),
            serde_json::json!({"operation": "components", "limit": 1000}),
            "nodes",
        ),
    ];

    for (tool, args, args_full, clave) in casos {
        // La verdad completa, en una sola página.
        let full = roundtrip(dir.path(), &[linea_call(1, tool, args_full).as_str()], 1);
        let sc_full = sc_ok(&full[0], tool);
        let completo = lista(sc_full, clave);
        assert_eq!(
            cursor_de(sc_full),
            None,
            "precondición de «{tool}/{clave}»: la llamada de referencia debe caber entera en una \
             página (`nextCursor` nulo): {}",
            full[0]
        );

        // El recorrido, cursor a cursor, proceso a proceso.
        let (recorrido, paginas) = recorre_paginas(dir.path(), tool, &args, clave);
        assert!(
            paginas >= 2,
            "el recorrido de «{tool}/{clave}» se agotó en {paginas} página(s): sin más de una \
             página este criterio sería vacuo (no habría ni un cursor real que seguir)"
        );
        assert_eq!(
            recorrido, completo,
            "la concatenación de las {paginas} páginas de «{tool}/{clave}» debe ser EXACTAMENTE el \
             resultado sin paginar, en el mismo orden: la firma de origen del cursor no puede \
             perder ni duplicar nada por el camino"
        );
        let distintos: std::collections::BTreeSet<String> =
            recorrido.iter().map(ToString::to_string).collect();
        assert_eq!(
            distintos.len(),
            recorrido.len(),
            "…y ninguna entrada puede aparecer en dos páginas de «{tool}/{clave}»"
        );
    }
}

/// **E30-H01** · Criterio del `nextCursor` al agotar (segunda mitad del anti-vacuo):
/// **Dado** el recorrido completo de cada tool paginada, **Cuando** se sirve la última página,
/// **Entonces** su `nextCursor` está **ausente o es `null`** — nunca un cursor que apunte más allá
/// del final ni una cadena vacía.
///
/// Sin esto, un cursor firmado que nunca se agotara dejaría al agente en un bucle infinito de
/// páginas vacías, y el recorrido de arriba pasaría igual (su tope de 20 páginas lo cortaría antes).
/// `cursor_de` ya rechaza la cadena vacía como forma degenerada de «agotado».
#[test]
fn el_ultimo_next_cursor_es_nulo_al_agotar() {
    let dir = ws_cota_rota();

    let casos: [(&str, serde_json::Value, &str); 4] = [
        (
            "knowledge_search",
            serde_json::json!({"text": TOKEN_BUSCABLE, "limit": 20}),
            "results",
        ),
        (
            "knowledge_check",
            serde_json::json!({"scope": {"kind": "workspace"}, "limit": 20}),
            "diagnostics",
        ),
        (
            "metadata_inspect",
            serde_json::json!({"mode": "field", "field": "uid", "limit": 20}),
            "values",
        ),
        (
            "graph_query",
            serde_json::json!({"operation": "components", "limit": 20}),
            "nodes",
        ),
    ];

    for (tool, args, clave) in casos {
        let mut cursor: Option<String> = None;
        let mut ultima = serde_json::Value::Null;
        for vuelta in 0..20 {
            let mut a = args.clone();
            if let Some(c) = &cursor {
                a["cursor"] = serde_json::Value::String(c.clone());
            }
            let resp = roundtrip(dir.path(), &[linea_call(1, tool, a).as_str()], 1);
            let sc = sc_ok(&resp[0], tool).clone();
            ultima = resp[0].clone();
            match cursor_de(&sc) {
                Some(c) => cursor = Some(c),
                None => {
                    assert!(
                        vuelta >= 1,
                        "«{tool}/{clave}» agotó el recorrido en la primera página: el criterio no \
                         mide nada si nunca hubo un cursor real que seguir"
                    );
                    cursor = None;
                    break;
                }
            }
            assert!(
                vuelta < 19,
                "«{tool}/{clave}» sigue emitiendo `nextCursor` tras 20 páginas: un cursor que no \
                 se agota deja al agente en un bucle de páginas: última respuesta {ultima}"
            );
        }
        assert!(
            cursor.is_none(),
            "…y la última página de «{tool}/{clave}» debe traer `nextCursor` nulo o ausente: \
             {ultima}"
        );
    }
}

/// **E30-H01** · Decisión declarada de la fase roja (`cursor: ""` == ausente):
/// **Dado** una llamada con `cursor: ""` (cadena vacía), **Cuando** se ejecuta, **Entonces** se
/// trata como si el parámetro no viniera: la primera página, sin error.
///
/// Es el comportamiento de hoy y el que `descubribilidad.rs::el_schema_declarado_coincide_con_lo_aceptado`
/// da por bueno al mandar `json!("")` como valor de ejemplo de este parámetro declarado. Convertir
/// la cadena vacía en `INVALID_SCHEMA` rompería esa guarda por una razón ajena al defecto que esta
/// historia cierra (un cursor de wire nunca se emite vacío: `cursor_de` ya prohíbe esa forma). Se
/// deja **fijado con test** para que la decisión no quede a interpretación de quien la lea después.
#[test]
fn cursor_vacio_cuenta_como_ausente() {
    let dir = ws_cota();

    for (tool, args, clave) in casos_paginados() {
        let sin = roundtrip(dir.path(), &[linea_call(1, tool, args.clone()).as_str()], 1);
        let esperado = lista(sc_ok(&sin[0], tool), clave);

        let mut vacio = args.clone();
        vacio["cursor"] = serde_json::json!("");
        let resp = roundtrip(dir.path(), &[linea_call(2, tool, vacio).as_str()], 1);
        assert_eq!(
            error_de(&resp[0]),
            None,
            "un `cursor: \"\"` cuenta como AUSENTE (decisión declarada de E30-H01), no como \
             malformado: «{tool}» debe servir su primera página. Rechazarlo rompería además \
             `descubribilidad.rs::el_schema_declarado_coincide_con_lo_aceptado`, que manda \
             exactamente ese valor de ejemplo: {}",
            resp[0]
        );
        assert_eq!(
            lista(sc_ok(&resp[0], tool), clave),
            esperado,
            "…y esa página debe ser idéntica a la de la llamada sin `cursor`: {}",
            resp[0]
        );
    }
}

/// **E30-H01** · Hallazgo de seguimiento DECLARADO (no un hueco descubierto por accidente):
/// **Dado** un `nextCursor` legítimo de `knowledge_search`, **Cuando** se reusa en `knowledge_search`
/// con una **consulta distinta** (otro `text`/`where`), **Entonces** esta historia **no** lo rechaza:
/// la identidad del cursor se ata a la tool y a su contexto de listado, no al criterio de selección.
///
/// La justificación está escrita en la cabecera de la sección (resumen: `where`/`filter` admiten
/// formas equivalentes que hashearían distinto, y atarlas convertiría paginaciones legítimas en
/// `INVALID_SCHEMA`). Este test **no** bendice el comportamiento como correcto: fija lo que la
/// historia SÍ garantiza —que la llamada no revienta— para que quien retome el seguimiento sepa
/// exactamente qué había cuando lo dejó, y para que la decisión conste en la suite y no solo en la
/// prosa.
#[test]
fn cursor_de_otra_consulta_de_la_misma_tool_es_hallazgo_de_seguimiento() {
    let dir = ws_cota();

    let p1 = roundtrip(
        dir.path(),
        &[linea_call(
            1,
            "knowledge_search",
            serde_json::json!({"text": "nota", "limit": 20}),
        )
        .as_str()],
        1,
    );
    let cursor = cursor_de(sc_ok(&p1[0], "knowledge_search")).unwrap_or_else(|| {
        panic!(
            "precondición: con {DOCS_COTA} documentos y `limit: 20` debe haber `nextCursor`: {}",
            p1[0]
        )
    });

    // Misma tool, MISMO contexto de listado, distinta consulta: la historia lo deja pasar.
    let resp = roundtrip(
        dir.path(),
        &[linea_call(
            2,
            "knowledge_search",
            serde_json::json!({"text": "nota", "where": "status = \"draft\"",
                               "limit": 20, "cursor": cursor}),
        )
        .as_str()],
        1,
    );
    assert_eq!(
        error_de(&resp[0]),
        None,
        "SEGUIMIENTO DECLARADO de E30-H01: la identidad del cursor se ata a (tool, contexto de \
         listado), no al criterio de selección, así que un cursor reusado con otra consulta de la \
         MISMA tool sigue paginando. Si una historia futura decide atarlo también a la consulta, \
         este test es el que hay que reescribir —a conciencia—, no un rojo sorpresa: {}",
        resp[0]
    );
}

/// **E30-H01** — Workspace del recorrido completo: los [`DOCS_COTA`] documentos de [`ws_cota`] más un
/// enlace roto por documento, para que `knowledge_check` tenga un diagnóstico por documento y su
/// recorrido necesite varias páginas con `limit: 20`.
///
/// Las cuatro tools paginadas se recorren sobre el MISMO workspace: `knowledge_check` emite 150
/// `LINK-TARGET-MISSING`, `metadata_inspect{field:uid}` tiene 150 valores distintos,
/// `graph_query{components}` 150 nodos y `knowledge_search` casa los [`DOCS_BUSCABLES`] documentos
/// que llevan el token único [`TOKEN_BUSCABLE`] en su cuerpo.
///
/// **Por qué la búsqueda se acota a un subconjunto**: el máximo de `knowledge_search.limit` es 100
/// (E24-H09), así que la llamada de REFERENCIA sin paginar solo existe si el conjunto que casa cabe
/// en 100. Con un token que casa 60 documentos el recorrido sigue necesitando 3 páginas de 20 y la
/// referencia cabe entera — que es lo que el criterio compara.
fn ws_cota_rota() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..DOCS_COTA {
        let siguiente = format!("n{:03}.md", (i + 1) % DOCS_COTA);
        let buscable = if i < DOCS_BUSCABLES {
            format!("\n\n{TOKEN_BUSCABLE}\n")
        } else {
            String::new()
        };
        write(
            dir.path(),
            &format!("n{i:03}.md"),
            &format!(
                "---\ncampo{i:03}: {i}\nstatus: draft\nuid: u{i:03}\n---\n\n# Nota {i}\n\n\
                 [siguiente]({siguiente})\n\n[roto](no-existe-{i:03}.md){buscable}\n"
            ),
        );
    }
    dir
}

/// Token que solo llevan los [`DOCS_BUSCABLES`] primeros documentos de [`ws_cota_rota`].
const TOKEN_BUSCABLE: &str = "buscableunico";

/// Documentos de [`ws_cota_rota`] que casan [`TOKEN_BUSCABLE`]: más de una página de 20 (el
/// recorrido necesita 3) y menos que el máximo de `knowledge_search` (100), para que exista una
/// llamada de referencia sin paginar contra la que comparar.
const DOCS_BUSCABLES: usize = 60;
