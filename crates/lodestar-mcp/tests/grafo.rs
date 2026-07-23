//! Superficie de grafo sobre el modelo nuevo — **E17-H05** (`ARCHITECTURE.md §20.7`, `§20.10`).
//!
//! Fase ROJA de los tres criterios que viajan por el wire MCP: `graph_query(backlinks)`,
//! `graph_query(isolated)` y `knowledge_get` con `outgoingLinks`/`backlinks`. El cuarto criterio
//! (`impacto_sin_tipos_okf`) vive en `crates/lodestar-app/tests/grafo.rs`, que es donde está su
//! superficie.
//!
//! Fichero propio y no una ampliación de `mcp.rs` por la misma razón que E17-H01/H02 no ampliaron
//! `core.rs`: cada fichero de `tests/` es un binario independiente y `mcp.rs` tiene ~90 tests
//! verdes que no pueden dejar de ejecutarse mientras dure el rojo.
//!
//! ---
//!
//! ## Lo que estos tests fijan del wire
//!
//! ```jsonc
//! // graph_query → structuredContent.nodes[i]  (GraphNode, §20.7)
//! { "id": "docs/guia.md", "title": "Guía de uso", "ghost": false }
//! //   ↑ pierde `type`/`status` (campos OKF) y gana el TÍTULO DERIVADO de E16-H03;
//! //     conserva `ghost` para los destinos `Missing`.
//!
//! // knowledge_get → structuredContent.outgoingLinks[i]  (ResolvedLink, §20.6)
//! { "href": "../../../README.md",          // el href CRUDO, byte a byte
//!   "text": "visión general",
//!   "span": { "start": 25, "end": 43 },     // bytes DEL DESTINO dentro del cuerpo
//!   "kind": "inline",
//!   "target": { "kind": "document", "value": "README.md" },
//!   "fragment": null }
//!
//! // knowledge_get → structuredContent.backlinks.inbound[i]  (LinkReference, §20.7)
//! { "from": "three/levels/deep/third.md", "link": { /* el mismo ResolvedLink */ } }
//! ```
//!
//! `outgoingLinks` deja de ser una lista de paths (`Analysis::out`) y pasa a ser la lista de
//! **enlaces resueltos**: es lo que necesita un agente para reescribir un destino sin volver a
//! parsear el Markdown, y lo que `§20.12` guarda en la tabla `links` del store v2.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

/// Escribe un fichero dentro del workspace temporal, creando los directorios intermedios.
fn write(dir: &std::path::Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, content).unwrap();
}

/// Arranca el servidor sobre un workspace, envía `lines` y devuelve las primeras `expect`
/// respuestas (mismo arnés que `mcp.rs`).
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

/// El `structuredContent` de una respuesta de tool.
fn sc(resp: &serde_json::Value) -> &serde_json::Value {
    let v = &resp["result"]["structuredContent"];
    assert!(
        v.is_object(),
        "toda tool devuelve `structuredContent` como objeto: {resp:?}"
    );
    v
}

