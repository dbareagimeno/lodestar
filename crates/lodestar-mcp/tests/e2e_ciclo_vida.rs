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

    /// `change_plan` con una `policy` EXPLÍCITA (E29-H07): el veredicto `canApply` depende de ella,
    /// así que los tests de esta historia necesitan fijarla y no heredar la de la tool.
    fn planifica_con_policy(&mut self, ops: Value, policy: Value) -> Value {
        self.tool(
            "change_plan",
            json!({ "operations": ops, "policy": policy }),
        )
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

// ===========================================================================
// E28-H01 — Revertir un recibo `-revert` restaura de verdad (M-01 del testbench homelab)
//
// EL DEFECTO (`docs/qa/testbench/batches/verify_G1-18b.json`, caso G1-18)
//
// `App::change_revert_uncounted` (`crates/lodestar-app/src/lib.rs` ~L2168) deriva la identidad de la
// transacción inversa del `changeSetId` del recibo que revierte:
//
//     let orig_txn_id = transaction_id(&receipt.change_set_id);
//     let revert_txn_id = format!("{orig_txn_id}-revert");
//
// El recibo `X-revert` **hereda** el `changeSetId` de la transacción original, así que revertirlo
// recalcula `orig_txn_id = X` (no `X-revert`) y `revert_txn_id = "X-revert"` (su propio id). Doble
// consecuencia: se restaura desde `recovery/X/` —el árbol pre-apply, que YA es el estado vigente:
// no-op silencioso— y la transacción inversa colisiona consigo misma, sobrescribiendo
// `recovery/X-revert/` (que guardaba el estado **redo**) y `receipts/X-revert.json`. El redo queda
// destruido para siempre y `change_revert` responde `reverted: true`.
//
// EL COMPORTAMIENTO OBJETIVO QUE FIJAN ESTOS TESTS
//
// Revertir un `-revert` es una transacción real y componible: devuelve el fichero al estado que ese
// recibo dejó atrás, gana un `receiptId` propio (distinto del que revierte), no toca ni un byte del
// material de recuperación ni de los recibos previos, y encadena sin límite.
//
// Los cuatro viven en la sesión VIVA a propósito: es la única forma de observar la secuencia
// `plan → apply → revert → revert` como la vio el testbench (mismo proceso, mismo estado).
// ===========================================================================

/// Ruta del plano de control runtime de un workspace.
fn runtime(root: &Path) -> std::path::PathBuf {
    root.join(".lodestar").join("runtime")
}

/// Instantánea `ruta relativa POSIX → bytes` de todos los ficheros bajo `dir` (recursivo). Compara
/// **bytes**, no texto: las copias de recuperación son ficheros opacos para este test y el criterio
/// es «intactos byte a byte».
fn ficheros_bajo(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn recorre(d: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entradas) = std::fs::read_dir(d) else {
            return;
        };
        for e in entradas.flatten() {
            let ruta = e.path();
            if ruta.is_dir() {
                recorre(&ruta, base, out);
                continue;
            }
            let rel = ruta
                .strip_prefix(base)
                .unwrap_or(&ruta)
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, std::fs::read(&ruta).unwrap_or_default());
        }
    }
    let mut out = BTreeMap::new();
    recorre(dir, dir, &mut out);
    out
}

/// Los bytes de un artefacto del plano de control como texto (son JSON o `.md`), para que un fallo
/// de comparación se lea como lo que es y no como una lista de enteros.
fn legible(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).to_string()
}

/// Un workspace de un solo documento mutable: la forma mínima en la que el defecto se manifiesta
/// (una única ruta afectada, sin enlaces que enturbien el diff).
fn proyecto_de_un_documento(root: &Path) {
    escribe(
        root,
        "pendientes/wol-bastion.md",
        "---\nprioridad: 5\n---\n\n# WOL bastión\n\nDespertar el bastión por red.\n",
    );
}

/// El estado `A` (pre-apply) del documento mutable.
const ESTADO_A: &str = "---\nprioridad: 5\n---\n\n# WOL bastión\n\nDespertar el bastión por red.\n";

/// El estado `B` (post-apply): el mismo documento con la prioridad parcheada.
const ESTADO_B: &str = "---\nprioridad: 1\n---\n\n# WOL bastión\n\nDespertar el bastión por red.\n";

/// El documento mutable, tal y como está en disco.
fn wol(root: &Path) -> String {
    lee(root, "pendientes/wol-bastion.md")
}

