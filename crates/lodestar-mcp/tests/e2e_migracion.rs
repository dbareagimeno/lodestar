//! **E22-H04** — Verificación end-to-end de la migración a workspaces Markdown universales.
//!
//! Recorre el flujo completo de `docs/REFACTOR_PHASE_2 §Resultado esperado`/`§Criterios de
//! aceptación` **por la superficie MCP JSON-RPC real** (el binario `lodestar-mcp`, arrancado sobre
//! un directorio arbitrario), sobre un proyecto que **nunca ha visto Lodestar**: sin `.lodestar/`,
//! sin `index.md`, sin frontmatter obligatorio, con documentación repartida a varias profundidades.
//!
//! Cada paso comprueba un criterio de aceptación final del documento; el conjunto es la prueba de
//! que la migración funciona de punta a punta, no solo por tests unitarios.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Command, Stdio};

use serde_json::{json, Value};

/// Escribe un fichero bajo `root`, creando los directorios intermedios.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    std::fs::create_dir_all(ruta.parent().unwrap()).unwrap();
    std::fs::write(ruta, contenido).unwrap();
}

/// Arranca `lodestar-mcp --root <dir>`, envía las líneas JSON-RPC y recoge `expect` respuestas.
fn mcp(dir: &Path, lineas: &[String], expect: usize) -> Vec<Value> {
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
    for l in lineas {
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

/// Una llamada `tools/call` como línea JSON-RPC.
fn call(id: u32, name: &str, args: Value) -> String {
    json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
           "params":{"name":name,"arguments":args}})
    .to_string()
}

fn init() -> String {
    json!({"jsonrpc":"2.0","id":0,"method":"initialize","params":{}}).to_string()
}

/// El `structuredContent` de una respuesta `tools/call`.
fn sc(resp: &Value) -> &Value {
    &resp["result"]["structuredContent"]
}

/// Monta el proyecto arbitrario del `§Resultado esperado`: documentación a varias profundidades,
/// **sin** `.lodestar/`/`index.md`/frontmatter obligatorio, con enlaces cruzados raíz↔profundo y
/// frontmatter YAML arbitrario (tipos reales) para ejercitar la consulta tipada.
///
/// **AMPLIADO en E23-H08** con la otra mitad de un repo real: un `.gitignore` con `vendor/` y un
/// `.lodestarignore` con `borradores/`, cada uno con un `.md` dentro. Los dos documentos ocultos
/// llevan **el mismo frontmatter que los visibles** (`type: decision`, `status: draft`) a propósito:
/// así todas las aserciones de inventario del fichero pasan a ser discriminantes. Si el
/// descubrimiento dejara de aplicar los ficheros de exclusión, `counts.documents` sería 7 en vez de
/// 5, `where type = "decision"` daría 4 en vez de 2 y la selección masiva expandiría 4 ops en vez de
/// 2 — y el `change_apply` escribiría dentro de `vendor/`.
fn proyecto_arbitrario(root: &Path) {
    // Raíz: enlaza a un documento profundo (ida) y a código del proyecto.
    escribe(
        root,
        "README.md",
        "# Mi proyecto\n\nArquitectura de [autenticación](packages/api/docs/auth.md).\n\
         Código: [token service](src/auth/token.rs).\n",
    );
    // Documento profundo con frontmatter YAML arbitrario (número, lista, objeto anidado) y vuelta a
    // la raíz por un enlace relativo de tres niveles.
    escribe(
        root,
        "packages/api/docs/auth.md",
        "---\ntype: decision\nstatus: draft\npriority: 3\nowners: [platform, security]\n\
         service:\n  name: authentication\n  tier: critical\n---\n\n\
         # Autenticación\n\nVolver a la [visión general](../../../README.md).\n",
    );
    // Otra decisión draft (para la selección masiva) y una guía accepted (para el filtro).
    escribe(
        root,
        "docs/decisions/cache.md",
        "---\ntype: decision\nstatus: draft\npriority: 1\n---\n# Cache\n",
    );
    escribe(
        root,
        "docs/guide.md",
        "---\ntype: guide\nstatus: accepted\npriority: 2\n---\n# Guía\n",
    );
    // Un documento aislado (sin enlaces internos en ningún sentido).
    escribe(root, "knowledge/roadmap/2027.md", "# Roadmap 2027\n");
    // Un fichero de código que EXISTE (destino del enlace WorkspaceFile de README).
    escribe(root, "src/auth/token.rs", "// token service\n");

    // --- E23-H08: lo que un repo real trae y Lodestar NO debe mirar ---------------------------
    // `.gitignore` del proyecto: el caso del pitch (`node_modules/`, `target/`, `vendor/`). Se
    // respeta aunque no haya repo git (`discovery.rs` construye el walker con `require_git(false)`).
    escribe(root, ".gitignore", "vendor/\n");
    escribe(
        root,
        "vendor/basura.md",
        "---\ntype: decision\nstatus: draft\npriority: 9\n---\n\n\
         # Dependencia vendorizada\n\nPalabra que no existe en ningún otro documento: sarpullido.\n",
    );
    // `.lodestarignore`: exclusiones propias, independientes de git.
    escribe(root, ".lodestarignore", "borradores/\n");
    escribe(
        root,
        "borradores/wip.md",
        "---\ntype: decision\nstatus: draft\npriority: 8\n---\n\n\
         # Borrador\n\nPalabra que no existe en ningún otro documento: ornitorrinco.\n",
    );
}

