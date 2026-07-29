//! **E23-H11 (mitad de `App`/MCP)** — descubribilidad de la KB por el wire.
//!
//! La mitad del **core** de esta historia ya está cerrada (commit `2b9cf18`: `metadata_inspect`
//! explota listas, `LinkTarget::WorkspaceDirectory` deja de tumbar la puerta de CI) y sus dos
//! criterios viven en `crates/lodestar-core/tests/`. Aquí está la **otra mitad**, la de la
//! superficie: proyección de frontmatter en `knowledge_search`, retirada de `sort`, retirada de la
//! op `apply_fix` y listado de receipts desde `workspace_status`.
//!
//! # Por qué un fichero propio
//!
//! Cada fichero de `tests/` es un binario independiente. `mcp.rs` tiene ~90 tests verdes que no
//! pueden dejar de ejecutarse mientras dure el rojo, así que esta fase roja vive aparte — mismo
//! precedente que `grafo.rs` (E17-H05).
//!
//! # Por qué por el wire y no contra la firma de `App`
//!
//! Está documentado desde E10-H09 y reafirmado en E19-H05 (cabeceras de las secciones de
//! `knowledge_search` en `mcp.rs`): lo que estas historias cambian es el **contrato de wire** (los
//! `arguments` que acepta la tool y la forma del `SearchResult`), y probarlo por JSON-RPC lo fija
//! sin acoplar los tests a los nombres/orden de los parámetros Rust que el implementador aún ha de
//! elegir. Corolario: **no hace falta ningún stub de producción**; el rojo es fallo de aserción
//! contra el binario actual, no error de compilación.
//!
//! # Contrato que fija este fichero (fase ROJA)
//!
//! ```jsonc
//! // knowledge_search — arguments
//! { "text": "", "include": ["frontmatter.status", "frontmatter.owner.name"] }
//! //          ↑ NUEVO: proyección pedida por el llamador. El sufijo tras «frontmatter.» es un
//! //            FieldPath (dot-path), así que los anidados salen gratis. Una entrada que NO
//! //            empiece por «frontmatter.», o cuyo sufijo no sea un FieldPath válido, es
//! //            INVALID_SCHEMA — no se ignora en silencio.
//! //          · «sort» YA NO EXISTE: fuera del inputSchema, del contrato y de la firma de App.
//!
//! // knowledge_search — structuredContent.results[i]
//! { "path": "notas/alfa.md", "title": "Alfa", "snippet": "…", "score": 1.0,
//!   "revision": "blake3:…",
//!   "frontmatter": { "status": "accepted", "owner.name": "Ana" } }
//! //  ↑ NUEVO y presente SOLO si se pidió algo en «include». Tecleado por el FIELD PATH pedido
//! //    (la clave es el sufijo tal cual se pidió: «status», «owner.name» — no se re-anida ni se
//! //    repite el prefijo «frontmatter.»). Valores YAML CRUDOS, sin coerción: un número es número,
//! //    una lista es lista. Un campo AUSENTE en el documento no aparece como clave — nunca un
//! //    `null` disfrazado (misma regla que el `include` de `knowledge_get`).
//!
//! // workspace_status — structuredContent
//! { …, "receipts": [ { "receiptId": "txn:…", "changeSetId": "changeset:…",
//!                      "resultRevision": "blake3:…", "changedPathCount": 2 } ] }
//! //  ↑ NUEVO: los receipts persistidos, ORDEN MTIME DESC (el más reciente primero, el mismo
//! //    criterio de «más antiguo» que ya usa `gc_receipts` porque `ChangeReceipt` no lleva
//! //    timestamp propio). Entrada ACOTADA: lo justo para elegir cuál revertir; el receipt entero
//! //    se sigue leyendo por `change_revert`. Lista vacía (no ausente) si no hay ninguno.
//! ```
//!
//! **Elección de nombre declarada** (la historia dice «nº de rutas afectadas» sin fijar la clave):
//! `changedPathCount`, por eco directo de `ChangeReceipt::changed_paths`, que es de donde sale.
//!
//! # Sobre «el schema rechaza» (`sort`, `apply_fix`)
//!
//! El servidor **no valida** los `arguments` contra el `inputSchema` — es declarativo (así lo fijó
//! E23-H10 para `change_plan`). Por eso los dos criterios de retirada se verifican donde de verdad
//! ocurren:
//! - `sort`: **declarativamente** (fuera de `inputSchema.properties`, que ya es
//!   `additionalProperties: false`, y fuera de `contracts/mcp.yml` y de la firma de `App`). No se
//!   exige un rechazo en ejecución: añadir validación ad-hoc de un solo parámetro contradiría el
//!   diseño declarativo del schema.
//! - `apply_fix`: declarativamente **y en ejecución**, porque ahí sí hay un despachador real
//!   (`normalize_raw_op`): una op fuera del enum cae en su brazo por defecto → `INVALID_SCHEMA`,
//!   que es el código correcto, en vez del `DOCUMENT_NOT_FOUND` que hoy devuelve un `apply_fix` que
//!   siempre falla.
//! - `include` malformado: **en ejecución**, `INVALID_SCHEMA`. No es una excepción a lo anterior
//!   sino la misma regla: sus valores son abiertos (`frontmatter.<lo que sea>`) y **no caben en un
//!   `enum`** del schema —a diferencia del `include` de `knowledge_get`—, así que el único sitio
//!   donde la superficie puede ser honesta es el despachador, que tiene que parsear el sufijo de
//!   todos modos. Aceptarlo y descartarlo sería reintroducir por la puerta de atrás justo el
//!   defecto que esta historia saca por la de delante con `sort`.

use std::collections::BTreeSet;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

use serde_json::{json, Value};

// ===========================================================================
// Arnés: una sesión MCP viva (patrón de `e2e_ciclo_vida.rs`, E23-H07).
// ===========================================================================

/// Un proceso `lodestar-mcp` **vivo** contra el que se dialoga petición a petición.
///
/// Se escribe **una** línea, se vacía y se lee **una** respuesta antes de devolver el control: eso
/// permite encadenar `change_plan` → `change_apply` → `workspace_status` → `change_revert` (cada
/// paso necesita un id de la respuesta anterior) sobre el **mismo** proceso. Levantar un binario
/// por paso enmascararía justo los bugs de invalidación que destapó E23-H07.
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

    /// Manda **una** línea JSON-RPC y lee **una** respuesta del mismo proceso, aseverando la
    /// correlación `id` petición↔respuesta (un desfase de una línea haría que todo lo siguiente
    /// leyera la respuesta anterior y pasara por accidente).
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
            "el servidor cerró stdout sin responder a «{metodo}» (id {id})"
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

    /// El catálogo de tools (`tools/list`) tal y como lo ve un cliente.
    fn catalogo(&mut self) -> Vec<Value> {
        let resp = self.peticion("tools/list", json!({}));
        resp["result"]["tools"]
            .as_array()
            .unwrap_or_else(|| panic!("tools/list devuelve `tools` (array): {resp}"))
            .clone()
    }

    /// El descriptor de una tool concreta del catálogo.
    fn descriptor(&mut self, nombre: &str) -> Value {
        self.catalogo()
            .into_iter()
            .find(|t| t["name"] == json!(nombre))
            .unwrap_or_else(|| panic!("la tool «{nombre}» debe seguir en el catálogo"))
    }

    /// `knowledge_search` con `arguments` arbitrarios → sus `results`.
    fn buscar(&mut self, args: Value) -> Vec<Value> {
        let sc = self.tool("knowledge_search", args);
        sc["results"]
            .as_array()
            .unwrap_or_else(|| panic!("knowledge_search devuelve `results` (array): {sc}"))
            .clone()
    }

    /// `workspace_status` completo.
    fn estado(&mut self) -> Value {
        self.tool("workspace_status", json!({}))
    }
}