/// Los nodos de una respuesta `graph_query`.
fn nodos(resp: &serde_json::Value) -> Vec<serde_json::Value> {
    sc(resp)["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("graph_query devuelve `nodes` (array): {resp:?}"))
        .clone()
}

/// Los `id` de una lista de nodos.
fn ids(nodes: &[serde_json::Value]) -> BTreeSet<String> {
    nodes
        .iter()
        .map(|n| {
            n["id"]
                .as_str()
                .unwrap_or_else(|| panic!("cada nodo lleva `id` string: {n:?}"))
                .to_string()
        })
        .collect()
}

/// La forma de `GraphNode` que fija E17-H05: `{id, title, ghost}` — **sin** `type`/`status`.
fn assert_forma_de_nodo(n: &serde_json::Value, titulo_esperado: &str) {
    assert_eq!(
        n["title"].as_str(),
        Some(titulo_esperado),
        "`GraphNode` gana el título derivado (`model::derived_title`, E16-H03): {n}"
    );
    for campo in ["type", "status"] {
        assert!(
            n.get(campo).is_none(),
            "`GraphNode` pierde el campo OKF `{campo}` (`§20.7`): {n}"
        );
    }
    assert!(
        n["ghost"].is_boolean(),
        "`ghost` sobrevive: distingue el destino `Missing` del documento real: {n}"
    );
}

/// Workspace de tres niveles con enlaces cruzados en ambos sentidos, sin `index.md` y sin
/// frontmatter (el `§Resultado esperado` de `REFACTOR_PHASE_2`), más lo que cada criterio necesita.
///
/// ```text
/// README.md                    → one/first.md, three/levels/deep/third.md
/// one/first.md                 → ../two/levels/second.md
/// two/levels/second.md         → (sin salientes)
/// three/levels/deep/third.md   → ../../../README.md          (inline)
/// docs/nota.md                 → ../README.md                (DE REFERENCIA: `[t][r]` + `[r]: …`)
/// suelto.md                    → (ni enlaza ni lo enlazan)
/// solo-externo.md              → https://example.com, #solo-externo
/// ```
///
/// `docs/nota.md` usa un enlace **de referencia**: la regex heredada (`model::LINK_RE`, que solo ve
/// `[t](href)`) no lo ve, así que hoy es un documento aislado y el README no tiene su backlink.
/// Es el enlace que hace que estos criterios no sean vacuos sobre la implementación actual.
fn workspace_profundo() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    write(
        dir.path(),
        "README.md",
        "# Proyecto\n\nEmpieza por [lo primero](one/first.md) y mira lo \
         [profundo](three/levels/deep/third.md).\n",
    );
    write(
        dir.path(),
        "one/first.md",
        "# Primero\n\nHermano en otro árbol: [segundo](../two/levels/second.md).\n",
    );
    write(
        dir.path(),
        "two/levels/second.md",
        "# Segundo\n\nNo enlaza a nadie; solo lo enlazan.\n",
    );
    write(
        dir.path(),
        "three/levels/deep/third.md",
        "# Tercero\n\nVolver a la [visión general](../../../README.md).\n",
    );
    write(
        dir.path(),
        "docs/nota.md",
        "# Nota\n\nVer [el proyecto][r] para el contexto.\n\n[r]: ../README.md\n",
    );
    write(
        dir.path(),
        "suelto.md",
        "# Suelto\n\nNi enlazo ni me enlazan.\n",
    );
    write(
        dir.path(),
        "solo-externo.md",
        "# Solo externo\n\nWeb: [ejemplo](https://example.com).\nAnchor: [aquí](#solo-externo).\n",
    );
    dir
}

// =============================================================================
// Criterio 1 — `graph_backlinks_globales`
// =============================================================================