/// `plan → apply` del patch que lleva el documento de `A` a `B`, con las precondiciones aseveradas:
/// sin una publicación real no hay nada que revertir y el escenario no significa nada. Devuelve el
/// `receiptId` del apply.
fn aplica_el_patch(s: &mut Sesion, root: &Path, prioridad: i64) -> String {
    let plan = s.planifica(json!([{
        "op": "patch_frontmatter",
        "path": "pendientes/wol-bastion.md",
        "patch": {"prioridad": prioridad}
    }]));
    assert_eq!(
        plan["canApply"], true,
        "precondición: parchear el frontmatter del documento debe ser aplicable: {plan}"
    );
    let cs = plan["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();
    let apply = s.aplica(&cs);
    assert_eq!(apply["applied"], true, "precondición: el apply: {apply}");
    assert_eq!(
        paths_cambiados(&apply),
        vec!["pendientes/wol-bastion.md"],
        "precondición: el patch toca exactamente una ruta: {apply}"
    );
    assert_eq!(
        wol(root),
        ESTADO_B,
        "precondición: el apply tiene que haber publicado de verdad el estado B"
    );
    apply["receiptId"]
        .as_str()
        .expect("receiptId del apply")
        .to_string()
}

/// **Criterio 1** — **Dado** un documento en `A`, **Cuando** se hace
/// `plan → apply` (queda en `B`) → `revert` (vuelve a `A`) → `revert` de ese recibo `-revert`,
/// **Entonces** el fichero queda exactamente en el estado post-apply `B`.
///
/// Hoy el segundo `revert` restaura desde `recovery/X/` —el árbol pre-apply, que ya es el estado
/// vigente—, así que el fichero se queda en `A` y la reversión es un no-op que se declara exitoso.
#[test]
fn revertir_el_revert_restaura_el_estado_post_apply() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_de_un_documento(root);

    let mut s = Sesion::abrir(root);
    let recibo_apply = aplica_el_patch(&mut s, root, 1);

    let revert1 = s.revierte(&recibo_apply);
    assert_eq!(
        revert1["reverted"], true,
        "precondición: el primer revert debe revertir: {revert1}"
    );
    assert_eq!(
        wol(root),
        ESTADO_A,
        "precondición: el primer revert devuelve el documento al estado A"
    );
    let recibo_revert = revert1["receiptId"]
        .as_str()
        .expect("receiptId del primer revert")
        .to_string();

    // Deshacer el *undo*: el estado al que hay que volver es el que ese recibo dejó atrás (B).
    let revert2 = s.revierte(&recibo_revert);
    assert_eq!(
        revert2["reverted"], true,
        "revertir un recibo `-revert` es una transacción como cualquier otra: {revert2}"
    );
    assert_eq!(
        wol(root),
        ESTADO_B,
        "revertir el `-revert` tiene que devolver el documento al estado POST-APPLY (B): el recibo \
         que se revierte llevó el workspace de B a A, así que deshacerlo lo devuelve a B. Si queda \
         en A, la reversión fue un no-op silencioso y el redo se perdió"
    );
    assert_eq!(
        paths_cambiados(&revert2),
        vec!["pendientes/wol-bastion.md"],
        "y declara la ruta que restauró: {revert2}"
    );

    // La sesión viva sirve el estado restaurado, sin reiniciar el proceso.
    let doc = s.tool(
        "knowledge_get",
        json!({"ref":{"path":"pendientes/wol-bastion.md"},"include":["frontmatter"]}),
    )["document"]
        .clone();
    assert_eq!(
        doc["frontmatter"]["prioridad"],
        json!(1),
        "la lectura posterior ve el valor restaurado por la reversión de la reversión: {doc}"
    );
}

/// **Criterio 2** — **Dado** ese mismo encadenamiento, **Cuando** se compara el `receiptId` del
/// segundo `revert` con el del primero, **Entonces** son **distintos**, y `previousWorkspaceRevision`
/// y `workspaceRevision` del segundo también difieren entre sí (hubo cambio efectivo, no un no-op).
///
/// Hoy el sufijo no apila: `revert_txn_id = "{orig_txn_id}-revert"` recalculado sobre el
/// `changeSetId` heredado vuelve a producir literalmente `X-revert`, el id de la transacción que se
/// está revirtiendo — la nueva transacción colisiona consigo misma.
#[test]
fn revertir_un_revert_produce_receipt_id_distinto() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_de_un_documento(root);

    let mut s = Sesion::abrir(root);
    let recibo_apply = aplica_el_patch(&mut s, root, 1);

    let revert1 = s.revierte(&recibo_apply);
    let recibo_revert = revert1["receiptId"]
        .as_str()
        .expect("receiptId del primer revert")
        .to_string();
    assert_ne!(
        recibo_revert, recibo_apply,
        "precondición: el recibo de la inversa ya tiene hoy identidad propia frente al del apply"
    );
    let rev_a = revert1["workspaceRevision"]
        .as_str()
        .expect("workspaceRevision")
        .to_string();

    let revert2 = s.revierte(&recibo_revert);
    let recibo_revert2 = revert2["receiptId"]
        .as_str()
        .expect("receiptId del segundo revert")
        .to_string();

    assert_ne!(
        recibo_revert2, recibo_revert,
        "revertir un recibo `-revert` produce una transacción NUEVA, con identidad propia: si \
         devuelve el mismo `receiptId` que revierte, está sobrescribiendo su propio material de \
         recuperación en vez de crear el suyo"
    );
    assert_eq!(
        revert2["previousWorkspaceRevision"],
        json!(rev_a),
        "la inversa parte de la revisión que dejó el recibo revertido: {revert2}"
    );
    assert_ne!(
        revert2["previousWorkspaceRevision"], revert2["workspaceRevision"],
        "y la deja en OTRA revisión: `previousWorkspaceRevision == workspaceRevision` es la firma \
         exacta del no-op silencioso que reportó el testbench (caso G1-18): {revert2}"
    );

    // El recibo nuevo es utilizable por un agente: aparece en el inventario de la sesión viva.
    let estado = s.estado();
    let recibos: Vec<String> = estado["receipts"]
        .as_array()
        .expect("workspace_status expone `receipts`")
        .iter()
        .map(|r| r["receiptId"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        recibos.contains(&recibo_revert2),
        "el recibo de la segunda reversión tiene que quedar listado para poder encadenar: \
         {recibos:?}"
    );
    assert!(
        recibos.contains(&recibo_revert),
        "y sin desplazar al del primer revert, que sigue describiendo su propia transacción: \
         {recibos:?}"
    );
}