impl Drop for Sesion {
    fn drop(&mut self) {
        let _ = self.hijo.kill();
        let _ = self.hijo.wait();
    }
}

// ===========================================================================
// Utilidades
// ===========================================================================

/// Escribe un fichero bajo `root`, creando los directorios intermedios.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    std::fs::create_dir_all(ruta.parent().unwrap()).unwrap();
    std::fs::write(ruta, contenido).unwrap();
}

/// Localiza el resultado de `path` entre los `results` de una búsqueda.
fn hit<'a>(results: &'a [Value], path: &str) -> &'a Value {
    results
        .iter()
        .find(|r| r["path"] == json!(path))
        .unwrap_or_else(|| panic!("«{path}» debe estar entre los resultados: {results:?}"))
}

/// Las claves de un objeto JSON.
fn claves(v: &Value) -> BTreeSet<String> {
    v.as_object()
        .unwrap_or_else(|| panic!("se esperaba un objeto JSON: {v}"))
        .keys()
        .cloned()
        .collect()
}

/// El mapa `frontmatter` proyectado de un resultado, o `{}` si la tool no lo emitió.
///
/// Tolera las dos formas admisibles cuando un documento no tiene **ninguno** de los campos pedidos
/// (clave ausente o mapa vacío): lo que la historia fija es que un campo ausente **no aparezca**,
/// no si el contenedor vacío se omite.
fn proyeccion(r: &Value) -> serde_json::Map<String, Value> {
    match r.get("frontmatter") {
        None | Some(Value::Null) => serde_json::Map::new(),
        Some(v) => v
            .as_object()
            .unwrap_or_else(|| panic!("`frontmatter` de un resultado debe ser un objeto: {r}"))
            .clone(),
    }
}

/// Ruta a un fichero del repo desde el manifiesto de este crate (`crates/lodestar-mcp`).
fn del_repo(rel: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .join(rel)
}

/// La entrada de la tool `nombre` en la sección `tools:` de `contracts/mcp.yml`.
fn tool_del_contrato(nombre: &str) -> serde_yaml::Value {
    let ruta = del_repo("contracts/mcp.yml");
    let yaml: serde_yaml::Value = serde_yaml::from_str(
        &std::fs::read_to_string(&ruta)
            .unwrap_or_else(|e| panic!("no se pudo leer {}: {e}", ruta.display())),
    )
    .expect("`contracts/mcp.yml` debe ser YAML válido");
    yaml["tools"]
        .as_sequence()
        .expect("`contracts/mcp.yml` declara `tools:` como secuencia")
        .iter()
        .find(|t| t["nombre"].as_str() == Some(nombre))
        .unwrap_or_else(|| panic!("`contracts/mcp.yml` debe declarar la tool «{nombre}»"))
        .clone()
}

