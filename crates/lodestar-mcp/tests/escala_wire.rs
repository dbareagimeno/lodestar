//! **E24-H16** — Escala **por el wire**: los ~10k documentos, por JSON-RPC.
//!
//! `crates/lodestar-app/tests/escala.rs` mide 10.000 documentos llamando a `App` **en proceso**. Es
//! deliberado (lo documenta su cabecera), pero deja un hueco: el coste de serialización del wire y
//! el tamaño real del payload que recibe un agente **nunca se han medido**. Y el payload es
//! justamente lo que el producto promete acotar — «busca primero y recupera después solo el
//! documento, la sección o los campos necesarios; no vuelca todo el repositorio en el contexto».
//!
//! Este test cierra ese hueco arrancando el binario real. No fija umbrales de latencia (dependen de
//! la máquina y un umbral frágil se acaba subiendo hasta que no significa nada): fija el
//! **invariante de payload**, que sí es del producto — ningún cuerpo completo viaja en una
//! respuesta de búsqueda.

use std::io::{BufRead, BufReader, Write};
use std::process::{Command, Stdio};

/// Documentos del workspace sintético. Mismo orden de magnitud que `app/tests/escala.rs`.
const N: usize = 10_000;

/// Centinela que va al **final** del cuerpo de cada documento y no debe viajar jamás en una
/// respuesta de `knowledge_search`. Mismo mecanismo (y misma colocación) que el arnés en proceso de
/// `app/tests/escala.rs`: al final, porque el `snippet` **sí** es una ventana acotada del principio
/// del cuerpo y debe seguir viajando — lo que no puede viajar es el cuerpo ENTERO.
const CENTINELA: &str = "CENTINELA-CUERPO-QUE-NO-DEBE-VIAJAR";
const INITIALIZE_ID: &str = "__lodestar_legacy_harness_initialize__";

fn inicializa(stdin: &mut impl Write, stdout: &mut impl BufRead) {
    let init = serde_json::json!({"jsonrpc":"2.0","id":INITIALIZE_ID,"method":"initialize",
        "params":{"protocolVersion":"2025-11-25","capabilities":{},
                  "clientInfo":{"name":"lodestar-tests","version":"1"}}});
    writeln!(stdin, "{init}").unwrap();
    stdin.flush().unwrap();
    let mut linea = String::new();
    stdout.read_line(&mut linea).unwrap();
    let respuesta: serde_json::Value = serde_json::from_str(&linea).expect("initialize JSON-RPC");
    assert_eq!(
        respuesta["id"], INITIALIZE_ID,
        "respuesta de initialize desalineada"
    );
    let notification = serde_json::json!({"jsonrpc":"2.0","method":"notifications/initialized"});
    writeln!(stdin, "{notification}").unwrap();
    stdin.flush().unwrap();
}

fn genera(root: &std::path::Path) {
    let relleno = "lorem ipsum dolor sit amet ".repeat(40);
    for i in 0..N {
        let dir = root.join(format!("d{:02}", i % 50));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("n{i:05}.md")),
            format!(
                "---\nstatus: {}\nidx: {i}\n---\n\n# Nota {i}\n\n{relleno}\n\n{CENTINELA}-{i}\n",
                if i % 3 == 0 { "draft" } else { "accepted" }
            ),
        )
        .unwrap();
    }
}

/// **E24-H16** — a escala, `knowledge_search` sigue sin volcar cuerpos por el wire.
#[test]
fn escala_por_el_wire_acota_payload() {
    let dir = tempfile::tempdir().unwrap();
    genera(dir.path());

    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("arrancar lodestar-mcp");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    inicializa(&mut stdin, &mut stdout);

    let mut pide = |id: u64, nombre: &str, args: serde_json::Value| -> (serde_json::Value, usize) {
        let msg = serde_json::json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
            "params":{"name":nombre,"arguments":args}});
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut linea = String::new();
        stdout.read_line(&mut linea).unwrap();
        let bytes = linea.len();
        let v: serde_json::Value = serde_json::from_str(&linea).expect("JSON-RPC puro");
        (v, bytes)
    };

    // Orientación: el workspace tiene los N documentos.
    let (estado, _) = pide(1, "workspace_status", serde_json::json!({}));
    assert_eq!(
        estado["result"]["structuredContent"]["counts"]["documents"]
            .as_u64()
            .unwrap_or(0) as usize,
        N,
        "precondición: los {N} documentos deben estar en el inventario"
    );

    // Búsqueda amplia con proyección de frontmatter, que es el caso caro.
    let inicio = std::time::Instant::now();
    let (resp, bytes) = pide(
        2,
        "knowledge_search",
        serde_json::json!({"text": "", "where": "status = \"draft\"",
                           "include": ["frontmatter.status"], "limit": 100}),
    );
    let transcurrido = inicio.elapsed();

    let linea_cruda = serde_json::to_string(&resp).unwrap();
    assert!(
        !linea_cruda.contains(CENTINELA),
        "ningún CUERPO puede viajar en una respuesta de búsqueda, tampoco a escala: el producto \
         promete no volcar el repositorio en el contexto del agente. Payload: {bytes} bytes"
    );

    let resultados = resp["result"]["structuredContent"]["results"]
        .as_array()
        .expect("results");
    assert_eq!(
        resultados.len(),
        100,
        "el `limit` debe acotar la página incluso con miles de coincidencias"
    );
    assert!(
        resultados
            .iter()
            .all(|r| r["frontmatter"]["status"] == "draft"),
        "la proyección debe traer el campo pedido, y el filtro debe haber filtrado de verdad"
    );

    // El payload de una página acotada tiene que ser una fracción minúscula del corpus. No es un
    // umbral de rendimiento (frágil): es el invariante de que la paginación acota el wire.
    let corpus: usize = N * (CENTINELA.len() + 27 * 40);
    assert!(
        bytes < corpus / 100,
        "una página de 100 resultados sobre {N} documentos ocupó {bytes} bytes, más del 1% del \
         corpus (~{corpus}): la paginación no está acotando el payload"
    );

    eprintln!(
        "E24-H16: knowledge_search sobre {N} documentos por el wire → {bytes} bytes en {transcurrido:?}"
    );

    drop(stdin);
    let _ = child.wait();
}