/// **Criterio 3** — **Dado** `recovery/X/` + `receipts/X.json` de la transacción original y
/// `recovery/X-revert/` + `receipts/X-revert.json` del primer `revert`, **Cuando** se revierte
/// `X-revert`, **Entonces** los cuatro quedan **intactos byte a byte**.
///
/// Es el criterio de la pérdida de datos: `recovery/X-revert/` guarda el estado **redo** (el
/// resultado del apply). Hoy la colisión de identidad lo sobrescribe con el estado actual y reescribe
/// su recibo como un registro degenerado (`A→A`), de modo que el redo desaparece de forma permanente
/// y silenciosa.
#[test]
fn revertir_un_revert_no_toca_recovery_ni_receipts_previos() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_de_un_documento(root);

    let mut s = Sesion::abrir(root);
    let recibo_apply = aplica_el_patch(&mut s, root, 1);

    let revert1 = s.revierte(&recibo_apply);
    let recibo_revert = revert1["receiptId"]
        .as_str()
        .expect("receiptId del primer revert")
        .to_string();

    let recovery = runtime(root).join("recovery");
    let receipts = runtime(root).join("receipts");
    let recovery_antes = ficheros_bajo(&recovery);
    let receipts_antes = ficheros_bajo(&receipts);

    // Precondiciones: los cuatro artefactos existen y el de la inversa guarda el estado REDO.
    assert!(
        recovery_antes
            .keys()
            .any(|k| k.starts_with(&format!("{recibo_apply}/"))),
        "precondición: el apply deja copias de recuperación en `recovery/{recibo_apply}/`: {:?}",
        recovery_antes.keys().collect::<Vec<_>>()
    );
    let redo = recovery_antes
        .get(&format!("{recibo_revert}/pendientes/wol-bastion.md"))
        .map(|b| String::from_utf8_lossy(b).to_string())
        .unwrap_or_else(|| {
            panic!(
                "precondición: el primer revert respalda el estado que deshace en \
                 `recovery/{recibo_revert}/`: {:?}",
                recovery_antes.keys().collect::<Vec<_>>()
            )
        });
    assert_eq!(
        redo, ESTADO_B,
        "precondición: esa copia ES el redo — el estado post-apply que el revert dejó atrás"
    );
    assert!(
        receipts_antes.contains_key(&format!("{recibo_apply}.json"))
            && receipts_antes.contains_key(&format!("{recibo_revert}.json")),
        "precondición: los dos recibos están persistidos: {:?}",
        receipts_antes.keys().collect::<Vec<_>>()
    );

    let revert2 = s.revierte(&recibo_revert);
    assert_eq!(
        revert2["reverted"], true,
        "precondición del criterio: la segunda reversión se ejecuta: {revert2}"
    );

    let recovery_despues = ficheros_bajo(&recovery);
    let receipts_despues = ficheros_bajo(&receipts);

    for clave in recovery_antes.keys() {
        assert_eq!(
            recovery_despues.get(clave).map(|b| legible(b)),
            recovery_antes.get(clave).map(|b| legible(b)),
            "revertir el `-revert` no puede tocar ni un byte del material de recuperación previo, y \
             pisó «{clave}». Si lo que pisa es `recovery/{recibo_revert}/`, el estado REDO queda \
             destruido para siempre y ninguna operación posterior puede recuperarlo"
        );
    }
    for clave in receipts_antes.keys() {
        assert_eq!(
            receipts_despues.get(clave).map(|b| legible(b)),
            receipts_antes.get(clave).map(|b| legible(b)),
            "ni reescribir el recibo «{clave}» de una transacción previa: cada reversión registra \
             el suyo, con su propia identidad"
        );
    }
    // Control anti-vacuo: la segunda reversión SÍ deja material propio (si no, «nada cambió»
    // pasaría el criterio de intactos por la razón equivocada).
    assert!(
        recovery_despues.len() > recovery_antes.len(),
        "y además deja SUS copias de recuperación, bajo una identidad nueva: antes {:?}, después \
         {:?}",
        recovery_antes.keys().collect::<Vec<_>>(),
        recovery_despues.keys().collect::<Vec<_>>()
    );
    assert!(
        receipts_despues.len() > receipts_antes.len(),
        "y su recibo propio: antes {:?}, después {:?}",
        receipts_antes.keys().collect::<Vec<_>>(),
        receipts_despues.keys().collect::<Vec<_>>()
    );
}

/// **Criterio 4** — **Dado** el encadenamiento de tres reversiones
/// (`apply` → `revert(X)` → `revert(X-revert)` → `revert` del resultado anterior), **Cuando** se
/// ejecuta la tercera, **Entonces** el fichero vuelve al estado que dejó el **primer** `revert`
/// (composición sin límite: cada reversión es una operación real).
///
/// Es el criterio que descarta un arreglo que solo funcione «una vez»: la identidad de la inversa
/// tiene que poder derivarse indefinidamente sin colisionar con ninguna transacción previa.
#[test]
fn revertir_tres_veces_compone_sin_perder_estado() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_de_un_documento(root);

    let mut s = Sesion::abrir(root);
    let recibo_apply = aplica_el_patch(&mut s, root, 1);

    let revert1 = s.revierte(&recibo_apply);
    let recibo1 = revert1["receiptId"]
        .as_str()
        .expect("receiptId")
        .to_string();
    assert_eq!(wol(root), ESTADO_A, "revert 1: el documento vuelve a A");

    let revert2 = s.revierte(&recibo1);
    let recibo2 = revert2["receiptId"]
        .as_str()
        .expect("receiptId")
        .to_string();
    assert_eq!(
        wol(root),
        ESTADO_B,
        "revert 2: deshacer el *undo* devuelve el documento a B: {revert2}"
    );

    let revert3 = s.revierte(&recibo2);
    let recibo3 = revert3["receiptId"]
        .as_str()
        .expect("receiptId")
        .to_string();
    assert_eq!(
        revert3["reverted"], true,
        "la tercera reversión es tan válida como las dos anteriores: {revert3}"
    );
    assert_eq!(
        wol(root),
        ESTADO_A,
        "revert 3: deshacer la reversión anterior devuelve el documento al estado que dejó el \
         PRIMER revert (A). La composición no tiene límite: {revert3}"
    );

    let ids = [&recibo_apply, &recibo1, &recibo2, &recibo3];
    for (i, a) in ids.iter().enumerate() {
        for b in ids.iter().skip(i + 1) {
            assert_ne!(
                a, b,
                "cada transacción de la cadena tiene identidad PROPIA: dos ids iguales significan \
                 dos transacciones compartiendo `recovery/`, `journal/` y `receipts/`: {ids:?}"
            );
        }
    }

    // El estado final es coherente para la sesión viva y para el disco, sin reiniciar el proceso.
    let doc = s.tool(
        "knowledge_get",
        json!({"ref":{"path":"pendientes/wol-bastion.md"},"include":["frontmatter"]}),
    )["document"]
        .clone();
    assert_eq!(
        doc["frontmatter"]["prioridad"],
        json!(5),
        "y la lectura de la sesión coincide con el disco tras las tres reversiones: {doc}"
    );
    assert_eq!(
        s.revision(),
        revert3["workspaceRevision"].as_str().unwrap(),
        "la revisión que reporta el servidor es la que declaró la última reversión"
    );
}