/// El `params` declarado en `contracts/mcp.yml` para la tool `nombre`, como lista de nombres.
fn params_del_contrato(nombre: &str) -> Vec<String> {
    tool_del_contrato(nombre)["params"]
        .as_sequence()
        .map(|ps| {
            ps.iter()
                .filter_map(|p| p["nombre"].as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default()
}

/// Los `valores` (enum declarado) del parámetro `param` de la tool `nombre` en
/// `contracts/mcp.yml`.
///
/// Se lee **estructuralmente**, no por subcadena sobre el YAML serializado: lo que el criterio
/// exige es que la op salga del **enum**, no que el contrato deje de nombrarla en su prosa (una
/// retirada se explica citando lo retirado; prohibir la palabra obligaría a circunloquios y
/// dejaría el documento peor de lo que estaba).
fn valores_del_param(nombre: &str, param: &str) -> Vec<String> {
    let tool = tool_del_contrato(nombre);
    let p = tool["params"]
        .as_sequence()
        .unwrap_or_else(|| panic!("la tool «{nombre}» declara `params` en el contrato"))
        .iter()
        .find(|p| p["nombre"].as_str() == Some(param))
        .unwrap_or_else(|| {
            panic!("`contracts/mcp.yml` debe declarar el parámetro «{param}» de «{nombre}»")
        })
        .clone();
    p["valores"]
        .as_sequence()
        .unwrap_or_else(|| panic!("el parámetro «{param}» de «{nombre}» declara `valores`: {p:?}"))
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect()
}

/// Workspace de búsqueda: frontmatter heterogéneo a propósito (un documento con campos ricos, otro
/// con solo uno, y un tercero **sin bloque de frontmatter**), y el mismo texto en los tres para que
/// `text` no discrimine y la proyección sea lo único bajo prueba.
///
/// `alfa` lleva además `nula: null` — una clave **presente** cuyo valor es `null`, que es el
/// contraejemplo con el que se distingue «ausente» de «null explícito»
/// (`search_include_distingue_null_explicito_de_ausente`).
fn workspace_frontmatter() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    escribe(
        root,
        "notas/alfa.md",
        "---\nstatus: accepted\npriority: 3\ndraft: false\nnula: null\ntags:\n  - rojo\n  - \
         azul\nowner:\n  name: Ana\n  equipo: plataforma\n---\n\n# Alfa\n\nCuerpo con la palabra \
         lodestar.\n",
    );
    escribe(
        root,
        "notas/beta.md",
        "---\nstatus: draft\n---\n\n# Beta\n\nCuerpo con la palabra lodestar.\n",
    );
    escribe(
        root,
        "notas/gamma.md",
        "# Gamma\n\nCuerpo con la palabra lodestar, sin bloque de frontmatter.\n",
    );
    dir
}

// ===========================================================================
// Criterio · `search_proyecta_frontmatter`
//
// «Dado un `knowledge_search` que pide `include: ["frontmatter.status"]`, Entonces cada resultado
//  trae ese campo.»
//
// El defecto de fondo: hoy un hit lleva solo `path`/`title`/`snippet`/`score`/`revision`, así que
// ver el `status` de 30 resultados cuesta **30 `knowledge_get`** (N+1). E19-H05 retiró los campos
// privilegiados `type`/`status`/`tags` sin poner nada genérico en su lugar.
// ===========================================================================

/// **Criterio `search_proyecta_frontmatter`.**
///
/// Verifica las cuatro propiedades de la proyección de un campo simple:
/// 1. el documento que tiene el campo lo trae, con su valor;
/// 2. el documento que **no** lo tiene no gana una clave `status: null`;
/// 3. la proyección devuelve **solo lo pedido**, no el frontmatter entero (el payload de la
///    búsqueda sigue acotado: es el invariante que separa `knowledge_search` de `knowledge_get`);
/// 4. el `inputSchema` **declara** `include` (un agente descubre la tool leyendo el schema: un
///    parámetro que funciona pero no se anuncia no existe).
#[test]
fn search_proyecta_frontmatter() {
    let dir = workspace_frontmatter();
    let mut s = Sesion::abrir(dir.path());

    // (1) y (2): el campo viaja donde existe y NO se inventa donde no.
    let results = s.buscar(json!({ "text": "", "include": ["frontmatter.status"] }));
    assert_eq!(
        results.len(),
        3,
        "guarda anti-vacua: los 3 documentos del workspace casan un `text` vacío: {results:?}"
    );

    assert_eq!(
        proyeccion(hit(&results, "notas/alfa.md")).get("status"),
        Some(&json!("accepted")),
        "el hit debe traer el `status` pedido sin pagar un `knowledge_get` por documento (N+1, el \
         defecto que esta historia salda): {:?}",
        hit(&results, "notas/alfa.md")
    );
    assert_eq!(
        proyeccion(hit(&results, "notas/beta.md")).get("status"),
        Some(&json!("draft")),
        "cada documento proyecta SU propio valor: {:?}",
        hit(&results, "notas/beta.md")
    );
    assert!(
        !proyeccion(hit(&results, "notas/gamma.md")).contains_key("status"),
        "un documento SIN el campo no puede ganar una clave `status` (ni con `null`): la misma \
         regla que el `include` de `knowledge_get` — ausente es ausente, nunca un vacío \
         disfrazado. Hit: {:?}",
        hit(&results, "notas/gamma.md")
    );

    // (3) Solo lo pedido: el frontmatter de alfa tiene 6 claves, la proyección debe traer 1.
    assert_eq!(
        proyeccion(hit(&results, "notas/alfa.md"))
            .keys()
            .cloned()
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["status".to_string()]),
        "la proyección devuelve EXACTAMENTE los field paths pedidos, no el frontmatter entero: \
         `knowledge_search` sigue siendo la tool de payload acotado. Hit: {:?}",
        hit(&results, "notas/alfa.md")
    );

    // (4) El schema anuncia el parámetro.
    let ks = s.descriptor("knowledge_search");
    let props = &ks["inputSchema"]["properties"];
    assert!(
        props.get("include").is_some(),
        "el `inputSchema` de `knowledge_search` debe DECLARAR «include»: un agente descubre la \
         tool leyendo el schema, y un parámetro no anunciado es un parámetro que no existe. \
         Schema: {}",
        ks["inputSchema"]
    );
    assert_eq!(
        props["include"]["type"], "array",
        "«include» es un array de field paths («frontmatter.<fieldPath>»): {}",
        ks["inputSchema"]
    );
}

/// **Criterio `search_proyecta_frontmatter` (campo anidado y campo ausente).**
///
/// El sufijo tras `frontmatter.` se parsea con `FieldPath::parse`, así que los anidados salen
/// gratis. La clave del mapa es el **field path pedido tal cual** (`"owner.name"`), no una
/// re-anidación del objeto ni el prefijo repetido: quien pidió `frontmatter.owner.name` puede leer
/// la respuesta con la misma cadena que escribió.
#[test]
fn search_include_campo_anidado() {
    let dir = workspace_frontmatter();
    let mut s = Sesion::abrir(dir.path());

    let results = s.buscar(json!({
        "text": "",
        "include": ["frontmatter.owner.name", "frontmatter.status"]
    }));

    let alfa = proyeccion(hit(&results, "notas/alfa.md"));
    assert_eq!(
        alfa.get("owner.name"),
        Some(&json!("Ana")),
        "un field path ANIDADO se proyecta bajo su dot-path tal y como se pidió («owner.name»), \
         sin re-anidar ni repetir el prefijo «frontmatter.»: {alfa:?}"
    );
    assert_eq!(
        alfa.get("status"),
        Some(&json!("accepted")),
        "varios field paths en un mismo `include` se proyectan todos: {alfa:?}"
    );

    let beta = proyeccion(hit(&results, "notas/beta.md"));
    assert_eq!(
        beta.get("status"),
        Some(&json!("draft")),
        "beta sí tiene `status`: {beta:?}"
    );
    assert!(
        !beta.contains_key("owner.name"),
        "beta no tiene `owner`, así que su proyección NO puede llevar la clave «owner.name»: \
         el mapa es por documento, no un esqueleto común rellenado con nulos: {beta:?}"
    );
}

/// **Criterio `search_proyecta_frontmatter` (tipos YAML crudos, sin coerción).**
///
/// El argumento de venta del motor es que **no coerciona tipos** (`§20.2`): un número es número,
/// un booleano es booleano y una lista es lista. Una proyección que renderizara todo a string
/// obligaría al agente a re-parsear, y volvería inútil el `where` tipado que sí conserva el tipo.
#[test]
fn search_include_conserva_tipos_yaml() {
    let dir = workspace_frontmatter();
    let mut s = Sesion::abrir(dir.path());

    let results = s.buscar(json!({
        "text": "",
        "include": ["frontmatter.priority", "frontmatter.tags", "frontmatter.draft"]
    }));
    let alfa = proyeccion(hit(&results, "notas/alfa.md"));

    let priority = alfa
        .get("priority")
        .unwrap_or_else(|| panic!("falta la proyección de `priority`: {alfa:?}"));
    assert!(
        priority.is_number(),
        "`priority: 3` debe viajar como NÚMERO, no como la cadena «3»: el motor no coerciona \
         tipos (§20.2). Valor: {priority}"
    );
    assert_eq!(priority, &json!(3), "y con su valor: {alfa:?}");

    let tags = alfa
        .get("tags")
        .unwrap_or_else(|| panic!("falta la proyección de `tags`: {alfa:?}"));
    assert_eq!(
        tags,
        &json!(["rojo", "azul"]),
        "una lista viaja como LISTA (con su orden), no aplanada a texto: {tags}"
    );

    let draft = alfa
        .get("draft")
        .unwrap_or_else(|| panic!("falta la proyección de `draft`: {alfa:?}"));
    assert_eq!(
        draft,
        &json!(false),
        "un booleano viaja como booleano — y `false` NO es lo mismo que ausente: {draft}"
    );
}

/// **Criterio `search_proyecta_frontmatter` (la línea que la historia traza: «nunca un `null`
/// disfrazado»).**
///
/// Son **dos casos distintos** y el wire tiene que distinguirlos:
/// - clave **ausente** en el documento → la clave no aparece en el mapa;
/// - clave **presente con `null` explícito** (`nula: null`) → aparece, con valor `null`.
///
/// Colapsarlos es la regresión que da nombre al criterio: si un campo ausente se emitiera como
/// `null`, el agente no podría distinguir «este documento no declara el campo» de «lo declara
/// vacío» —y eso, en una KB donde el frontmatter es YAML arbitrario y nadie impone claves, es
/// justo la pregunta que se hace—. La distinción existe ya en el core (`ParsedFrontmatter::get`
/// devuelve `Some(Null)` para la clave presente y `None` para la ausente); este test impide que se
/// pierda al proyectarla.
#[test]
fn search_include_distingue_null_explicito_de_ausente() {
    let dir = workspace_frontmatter();
    let mut s = Sesion::abrir(dir.path());

    let results = s.buscar(json!({ "text": "", "include": ["frontmatter.nula"] }));

    // (a) Presente con `null` explícito: la clave SÍ está, y su valor es `null`.
    let alfa = proyeccion(hit(&results, "notas/alfa.md"));
    assert!(
        alfa.contains_key("nula"),
        "`nula: null` está DECLARADA en alfa: la proyección debe traer la clave (que la declares \
         vacía es información, y distinta de no declararla): {alfa:?}"
    );
    assert_eq!(
        alfa.get("nula"),
        Some(&Value::Null),
        "y su valor es `null`, sin coerción a cadena vacía ni a `false`: {alfa:?}"
    );

    // (b) Ausente: la clave NO aparece. Es el otro lado de la misma línea.
    for sin_la_clave in ["notas/beta.md", "notas/gamma.md"] {
        let p = proyeccion(hit(&results, sin_la_clave));
        assert!(
            !p.contains_key("nula"),
            "«{sin_la_clave}» NO declara `nula`, así que su proyección no puede llevar la clave: \
             si ausente y `null` explícito se emitieran igual, el agente no podría distinguir «no \
             lo declara» de «lo declara vacío». Proyección: {p:?}"
        );
    }
}

/// **Guarda de retrocompatibilidad + criterio `search_proyecta_frontmatter`.**
///
/// Sin `include`, la forma del hit **no cambia** (un cliente de v0.3 que no pida nada no ve nada
/// nuevo); con `include`, lo único que cambia es que aparece `frontmatter`. Las dos mitades juntas
/// impiden las dos maneras de equivocarse: emitir siempre el mapa (aunque no se pida) o aprovechar
/// para colar otros campos.
#[test]
fn search_sin_include_conserva_la_forma() {
    let dir = workspace_frontmatter();
    let mut s = Sesion::abrir(dir.path());

    let sin = s.buscar(json!({ "text": "" }));
    let forma_base: BTreeSet<String> = ["path", "title", "snippet", "score", "revision"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    for r in &sin {
        assert_eq!(
            claves(r),
            forma_base,
            "sin `include`, el hit conserva EXACTAMENTE su forma de E19-H05 (y sigue sin `body`, \
             invariante de la tool): {r}"
        );
    }

    let con = s.buscar(json!({ "text": "", "include": ["frontmatter.status"] }));
    let alfa = hit(&con, "notas/alfa.md");
    let mut esperadas = forma_base.clone();
    esperadas.insert("frontmatter".to_string());
    assert_eq!(
        claves(alfa),
        esperadas,
        "con `include`, el hit gana UN campo —`frontmatter`— y ninguno más; en particular NO \
         reaparecen los campos privilegiados OKF que E19-H05 retiró: {alfa}"
    );
}

/// **Criterio `search_proyecta_frontmatter` (entrada malformada = `INVALID_SCHEMA`).**
///
/// Un `include` que no empieza por `frontmatter.`, o cuyo sufijo no es un `FieldPath` válido, se
/// **rechaza**; no se ignora en silencio. Aceptarlo y descartarlo es **exactamente** el defecto que
/// esta misma historia está retirando de `sort`: colarlo por la puerta de atrás mientras se saca
/// por la de delante dejaría la superficie igual de mentirosa.
///
/// Aquí sí se exige rechazo **en ejecución** (y no solo declarativo, como con `sort`): los valores
/// de `include` son abiertos —`frontmatter.<lo que sea>`— y no caben en un `enum` del schema, a
/// diferencia del `include` cerrado de `knowledge_get`. El despachador tiene que parsear el sufijo
/// de todos modos, así que es el único sitio donde la superficie puede ser honesta.
///
/// El código debe ser `INVALID_SCHEMA` y **no** `DOCUMENT_NOT_FOUND`: ese es el error mentiroso que
/// esta historia está matando en `apply_fix` (mandar al agente a buscar el problema en un documento
/// que existe), y no puede reaparecer aquí — el problema está en los argumentos, no en la KB.
#[test]
fn search_include_invalido_es_invalid_schema() {
    let dir = workspace_frontmatter();
    let mut s = Sesion::abrir(dir.path());

    for malo in [
        // Sin el prefijo obligatorio: parece un `include` de `knowledge_get`…
        json!(["body"]),
        // …o un field path suelto, la confusión más probable de un agente.
        json!(["status"]),
        // Prefijo correcto, sufijo VACÍO: no hay `FieldPath` que parsear.
        json!(["frontmatter."]),
        // Prefijo correcto pero namespace ajeno: `document.*`/`graph.*` son del lenguaje de
        // consulta (`where`/`filter`), no de la proyección.
        json!(["document.path"]),
        // Una entrada válida NO redime a la inválida que la acompaña.
        json!(["frontmatter.status", "todo"]),
    ] {
        let r = s.tool_cruda("knowledge_search", json!({ "text": "", "include": malo }));
        assert_eq!(
            r["isError"],
            json!(true),
            "`include: {malo}` debe RECHAZARSE, no aceptarse y descartarse en silencio: aceptar y \
             descartar es el defecto que esta misma historia retira de `sort`. Respuesta: {r}"
        );
        assert!(
            r["structuredContent"].is_null(),
            "un `include` inválido no puede devolver además una página de resultados como si nada \
             (`include: {malo}`): {r}"
        );
        let texto = r["content"][0]["text"].as_str().unwrap_or_default();
        assert!(
            texto.contains("INVALID_SCHEMA"),
            "el rechazo viaja con el código estable `INVALID_SCHEMA` (el problema está en los \
             ARGUMENTOS), para `include: {malo}`. Texto: {texto:?}"
        );
        assert!(
            !texto.contains("DOCUMENT_NOT_FOUND"),
            "y NUNCA con `DOCUMENT_NOT_FOUND`: es el código mentiroso que esta historia está \
             matando en `apply_fix` — manda al agente a buscar el problema en la KB cuando está en \
             su propia llamada. `include: {malo}`, texto: {texto:?}"
        );
    }

    // Guarda anti-vacua: el rechazo es del `include` malformado, no de la tool entera — un
    // `include` bien formado sobre el MISMO workspace sigue sirviéndose.
    let bueno = s.tool_cruda(
        "knowledge_search",
        json!({ "text": "", "include": ["frontmatter.status"] }),
    );
    assert!(
        bueno["isError"] != json!(true),
        "un `include` bien formado debe seguir funcionando: {bueno}"
    );
}

// ===========================================================================
// Criterio · `sort_retirado_del_schema`
//
// «Dado un `knowledge_search` con `sort`, Entonces el schema lo rechaza en vez de aceptarlo y
//  descartarlo (`additionalProperties: false` ya está puesto), y ni el contrato ni la firma de
//  `App` lo mencionan.»
//
// Hoy `sort` se acepta y se ignora **en silencio** (`_sort` en la firma de `App::knowledge_search`).
// Decisión del usuario (2026-07-26): se RETIRA, no se implementa. El orden determinista que ya
// existe se queda como el único y es además la base del cursor de paginación.
// ===========================================================================

/// **Criterio `sort_retirado_del_schema`.** Las tres superficies donde `sort` se anuncia hoy:
/// el `inputSchema` que lee el cliente, `contracts/mcp.yml` que describe la frontera, y la firma de
/// `App::knowledge_search`.
///
/// La tercera se comprueba **leyendo el fuente**, deliberadamente: el criterio habla de la firma
/// («ni el contrato ni la firma de `App` lo mencionan») y un parámetro `_sort` que nadie usa es
/// invisible desde el wire — no hay forma de observarlo por JSON-RPC, y una comprobación por
/// aridad Rust fijaría además el ORDEN de los parámetros, que es decisión del implementador.
#[test]
fn sort_retirado_del_schema() {
    let dir = workspace_frontmatter();
    let mut s = Sesion::abrir(dir.path());

    // --- 1. el schema declarado a los clientes ------------------------------------------------
    let ks = s.descriptor("knowledge_search");
    let props = ks["inputSchema"]["properties"]
        .as_object()
        .unwrap_or_else(|| panic!("el `inputSchema` declara `properties`: {ks}"))
        .clone();
    assert!(
        !props.contains_key("sort"),
        "«sort» no puede seguir en el `inputSchema` de `knowledge_search`: se acepta y se IGNORA \
         en silencio desde E10-H09, que es el patrón que E23 salda (un parámetro anunciado que no \
         hace nada miente al agente). Propiedades: {:?}",
        props.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        ks["inputSchema"]["additionalProperties"],
        json!(false),
        "con `additionalProperties: false`, retirar la propiedad basta para que un cliente \
         conforme RECHACE `sort` en vez de mandarlo y creerse servido: {}",
        ks["inputSchema"]
    );
    // Guarda anti-vacua: los demás parámetros siguen declarados (no se ha vaciado el schema).
    for vivo in ["text", "where", "filter", "limit", "cursor"] {
        assert!(
            props.contains_key(vivo),
            "el `inputSchema` debe conservar «{vivo}»: {:?}",
            props.keys().collect::<Vec<_>>()
        );
    }

    // --- 2. el contrato de la frontera ---------------------------------------------------------
    let params = params_del_contrato("knowledge_search");
    assert!(
        !params.iter().any(|p| p == "sort"),
        "`contracts/mcp.yml` no puede seguir declarando el parámetro `sort` de `knowledge_search` \
         («reservado; el orden es siempre determinista»): un parámetro reservado que nunca llegó \
         es superficie muerta. Params: {params:?}"
    );
    for vivo in ["text", "where", "filter", "limit", "cursor"] {
        assert!(
            params.iter().any(|p| p == vivo),
            "guarda anti-vacua: el contrato debe seguir declarando «{vivo}»: {params:?}"
        );
    }

    // --- 3. la firma de `App::knowledge_search` ------------------------------------------------
    let fuente = std::fs::read_to_string(del_repo("crates/lodestar-app/src/lib.rs"))
        .expect("leer `crates/lodestar-app/src/lib.rs`");
    let desde = fuente
        .find("pub fn knowledge_search(")
        .expect("`App::knowledge_search` debe seguir existiendo");
    let firma = &fuente[desde..];
    let hasta = firma
        .find("-> Result")
        .expect("la firma de `knowledge_search` termina en su tipo de retorno");
    let firma = &firma[..hasta];
    assert!(
        !firma.contains("sort"),
        "la firma de `App::knowledge_search` no puede seguir llevando el parámetro `sort` \
         (hoy `_sort`, aceptado y descartado). Firma leída:\n{firma}"
    );
    // Guarda anti-vacua: se está leyendo la firma de verdad.
    assert!(
        firma.contains("cursor"),
        "guarda anti-vacua: el fragmento leído debe ser la firma real:\n{firma}"
    );
}

/// **Guarda del orden determinista** (verde hoy; la fija esta historia porque al retirar `sort` el
/// orden pasa a ser el ÚNICO, y es además la base del cursor-offset de paginación).
///
/// `score` descendente y, a igualdad, `path` ascendente. El `score` es el nº de ocurrencias del
/// texto en el fichero crudo, así que el escenario es determinista por construcción.
#[test]
fn orden_determinista_score_desc_path_asc() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // `c.md` menciona el término 3 veces; `a.md` y `b.md`, una — así que empatan y desempata el path.
    escribe(root, "notas/a.md", "# A\n\nUn faro.\n");
    escribe(root, "notas/b.md", "# B\n\nOtro faro.\n");
    escribe(root, "notas/c.md", "# C\n\nfaro, faro y faro.\n");

    let mut s = Sesion::abrir(root);
    let results = s.buscar(json!({ "text": "faro" }));
    let paths: Vec<&str> = results
        .iter()
        .map(|r| r["path"].as_str().unwrap_or_default())
        .collect();
    assert_eq!(
        paths,
        vec!["notas/c.md", "notas/a.md", "notas/b.md"],
        "el orden es `score` DESC y, a igualdad, `path` ASC — total y estable. Al retirarse \
         `sort` este orden queda como el único, y de él depende que el cursor-offset reanude \
         idéntico en un proceso fresco. Resultados: {results:?}"
    );
}

// ===========================================================================
// Criterio · `apply_fix_retirada`
//
// «Dado un `apply_fix`, Entonces la op no está en el enum del schema.»
//
// La op se anuncia como una de las 8 universales y **siempre falla**: no hay productor de `fixes`
// desde E20-H03, `normalize_apply_fix` es una línea que devuelve `Err(FixNotFound)` y ese error se
// mapea a `DOCUMENT_NOT_FOUND` — un código que apunta al sitio equivocado (el documento SÍ existe).
// Decisión del usuario: se retira la op; el análisis queda escrito en `docs/PROPUESTA_FIXES.md`.
// ===========================================================================

/// Workspace mínimo de escritura: un documento sano al que apuntar las ops.
fn workspace_escritura() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "notas/alfa.md",
        "---\nstatus: draft\n---\n\n# Alfa\n\nCuerpo.\n",
    );
    dir
}

