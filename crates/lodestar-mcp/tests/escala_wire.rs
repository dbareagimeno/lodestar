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