/// **Dado** un workspace con enlaces a 3 niveles, **Cuando** se pide `graph_query(backlinks)` sobre
/// el documento raíz, **Entonces** devuelve el documento profundo que lo enlaza.
#[test]
fn graph_backlinks_globales() {
    let dir = workspace_profundo();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"graph_query","arguments":{"operation":"backlinks","ref":{"path":"README.md"}}}}"#,
        ],
        1,
    );

    let nodes = nodos(&resp[0]);
    let node_ids = ids(&nodes);
    assert!(
        node_ids.contains("three/levels/deep/third.md"),
        "el backlink del README viene de TRES niveles abajo, con un href `../../../README.md`: los \
         backlinks son globales, no por directorio: {:?}",
        resp[0]
    );

    // Las aristas entrantes del README: el documento profundo (enlace inline) y la nota (enlace DE
    // REFERENCIA, que la regex heredada no ve).
    let edges = sc(&resp[0])["edges"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let fuentes: BTreeSet<String> = edges
        .iter()
        .filter(|e| e["target"] == "README.md")
        .map(|e| e["source"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        fuentes,
        BTreeSet::from([
            "docs/nota.md".to_string(),
            "three/levels/deep/third.md".to_string(),
        ]),
        "el README recibe dos entrantes, uno inline y otro de referencia (`[t][r]` + `[r]: \
         ../README.md`): el grafo se construye con el extractor de E17-H01, no con la regex \
         heredada: {:?}",
        resp[0]
    );

    // No vacuo por partida doble: ni el documento que el README enlaza (saliente, no entrante) ni
    // uno sin relación con él pueden aparecer como fuentes.
    for decoy in ["one/first.md", "suelto.md"] {
        assert!(
            !fuentes.contains(decoy),
            "«{decoy}» no enlaza al README y no puede ser un backlink suyo: {:?}",
            resp[0]
        );
    }

    // La forma de `GraphNode` (`§20.7`): título derivado del H1, sin `type`/`status`.
    let profundo = nodes
        .iter()
        .find(|n| n["id"] == "three/levels/deep/third.md")
        .expect("el nodo del documento profundo está en la respuesta");
    assert_forma_de_nodo(profundo, "Tercero");
    let raiz = nodes
        .iter()
        .find(|n| n["id"] == "README.md")
        .expect("el documento consultado es nodo de su propio subgrafo");
    assert_forma_de_nodo(raiz, "Proyecto");
}

// =============================================================================
// Criterio 2 — `graph_isolated`
// =============================================================================

/// **Dado** un workspace, **Cuando** se pide `graph_query(isolated)`, **Entonces** devuelve los
/// documentos sin enlaces en ningún sentido.
#[test]
fn graph_isolated() {
    let dir = workspace_profundo();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"graph_query","arguments":{"operation":"isolated"}}}"#,
        ],
        1,
    );

    let nodes = nodos(&resp[0]);
    assert_eq!(
        ids(&nodes),
        BTreeSet::from(["solo-externo.md".to_string(), "suelto.md".to_string()]),
        "aislado = sin enlaces INTERNOS entrantes ni salientes. `solo-externo.md` solo tiene una \
         URI externa y un anchor propio, que no conectan con ningún documento: {:?}",
        resp[0]
    );

    // No vacuo: los documentos del grafo profundo participan en él (emisores, receptores o ambos),
    // a cualquier profundidad — incluida la nota, cuyo único enlace es DE REFERENCIA (hoy invisible
    // para la regex heredada, que la dejaría aislada por error).
    for conectado in [
        "README.md",
        "one/first.md",
        "two/levels/second.md",
        "three/levels/deep/third.md",
        "docs/nota.md",
    ] {
        assert!(
            !ids(&nodes).contains(conectado),
            "«{conectado}» tiene enlaces internos y no está aislado: {:?}",
            resp[0]
        );
    }

    // Un aislado NO tiene aristas que mostrar, por definición.
    assert_eq!(
        sc(&resp[0])["edges"].as_array().map(Vec::len),
        Some(0),
        "`isolated` no tiene aristas: {:?}",
        resp[0]
    );

    // Y sus nodos llevan la forma nueva.
    let suelto = nodes
        .iter()
        .find(|n| n["id"] == "suelto.md")
        .expect("«suelto.md» está aislado");
    assert_forma_de_nodo(suelto, "Suelto");
}

// =============================================================================
// Criterio 3 — `knowledge_get_enlaces`
// =============================================================================