// ===========================================================================
// E28-H03 — Identidad de transacción LIBRE en la publicación (corrige el bloqueante de H01)
//
// EL DEFECTO (verificado por dos jueces ciegos ejecutando el binario por JSON-RPC)
//
// El `changeSetId` es determinista —`blake3(baseRevision, normalizedOperations)`,
// `crates/lodestar-app/src/lib.rs` ~L1792—, así que replanificar EXACTAMENTE el mismo cambio sobre
// la misma base produce el mismo `changeSetId` y, con él, el mismo `txnId`
// (`transaction_id(&change_set.id)`, `transaction.rs:68`). La secuencia
// `plan → apply(X) → revert(X) → re-plan idéntico → apply` reutiliza literalmente el `txnId` `X`:
//
//   - el segundo `apply` **sobrescribe** `recovery/X/` y `receipts/X.json` de la primera transacción,
//     porque `apply_transaction_con_recibo` (`transaction.rs:280`) llama a `backup_originals` /
//     `create_journal` / `write_pending_receipt` sin pasar por el guard `assert_txn_id_libre` que
//     H01 escribió (`recovery.rs:912`) — ese guard solo lo llama el camino del `revert`;
//   - el `revert` posterior a ese segundo `apply` falla `WRITE_CONFLICT` **sin salida**: el
//     `revert_transaction_id` deriva `X-revert`, que ya tiene recibo persistido (el de la PRIMERA
//     reversión), `assert_txn_id_libre` lo rechaza correctamente y no hay id alternativo que probar.
//     El re-apply queda permanentemente no revertible.
//
// EL COMPORTAMIENTO OBJETIVO QUE FIJAN ESTOS TESTS
//
// La identidad efectiva de TODA transacción de publicación —`apply` y `revert` por igual— se resuelve
// buscando de forma determinista la primera variante LIBRE del `txnId`, con el mismo criterio de
// «libre» del guard de H01 (`journal/ ∪ receipts/`). Ni se sobrescribe en silencio (lo que hace hoy
// el apply) ni se falla sin salida (lo que hace hoy el revert).
//
// Viven en la sesión VIVA a propósito: es como el testbench reprodujo la secuencia (mismo proceso,
// mismo estado entre llamadas).
// ===========================================================================

/// **Testigo de identidad de fichero** de todo lo que cuelga de `ruta` (un fichero suelto o un
/// árbol): `ruta relativa POSIX → identidad de fichero`, en orden determinista.
///
/// Por qué hace falta además de la comparación por bytes: cuando dos transacciones comparten `txnId`,
/// el material que la segunda escribe encima de la primera puede ser **byte a byte idéntico** (mismo
/// estado respaldado, mismas revisiones en el recibo), así que «intactos byte a byte» pasaría sin
/// que nada esté intacto. La identidad distingue las dos cosas: `io::write_atomic` publica por
/// `temp+rename` y `backup_originals` empieza por `remove_dir_all`, de modo que una reescritura
/// estrena identidad aunque el contenido no cambie.
///
/// Multiplataforma con garantías distintas por SO:
/// - **Unix**: `(dev, ino)`. El inodo es estable frente a cualquier operación que no sea
///   crear/borrar el fichero, así que distingue con precisión «no lo tocó» de «lo reescribió».
/// - **Windows**: no hay noción de inodo portable, así que se usa
///   `(creation_time, last_write_time, file_size)`. Un `rename` atómico crea un fichero nuevo con
///   `creation_time` distinto del original, que es justo el mecanismo que el motor usa para
///   publicar (`temp+rename`), así que la garantía observable —distinguir «intacto» de
///   «reescrito»— se conserva aunque el campo no sea el mismo concepto de bajo nivel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct IdentidadFichero(u64, u64, u64);

fn testigo(ruta: &Path) -> BTreeMap<String, IdentidadFichero> {
    #[cfg(unix)]
    fn identidad(m: &std::fs::Metadata) -> IdentidadFichero {
        use std::os::unix::fs::MetadataExt;
        IdentidadFichero(m.dev(), m.ino(), 0)
    }
    #[cfg(windows)]
    fn identidad(m: &std::fs::Metadata) -> IdentidadFichero {
        use std::os::windows::fs::MetadataExt;
        IdentidadFichero(m.creation_time(), m.last_write_time(), m.file_size())
    }
    fn recorre(d: &Path, base: &Path, out: &mut BTreeMap<String, IdentidadFichero>) {
        let Ok(entradas) = std::fs::read_dir(d) else {
            return;
        };
        for e in entradas.flatten() {
            let ruta = e.path();
            if ruta.is_dir() {
                recorre(&ruta, base, out);
                continue;
            }
            let rel = ruta
                .strip_prefix(base)
                .unwrap_or(&ruta)
                .to_string_lossy()
                .replace('\\', "/");
            if let Ok(m) = std::fs::metadata(&ruta) {
                out.insert(rel, identidad(&m));
            }
        }
    }
    let mut out = BTreeMap::new();
    if ruta.is_dir() {
        recorre(ruta, ruta, &mut out);
    } else if let Ok(m) = std::fs::metadata(ruta) {
        out.insert(String::new(), identidad(&m));
    }
    assert!(
        !out.is_empty(),
        "precondición del testigo: «{}» tiene que existir para poder vigilarlo",
        ruta.display()
    );
    out
}

/// Los `receiptId` que `workspace_status` lista, en el orden en que los sirve.
fn recibos_listados(s: &mut Sesion) -> Vec<String> {
    s.estado()["receipts"]
        .as_array()
        .expect("workspace_status expone `receipts`")
        .iter()
        .map(|r| r["receiptId"].as_str().unwrap_or_default().to_string())
        .collect()
}