/// Los 5 documentos que la fachada **sí** debe ver, ordenados.
fn documentos_visibles() -> Vec<String> {
    let mut v = vec![
        "README.md".to_string(),
        "docs/decisions/cache.md".to_string(),
        "docs/guide.md".to_string(),
        "knowledge/roadmap/2027.md".to_string(),
        "packages/api/docs/auth.md".to_string(),
    ];
    v.sort();
    v
}

/// Los `path` de los resultados de un `knowledge_search`, ordenados.
fn paths_de_resultados(resp: &Value) -> Vec<String> {
    let mut v: Vec<String> = sc(resp)["results"]
        .as_array()
        .unwrap_or_else(|| panic!("knowledge_search debe devolver `results`: {resp}"))
        .iter()
        .map(|x| x["path"].as_str().unwrap().to_string())
        .collect();
    v.sort();
    v
}

/// Comprueba **por la superficie MCP** —una sola sesión, el binario real— que un documento excluido
/// por el descubrimiento es invisible para el agente: ni lo cuenta `workspace_status`, ni lo
/// devuelve `knowledge_search` (ni por su palabra única ni en el inventario completo), ni lo deja
/// leer `knowledge_get`.
///
/// `palabra_unica` es un término que **solo** está en el documento oculto; `Roadmap` hace de
/// **control anti-vacuo**: si la búsqueda estuviera rota (o devolviera siempre vacío), el «0
/// resultados» de la palabra oculta no probaría nada, así que se exige en la MISMA sesión que la
/// búsqueda sí encuentre un documento visible.
fn asevera_documento_ignorado(root: &Path, ruta_ignorada: &str, palabra_unica: &str) {
    // Precondición: el `.md` oculto EXISTE en disco y es legible. Sin esto, «no aparece» podría
    // deberse a que el fixture no lo escribió.
    let contenido = std::fs::read_to_string(root.join(ruta_ignorada))
        .unwrap_or_else(|e| panic!("precondición: `{ruta_ignorada}` debe existir en disco ({e})"));
    assert!(
        contenido.contains(palabra_unica),
        "precondición: `{ruta_ignorada}` debe contener la palabra única «{palabra_unica}»"
    );

    let r = mcp(
        root,
        &[
            init(),
            call(1, "workspace_status", json!({})),
            call(2, "knowledge_search", json!({ "text": palabra_unica })),
            call(3, "knowledge_search", json!({"text": "Roadmap"})),
            call(4, "knowledge_search", json!({"text": ""})),
            call(5, "knowledge_get", json!({"ref": {"path": ruta_ignorada}})),
        ],
        6,
    );

    // (1) El inventario: los 5 visibles, ni uno más.
    assert_eq!(
        sc(&r[1])["counts"]["documents"],
        5,
        "`{ruta_ignorada}` está excluido por el descubrimiento: no puede contar como documento. \
         counts = {}",
        sc(&r[1])["counts"]
    );

    // (2) Su palabra única no encuentra nada…
    assert_eq!(
        paths_de_resultados(&r[2]),
        Vec::<String>::new(),
        "`knowledge_search` no puede devolver un documento excluido (búsqueda por «{palabra_unica}»)"
    );
    // (3) …y el control demuestra que la búsqueda de esa misma sesión SÍ funciona.
    assert_eq!(
        paths_de_resultados(&r[3]),
        vec!["knowledge/roadmap/2027.md".to_string()],
        "control anti-vacuo: la búsqueda debe encontrar el documento visible «Roadmap»"
    );

    // (4) El inventario completo que ve el agente son exactamente los 5 visibles.
    assert_eq!(
        paths_de_resultados(&r[4]),
        documentos_visibles(),
        "el inventario que expone `knowledge_search` son los documentos NO excluidos"
    );

    // (5) Tampoco se puede leer pidiéndolo por su path exacto.
    assert_eq!(
        r[5]["result"]["isError"], true,
        "`knowledge_get` de un documento excluido debe ser error, no una lectura silenciosa: {}",
        r[5]
    );
    assert!(
        r[5]["result"].to_string().contains("DOCUMENT_NOT_FOUND"),
        "el error debe llevar el código estable `DOCUMENT_NOT_FOUND`: {}",
        r[5]
    );
}