// ---------------------------------------------------------------------------
// E26-H10 — `graph_query` tampoco puede servir una respuesta del tamaño del workspace
//
// El invariante de payload que fija el test de arriba lo cumplía `knowledge_search` porque tiene
// cota (20/100) desde E10. `graph_query` NO tenía ni default ni máximo: `limit` era opcional y
// `None => total`, así que un `operation:"components"` —que sirve el `graph_model` ENTERO— volcaba
// el grafo completo del repositorio en una sola respuesta. Este es el caso grande de esa historia,
// aquí y no en `mcp.rs` porque el arnés de proceso real vive en este fichero (E24-H16).
//
// CONSECUENCIA DECLARADA por la historia (material de nota de release): `graph_query` sin `limit`
// deja de devolver el grafo completo; devuelve 100 nodos y un `nextCursor`.
// ---------------------------------------------------------------------------

/// Documentos del workspace de grafo: ~1.000, el orden de magnitud que nombra el criterio.
const N_GRAFO: usize = 1_000;

/// Genera `N_GRAFO` notas encadenadas en ciclo (`g0000 → g0001 → … → g0000`), todas en la raíz para
/// que los enlaces relativos resuelvan sin ambigüedad. Ningún enlace queda colgante, así que el
/// grafo tiene exactamente `N_GRAFO` nodos y ningún nodo fantasma: el número contra el que se
/// compara la página es exacto, no aproximado.
fn genera_grafo(root: &std::path::Path) {
    for i in 0..N_GRAFO {
        let siguiente = format!("g{:04}.md", (i + 1) % N_GRAFO);
        std::fs::write(
            root.join(format!("g{i:04}.md")),
            format!(
                "---\nidx: {i}\nstatus: draft\n---\n\n# Nota {i}\n\n[siguiente]({siguiente})\n"
            ),
        )
        .unwrap();
    }
}