/// **Criterio 1** — **Dado** un documento en estado `A`, **Cuando** se ejecuta `plan → apply →
/// revert → re-plan idéntico (misma base, mismas ops) → apply → revert`, **Entonces** las cuatro
/// operaciones completan con éxito, cada una con un `receiptId` **distinto** de los tres anteriores,
/// y el fichero queda en el estado correcto tras cada paso.
///
/// El re-plan es **idéntico a propósito**: tras el primer `revert` el workspace vuelve a la misma
/// `baseRevision` que tenía al planificar, así que `compute_plan_hash` devuelve el mismo
/// `changeSetId` y el `txnId` colisiona. Es exactamente la secuencia que un agente ejecuta al
/// deshacer un cambio y rehacerlo, y hoy muere en el último paso con `WRITE_CONFLICT`.
#[test]
fn apply_revert_reapply_revert_de_plan_identico_completa_con_cuatro_receipts_unicos() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_de_un_documento(root);

    let mut s = Sesion::abrir(root);

    // (1) apply → el documento queda en B.
    let recibo_apply1 = aplica_el_patch(&mut s, root, 1);

    // (2) revert → vuelve a A.
    let revert1 = s.revierte(&recibo_apply1);
    assert_eq!(
        revert1["reverted"], true,
        "paso 2: el primer revert debe revertir: {revert1}"
    );
    assert_eq!(wol(root), ESTADO_A, "paso 2: el documento vuelve a A");
    let recibo_revert1 = revert1["receiptId"]
        .as_str()
        .expect("receiptId del primer revert")
        .to_string();

    // (3) re-plan IDÉNTICO + apply → vuelve a B. Como el workspace está de nuevo en la revisión de
    //     partida y las ops son las mismas, el `changeSetId` es el mismo de (1) por determinismo del
    //     planHash — y el `txnId` «natural» de esta transacción colisiona con el de aquella.
    let recibo_apply2 = aplica_el_patch(&mut s, root, 1);
    assert_ne!(
        recibo_apply2, recibo_apply1,
        "el segundo apply publica una transacción NUEVA, con identidad propia: reutilizar el \
         `txnId` del primer apply sobrescribe su `recovery/` y su recibo, y las copias con las que \
         se deshacía aquella transacción se pierden. El `changeSetId` puede repetirse (es \
         determinista y eso es deseado); el `txnId` efectivo, no"
    );
    assert_eq!(wol(root), ESTADO_B, "paso 3: el re-apply vuelve a dejar B");

    // (4) revert del re-apply → vuelve a A. Es el paso que hoy muere: el `txnId` derivado para la
    //     inversa ya tiene recibo persistido (el de la reversión del paso 2) y el guard de H01 lo
    //     rechaza sin ofrecer alternativa.
    let revert2 = s.revierte(&recibo_apply2);
    assert_eq!(
        revert2["reverted"], true,
        "paso 4: revertir el re-apply tiene que funcionar. Un `WRITE_CONFLICT` aquí deja el \
         re-apply permanentemente NO revertible, que es peor que el defecto que H01 cerró: la \
         secuencia legítima «deshacer y rehacer» se queda sin salida: {revert2}"
    );
    assert_eq!(
        wol(root),
        ESTADO_A,
        "paso 4: y devuelve el documento al estado A: {revert2}"
    );
    let recibo_revert2 = revert2["receiptId"]
        .as_str()
        .expect("receiptId del segundo revert")
        .to_string();

    // Los CUATRO ids son distintos entre sí: cada uno nombra su propio `recovery/`, `journal/` y
    // `receipts/`, y dos iguales significan dos transacciones compartiendo material.
    let ids = [
        &recibo_apply1,
        &recibo_revert1,
        &recibo_apply2,
        &recibo_revert2,
    ];
    for (i, a) in ids.iter().enumerate() {
        for b in ids.iter().skip(i + 1) {
            assert_ne!(
                a, b,
                "las cuatro transacciones de la secuencia tienen identidad PROPIA: {ids:?}"
            );
        }
    }

    // Y los cuatro recibos siguen listados: la cadena entera es auditable y encadenable.
    let recibos = recibos_listados(&mut s);
    for id in ids {
        assert!(
            recibos.contains(id),
            "el recibo «{id}» tiene que quedar listado en `workspace_status`: si desapareció es \
             que otra transacción lo pisó. Listados: {recibos:?}"
        );
    }

    // La sesión viva ve el estado final, sin reiniciar el proceso.
    let doc = s.tool(
        "knowledge_get",
        json!({"ref":{"path":"pendientes/wol-bastion.md"},"include":["frontmatter"]}),
    )["document"]
        .clone();
    assert_eq!(
        doc["frontmatter"]["prioridad"],
        json!(5),
        "la lectura posterior coincide con el disco tras las cuatro operaciones: {doc}"
    );
}

