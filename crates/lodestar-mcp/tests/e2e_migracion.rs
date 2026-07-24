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

    // La regla de tipos viva: priority >= "high" (string vs número) NO devuelve todo, EXCLUYE los
    // numéricos → resultado vacío (no una comparación lexicográfica).
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
    assert!(
        sc(&r[1])["results"].as_array().unwrap().is_empty(),
        "priority >= \"high\" respeta el tipo (excluye numéricos), no compara texto"
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