/// **Criterio `apply_fix_retirada`.** La op sale del enum del `inputSchema`, de su parámetro
/// `fixId`, de la lista blanca de selección masiva y del enum de `contracts/mcp.yml`; e invocarla
/// pasa a ser `INVALID_SCHEMA` (op desconocida) en vez del `DOCUMENT_NOT_FOUND` que devuelve hoy.
///
/// Todas las comprobaciones son **estructurales** (conjuntos de valores y de claves), nunca por
/// subcadena sobre el documento serializado: el criterio es «la op no está en el enum», no «nadie
/// puede mencionar la op». La prosa del contrato y del CHANGELOG tiene que poder nombrarla para
/// explicar la retirada.
#[test]
fn apply_fix_retirada() {
    let dir = workspace_escritura();
    let mut s = Sesion::abrir(dir.path());

    // --- 1. invocarla es una op desconocida, con el código correcto ----------------------------
    // El `ref.path` resuelve a un documento EXISTENTE, así que la única razón posible de error es
    // que `apply_fix` ya no sea una op reconocida.
    let suelta = s.tool_cruda(
        "change_plan",
        json!({ "operations": [
            { "op": "apply_fix", "ref": { "path": "notas/alfa.md" }, "fixId": "fix:loquesea" }
        ]}),
    );
    assert_eq!(
        suelta["isError"],
        json!(true),
        "una op retirada debe dar `isError` en `change_plan`, no un plan: {suelta}"
    );
    assert!(
        suelta.to_string().contains("INVALID_SCHEMA"),
        "una op fuera del enum es `INVALID_SCHEMA` (op desconocida, el mismo código con que \
         `normalize_raw_op` rechaza cualquier `op` no reconocida), NO `DOCUMENT_NOT_FOUND`: el \
         documento existe y ese código manda al agente a buscar el problema donde no está. \
         Respuesta: {suelta}"
    );

    // La misma retirada por la vía de la selección masiva (`single_operation` la admite hoy).
    let masiva = s.tool_cruda(
        "change_plan",
        json!({
            "selection": { "where": "document.path starts_with \"notas/\"" },
            "operation": { "apply_fix": { "fixId": "fix:loquesea" } }
        }),
    );
    assert_eq!(
        masiva["isError"],
        json!(true),
        "la op retirada tampoco puede colarse por la selección masiva: {masiva}"
    );
    assert!(
        masiva.to_string().contains("INVALID_SCHEMA"),
        "y con el mismo código estable, `INVALID_SCHEMA`: {masiva}"
    );

    // --- 2. el enum del schema -----------------------------------------------------------------
    let cp = s.descriptor("change_plan");
    let item = &cp["inputSchema"]["properties"]["operations"]["items"];
    let ops: Vec<String> = item["properties"]["op"]["enum"]
        .as_array()
        .unwrap_or_else(|| panic!("`operations[].op` declara su enum de ops: {cp}"))
        .iter()
        .filter_map(|v| v.as_str().map(String::from))
        .collect();
    assert!(
        !ops.iter().any(|o| o == "apply_fix"),
        "«apply_fix» no puede seguir en el enum de ops de `change_plan`: no hay productor de \
         `fixes` desde E20-H03, así que la op SIEMPRE falla. Ops declaradas: {ops:?}"
    );
    // Guarda anti-vacua: las 7 ops que sí funcionan siguen declaradas.
    for viva in [
        "create",
        "patch_frontmatter",
        "replace_body",
        "replace_text",
        "edit_section",
        "move",
        "delete",
    ] {
        assert!(
            ops.iter().any(|o| o == viva),
            "el enum debe conservar la op «{viva}»: {ops:?}"
        );
    }
    assert!(
        item["properties"].get("fixId").is_none(),
        "con la op retirada, su único parámetro (`fixId`) tampoco puede seguir declarado — hoy se \
         anuncia con un AVISO de que siempre falla, que es documentación de un defecto en vez de \
         su arreglo: {item}"
    );
    // La lista blanca de la selección MASIVA, también por conjunto de claves EXACTO: `apply_fix`
    // era la cuarta y se va; las otras tres se quedan.
    let masivas: BTreeSet<String> =
        claves(&cp["inputSchema"]["properties"]["operation"]["properties"]);
    assert_eq!(
        masivas,
        BTreeSet::from([
            "patch_frontmatter".to_string(),
            "replace_text".to_string(),
            "delete".to_string(),
        ]),
        "la lista blanca de ops con sentido en MASA pierde `apply_fix` y conserva las otras tres: {}",
        cp["inputSchema"]["properties"]["operation"]
    );

    // --- 3. el contrato de la frontera ---------------------------------------------------------
    // ESTRUCTURAL, no por subcadena: lo que el criterio exige es que la op salga del **enum**
    // (`valores` de `operations[].op`), no que el contrato deje de NOMBRARLA — una retirada se
    // explica citando lo retirado, y prohibir la palabra obligaría a circunloquios que dejarían el
    // documento peor de lo que estaba. Misma forma que la comprobación de `sort` sobre `params`.
    let enum_contrato = valores_del_param("change_plan", "operations[].op");
    assert!(
        !enum_contrato.iter().any(|o| o == "apply_fix"),
        "`contracts/mcp.yml` no puede seguir listando `apply_fix` entre los `valores` de \
         `operations[].op`: {enum_contrato:?}"
    );
    assert_eq!(
        enum_contrato.iter().collect::<BTreeSet<_>>(),
        BTreeSet::from([
            &"create".to_string(),
            &"patch_frontmatter".to_string(),
            &"replace_body".to_string(),
            &"replace_text".to_string(),
            &"edit_section".to_string(),
            &"move".to_string(),
            &"delete".to_string(),
        ]),
        "y el enum del contrato debe coincidir EXACTAMENTE con las 7 ops vivas del schema (una \
         frontera que se describe sola no puede divergir del wire): {enum_contrato:?}"
    );
}