/// **E26-H10** · Criterio `graph_query_tiene_default`:
/// **Dado** un workspace de ~1.000 documentos, **Cuando** se llama a
/// `graph_query{operation:"components"}` **sin** `limit`, **Entonces** devuelve como mucho 100
/// nodos, `truncated: true` y un `nextCursor`.
///
/// Hoy devuelve el grafo entero (los 1.000 nodos y sus 1.000 aristas) sin truncar y sin cursor.
///
/// El control anti-vacuo es la segunda llamada: con el `limit` **máximo** (1000) el grafo completo
/// sigue siendo alcanzable en una respuesta, y la página por defecto es su **prefijo** (orden total
/// estable por `id`) — la cota no puede consistir en tirar datos ni en barajar el orden.
#[test]
fn graph_query_tiene_default() {
    let dir = tempfile::tempdir().unwrap();
    genera_grafo(dir.path());

    let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--root")
        .arg(dir.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("arrancar lodestar-mcp");
    let mut stdin = child.stdin.take().unwrap();
    let mut stdout = BufReader::new(child.stdout.take().unwrap());
    inicializa(&mut stdin, &mut stdout);

    let mut pide = |id: u64, nombre: &str, args: serde_json::Value| -> (serde_json::Value, usize) {
        let msg = serde_json::json!({"jsonrpc":"2.0","id":id,"method":"tools/call",
            "params":{"name":nombre,"arguments":args}});
        writeln!(stdin, "{msg}").unwrap();
        stdin.flush().unwrap();
        let mut linea = String::new();
        stdout.read_line(&mut linea).unwrap();
        let bytes = linea.len();
        let v: serde_json::Value = serde_json::from_str(&linea).expect("JSON-RPC puro");
        (v, bytes)
    };

    // Precondición: el workspace tiene los ~1.000 documentos.
    let (estado, _) = pide(1, "workspace_status", serde_json::json!({}));
    assert_eq!(
        estado["result"]["structuredContent"]["counts"]["documents"]
            .as_u64()
            .unwrap_or(0) as usize,
        N_GRAFO,
        "precondición: los {N_GRAFO} documentos deben estar en el inventario"
    );

    // (1) Sin `limit`: la cota por defecto.
    let (resp, bytes) = pide(
        2,
        "graph_query",
        serde_json::json!({"operation": "components"}),
    );
    let sc = &resp["result"]["structuredContent"];
    assert!(
        !resp["result"]["isError"].as_bool().unwrap_or(false),
        "`graph_query{{components}}` sin limit no puede fallar: {resp}"
    );
    let nodos = sc["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("graph_query devuelve `nodes`: {resp}"));
    // El default se clava EXACTO, no como cota superior: con {N_GRAFO} nodos disponibles la página
    // por defecto se llena entera, así que `== 100` es el valor observable del default. Un `<= 100`
    // dejaría vivo cualquier otro default menor (75, 10, 1), que también acota el payload pero
    // **no** es el número que fijan la historia y el `inputSchema` — y un cliente que lea
    // `default: 100` recibiría otra cosa.
    assert_eq!(
        nodos.len(),
        100,
        "`graph_query{{operation:\"components\"}}` SIN `limit` debe devolver exactamente los 100 \
         nodos del default declarado, de los {N_GRAFO} del grafo. Hasta v0.4.0 `limit` era opcional \
         con `None => total`, así que esta llamada volcaba el grafo COMPLETO ({} nodos, {bytes} \
         bytes) en una sola respuesta: es el defecto U5",
        nodos.len()
    );
    assert_eq!(
        sc["summary"]["truncated"],
        serde_json::Value::Bool(true),
        "…con `summary.truncated: true`, porque quedan {N_GRAFO} - 100 nodos fuera: {}",
        &resp.to_string()[..resp.to_string().len().min(600)]
    );
    let cursor = sc["nextCursor"]
        .as_str()
        .filter(|c| !c.is_empty())
        .unwrap_or_else(|| {
            panic!(
                "…y un `nextCursor` no vacío: una respuesta truncada sin cursor deja al agente sin \
                 forma de recorrer el resto, que es la mitad de la consecuencia declarada. \
                 Respuesta: {}",
                &resp.to_string()[..resp.to_string().len().min(600)]
            )
        })
        .to_string();

    // (2) Control anti-vacuo: con el máximo declarado, el grafo entero sigue siendo alcanzable.
    let (full, bytes_full) = pide(
        3,
        "graph_query",
        serde_json::json!({"operation": "components", "limit": 1000}),
    );
    let sc_full = &full["result"]["structuredContent"];
    let nodos_full = sc_full["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("graph_query devuelve `nodes`: {full}"));
    assert_eq!(
        nodos_full.len(),
        N_GRAFO,
        "con `limit: 1000` (el máximo declarado) los {N_GRAFO} nodos siguen siendo alcanzables en \
         una respuesta: la cota es un DEFAULT, no un recorte del cómputo"
    );
    assert_eq!(
        sc_full["summary"]["truncated"],
        serde_json::Value::Bool(false),
        "…y ahí no hay truncamiento: {}",
        &full.to_string()[..full.to_string().len().min(600)]
    );

    // (3) La página por defecto es el PREFIJO del orden total por `id`.
    assert_eq!(
        nodos,
        &nodos_full[..nodos.len()].to_vec(),
        "la página por defecto debe ser el prefijo del orden total estable por `id`, no una \
         muestra: es lo que hace que el cursor-offset no pierda ni duplique nada"
    );

    // (4) …y la razón de ser de todo esto: el payload deja de ser proporcional al workspace.
    assert!(
        bytes * 5 < bytes_full,
        "una respuesta acotada a 100 nodos ocupó {bytes} bytes frente a los {bytes_full} del grafo \
         completo ({N_GRAFO} nodos): la cota no está acotando el payload"
    );
    eprintln!(
        "E26-H10: graph_query{{components}} sobre {N_GRAFO} documentos → {bytes} bytes acotados \
         (grafo completo: {bytes_full} bytes), cursor «{cursor}»"
    );

    // (5) Y el cursor reanuda: la segunda página completa el recorrido sin huecos ni repeticiones.
    let (p2, _) = pide(
        4,
        "graph_query",
        serde_json::json!({"operation": "components", "cursor": cursor}),
    );
    let nodos_p2 = p2["result"]["structuredContent"]["nodes"]
        .as_array()
        .unwrap_or_else(|| panic!("graph_query devuelve `nodes`: {p2}"))
        .clone();
    assert_eq!(
        nodos_p2,
        nodos_full[nodos.len()..nodos.len() + nodos_p2.len()].to_vec(),
        "la página reanudada por el cursor debe continuar EXACTAMENTE donde acabó la anterior"
    );

    drop(stdin);
    let _ = child.wait();
}