/// **Criterio 2** — **Dado** ese mismo encadenamiento, **Cuando** se inspecciona
/// `recovery/`/`receipts/` tras el segundo `apply`, **Entonces** las copias y el recibo de la
/// **primera** transacción (`recovery/X/`, `receipts/X.json`) siguen **intactos byte a byte** — el
/// segundo `apply` publicó bajo un `txnId` distinto.
///
/// Es el criterio de la pérdida de datos por la vía del `apply`: hoy `backup_originals` empieza por
/// `remove_dir_all` del árbol previo y `write_pending_receipt` reescribe su recibo, así que el
/// material de la primera transacción desaparece en silencio y `change_apply` responde
/// `applied: true`.
#[test]
fn reapply_de_changeset_identico_no_pisa_recovery_ni_receipts_previos() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_de_un_documento(root);

    let mut s = Sesion::abrir(root);
    let recibo_apply1 = aplica_el_patch(&mut s, root, 1);

    let revert1 = s.revierte(&recibo_apply1);
    assert_eq!(
        revert1["reverted"], true,
        "precondición: el primer revert debe revertir: {revert1}"
    );
    let recibo_revert1 = revert1["receiptId"]
        .as_str()
        .expect("receiptId del primer revert")
        .to_string();

    let recovery = runtime(root).join("recovery");
    let receipts = runtime(root).join("receipts");
    let recovery_antes = ficheros_bajo(&recovery);
    let receipts_antes = ficheros_bajo(&receipts);

    // Precondiciones: el material de las dos transacciones ya publicadas está en disco, y el del
    // primer apply guarda el estado A (con el que se deshacía aquel apply).
    let pre_apply = recovery_antes
        .get(&format!("{recibo_apply1}/pendientes/wol-bastion.md"))
        .map(|b| legible(b))
        .unwrap_or_else(|| {
            panic!(
                "precondición: el primer apply respalda el estado que pisó en \
                 `recovery/{recibo_apply1}/`: {:?}",
                recovery_antes.keys().collect::<Vec<_>>()
            )
        });
    assert_eq!(
        pre_apply, ESTADO_A,
        "precondición: esa copia es el estado A, el que hace reversible al primer apply"
    );
    assert!(
        receipts_antes.contains_key(&format!("{recibo_apply1}.json"))
            && receipts_antes.contains_key(&format!("{recibo_revert1}.json")),
        "precondición: los dos recibos están persistidos: {:?}",
        receipts_antes.keys().collect::<Vec<_>>()
    );

    // Testigos de identidad de fichero (device+inode) del material de la PRIMERA transacción. La
    // comparación por bytes no basta aquí y hay que decirlo: el re-apply respalda el MISMO estado A
    // y compone un recibo con las MISMAS revisiones, así que si sobrescribe deja unos bytes
    // idénticos y el criterio «intactos byte a byte» pasaría por la razón equivocada. El inodo sí
    // distingue «no lo tocó» de «lo reescribió con lo mismo»: `write_atomic` publica por
    // `temp+rename`, que estrena inodo.
    let testigo_recovery_antes = testigo(&recovery.join(&recibo_apply1));
    let testigo_recibo_antes = testigo(&receipts.join(format!("{recibo_apply1}.json")));

    // Re-plan idéntico + apply: el `changeSetId` vuelve a ser el mismo, así que el `txnId` «natural»
    // de esta transacción es el que ya ocupa la primera.
    let recibo_apply2 = aplica_el_patch(&mut s, root, 1);

    let recovery_despues = ficheros_bajo(&recovery);
    let receipts_despues = ficheros_bajo(&receipts);

    for clave in recovery_antes.keys() {
        assert_eq!(
            recovery_despues.get(clave).map(|b| legible(b)),
            recovery_antes.get(clave).map(|b| legible(b)),
            "un re-apply del mismo change set no puede tocar ni un byte del material de \
             recuperación previo, y pisó «{clave}». Ese árbol es la única copia con la que se \
             deshace la transacción que lo dejó: sobrescribirlo la vuelve irreversible"
        );
    }
    for clave in receipts_antes.keys() {
        assert_eq!(
            receipts_despues.get(clave).map(|b| legible(b)),
            receipts_antes.get(clave).map(|b| legible(b)),
            "ni reescribir el recibo «{clave}» de una transacción previa: cada publicación registra \
             el suyo, bajo su propia identidad"
        );
    }

    // Control anti-vacuo: el re-apply SÍ publicó (si no hubiera hecho nada, «intactos» pasaría por
    // la razón equivocada).
    assert_eq!(
        wol(root),
        ESTADO_B,
        "control anti-vacuo: el re-apply publica de verdad el estado B"
    );

    // El material de la primera transacción sigue siendo EL MISMO fichero, no una copia recién
    // escrita encima con el mismo contenido.
    assert_eq!(
        testigo(&recovery.join(&recibo_apply1)),
        testigo_recovery_antes,
        "`recovery/{recibo_apply1}/` tiene que seguir siendo el árbol que dejó el PRIMER apply, no \
         uno reescrito por el segundo: el re-apply respalda el mismo estado A, así que la \
         sobrescritura deja los mismos bytes y solo la identidad del fichero la delata"
    );
    assert_eq!(
        testigo(&receipts.join(format!("{recibo_apply1}.json"))),
        testigo_recibo_antes,
        "y `receipts/{recibo_apply1}.json` tiene que seguir siendo el recibo del PRIMER apply"
    );

    // Y el re-apply dejó material PROPIO, bajo una identidad que ninguna transacción previa usaba:
    // dos transacciones publicadas = dos árboles de recuperación y dos recibos.
    assert_ne!(
        recibo_apply2, recibo_apply1,
        "el re-apply publica bajo un `txnId` distinto: si reutiliza el de la primera transacción, \
         el material que acaba de aseverarse intacto es en realidad el suyo, escrito encima"
    );
    assert!(
        recovery_despues.contains_key(&format!("{recibo_apply2}/pendientes/wol-bastion.md")),
        "y deja SUS copias de recuperación bajo esa identidad nueva «{recibo_apply2}»: antes {:?}, \
         después {:?}",
        recovery_antes.keys().collect::<Vec<_>>(),
        recovery_despues.keys().collect::<Vec<_>>()
    );
    assert!(
        receipts_despues.contains_key(&format!("{recibo_apply2}.json")),
        "y su recibo propio: antes {:?}, después {:?}",
        receipts_antes.keys().collect::<Vec<_>>(),
        receipts_despues.keys().collect::<Vec<_>>()
    );
}

// ===========================================================================
// E29-H07 — `canApply: false` VINCULA a `change_apply` (`decisiones §18`), por el WIRE.
//
// La mitad de servicio de esta historia vive en `crates/lodestar-app/tests/plan.rs`
// (módulo `can_apply_vincula_al_apply`). Lo que se ejerce aquí es lo que `§18` describe literalmente:
// un agente que planifica y, viendo `canApply:false`, **insiste** con `change_apply` en la MISMA
// sesión y con el `changeSetId` que acaba de recibir. Ese encadenamiento plan→apply sobre un proceso
// vivo es exactamente lo que el arnés `Sesion` existe para reproducir, y es donde la incoherencia se
// observa como la observó el guion de la demo: la respuesta de la tool dice «no aplicable» y la
// siguiente responde `applied:true`.
//
// QUÉ NO SE DUPLICA AQUÍ. `mcp.rs::apply_de_plan_con_colision_rechaza_sin_tocar_disco` (E28-H02) y
// los tests de E28-H04 ya cubren el rechazo del apply por **colisión de paths** —dos operaciones
// que reclaman el mismo destino, o un destino ocupado—, que se juzga sobre las OPERACIONES y sale
// con `DOCUMENT_ALREADY_EXISTS`. Esta historia cubre el rechazo por el **VEREDICTO** del plan
// (`canApply:false` bajo su propia `PlanPolicy`, por resultado no conforme o por warnings), que hoy
// no bloquea absolutamente nada: son gates distintos, con códigos distintos y escenarios disjuntos.
// ===========================================================================