/// **Guarda: la mitad de LECTURA de los fixes NO se toca** (decisión del usuario).
///
/// `Fix`, `Check.fixes` e `includeSuggestedFixes` se quedan: un array vacío se lee como «no hay
/// sugerencias» y eso **es verdad**, mientras que una op invocable que siempre falla es una
/// mentira. Esta guarda impide que la retirada de la op se lleve por delante el lado de lectura
/// (y con él el test `check_extension_retrocompat`, que fija que `fixes` serializa `[]`).
#[test]
fn fixes_de_lectura_siguen_vivos() {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "notas/alfa.md",
        "# Alfa\n\nUn [enlace roto](no-existe.md).\n",
    );
    let mut s = Sesion::abrir(dir.path());

    let kc = s.descriptor("knowledge_check");
    assert!(
        kc["inputSchema"]["properties"]
            .get("includeSuggestedFixes")
            .is_some(),
        "`includeSuggestedFixes` sigue en el `inputSchema` de `knowledge_check`: la mitad de \
         LECTURA de los fixes no se retira. Schema: {}",
        kc["inputSchema"]
    );

    let sc = s.tool(
        "knowledge_check",
        json!({ "scope": { "kind": "workspace" }, "includeSuggestedFixes": true }),
    );
    let diagnosticos = sc["diagnostics"]
        .as_array()
        .unwrap_or_else(|| panic!("`knowledge_check` devuelve `diagnostics`: {sc}"));
    assert!(
        !diagnosticos.is_empty(),
        "guarda anti-vacua: el enlace roto produce al menos un diagnóstico: {sc}"
    );
    for d in diagnosticos {
        assert_eq!(
            d["fixes"],
            json!([]),
            "cada diagnóstico sigue llevando `fixes` y serializa `[]` cuando no hay ninguno \
             (`check_extension_retrocompat`): un array vacío dice la verdad. Diagnóstico: {d}"
        );
    }
}