/// **E23-H08** · Criterio `gitignore_respetado_por_la_fachada`: **Dado** un proyecto con `vendor/`
/// en el `.gitignore` y un `.md` dentro, **Cuando** se pregunta a la superficie MCP, **Entonces** ni
/// `counts` ni `knowledge_search` lo incluyen.
///
/// HUECO QUE CIERRA: el descubrimiento tiene 13 tests a nivel de `lodestar-workspace`
/// (`crates/lodestar-workspace/tests/discovery.rs`) pero **ninguno por una fachada**, siendo la
/// promesa central del refactor: apuntar Lodestar a un repo real —con `node_modules/`, `target/`,
/// `vendor/` llenos de `.md`— y que solo vea el conocimiento del proyecto (`ARCHITECTURE.md §20.5`).
/// Un cableado que perdiera la `DiscoveryPolicy` entre `App` y `Workspace` sería invisible para
/// aquellos tests y catastrófico para el usuario.
///
/// NO es fase roja: el descubrimiento ya funciona y el test sale verde. Su valor es de cobertura y
/// regresión sobre la frontera.
#[test]
fn gitignore_respetado_por_la_fachada() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_arbitrario(root);
    asevera_documento_ignorado(root, "vendor/basura.md", "sarpullido");
}

/// **E23-H08** (misma garantía, el otro fichero de exclusiones): **Dado** un `.lodestarignore` con
/// `borradores/` y un `.md` dentro, **Cuando** se pregunta a la superficie MCP, **Entonces** tampoco
/// aparece.
///
/// Va aparte de su gemelo porque son **dos mecanismos independientes** (`git_ignore` del walker vs
/// `add_custom_ignore_filename`): un cambio en la construcción del walker puede romper uno y dejar
/// el otro en pie, y `.lodestarignore` es además el único mecanismo de exclusión de un proyecto que
/// no usa git.
#[test]
fn lodestarignore_respetado_por_la_fachada() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_arbitrario(root);
    asevera_documento_ignorado(root, "borradores/wip.md", "ornitorrinco");
}