/// Workspace con un **error preexistente** deliberado —`roto.md` enlaza a un `.md` que no existe,
/// que `§20.9` clasifica `danglingDocumentLinks: error`— y un documento limpio sobre el que operar.
/// Es el montaje con el que `§18` se descubrió (el guion de la demo contra un workspace con un
/// error a propósito).
fn proyecto_con_error_preexistente(root: &Path) {
    escribe(
        root,
        "roto.md",
        "# Roto\n\nEnlace a un documento que no existe: [falta](inexistente-previo.md).\n",
    );
    escribe(
        root,
        "limpio.md",
        "---\ntitle: Limpio\n---\n\n# Limpio\n\nDocumento sin problemas.\n",
    );
}

/// Un patch inocuo sobre `limpio.md`: no crea ni borra nada y no introduce enlaces, así que **no
/// puede** disparar el gate de staging (`rejectNewErrors`) — si el apply se rechaza, es por el
/// veredicto del plan.
fn patch_inocuo() -> Value {
    json!([
        { "op": "patch_frontmatter", "ref": { "path": "limpio.md" },
          "patch": { "status": "revisado" } },
    ])
}

/// **Criterio `apply_de_plan_no_aplicable_se_rechaza_sin_escribir`** (mitad e2e) — **Dado** un
/// workspace con un error preexistente y un `change_plan` con la policy por defecto que devuelve
/// `canApply:false`, **Cuando** el agente insiste con `change_apply` en la misma sesión, **Entonces**
/// la tool falla con `INVALID_RESULT`, el mensaje nombra la cláusula `requireValidResult` que
/// bloqueó —no un genérico— y todos los `.md` quedan byte-idénticos.
///
/// ROJO HOY: el apply responde `applied:true` con `validation.valid:false` y `limpio.md` cambia en
/// disco (medido antes de escribir el test).
#[test]
fn apply_de_plan_no_aplicable_se_rechaza_sin_escribir_en_sesion_viva() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_con_error_preexistente(root);

    let mut s = Sesion::abrir(root);

    // La policy va explícita aunque coincida con el default: el criterio es sobre `requireValidResult`
    // y dejarlo implícito haría el test ilegible.
    let plan = s.planifica_con_policy(
        patch_inocuo(),
        json!({"requireValidResult": true, "allowWarnings": true}),
    );
    assert_eq!(
        plan["canApply"], false,
        "precondición: con `requireValidResult:true` y un error preexistente el plan NO es \
         aplicable: {plan}"
    );
    assert_eq!(
        plan["diagnosticsAfter"]["errors"], 1,
        "precondición: el resultado simulado conserva el error preexistente de `roto.md`: {plan}"
    );
    let cs = plan["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();

    let antes = instantanea_md(root);
    let rev_antes = s.revision();

    // El agente INSISTE, que es el escenario de `§18`.
    let error = {
        let r = s.tool_cruda("change_apply", json!({ "changeSetId": cs }));
        assert_eq!(
            r["isError"],
            json!(true),
            "aplicar un plan cuyo `canApply` era FALSE debe RECHAZARSE: la superficie prometió «este \
             plan no es aplicable bajo tu policy» y el motor lo publicó igual. Respuesta: {r}. \
             Disco resultante: {:?}",
            instantanea_md(root)
        );
        r["content"][0]["text"]
            .as_str()
            .unwrap_or_default()
            .to_string()
    };
    assert!(
        error.contains("INVALID_RESULT"),
        "el rechazo reusa la fila existente `INVALID_RESULT` («el resultado del plan no es \
         aceptable»), sin abrir el catálogo (`decisiones §18`); fue: {error}"
    );
    assert!(
        error.contains("requireValidResult"),
        "y el mensaje debe NOMBRAR la cláusula de la policy que bloqueó, que es lo único que le dice \
         al agente si replanificar o relajar la política; fue: {error}"
    );

    assert_eq!(
        instantanea_md(root),
        antes,
        "un plan rechazado no escribe un solo byte del conocimiento (invariante #1)"
    );
    assert_eq!(
        s.revision(),
        rev_antes,
        "y la revisión del workspace no se mueve: no hubo publicación"
    );
}