/// **Dado** un documento cualquiera, **Cuando** se pide `knowledge_get` con `outgoingLinks` y
/// `backlinks`, **Entonces** ambos reflejan el grafo universal, con hrefs crudos.
#[test]
fn knowledge_get_enlaces() {
    let dir = workspace_profundo();
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"three/levels/deep/third.md"},"include":["outgoingLinks","backlinks"]}}}"#,
        ],
        1,
    );
    let doc = &sc(&resp[0])["document"];

    // --- outgoingLinks: enlaces RESUELTOS, no paths ------------------------------------
    let salientes = doc["outgoingLinks"]
        .as_array()
        .unwrap_or_else(|| panic!("`outgoingLinks` debe ser un array de enlaces resueltos: {doc}"));
    assert_eq!(
        salientes.len(),
        1,
        "el documento profundo enlaza una vez: {doc}"
    );
    let l = &salientes[0];
    assert_eq!(
        l["href"].as_str(),
        Some("../../../README.md"),
        "el href viaja CRUDO, tal como se escribió: es lo que un agente tiene que reescribir. \
         `outgoingLinks` ya NO es la lista de paths resueltos de `Analysis::out`: {doc}"
    );
    assert_eq!(
        l["target"],
        serde_json::json!({ "kind": "document", "value": "README.md" }),
        "…junto al destino ya clasificado y normalizado (`LinkTarget`, §20.6): {doc}"
    );
    assert_eq!(
        l["text"].as_str(),
        Some("visión general"),
        "el texto visible del enlace viaja aparte del destino: {doc}"
    );
    assert_eq!(
        l["kind"].as_str(),
        Some("inline"),
        "la forma sintáctica: {doc}"
    );
    assert!(
        l["fragment"].is_null(),
        "este enlace no tiene fragmento (`None`, no cadena vacía): {doc}"
    );
    let (inicio, fin) = (
        l["span"]["start"].as_u64().unwrap_or_else(|| {
            panic!("el enlace lleva el `span` de bytes del destino dentro del cuerpo: {doc}")
        }),
        l["span"]["end"].as_u64().unwrap_or_default(),
    );
    assert_eq!(
        fin - inicio,
        "../../../README.md".len() as u64,
        "el `span` acota exactamente el destino (lo necesitan `move_document` y el `range` de los \
         diagnósticos): {doc}"
    );

    // --- backlinks: la inversa, también con href crudo ---------------------------------
    let entrantes = doc["backlinks"]["inbound"].as_array().unwrap_or_else(|| {
        panic!("`backlinks.inbound` debe ser un array de referencias entrantes: {doc}")
    });
    assert_eq!(
        entrantes.len(),
        1,
        "al documento profundo lo enlaza solo el README: {doc}"
    );
    assert_eq!(
        entrantes[0]["from"].as_str(),
        Some("README.md"),
        "`LinkReference.from` es el documento que escribe el enlace: {doc}"
    );
    assert_eq!(
        entrantes[0]["link"]["href"].as_str(),
        Some("three/levels/deep/third.md"),
        "…y `link` es el enlace resuelto completo, con SU href crudo (el que el README escribió, \
         no el del documento consultado): {doc}"
    );

    // El grafo es global: el README, a tres niveles de distancia, sí ve el enlace de vuelta.
    let resp = roundtrip(
        dir.path(),
        &[
            r#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"knowledge_get","arguments":{"ref":{"path":"README.md"},"include":["outgoingLinks","backlinks"]}}}"#,
        ],
        1,
    );
    let doc = &sc(&resp[0])["document"];
    let hrefs: Vec<String> = doc["outgoingLinks"]
        .as_array()
        .unwrap_or_else(|| panic!("`outgoingLinks` del README: {doc}"))
        .iter()
        .map(|l| l["href"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        hrefs,
        ["one/first.md", "three/levels/deep/third.md"],
        "los salientes van en ORDEN DE APARICIÓN en el cuerpo, con sus hrefs crudos: {doc}"
    );
    let origenes: BTreeSet<String> = doc["backlinks"]["inbound"]
        .as_array()
        .unwrap_or_else(|| panic!("`backlinks.inbound` del README: {doc}"))
        .iter()
        .map(|r| r["from"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        origenes,
        BTreeSet::from([
            "docs/nota.md".to_string(),
            "three/levels/deep/third.md".to_string(),
        ]),
        "los entrantes del README vienen de tres niveles abajo (inline) y de la nota (de \
         referencia): {doc}"
    );
    let de_referencia = doc["backlinks"]["inbound"]
        .as_array()
        .unwrap()
        .iter()
        .find(|r| r["from"] == "docs/nota.md")
        .expect("la nota enlaza al README");
    assert_eq!(
        de_referencia["link"]["href"].as_str(),
        Some("../README.md"),
        "el href crudo de un enlace de referencia es el destino de SU DEFINICIÓN (`[r]: \
         ../README.md`), que es el byte que hay que reescribir al mover el documento: {doc}"
    );
    assert_eq!(
        de_referencia["link"]["kind"].as_str(),
        Some("reference"),
        "…y su forma sintáctica se conserva: {doc}"
    );
}