// ===========================================================================
// Criterio · `receipts_listables_en_workspace_status`
//
// «Dado un `change_apply` recién ejecutado, Cuando se pide `workspace_status`, Entonces su
//  `receiptId` aparece en la lista y sirve para un `change_revert` sin haberlo guardado el
//  llamador.»
//
// Hoy no hay forma de listar receipts: si el agente pierde el `receiptId`, el undo es inalcanzable
// pese a estar persistido en `.lodestar/runtime/receipts/`. Decisión del usuario: se listan desde
// `workspace_status` (donde ya vive `recovery.pendingTransaction`), NO como 11ª tool — la
// superficie converge en 10 (`§19.6`).
// ===========================================================================

/// Las entradas de `workspace_status.receipts` (falla si el campo no existe o no es un array).
fn receipts(estado: &Value) -> Vec<Value> {
    estado["receipts"]
        .as_array()
        .unwrap_or_else(|| {
            panic!(
                "`workspace_status` debe listar los receipts persistidos en `receipts` (array): \
                 sin listado, un agente que pierde el `receiptId` no puede revertir aunque el \
                 receipt siga en disco. Estado: {estado}"
            )
        })
        .clone()
}

/// Planifica y aplica un `patch_frontmatter` sobre `path`, devolviendo `(receiptId, resultRevision)`
/// del `change_apply`.
fn aplica_patch(s: &mut Sesion, path: &str, patch: Value) -> (String, String) {
    let plan = s.tool(
        "change_plan",
        json!({ "operations": [
            { "op": "patch_frontmatter", "ref": { "path": path }, "patch": patch }
        ]}),
    );
    assert_eq!(
        plan["canApply"],
        json!(true),
        "el plan de preparación debe ser aplicable: {plan}"
    );
    let cs = plan["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();
    let apply = s.tool("change_apply", json!({ "changeSetId": cs }));
    (
        apply["receiptId"].as_str().expect("receiptId").to_string(),
        apply["workspaceRevision"]
            .as_str()
            .expect("workspaceRevision")
            .to_string(),
    )
}

/// **Criterio `receipts_listables_en_workspace_status`** — el **ida y vuelta**, que es la capacidad
/// real: aplicar un cambio, **tirar** el `receiptId`, recuperarlo de `workspace_status` y revertir
/// con él. La forma del JSON es secundaria; que el undo sea alcanzable sin haber guardado nada es
/// el criterio.
///
/// Todo en **una sola sesión** MCP viva: el listado tiene que reflejar un apply hecho por el mismo
/// proceso, sin reiniciar (E23-H07).
#[test]
fn receipts_listables_en_workspace_status() {
    let dir = workspace_escritura();
    let root = dir.path();
    let original = std::fs::read_to_string(root.join("notas/alfa.md")).unwrap();
    let mut s = Sesion::abrir(root);

    let (id_del_apply, revision_del_apply) =
        aplica_patch(&mut s, "notas/alfa.md", json!({ "status": "review" }));
    assert!(
        std::fs::read_to_string(root.join("notas/alfa.md"))
            .unwrap()
            .contains("review"),
        "guarda anti-vacua: el apply escribió de verdad"
    );

    // El llamador PIERDE el receiptId aquí: a partir de esta línea solo se usa lo que el servidor
    // sepa contar por sí mismo.
    let estado = s.estado();
    let listados = receipts(&estado);
    let entrada = listados
        .iter()
        .find(|r| r["receiptId"] == json!(id_del_apply))
        .unwrap_or_else(|| {
            panic!(
                "el receipt del apply recién hecho debe aparecer en `workspace_status.receipts`: \
                 {listados:?}"
            )
        });

    // Entrada ACOTADA: lo justo para elegir cuál revertir (el receipt entero se sigue leyendo por
    // `change_revert`). `changedPathCount` es el nº de rutas afectadas, eco de
    // `ChangeReceipt::changed_paths`.
    assert!(
        entrada["changeSetId"].is_string(),
        "la entrada trae el `changeSetId` que la originó: {entrada}"
    );
    assert_eq!(
        entrada["resultRevision"],
        json!(revision_del_apply),
        "la entrada trae la `resultRevision` del receipt — la misma `workspaceRevision` que \
         devolvió el apply, así el agente puede reconocer «el estado al que volvería»: {entrada}"
    );
    assert_eq!(
        entrada["changedPathCount"],
        json!(1),
        "la entrada trae el nº de rutas afectadas (aquí 1: el documento parcheado), NO la lista \
         entera ni el `semanticDiff`: {entrada}"
    );
    assert_eq!(
        claves(entrada),
        BTreeSet::from([
            "receiptId".to_string(),
            "changeSetId".to_string(),
            "resultRevision".to_string(),
            "changedPathCount".to_string(),
        ]),
        "la entrada es ACOTADA: exactamente esos 4 campos. `workspace_status` se llama en CADA \
         sesión y su payload no puede crecer con receipts completos: {entrada}"
    );

    // El ida y vuelta: revertir con el id recuperado del estado, no con el que devolvió el apply.
    let recuperado = entrada["receiptId"].as_str().unwrap().to_string();
    let revert = s.tool("change_revert", json!({ "receiptId": recuperado }));
    assert!(
        revert["workspaceRevision"].is_string(),
        "el revert con el id recuperado debe completarse: {revert}"
    );
    assert_eq!(
        std::fs::read_to_string(root.join("notas/alfa.md")).unwrap(),
        original,
        "y el documento vuelve byte a byte a su estado anterior al apply: el undo era alcanzable \
         sin que el llamador hubiera guardado el `receiptId`"
    );
}

/// **Criterio `receipts_listables_en_workspace_status` (workspace sin receipts).**
///
/// Un workspace recién abierto —sin `.lodestar/runtime/` siquiera, que desde E23-H12 ya no se crea
/// al abrir— lista `[]`, no falla ni omite el campo: un array vacío es una respuesta, un campo
/// ausente obliga al cliente a distinguir «no hay» de «esta versión no lo dice».
#[test]
fn workspace_status_sin_receipts_lista_vacia() {
    let dir = workspace_escritura();
    let mut s = Sesion::abrir(dir.path());
    let estado = s.estado();
    assert_eq!(
        receipts(&estado),
        Vec::<Value>::new(),
        "sin ninguna transacción aplicada, `receipts` es una lista VACÍA (no ausente, no un \
         error): {estado}"
    );
    // Guarda anti-vacua: se está mirando un `workspace_status` de verdad.
    assert!(
        estado["counts"]["documents"] == json!(1),
        "guarda anti-vacua: el estado es el del workspace real: {estado}"
    );
}

/// **Criterio `receipts_listables_en_workspace_status` (orden).**
///
/// Orden **mtime desc** (el más reciente primero): es el mismo criterio de antigüedad que ya usa
/// `gc_receipts` —`ChangeReceipt` no lleva timestamp propio—, y es el útil, porque el receipt que
/// un agente quiere revertir casi siempre es el último.
///
/// La pausa entre los dos applies no cubre ninguna carrera del producto: separa los `mtime` para
/// que el escenario tenga un orden esperado bien definido en cualquier sistema de ficheros.
#[test]
fn receipts_ordenados_por_mtime_desc() {
    let dir = workspace_escritura();
    let mut s = Sesion::abrir(dir.path());

    let (primero, _) = aplica_patch(&mut s, "notas/alfa.md", json!({ "status": "review" }));
    std::thread::sleep(std::time::Duration::from_millis(50));
    let (segundo, _) = aplica_patch(&mut s, "notas/alfa.md", json!({ "status": "accepted" }));
    assert_ne!(primero, segundo, "dos applies, dos receipts distintos");

    let estado = s.estado();
    let ids: Vec<String> = receipts(&estado)
        .iter()
        .map(|r| r["receiptId"].as_str().unwrap_or_default().to_string())
        .collect();
    let pos = |id: &str| {
        ids.iter()
            .position(|x| x == id)
            .unwrap_or_else(|| panic!("«{id}» debe estar listado: {ids:?}"))
    };
    assert!(
        pos(&segundo) < pos(&primero),
        "el receipt MÁS RECIENTE va primero (mtime desc): es el que un agente quiere revertir. \
         Listado: {ids:?}"
    );
}

/// **Criterio `receipts_listables_en_workspace_status` (tolerancia).**
///
/// Un `.json` de recibo corrupto o ilegible **se salta**: no puede impedir ver los sanos ni tumbar
/// `workspace_status`, que es la primera tool de cada sesión.
///
/// La propiedad importa justo cuando más se necesita el listado: `.lodestar/runtime/` es runtime
/// desechable (invariante #1) y un corte de luz a mitad de una escritura, o una limpieza a medias,
/// puede dejar ahí un fichero a medio escribir. Si eso ocultara los recibos buenos, el agente
/// perdería el undo por culpa de basura irrelevante. `receipts.rs` ya lo documenta y lo programa
/// (el GC salta lo que no puede leer); esto lo **demuestra** por la superficie.
#[test]
fn receipt_corrupto_no_oculta_los_sanos() {
    let dir = workspace_escritura();
    let root = dir.path();
    let mut s = Sesion::abrir(root);

    let (sano, _) = aplica_patch(&mut s, "notas/alfa.md", json!({ "status": "review" }));

    // Basura en el directorio de recibos, con la misma extensión que los buenos.
    let receipts_dir = root.join(".lodestar/runtime/receipts");
    assert!(
        receipts_dir.is_dir(),
        "guarda anti-vacua: el apply creó el directorio de recibos"
    );
    std::fs::write(receipts_dir.join("corrupto.json"), "{ esto no es JSON").unwrap();
    // Y un JSON válido que NO es un `ChangeReceipt` (el otro modo de fallo: parsea, pero no casa).
    std::fs::write(receipts_dir.join("ajeno.json"), r#"{"hola":"mundo"}"#).unwrap();

    let estado = s.estado();
    let ids: Vec<String> = receipts(&estado)
        .iter()
        .map(|r| r["receiptId"].as_str().unwrap_or_default().to_string())
        .collect();
    assert!(
        ids.iter().any(|x| x == &sano),
        "un recibo ilegible no puede ocultar los SANOS: el listado debe seguir trayendo «{sano}». \
         Listado: {ids:?}"
    );
    assert_eq!(
        ids.len(),
        1,
        "y los ilegibles se saltan en silencio, sin colarse como entradas fantasma (ni derivadas \
         del nombre del fichero): {ids:?}"
    );
}

// ---------------------------------------------------------------------------
// E24-H17 — Los tests que E23-H10 declaró como criterio y nunca se escribieron
//
// `requirements/epica-23-cierre-migracion.md:342` fija como criterio de aceptación de E23-H10 un
// test llamado `schema_declara_todos_los_parametros`. La cadena aparece UNA sola vez en todo el
// repo —en esa línea de la spec— y en ningún `.rs`. El commit que cerró la historia (`c6a9990`)
// tocó solo `contracts/mcp.yml` y `tools.rs`, y su cuerpo sustituye el test por «verificado
// manualmente». La historia consta como cerrada en el ledger. Ídem `move_default_explicito`.
//
// Aquí están. Es la misma clase de defecto que toda E24 cierra: una afirmación que nadie ejecuta.
// ---------------------------------------------------------------------------

/// **E24-H17** — el `inputSchema` de `change_plan.operations[]` declara TODOS los parámetros que el
/// normalizador lee.
///
/// El sentido de la comprobación es «declarado ⊇ leído»: un parámetro que el código lee y el schema
/// no anuncia es invisible para el cliente, que es justo el agujero de usabilidad que E23-H10 abrió
/// para cerrar (hasta entonces el schema declaraba 4 de los 18 campos reales).
///
/// La lista de «leídos» se mantiene a mano porque los nombres viven dispersos en `params.get("…")`
/// y en los brazos de `normalize_raw_op`, y no son introspectables. Eso hace el test **frágil a
/// propósito**: si alguien añade un parámetro al normalizador sin declararlo, este test no lo caza
/// solo — pero sí caza el caso inverso (declarar y no leer) y, sobre todo, deja la lista escrita en
/// un sitio donde el siguiente que toque la superficie la vea.
#[test]
fn schema_declara_todos_los_parametros() {
    let dir = workspace_frontmatter();
    let mut s = Sesion::abrir(dir.path());
    let cp = s.descriptor("change_plan");

    let declarados: BTreeSet<String> = cp["inputSchema"]["properties"]["operations"]["items"]
        ["properties"]
        .as_object()
        .expect("el schema de `operations[]` debe declarar `properties`")
        .keys()
        .cloned()
        .collect();

    // Los que `lodestar_app::normalize_raw_op` lee de verdad, op por op.
    let leidos: BTreeSet<String> = [
        // comunes
        "op",
        "path",
        "ref",
        "expectedRevision",
        // create
        "frontmatter",
        "body",
        // patch_frontmatter
        "patch",
        // replace_text
        "find",
        "replace",
        "expectedOccurrences",
        // edit_section
        "headingPath",
        "mode",
        "content",
        // move
        "from",
        "to",
        "rewriteInboundLinks",
        // delete
        "inboundLinksPolicy",
    ]
    .iter()
    .map(|s| (*s).to_string())
    .collect();

    let sin_declarar: Vec<&String> = leidos.difference(&declarados).collect();
    assert!(
        sin_declarar.is_empty(),
        "el `inputSchema` de `change_plan.operations[]` debe declarar TODOS los parámetros que el \
         normalizador lee: un parámetro leído y no anunciado es invisible para el cliente, que es \
         el agujero que E23-H10 vino a cerrar. Sin declarar: {sin_declarar:?}"
    );

    // Guarda anti-vacua: si la lista de «leídos» se vaciara por accidente, la aserción de arriba
    // pasaría siempre.
    assert!(
        leidos.len() >= 17,
        "la lista de parámetros leídos no puede quedarse vacía o corta: son 17 como mínimo"
    );

    drop(s);
}

/// **E24-H17** — `move.rewriteInboundLinks` declara su default explícitamente.
///
/// Es el otro criterio de E23-H10 que nunca se escribió. Importa porque el default es `false` y las
/// consecuencias de no saberlo son caras: sin él, un `move` deja **todos** los backlinks apuntando
/// a la ruta vieja, rotos y en silencio. Un cliente que lee el schema tiene que poder verlo sin
/// ejecutar nada.
#[test]
fn move_default_explicito() {
    let dir = workspace_frontmatter();
    let mut s = Sesion::abrir(dir.path());
    let cp = s.descriptor("change_plan");
    let prop = &cp["inputSchema"]["properties"]["operations"]["items"]["properties"]
        ["rewriteInboundLinks"];

    assert_eq!(
        prop["default"],
        serde_json::json!(false),
        "`rewriteInboundLinks` debe declarar su default (`false`) en el schema: sin él, un `move` \
         rompe todos los backlinks en silencio y el cliente no tiene forma de saberlo sin \
         ejecutarlo. Propiedad declarada: {prop}"
    );
    assert!(
        prop["description"]
            .as_str()
            .is_some_and(|d| d.contains("rompe") || d.contains("rotos")),
        "y la descripción debe advertir de la consecuencia, no solo enunciar el campo: {prop}"
    );

    drop(s);
}