/// **Criterio `apply_rechaza_tambien_por_allow_warnings`** (mitad e2e) — **Dado** un plan con
/// `allowWarnings:false` sobre un resultado **válido pero con warnings**, **Cuando** se aplica,
/// **Entonces** también se rechaza y el mensaje nombra `allowWarnings`.
///
/// Este escenario es el que aísla el gate nuevo de cualquier otro: el resultado simulado tiene cero
/// errores, así que ninguna política de staging tiene nada que objetar. Hoy el apply publica.
#[test]
fn apply_rechaza_tambien_por_allow_warnings_en_sesion_viva() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // `assets/logo.png` no existe: enlace a un fichero de proyecto ausente ⇒ `missingWorkspaceFiles`,
    // que `§20.9` clasifica como WARNING (no error).
    escribe(
        root,
        "nota.md",
        "---\ntitle: Nota\n---\n\n# Nota\n\nDiagrama: [logo](assets/logo.png)\n",
    );

    let mut s = Sesion::abrir(root);

    let plan = s.planifica_con_policy(
        json!([
            { "op": "patch_frontmatter", "ref": { "path": "nota.md" },
              "patch": { "status": "revisado" } },
        ]),
        json!({"requireValidResult": true, "allowWarnings": false}),
    );
    assert_eq!(
        plan["canApply"], false,
        "precondición: `allowWarnings:false` sobre un resultado con warnings da `canApply:false`: {plan}"
    );
    assert_eq!(
        plan["diagnosticsAfter"]["errors"], 0,
        "precondición: el resultado es VÁLIDO — lo único que bloquea es el warning, de modo que \
         ningún gate de staging puede reclamar el mérito del rechazo: {plan}"
    );
    assert!(
        plan["diagnosticsAfter"]["warnings"].as_i64().unwrap_or(0) >= 1,
        "precondición: el enlace a `assets/logo.png` aporta al menos un warning: {plan}"
    );
    let cs = plan["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();

    let antes = instantanea_md(root);
    let error = s.tool_falla("change_apply", json!({ "changeSetId": cs }));

    assert!(
        error.contains("INVALID_RESULT"),
        "las dos cláusulas de la policy rechazan con el mismo código de wire; fue: {error}"
    );
    assert!(
        error.contains("allowWarnings"),
        "y el mensaje debe nombrar la cláusula CONCRETA que bloqueó (`allowWarnings`), no la otra ni \
         un genérico; fue: {error}"
    );
    assert_eq!(
        instantanea_md(root),
        antes,
        "y tampoco aquí se escribe nada"
    );
}

/// **Criterio `apply_de_plan_aplicable_no_cambia` + `apply_con_policy_permisiva_sigue_aplicando`**
/// (control anti-vacuo, mitad e2e) — **Dado** el MISMO workspace con el error preexistente,
/// **Cuando** se planifica con `policy: {requireValidResult:false}` y se aplica, **Entonces** el
/// ciclo funciona exactamente como antes de la historia: `applied:true`, la revisión cambia, el
/// cambio está en disco y el recibo sirve para revertir.
///
/// Es el control que impide que el arreglo degenere en «todo plan sobre un workspace con errores se
/// rechaza». Verde hoy, y tiene que seguir verde después: el gate solo puede morder donde el plan
/// dijo que mordería.
#[test]
fn apply_con_policy_permisiva_sigue_aplicando_en_sesion_viva() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_con_error_preexistente(root);

    let mut s = Sesion::abrir(root);
    let rev0 = s.revision();

    let plan = s.planifica_con_policy(
        patch_inocuo(),
        json!({"requireValidResult": false, "allowWarnings": true}),
    );
    assert_eq!(
        plan["canApply"], true,
        "precondición del control: con `requireValidResult:false` el MISMO plan sí es aplicable pese \
         al error preexistente: {plan}"
    );
    let cs = plan["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();

    let apply = s.aplica(&cs);
    assert_eq!(
        apply["applied"], true,
        "un plan con `canApply:true` debe seguir aplicándose exactamente como antes: {apply}"
    );
    assert_eq!(
        paths_cambiados(&apply),
        vec!["limpio.md"],
        "y tocar exactamente el documento parcheado: {apply}"
    );
    assert_ne!(
        apply["workspaceRevision"].as_str().unwrap(),
        rev0,
        "la publicación mueve la revisión"
    );
    assert!(
        lee(root, "limpio.md").contains("status: revisado"),
        "el cambio está de verdad en disco:\n{}",
        lee(root, "limpio.md")
    );
    assert!(
        lee(root, "roto.md").contains("inexistente-previo.md"),
        "el error preexistente sigue ahí: el apply lo TOLERA, no lo repara — que es justo por lo que \
         el plan por defecto lo declaraba no aplicable"
    );

    // El recibo sigue siendo utilizable: el camino feliz completo, no solo el apply.
    let recibo = apply["receiptId"]
        .as_str()
        .expect("el apply devuelve receiptId")
        .to_string();
    let revert = s.revierte(&recibo);
    assert_eq!(
        revert["reverted"], true,
        "y el ciclo entero sigue cerrando (plan → apply → revert): {revert}"
    );
    assert!(
        !lee(root, "limpio.md").contains("status: revisado"),
        "el revert deshace el patch:\n{}",
        lee(root, "limpio.md")
    );
}

/// **Criterio `el_rechazo_por_can_apply_no_deja_rastro_transaccional`** (mitad e2e) — **Dado** un
/// plan rechazado por este gate, **Cuando** se inspecciona `.lodestar/runtime/`, **Entonces** no hay
/// journal, ni staging, ni recibo, ni copias de recuperación de esa transacción, y `workspace_status`
/// no lista recibo alguno: el rechazo ocurre ANTES del lock.
///
/// El plan persistido (`runtime/plans/`) y `runtime/audit.jsonl` quedan fuera del criterio a
/// propósito: el primero es el artefacto que se está juzgando (caduca por TTL) y la segunda se anexa
/// en todo intento, con éxito o sin él (E13-H10).
#[test]
fn el_rechazo_por_can_apply_no_deja_rastro_transaccional() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    proyecto_con_error_preexistente(root);

    let mut s = Sesion::abrir(root);

    let plan = s.planifica_con_policy(
        patch_inocuo(),
        json!({"requireValidResult": true, "allowWarnings": true}),
    );
    assert_eq!(
        plan["canApply"], false,
        "precondición: plan no aplicable: {plan}"
    );
    let cs = plan["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();

    let r = s.tool_cruda("change_apply", json!({ "changeSetId": cs }));
    assert_eq!(
        r["isError"],
        json!(true),
        "precondición de este criterio: el apply debe rechazarse (hoy publica): {r}"
    );

    for sub in ["journal", "staging", "receipts", "recovery"] {
        let residuos = ficheros_bajo(&runtime(root).join(sub));
        assert!(
            residuos.is_empty(),
            "un rechazo por `canApply:false` ocurre ANTES del lock, así que \
             `.lodestar/runtime/{sub}/` no puede contener nada de esa transacción; contiene {:?}",
            residuos.keys().collect::<Vec<_>>()
        );
    }
    assert_eq!(
        recibos_listados(&mut s),
        Vec::<String>::new(),
        "ni `workspace_status` puede listar un recibo de algo que nunca se publicó"
    );
}