/// El flujo completo del documento, paso a paso, cada uno contra su criterio de aceptación.
#[test]
fn flujo_completo_migracion() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_arbitrario(root);

    // Precondición dura: el proyecto NO tiene nada de Lodestar (criterios "no es obligatorio …").
    assert!(!root.join(".lodestar").exists(), "sin .lodestar/");
    assert!(!root.join("index.md").exists(), "sin index.md");

    // --- 1. workspace_status: arranca sin ceremonia y descubre a cualquier profundidad ----------
    let r = mcp(root, &[init(), call(1, "workspace_status", json!({}))], 2);
    let status = sc(&r[1]);
    let counts = &status["counts"];
    // 5 documentos .md (token.rs NO es documento), a 3 niveles de profundidad.
    //
    // E23-H08: este `5` es ahora una aserción MÁS FUERTE que antes. El fixture tiene 7 ficheros
    // `.md` en disco; dos de ellos están excluidos por el `.gitignore` (`vendor/basura.md`) y por el
    // `.lodestarignore` (`borradores/wip.md`). Que el inventario siga siendo 5 —y no 7— es
    // exactamente la prueba de que el descubrimiento aplica los ficheros de exclusión por la
    // fachada. El detalle (búsqueda, lectura, control anti-vacuo) va en
    // `gitignore_respetado_por_la_fachada`/`lodestarignore_respetado_por_la_fachada`.
    assert_eq!(counts["documents"], 5, "descubre los 5 .md: {counts}");

    // --- 2. knowledge_search con `where` tipado sobre frontmatter arbitrario --------------------
    // status = accepted → solo la guía.
    let r = mcp(
        root,
        &[
            init(),
            call(
                1,
                "knowledge_search",
                json!({"where":"status = \"accepted\""}),
            ),
        ],
        2,
    );
    let paths: Vec<String> = sc(&r[1])["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|x| x["path"].as_str().unwrap().to_string())
        .collect();
    assert_eq!(paths, vec!["docs/guide.md"], "where status=accepted");

    // La regla de tipos viva: priority >= "high" (string vs número) NO compara texto. Hasta v0.4.0
    // el documento que erraba se EXCLUÍA (criterio de E19-H04) y esta llamada devolvía `results: []`
    // —una lista vacía indistinguible de «no hay resultados»—; **E26-H08 revisa ese criterio**: un
    // `TypeError` de evaluación aborta la consulta con INVALID_SCHEMA y un mensaje que nombra campo,
    // operador, los dos tipos y el documento. La aserción se AMPLÍA (no se borra): lo que ya
    // demostraba —que el lenguaje respeta los tipos y no cae en el orden lexicográfico— sigue
    // demostrándolo, ahora por el veredicto explícito.
    let r = mcp(
        root,
        &[
            init(),
            call(
                1,
                "knowledge_search",
                json!({"where":"priority >= \"high\""}),
            ),
        ],
        2,
    );
    assert_eq!(
        r[1]["result"]["isError"],
        json!(true),
        "priority >= \"high\" sobre `priority` numérico no es respondible: debe FALLAR, no \
         devolver una lista recortada (E26-H08): {}",
        r[1]
    );
    let e = r[1]["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_default()
        .to_string();
    assert!(
        e.starts_with("INVALID_SCHEMA: "),
        "…con el código del catálogo y su mensaje (E26-H07): «{e}»"
    );
    assert!(
        e.contains("priority") && e.contains("number") && e.contains("string"),
        "…nombrando el campo y los dos tipos que chocaron: «{e}»"
    );
    assert!(
        e.contains("docs/decisions/cache.md"),
        "…y el PRIMER documento del orden total que yerra (`README.md` va antes pero no tiene \
         `priority`, así que se excluye sin error): «{e}»"
    );

    // where y filter equivalentes dan el MISMO conjunto.
    let where_q = call(
        1,
        "knowledge_search",
        json!({"where":"type = \"decision\""}),
    );
    let filter_q = call(
        2,
        "knowledge_search",
        json!({"filter":{"field":"type","operator":"equals","value":"decision"}}),
    );
    let r = mcp(root, &[init(), where_q, filter_q], 3);
    let set = |resp: &Value| -> Vec<String> {
        let mut v: Vec<String> = sc(resp)["results"]
            .as_array()
            .unwrap()
            .iter()
            .map(|x| x["path"].as_str().unwrap().to_string())
            .collect();
        v.sort();
        v
    };
    assert_eq!(
        set(&r[1]),
        set(&r[2]),
        "where y filter dan el mismo resultado"
    );
    assert_eq!(set(&r[1]).len(), 2, "las 2 decisiones (auth, cache)");

    // --- 3. knowledge_get: enlaces resueltos por path y clasificados ----------------------------
    let r = mcp(
        root,
        &[
            init(),
            call(
                1,
                "knowledge_get",
                json!({"ref":{"path":"README.md"},"include":["outgoingLinks"]}),
            ),
        ],
        2,
    );
    let enlaces = sc(&r[1])["document"]["outgoingLinks"].as_array().unwrap();
    let clase = |href_sub: &str| -> String {
        enlaces
            .iter()
            .find(|l| l["href"].as_str().unwrap().contains(href_sub))
            .map(|l| l["target"]["kind"].as_str().unwrap().to_string())
            .unwrap_or_else(|| format!("(no encontrado: {href_sub})"))
    };
    // El .md profundo es Document; el .rs que existe es WorkspaceFile (no nodo del grafo).
    assert_eq!(clase("auth.md"), "document", "enlace a .md → document");
    assert_eq!(
        clase("token.rs"),
        "workspaceFile",
        "enlace a código existente → workspaceFile"
    );

    // --- 4. metadata_inspect: descubre las convenciones sin schema ------------------------------
    let r = mcp(
        root,
        &[
            init(),
            call(1, "metadata_inspect", json!({"mode":"catalog"})),
        ],
        2,
    );
    let campos: Vec<String> = sc(&r[1])["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["name"].as_str().unwrap().to_string())
        .collect();
    // Descubre el path anidado service.tier sin que exista ningún schema.
    assert!(
        campos.contains(&"service.tier".to_string()),
        "catálogo: {campos:?}"
    );
    assert!(campos.contains(&"status".to_string()));

    // --- 5. graph_query: backlinks globales entre profundidades ---------------------------------
    let r = mcp(
        root,
        &[
            init(),
            call(
                1,
                "graph_query",
                json!({"operation":"backlinks","ref":{"path":"README.md"}}),
            ),
        ],
        2,
    );
    // README tiene un backlink desde el documento profundo (la "vuelta a la raíz").
    let edges = sc(&r[1])["edges"].as_array().unwrap();
    assert!(
        edges
            .iter()
            .any(|e| e["source"] == "packages/api/docs/auth.md" && e["target"] == "README.md"),
        "backlink global raíz←profundo: {edges:?}"
    );

    // Documento aislado consultable (no inválido).
    let r = mcp(
        root,
        &[
            init(),
            call(1, "graph_query", json!({"operation":"isolated"})),
        ],
        2,
    );
    // `graph_query` devuelve los nodos del subgrafo en `nodes` (coherente con orphans/dangling).
    let aislados: Vec<String> = sc(&r[1])["nodes"]
        .as_array()
        .map(|a| {
            a.iter()
                .map(|n| n["id"].as_str().unwrap().to_string())
                .collect()
        })
        .unwrap_or_default();
    assert!(
        aislados.contains(&"knowledge/roadmap/2027.md".to_string()),
        "el documento sin enlaces es aislado (consultable): {aislados:?}"
    );

    // --- 6. change_plan (selección masiva por consulta) → apply → check → revert ----------------
    // Selecciona las decisiones draft y las pasa a review, en un solo plan.
    let plan_line = call(
        1,
        "change_plan",
        json!({
            "selection": {"where": "type = \"decision\" and status = \"draft\""},
            "operation": {"patch_frontmatter": {"status": "review"}}
        }),
    );
    let r = mcp(root, &[init(), plan_line], 2);
    let plan = sc(&r[1]);
    let change_set_id = plan["changeSetId"].as_str().expect("changeSetId");
    // Una op por documento draft (auth.md, cache.md) = 2.
    let n_ops = plan["normalizedOperations"].as_array().unwrap().len();
    assert_eq!(n_ops, 2, "selección masiva: una op por decisión draft");

    // Aplica el plan, luego verifica el estado, y revierte — todo en la misma sesión.
    let apply = call(2, "change_apply", json!({"changeSetId": change_set_id}));
    let r = mcp(root, &[init(), apply], 2);
    let receipt = sc(&r[1]);
    let receipt_id = receipt["receiptId"].as_str().expect("receiptId tras apply");

    // Tras el apply, en disco las dos decisiones son `review`.
    let auth = std::fs::read_to_string(root.join("packages/api/docs/auth.md")).unwrap();
    assert!(
        auth.contains("status: review"),
        "apply escribió status: review en auth.md"
    );
    let cache = std::fs::read_to_string(root.join("docs/decisions/cache.md")).unwrap();
    assert!(
        cache.contains("status: review"),
        "apply escribió status: review en cache.md"
    );

    // knowledge_check: el workspace sigue siendo interpretable (sin errores nuevos).
    let r = mcp(
        root,
        &[
            init(),
            call(1, "knowledge_check", json!({"scope":{"kind":"workspace"}})),
        ],
        2,
    );
    let errores = sc(&r[1])["summary"]["errors"].as_u64().unwrap_or(0);
    assert_eq!(errores, 0, "el workspace no tiene errores tras el cambio");

    // change_revert: vuelve al estado anterior desde las copias de recuperación.
    let revert = call(3, "change_revert", json!({"receiptId": receipt_id}));
    let r = mcp(root, &[init(), revert], 2);
    assert!(
        r[1]["result"]["isError"].as_bool() != Some(true),
        "revert no es error: {}",
        r[1]
    );
    // Tras el revert, auth.md vuelve a `draft`.
    let auth_rev = std::fs::read_to_string(root.join("packages/api/docs/auth.md")).unwrap();
    assert!(
        auth_rev.contains("status: draft"),
        "revert devolvió auth.md a status: draft:\n{auth_rev}"
    );
}
