//! **E23-H07** — e2e del ciclo de vida de una base de conocimiento en **UNA SOLA sesión MCP**.
//!
//! # Por qué existe este fichero
//!
//! `e2e_migracion.rs` dice «todo en la misma sesión», pero **no lo es**: cada paso levanta un
//! proceso `lodestar-mcp` nuevo, así que toda llamada arranca con estado **frío**. Eso enmascara la
//! clase entera de bugs de invalidación — el estado que un servidor vivo arrastra entre `tools/call`
//! —, que es justo la clase de bug que E23 ha estado cazando.
//!
//! Aquí el patrón es el real de un agente: **una única invocación del binario** y N líneas JSON-RPC
//! por el mismo `stdin`, leyendo cada respuesta **antes** de mandar la siguiente (encadenar
//! `change_plan` → `change_apply` obliga a leer el `changeSetId` antes de mandar el apply). Esa
//! lectura/escritura intercalada sobre el mismo proceso es la pieza técnica de la historia y vive en
//! [`Sesion`].
//!
//! # Qué se verifica entre pasos
//!
//! - El **disco** (contenido real de los `.md`, byte a byte donde toca).
//! - El **`workspaceRevision`**: cambia en cada apply, y dos lecturas sin escritura entre medias dan
//!   exactamente el mismo valor.
//! - Que las **lecturas posteriores** (`knowledge_get`, `knowledge_search`, `graph_query`,
//!   `knowledge_check`) ven el estado nuevo **dentro de la misma sesión**.
//!
//! # Semántica FIJADA de la edición externa (criterio `edicion_externa_en_sesion_viva`)
//!
//! El servidor MCP abre el workspace con `App::open` y **`Workspace::document_set()` redescubre y
//! reparsea el árbol desde disco en CADA llamada** (no hay snapshot cacheado en el proceso). Por
//! tanto la semántica esperada, y aseverada explícitamente abajo, es: **una edición externa hecha
//! con `std::fs::write` entre dos `tools/call` de la misma sesión SÍ la ve la lectura siguiente**,
//! sin reiniciar nada. Corolario aseverado también: un plan calculado antes de la edición externa
//! queda **caduco** (`PLAN_STALE`) y no escribe.

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

// ===========================================================================
// Arnés: una sesión MCP viva, con escritura y lectura INTERCALADAS.
// ===========================================================================

/// Un proceso `lodestar-mcp` **vivo** contra el que se dialoga petición a petición.
///
/// A diferencia del arnés `mcp()` de `e2e_migracion.rs` (que escribe todas las líneas y luego lee),
/// [`Sesion::peticion`] escribe **una** línea, la vacía y lee **una** respuesta antes de devolver el
/// control. Eso es lo que permite encadenar `change_plan` → `change_apply` (el `changeSetId` sale de
/// la respuesta anterior) y, sobre todo, mantener el mismo proceso —con su estado— durante todo el
/// ciclo de vida.
struct Sesion {
    hijo: Child,
    entrada: ChildStdin,
    salida: BufReader<ChildStdout>,
    siguiente_id: u32,
}

impl Sesion {
    /// Arranca `lodestar-mcp --root <root>` y completa el `initialize`.
    fn abrir(root: &Path) -> Sesion {
        let mut hijo = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
            .arg("--root")
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("arrancar el binario lodestar-mcp");
        let entrada = hijo.stdin.take().expect("stdin del servidor");
        let salida = BufReader::new(hijo.stdout.take().expect("stdout del servidor"));
        let mut sesion = Sesion {
            hijo,
            entrada,
            salida,
            siguiente_id: 1,
        };
        let hola = sesion.peticion("initialize", json!({"protocolVersion":"2025-06-18"}));
        assert_eq!(
            hola["result"]["serverInfo"]["name"], "lodestar-mcp",
            "el handshake debe identificar al servidor: {hola}"
        );
        sesion
    }

    /// Manda **una** línea JSON-RPC y lee **una** respuesta del mismo proceso.
    ///
    /// Asevera la correlación `id` petición↔respuesta: en un arnés intercalado, un desfase de una
    /// línea haría que todos los tests siguientes leyeran la respuesta del paso anterior y pasaran
    /// por accidente. Esta aserción es la que impide ese modo de fallo silencioso.
    fn peticion(&mut self, metodo: &str, params: Value) -> Value {
        let id = self.siguiente_id;
        self.siguiente_id += 1;
        let linea = json!({"jsonrpc":"2.0","id":id,"method":metodo,"params":params}).to_string();
        writeln!(self.entrada, "{linea}").expect("escribir en el stdin del servidor");
        self.entrada.flush().expect("vaciar el stdin del servidor");

        let mut buf = String::new();
        let leidos = self
            .salida
            .read_line(&mut buf)
            .expect("leer una línea del stdout del servidor");
        assert!(
            leidos > 0,
            "el servidor cerró stdout sin responder a «{metodo}» (id {id}): la sesión murió a \
             mitad del ciclo"
        );
        let resp: Value = serde_json::from_str(buf.trim_end())
            .expect("stdout = JSON-RPC puro, una línea por respuesta");
        assert_eq!(
            resp["id"],
            json!(id),
            "respuesta desalineada con su petición (esperado id {id}): {resp}"
        );
        resp
    }

    /// El `result` crudo de un `tools/call` (incluye `isError` cuando la tool falla).
    fn tool_cruda(&mut self, nombre: &str, args: Value) -> Value {
        let resp = self.peticion("tools/call", json!({"name": nombre, "arguments": args}));
        assert!(
            resp["error"].is_null(),
            "«{nombre}» no debía dar un error de PROTOCOLO: {resp}"
        );
        resp["result"].clone()
    }

    /// El `structuredContent` de un `tools/call` que debe tener éxito.
    fn tool(&mut self, nombre: &str, args: Value) -> Value {
        let r = self.tool_cruda(nombre, args);
        assert!(
            r["isError"] != json!(true),
            "«{nombre}» falló: {}",
            r["content"][0]["text"]
        );
        r["structuredContent"].clone()
    }

    /// El texto de error de un `tools/call` que debe FALLAR (y falla como error de ejecución).
    fn tool_falla(&mut self, nombre: &str, args: Value) -> String {
        let r = self.tool_cruda(nombre, args);
        assert_eq!(
            r["isError"],
            json!(true),
            "«{nombre}» debía fallar y no falló: {r}"
        );
        r["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    }

    /// `workspace_status` completo.
    fn estado(&mut self) -> Value {
        self.tool("workspace_status", json!({}))
    }

    /// El `workspaceRevision` vigente según el servidor.
    fn revision(&mut self) -> String {
        self.estado()["workspaceRevision"]
            .as_str()
            .expect("workspaceRevision es una cadena «blake3:…»")
            .to_string()
    }

    /// `change_plan` con las ops sueltas dadas y la política por defecto.
    fn planifica(&mut self, ops: Value) -> Value {
        self.tool("change_plan", json!({ "operations": ops }))
    }

    /// `change_apply` del plan `id`.
    fn aplica(&mut self, id: &str) -> Value {
        self.tool("change_apply", json!({ "changeSetId": id }))
    }

    /// `change_revert` del recibo `id`.
    fn revierte(&mut self, id: &str) -> Value {
        self.tool("change_revert", json!({ "receiptId": id }))
    }
}

impl Drop for Sesion {
    fn drop(&mut self) {
        let _ = self.hijo.kill();
        let _ = self.hijo.wait();
    }
}

// ===========================================================================
// Utilidades de disco (fuera de la sesión: el test mira el árbol REAL).
// ===========================================================================

/// Escribe un fichero bajo `root`, creando los directorios intermedios.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    std::fs::create_dir_all(ruta.parent().unwrap()).unwrap();
    std::fs::write(ruta, contenido).unwrap();
}

/// Lee un fichero del workspace (falla con el path si no existe).
fn lee(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap_or_else(|e| panic!("leer {rel}: {e}"))
}

/// Instantánea `path → contenido` de **todos** los `.md` bajo `root`, saltando `.lodestar/` (plano
/// de control, no conocimiento). Es el vehículo de las comparaciones byte a byte.
fn instantanea_md(root: &Path) -> BTreeMap<String, String> {
    fn recorre(base: &Path, dir: &Path, out: &mut BTreeMap<String, String>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let ruta = e.path();
            let nombre = e.file_name().to_string_lossy().to_string();
            if nombre == ".lodestar" || nombre == ".git" {
                continue;
            }
            if ruta.is_dir() {
                recorre(base, &ruta, out);
            } else if ruta.extension().is_some_and(|x| x == "md") {
                let rel = ruta
                    .strip_prefix(base)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                out.insert(rel, std::fs::read_to_string(&ruta).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    recorre(root, root, &mut out);
    out
}

/// Los `path` de un `knowledge_search`, ordenados.
fn paths_de_busqueda(sc: &Value) -> Vec<String> {
    let mut v: Vec<String> = sc["results"]
        .as_array()
        .expect("results es un array")
        .iter()
        .map(|r| r["path"].as_str().unwrap().to_string())
        .collect();
    v.sort();
    v
}

/// Los `changedPaths` de un apply/revert, ordenados.
fn paths_cambiados(sc: &Value) -> Vec<String> {
    let mut v: Vec<String> = sc["changedPaths"]
        .as_array()
        .expect("changedPaths es un array")
        .iter()
        .map(|p| p.as_str().unwrap().to_string())
        .collect();
    v.sort();
    v
}

// ===========================================================================
// El proyecto de partida.
// ===========================================================================

/// Proyecto ordinario (sin `.lodestar/`, sin `index.md`, sin frontmatter obligatorio) con un
/// **enlace roto a propósito**: `README.md` apunta a `notas/gamma.md`, que todavía no existe. El
/// primer paso del ciclo (`create`) es el que lo repara, así que el test puede aseverar que la
/// conformidad pasa de 1 error a 0 **dentro de la misma sesión**.
fn proyecto_inicial(root: &Path) {
    escribe(
        root,
        "README.md",
        "# Manual\n\n- [Alfa](notas/alfa.md)\n- [Gamma](notas/gamma.md)\n",
    );
    escribe(
        root,
        "notas/alfa.md",
        "---\nestado: publicado\n---\n\n# Alfa\n\nVer [Beta](beta.md).\n",
    );
    escribe(
        root,
        "notas/beta.md",
        "---\nestado: publicado\n---\n\n# Beta\n\nContenido de beta.\n",
    );
}

/// El cuerpo con el que se crea `gamma`: un enlace **relativo** a una vecina (lo que el `move` tiene
/// que recalcular, E23-H03) y una URI externa (lo que el `move` NO puede tocar).
const CUERPO_GAMMA: &str =
    "# Gamma\n\nRelacionado: [Alfa](alfa.md).\n\nFuente: [rfc](https://example.org/rfc).\n";

// ===========================================================================
// Criterio 1 — `ciclo_vida_una_sesion`
// ===========================================================================

/// **Dado** una sola sesión MCP, **Cuando** se recorre el ciclo completo
/// (`create → apply → patch_frontmatter → apply → move → apply → delete → apply → revert → revert`),
/// **Entonces** cada paso deja el disco en el estado esperado y `workspaceRevision` cambia en cada
/// apply.
///
/// El test es un único `#[test]` **a propósito**: la propiedad bajo prueba es precisamente que los
/// pasos comparten proceso y estado, así que trocearlo la destruiría. Cada fase va rotulada y sus
/// aserciones llevan el porqué.
#[test]
fn ciclo_vida_una_sesion() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_inicial(root);

    let mut s = Sesion::abrir(root);

    // --- Fase 0: estado inicial -------------------------------------------------------------
    let estado0 = s.estado();
    assert_eq!(
        estado0["counts"]["documents"], 3,
        "el proyecto de partida tiene 3 documentos: {estado0}"
    );
    let rev0 = estado0["workspaceRevision"].as_str().unwrap().to_string();
    assert!(
        rev0.starts_with("blake3:"),
        "la revisión es un hash con prefijo: {rev0}"
    );
    // Dos lecturas SIN escritura entre medias dan exactamente la misma revisión (si no, cualquier
    // «cambió la revisión» de las fases siguientes sería ruido, no señal).
    assert_eq!(
        s.revision(),
        rev0,
        "sin escrituras entre medias, dos lecturas deben dar la MISMA workspaceRevision"
    );

    let check0 = s.tool("knowledge_check", json!({"scope":{"kind":"workspace"}}));
    assert_eq!(
        check0["summary"]["errors"], 1,
        "el enlace roto de README a notas/gamma.md es 1 error de partida: {check0}"
    );

    // --- Fase 1: create + apply -------------------------------------------------------------
    let plan1 = s.planifica(json!([{
        "op": "create",
        "path": "notas/gamma.md",
        "frontmatter": {"estado": "borrador", "prioridad": 3, "tags": ["idea", "grafo"]},
        "body": CUERPO_GAMMA
    }]));
    assert_eq!(
        plan1["canApply"], true,
        "crear el documento que faltaba debe ser aplicable: {plan1}"
    );
    assert_eq!(
        plan1["diagnosticsBefore"]["errors"], 1,
        "el plan ve el error preexistente: {plan1}"
    );
    assert_eq!(
        plan1["diagnosticsAfter"]["errors"], 0,
        "y ve que crearlo lo repara: {plan1}"
    );
    let cs1 = plan1["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();

    let apply1 = s.aplica(&cs1);
    assert_eq!(apply1["applied"], true, "el apply del create: {apply1}");
    assert_eq!(
        apply1["previousWorkspaceRevision"],
        json!(rev0),
        "el apply parte de la revisión que la sesión venía viendo: {apply1}"
    );
    let rev1 = apply1["workspaceRevision"].as_str().unwrap().to_string();
    assert_ne!(rev1, rev0, "un apply que escribe DEBE cambiar la revisión");
    assert_eq!(
        paths_cambiados(&apply1),
        vec!["notas/gamma.md"],
        "el create toca exactamente un path: {apply1}"
    );
    // La sesión viva ya reporta la revisión nueva (no la que cargó al arrancar).
    assert_eq!(
        s.revision(),
        rev1,
        "workspace_status posterior al apply debe reportar la revisión resultante, no la vieja"
    );

    // Disco: EXACTAMENTE lo que se pidió. Sin `type:`, sin `title:`, sin bloque inventado (E23-H02).
    let gamma_creado = lee(root, "notas/gamma.md");
    assert_eq!(
        gamma_creado,
        format!(
            "---\nestado: borrador\nprioridad: 3\ntags:\n- idea\n- grafo\n---\n\n{CUERPO_GAMMA}"
        ),
        "el .md creado debe ser exactamente el frontmatter pedido + el cuerpo pedido"
    );
    assert!(
        !gamma_creado.contains("type:") && !gamma_creado.contains("title:"),
        "crear no puede inyectar claves OKF que nadie pidió:\n{gamma_creado}"
    );

    // Lecturas dentro de la MISMA sesión: ven el documento nuevo.
    let doc1 = s.tool(
        "knowledge_get",
        json!({"ref":{"path":"notas/gamma.md"},
               "include":["frontmatter","body","outgoingLinks","backlinks"]}),
    )["document"]
        .clone();
    assert_eq!(
        doc1["frontmatter"]["tags"],
        json!(["idea", "grafo"]),
        "el frontmatter arbitrario llega con su tipo YAML real (lista): {doc1}"
    );
    assert_eq!(
        doc1["frontmatter"]["prioridad"],
        json!(3),
        "número, no texto"
    );
    // El cuerpo servido es el escrito, precedido de la línea en blanco que separa el bloque de
    // frontmatter del cuerpo: `build_raw` emite `---\n…\n---\n\n{cuerpo}` y `parse_file` corta
    // justo tras el `---\n`, así que ese `\n` pertenece al cuerpo (quirk portado 1:1 de
    // `splitFront`). Se fija aquí para que un cambio en el corte no pase inadvertido.
    assert_eq!(
        doc1["body"].as_str().unwrap(),
        format!("\n{CUERPO_GAMMA}"),
        "el cuerpo servido es el escrito (con la línea en blanco separadora del frontmatter)"
    );
    let clases: Vec<(String, String)> = doc1["outgoingLinks"]
        .as_array()
        .unwrap()
        .iter()
        .map(|l| {
            (
                l["href"].as_str().unwrap().to_string(),
                l["target"]["kind"].as_str().unwrap().to_string(),
            )
        })
        .collect();
    assert!(
        clases.contains(&("alfa.md".to_string(), "document".to_string())),
        "el enlace relativo a la vecina resuelve a documento: {clases:?}"
    );
    assert!(
        clases
            .iter()
            .any(|(h, k)| h == "https://example.org/rfc" && k == "externalUri"),
        "la URI externa se clasifica como externalUri: {clases:?}"
    );
    assert_eq!(
        doc1["backlinks"]["inbound"]
            .as_array()
            .unwrap()
            .iter()
            .map(|b| b["from"].as_str().unwrap())
            .collect::<Vec<_>>(),
        vec!["README.md"],
        "README ya no cuelga: su enlace resuelve al documento recién creado: {doc1}"
    );

    let busca1 = s.tool("knowledge_search", json!({"text":"Gamma"}));
    assert!(
        paths_de_busqueda(&busca1).contains(&"notas/gamma.md".to_string()),
        "knowledge_search encuentra el documento creado en esta misma sesión: {busca1}"
    );
    let check1 = s.tool("knowledge_check", json!({"scope":{"kind":"workspace"}}));
    assert_eq!(
        check1["summary"]["errors"], 0,
        "tras el create la conformidad pasa a 0 errores EN LA MISMA SESIÓN: {check1}"
    );
    assert_eq!(
        s.estado()["counts"]["documents"],
        4,
        "el recuento sube a 4 sin reiniciar el servidor"
    );

    // --- Fase 2: patch_frontmatter + apply --------------------------------------------------
    let plan2 = s.planifica(json!([{
        "op": "patch_frontmatter",
        "path": "notas/gamma.md",
        "patch": {"estado": "revisado", "prioridad": 1}
    }]));
    assert_eq!(plan2["canApply"], true, "el patch es aplicable: {plan2}");
    let cs2 = plan2["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();
    assert_ne!(cs2, cs1, "un plan distinto sobre otra base tiene otro id");

    let apply2 = s.aplica(&cs2);
    let rev2 = apply2["workspaceRevision"].as_str().unwrap().to_string();
    assert_eq!(
        apply2["previousWorkspaceRevision"],
        json!(rev1),
        "el segundo apply encadena con la revisión del primero: {apply2}"
    );
    assert_ne!(rev2, rev1, "el patch cambia la revisión");

    let gamma_parcheado = lee(root, "notas/gamma.md");
    assert_eq!(
        gamma_parcheado,
        format!(
            "---\nestado: revisado\nprioridad: 1\ntags:\n- idea\n- grafo\n---\n\n{CUERPO_GAMMA}"
        ),
        "el patch toca SOLO las dos claves pedidas y respeta el orden y el resto del bloque"
    );

    // La consulta tipada ve el valor nuevo (y solo él) en la misma sesión.
    let busca2 = s.tool("knowledge_search", json!({"where":"estado = \"revisado\""}));
    assert_eq!(
        paths_de_busqueda(&busca2),
        vec!["notas/gamma.md"],
        "`where estado = \"revisado\"` debe devolver justo el documento recién parcheado: {busca2}"
    );
    let busca2_viejo = s.tool("knowledge_search", json!({"where":"estado = \"borrador\""}));
    assert!(
        paths_de_busqueda(&busca2_viejo).is_empty(),
        "y NINGUNO debe seguir en «borrador» (control anti-vacuo de la invalidación): {busca2_viejo}"
    );

    // --- Fase 3: move + apply ---------------------------------------------------------------
    let plan3 = s.planifica(json!([{
        "op": "move",
        "from": "notas/gamma.md",
        "to": "archivo/gamma.md",
        "rewriteInboundLinks": true
    }]));
    assert_eq!(
        plan3["canApply"], true,
        "mover una nota que enlaza a sus vecinas debe ser aplicable (E23-H03): {plan3}"
    );
    let ops3: Vec<String> = plan3["normalizedOperations"]
        .as_array()
        .unwrap()
        .iter()
        .map(|o| {
            o.as_object()
                .and_then(|m| m.keys().next().cloned())
                .unwrap_or_default()
        })
        .collect();
    assert_eq!(
        ops3.len(),
        3,
        "el move normaliza a 3 ops (mover + recalcular los salientes propios + reescribir el \
         entrante de README): {plan3}"
    );
    let cs3 = plan3["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();

    let apply3 = s.aplica(&cs3);
    let rev3 = apply3["workspaceRevision"].as_str().unwrap().to_string();
    assert_ne!(rev3, rev2, "el move cambia la revisión");
    assert_eq!(
        paths_cambiados(&apply3),
        vec!["README.md", "archivo/gamma.md", "notas/gamma.md"],
        "el move toca el origen, el destino y el emisor entrante, en UNA transacción: {apply3}"
    );

    assert!(
        !root.join("notas/gamma.md").exists(),
        "el origen del move desaparece del disco"
    );
    let gamma_movido = lee(root, "archivo/gamma.md");
    assert_eq!(
        gamma_movido,
        "---\nestado: revisado\nprioridad: 1\ntags:\n- idea\n- grafo\n---\n\n\
         # Gamma\n\nRelacionado: [Alfa](../notas/alfa.md).\n\n\
         Fuente: [rfc](https://example.org/rfc).\n",
        "mover recalcula el saliente relativo desde la ubicación nueva, deja la URI externa \
         intacta y conserva el frontmatter"
    );
    let readme_movido = lee(root, "README.md");
    assert_eq!(
        readme_movido, "# Manual\n\n- [Alfa](notas/alfa.md)\n- [Gamma](archivo/gamma.md)\n",
        "el entrante se reescribe al destino y el README (SIN frontmatter) no gana un bloque"
    );

    // Invalidación por la superficie: el path viejo ya no se puede leer, el nuevo sí, en la misma
    // sesión y sin reiniciar.
    let err_viejo = s.tool_falla("knowledge_get", json!({"ref":{"path":"notas/gamma.md"}}));
    assert!(
        err_viejo.contains("DOCUMENT_NOT_FOUND"),
        "leer el path de origen tras el move debe dar DOCUMENT_NOT_FOUND: {err_viejo}"
    );
    let doc3 = s.tool(
        "knowledge_get",
        json!({"ref":{"path":"archivo/gamma.md"},"include":["frontmatter"]}),
    )["document"]
        .clone();
    assert_eq!(
        doc3["frontmatter"]["estado"], "revisado",
        "el documento movido conserva su frontmatter: {doc3}"
    );
    let grafo3 = s.tool(
        "graph_query",
        json!({"operation":"backlinks","ref":{"path":"archivo/gamma.md"}}),
    );
    assert!(
        grafo3["edges"]
            .as_array()
            .unwrap()
            .iter()
            .any(|e| e["source"] == "README.md" && e["target"] == "archivo/gamma.md"),
        "el grafo de la sesión viva ya conoce el destino del move: {grafo3}"
    );
    let check3 = s.tool("knowledge_check", json!({"scope":{"kind":"workspace"}}));
    assert_eq!(
        check3["summary"]["errors"], 0,
        "el move deja el workspace sin errores: {check3}"
    );

    // Estado justo antes del borrado (referencia byte a byte para el revert).
    let antes_del_borrado = instantanea_md(root);

    // --- Fase 4: delete + apply -------------------------------------------------------------
    let plan4 = s.planifica(json!([{
        "op": "delete",
        "ref": {"path": "archivo/gamma.md"},
        "inboundLinksPolicy": "remove_links"
    }]));
    assert_eq!(
        plan4["canApply"], true,
        "borrar declarando `remove_links` debe ser aplicable: {plan4}"
    );
    let cs4 = plan4["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();

    let apply4 = s.aplica(&cs4);
    let rev4 = apply4["workspaceRevision"].as_str().unwrap().to_string();
    assert_ne!(rev4, rev3, "el delete cambia la revisión");
    let recibo4 = apply4["receiptId"].as_str().expect("receiptId").to_string();
    assert_eq!(
        paths_cambiados(&apply4),
        vec!["README.md", "archivo/gamma.md"],
        "el delete borra el documento y desenlaza a su emisor: {apply4}"
    );

    assert!(
        !root.join("archivo/gamma.md").exists(),
        "el documento borrado desaparece del disco"
    );
    assert_eq!(
        lee(root, "README.md"),
        "# Manual\n\n- [Alfa](notas/alfa.md)\n- Gamma\n",
        "`remove_links` quita el enlace y conserva su TEXTO"
    );
    assert_eq!(
        s.estado()["counts"]["documents"],
        3,
        "el recuento vuelve a 3 en la sesión viva"
    );
    let check4 = s.tool("knowledge_check", json!({"scope":{"kind":"workspace"}}));
    assert_eq!(
        check4["summary"]["errors"], 0,
        "el borrado no deja enlaces rotos: {check4}"
    );

    // --- Fase 5: revert del delete ----------------------------------------------------------
    let revert1 = s.revierte(&recibo4);
    assert_eq!(
        revert1["reverted"], true,
        "el delete es reversible: {revert1}"
    );
    assert_eq!(
        revert1["workspaceRevision"],
        json!(rev3),
        "revertir el delete devuelve el workspace a la revisión previa al delete: {revert1}"
    );
    assert_eq!(
        instantanea_md(root),
        antes_del_borrado,
        "revertir el delete restaura el documento BYTE A BYTE y devuelve su enlace al emisor"
    );
    // Y la sesión viva lo vuelve a servir sin reiniciar.
    let doc5 = s.tool(
        "knowledge_get",
        json!({"ref":{"path":"archivo/gamma.md"},"include":["body"]}),
    )["document"]
        .clone();
    assert!(
        doc5["body"]
            .as_str()
            .unwrap()
            .contains("[Alfa](../notas/alfa.md)"),
        "tras el revert el documento restaurado se lee entero: {doc5}"
    );

    // --- Fase 6: revert del move (segundo revert encadenado) --------------------------------
    let recibo3 = apply3["receiptId"].as_str().expect("receiptId").to_string();
    let revert2 = s.revierte(&recibo3);
    assert_eq!(
        revert2["reverted"], true,
        "encadenar un segundo revert sobre el apply anterior debe funcionar: {revert2}"
    );
    assert_eq!(
        revert2["workspaceRevision"],
        json!(rev2),
        "revertir el move devuelve el workspace a la revisión posterior al patch: {revert2}"
    );
    assert!(
        !root.join("archivo/gamma.md").exists(),
        "revertir el move borra el destino que el move había creado"
    );
    assert_eq!(
        lee(root, "notas/gamma.md"),
        gamma_parcheado,
        "revertir el move restaura el documento en su ubicación original, byte a byte"
    );
    assert_eq!(
        lee(root, "README.md"),
        "# Manual\n\n- [Alfa](notas/alfa.md)\n- [Gamma](notas/gamma.md)\n",
        "y el emisor recupera su enlace al path original"
    );

    // --- Cierre: la aritmética de revisiones de toda la sesión -------------------------------
    let revisiones = [&rev0, &rev1, &rev2, &rev3, &rev4];
    for (i, a) in revisiones.iter().enumerate() {
        for b in revisiones.iter().skip(i + 1) {
            assert_ne!(
                a, b,
                "cada apply del ciclo deja una revisión DISTINTA: {revisiones:?}"
            );
        }
    }
    assert_eq!(
        s.revision(),
        rev2,
        "el último workspace_status de la sesión coincide con la revisión que dejó el último revert"
    );
}

// ===========================================================================
// Criterio 2 — `edicion_externa_en_sesion_viva`
// ===========================================================================

/// **Dado** un `.md` editado con `std::fs::write` entre dos `tools/call` de la misma sesión,
/// **Cuando** se lee después, **Entonces** la lectura **SÍ ve** la edición externa.
///
/// La semántica está **fijada aquí a propósito** (criterio de la historia: «el test lo asevera
/// explícitamente, no lo deja al azar»). Es consecuencia del diseño, no una casualidad: el servidor
/// abre el workspace con `App::open` y `Workspace::document_set()` **redescubre y reparsea desde
/// disco en cada llamada**, así que no hay snapshot de proceso que quede obsoleto. Los cuatro
/// hechos que se aseveran: se ve el contenido nuevo, se deja de ver el viejo, cambia la revisión, y
/// un plan calculado ANTES de la edición queda caduco (`PLAN_STALE`) sin escribir nada.
#[test]
fn edicion_externa_en_sesion_viva() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    escribe(root, "README.md", "# Manual\n\n- [Beta](notas/beta.md)\n");
    escribe(
        root,
        "notas/beta.md",
        "---\nestado: publicado\n---\n\n# Beta\n\nTexto original inicial.\n",
    );

    let mut s = Sesion::abrir(root);

    // (1) Lectura antes de tocar nada.
    let doc_antes = s.tool(
        "knowledge_get",
        json!({"ref":{"path":"notas/beta.md"},"include":["body","frontmatter","revision"]}),
    )["document"]
        .clone();
    assert!(
        doc_antes["body"]
            .as_str()
            .unwrap()
            .contains("Texto original inicial."),
        "punto de partida: {doc_antes}"
    );
    let rev_doc_antes = doc_antes["revision"].as_str().unwrap().to_string();
    let rev_ws_antes = s.revision();
    assert_eq!(
        s.revision(),
        rev_ws_antes,
        "control: sin escrituras, la revisión del workspace no se mueve"
    );

    // (2) Un plan calculado ANTES de la edición externa (se intentará aplicar después).
    let plan = s.planifica(json!([{
        "op": "patch_frontmatter",
        "path": "notas/beta.md",
        "patch": {"estado": "archivado"}
    }]));
    let cs = plan["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();

    // (3) EDICIÓN EXTERNA con el proceso VIVO, entre dos `tools/call`.
    std::fs::write(
        root.join("notas/beta.md"),
        "---\nestado: congelado\nautor: alguien\n---\n\n# Beta\n\nTexto reemplazado desde fuera.\n",
    )
    .unwrap();
    escribe(
        root,
        "notas/delta.md",
        "# Delta\n\nDocumento nacido fuera.\n",
    );

    // (4) La lectura siguiente, en la MISMA sesión, ve la edición.
    let doc_despues = s.tool(
        "knowledge_get",
        json!({"ref":{"path":"notas/beta.md"},"include":["body","frontmatter","revision"]}),
    )["document"]
        .clone();
    let cuerpo = doc_despues["body"].as_str().unwrap();
    assert!(
        cuerpo.contains("Texto reemplazado desde fuera."),
        "la sesión viva DEBE ver la edición externa (document_set() relee de disco): {doc_despues}"
    );
    assert!(
        !cuerpo.contains("Texto original inicial."),
        "y no puede seguir sirviendo el contenido viejo: {doc_despues}"
    );
    assert_eq!(
        doc_despues["frontmatter"]["estado"], "congelado",
        "el frontmatter servido es el del disco: {doc_despues}"
    );
    assert_eq!(
        doc_despues["frontmatter"]["autor"], "alguien",
        "incluida una clave que no existía antes: {doc_despues}"
    );
    assert_ne!(
        doc_despues["revision"].as_str().unwrap(),
        rev_doc_antes,
        "la revisión del documento cambia con su contenido"
    );

    // Las demás lecturas también: búsqueda por el texto nuevo y recuento del fichero nacido fuera.
    let busca_nuevo = s.tool(
        "knowledge_search",
        json!({"text":"reemplazado desde fuera"}),
    );
    assert_eq!(
        paths_de_busqueda(&busca_nuevo),
        vec!["notas/beta.md"],
        "knowledge_search indexa el texto escrito externamente: {busca_nuevo}"
    );
    let busca_viejo = s.tool("knowledge_search", json!({"text":"Texto original inicial"}));
    assert!(
        paths_de_busqueda(&busca_viejo).is_empty(),
        "y deja de encontrar el texto que ya no está en disco: {busca_viejo}"
    );
    let estado = s.estado();
    assert_eq!(
        estado["counts"]["documents"], 3,
        "un .md creado externamente entra en el inventario de la sesión viva: {estado}"
    );
    assert_ne!(
        estado["workspaceRevision"].as_str().unwrap(),
        rev_ws_antes,
        "y la workspaceRevision refleja el disco nuevo"
    );

    // (5) El plan de (2) quedó caduco: el apply NO escribe.
    let err = s.tool_falla("change_apply", json!({"changeSetId": cs}));
    assert!(
        err.contains("PLAN_STALE"),
        "un plan calculado antes de una edición externa debe caducar con PLAN_STALE: {err}"
    );
    assert_eq!(
        lee(root, "notas/beta.md"),
        "---\nestado: congelado\nautor: alguien\n---\n\n# Beta\n\nTexto reemplazado desde fuera.\n",
        "el apply rechazado no puede haber tocado ni un byte del documento editado fuera"
    );
}

// ===========================================================================
// Criterio 3 — el `delete` aplicado y revertido BYTE A BYTE, en sesión viva (T3)
// ===========================================================================

/// **Dado** el `delete` aplicado, **Cuando** se revierte, **Entonces** el documento vuelve byte a
/// byte (y sus emisores recuperan los enlaces).
///
/// `benchmark.rs::delete_revert_byte_a_byte` cubre lo mismo con un proceso nuevo por llamada; aquí
/// la novedad es que **el plan, el apply y el revert comparten proceso**: si el servidor arrastrara
/// cualquier estado de la KB entre llamadas, el revert operaría sobre una foto vieja.
#[test]
fn delete_y_revert_byte_a_byte_en_sesion_viva() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    escribe(
        root,
        "objetivo.md",
        "---\nestado: publicado\n---\n\n# Objetivo\n\ncuerpo\n",
    );
    for slug in ["a", "b"] {
        escribe(
            root,
            &format!("{slug}.md"),
            &format!("# {slug}\n\nApunta a [Objetivo](objetivo.md) y sigue.\n"),
        );
    }
    let antes = instantanea_md(root);

    let mut s = Sesion::abrir(root);
    let rev_antes = s.revision();

    let plan = s.planifica(json!([{
        "op": "delete",
        "ref": {"path": "objetivo.md"},
        "inboundLinksPolicy": "remove_links"
    }]));
    assert_eq!(plan["canApply"], true, "el borrado es aplicable: {plan}");
    let cs = plan["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();

    let apply = s.aplica(&cs);
    let recibo = apply["receiptId"].as_str().expect("receiptId").to_string();
    assert_eq!(
        paths_cambiados(&apply),
        vec!["a.md", "b.md", "objetivo.md"],
        "el borrado con `remove_links` toca el documento y sus 2 emisores: {apply}"
    );
    assert_ne!(
        instantanea_md(root),
        antes,
        "guarda de no vacuidad: el apply TIENE que haber cambiado el árbol"
    );
    assert!(
        !root.join("objetivo.md").exists(),
        "el documento desaparece"
    );
    for slug in ["a", "b"] {
        let emisor = lee(root, &format!("{slug}.md"));
        assert!(
            !emisor.contains("objetivo.md"),
            "{slug}.md no puede conservar el enlace:\n{emisor}"
        );
        assert!(
            emisor.contains("Apunta a Objetivo y sigue."),
            "{slug}.md conserva el TEXTO del enlace:\n{emisor}"
        );
    }
    // La sesión viva ya no lo sirve.
    let err = s.tool_falla("knowledge_get", json!({"ref":{"path":"objetivo.md"}}));
    assert!(
        err.contains("DOCUMENT_NOT_FOUND"),
        "tras el borrado, leerlo en la misma sesión da DOCUMENT_NOT_FOUND: {err}"
    );

    let revert = s.revierte(&recibo);
    assert_eq!(
        revert["reverted"], true,
        "el borrado es reversible: {revert}"
    );
    assert_eq!(
        instantanea_md(root),
        antes,
        "el revert restaura el documento BYTE A BYTE y los enlaces de los 2 emisores"
    );
    assert_eq!(
        s.revision(),
        rev_antes,
        "y el workspace vuelve exactamente a la revisión de partida"
    );
    // Y la sesión viva lo vuelve a servir, sin reiniciar el proceso.
    let doc = s.tool(
        "knowledge_get",
        json!({"ref":{"path":"objetivo.md"},"include":["backlinks"]}),
    )["document"]
        .clone();
    let emisores: Vec<&str> = doc["backlinks"]["inbound"]
        .as_array()
        .unwrap()
        .iter()
        .map(|b| b["from"].as_str().unwrap())
        .collect();
    assert_eq!(
        emisores,
        vec!["a.md", "b.md"],
        "los 2 backlinks vuelven a existir tras el revert: {doc}"
    );
}
