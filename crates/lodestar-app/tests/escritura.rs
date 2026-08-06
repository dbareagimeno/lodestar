//! **E15-H09** — La política de escritura respeta el descubrimiento.
//!
//! Fase ROJA (TDD). Origen: hallazgo MAYOR-1 del juez ciego de E15-H07. La fuente normativa es
//! `docs/REFACTOR_PHASE_2.md §Principio 8 (Seguridad de escritura)`: *«Ninguna operación debe: …
//! **escribir sobre archivos excluidos** …»*.
//!
//! ## El defecto que fijan estos tests
//!
//! `Workspace::assert_writable` (`crates/lodestar-workspace/src/external_refs.rs:60`) solo
//! conoce `referenceRoots`/`writableRoots`. **No consulta la `DiscoveryPolicy`**, así que con la
//! config por defecto (`writableRoots` vacío = «todo escribible») acepta destinos que el
//! descubrimiento (`Workspace::discovery_policy`, E15-H07/H08) deja FUERA del inventario: el plano
//! de control `.lodestar/**` (suelo duro), lo ignorado por `.gitignore`/`.lodestarignore` y lo
//! excluido explícitamente por `discovery.exclude`.
//!
//! Un documento escrito ahí queda fuera del inventario y fuera de la revisión: invisible al grafo y
//! a `knowledge_search`, sin protección del control optimista (un segundo `create` en el mismo path
//! no vería colisión y lo sobrescribiría) y un `revert` lo trataría como creado y lo borraría.
//!
//! ## DÓNDE debe rechazarse: en `change_plan`
//!
//! Los tres criterios de aceptación de E15-H09 dicen literalmente **«Cuando se planifica, Entonces
//! se rechaza»**, y esa es la semántica que fijan estos tests: `App::change_plan` devuelve
//! `Err(ErrorCode::PermissionDenied)` y **no persiste plan alguno**. Un plan que se acepta y luego
//! revienta al aplicarse es exactamente el problema que reportó el juez: el agente recibe un
//! `semanticDiff.created` con el path colado, lo da por bueno, y el fallo llega tarde.
//!
//! **Esto NO mueve el momento de rechazo de `writableRoots`.** Hoy `writableRoots`/`referenceRoots`
//! se comprueban SOLO en el apply (`Workspace::apply_transaction`, paso 5) y hay un test de
//! benchmark que lo fija explícitamente (`crates/lodestar-mcp/tests/benchmark.rs`, escenario 13:
//! *«change_plan no valida writable, así que produce el plan; el rechazo recae en change_apply»*).
//! Si el implementador mete `assert_writable` ENTERO dentro de `change_plan`, rompe ese test. La
//! implementación tiene, por tanto, que poder preguntar **solo** por la exclusión de
//! descubrimiento en tiempo de plan (p. ej. un `Workspace::assert_discoverable(&RelPath)` que
//! `assert_writable` también invoque), y dejar la comprobación de raíces donde está.
//!
//! ## Y también en el apply (defensa en profundidad)
//!
//! `plan_valido_no_escribe_en_lo_ignorado_sobrevenido` cubre el segundo frente: el descubrimiento
//! no es config de sesión, es estado del árbol —un `.gitignore` puede aparecer entre el plan y el
//! apply sin mover la `WorkspaceRevision` (no es un `.md`), de modo que ni el control optimista ni
//! el `planHash` lo detectan—. Como el guard del único escritor ya corre en el paso 5 de la
//! transacción, folding la política de descubrimiento dentro de `assert_writable` cierra este
//! frente sin trabajo extra: es la ubicación que pide el «Alcance» de la historia.
//!
//! ## Código de error
//!
//! `PERMISSION_DENIED` (`ErrorCode::PermissionDenied`), el que ya usa el rechazo por raíces. La
//! historia lo fija y no se inventa ninguno nuevo.

use std::path::{Path, PathBuf};

use lodestar_app::App;
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::ErrorCode;
use serde_json::{json, Value};

/// Escribe un fichero dentro del workspace temporal, creando los directorios intermedios.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Un workspace mínimo y conforme: `index.md` raíz + un documento `alfa.md` que enlaza.
///
/// No escribe `.lodestar/config.yaml`: la config por defecto (`writableRoots` vacío = todo
/// escribible) es justo el escenario del defecto — sin ella, hoy nada frena la escritura fuera del
/// inventario.
fn semilla(root: &Path) {
    escribe(
        root,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Alfa](alfa.md)\n",
    );
    escribe(
        root,
        "alfa.md",
        "---\ntype: Nota\ntitle: Alfa\ndescription: Primer documento\n---\n\n# Resumen\n\ncuerpo\n",
    );
}

/// Política permisiva: el criterio a probar es el permiso de escritura, no el veredicto de
/// conformidad — sin esto un plan podría fallar por una razón distinta de la que se está fijando.
fn policy_permisiva() -> PlanPolicy {
    PlanPolicy {
        require_valid_result: false,
        allow_warnings: true,
    }
}

/// Los `.json` de plan persistidos en `.lodestar/runtime/plans/` (E12-H09).
fn planes_persistidos(root: &Path) -> Vec<PathBuf> {
    let dir = root.join(".lodestar").join("runtime").join("plans");
    let Ok(entries) = std::fs::read_dir(&dir) else {
        return Vec::new();
    };
    entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect()
}

/// Asevera el contrato completo de un rechazo en tiempo de plan:
///   1. `change_plan` devuelve `Err(ErrorCode::PermissionDenied)` (código estable de la historia);
///   2. NO persiste ningún plan (un plan rechazado no puede quedar aplicable después);
///   3. el `.md` no aparece en disco (change_plan no escribe canónico, pero se ancla igualmente).
fn asevera_rechazo_al_planificar(app: &App, root: &Path, ops: &Value, destino: &str) {
    let resultado = app.change_plan(None, ops, policy_permisiva());

    let err = match resultado {
        Err(e) => e,
        Ok(plan) => panic!(
            "planificar una escritura sobre «{destino}» (EXCLUIDO del descubrimiento) debe \
             rechazarse con PERMISSION_DENIED, pero el plan se aceptó: changeSetId={:?}, \
             semanticDiff.created={:?}, impact={:?}",
            plan.change_set_id, plan.semantic_diff.created, plan.impact
        ),
    };
    assert_eq!(
        err.code,
        ErrorCode::PermissionDenied,
        "el rechazo de una escritura sobre «{destino}» debe llevar el código estable \
         PERMISSION_DENIED (`REFACTOR_PHASE_2 §Principio 8`); era: {err:?} ({})",
        err.code.as_str()
    );

    let planes = planes_persistidos(root);
    assert!(
        planes.is_empty(),
        "un plan rechazado por PERMISSION_DENIED no debe quedar persistido en \
         `.lodestar/runtime/plans/` (sería aplicable después): {planes:?}"
    );
    assert!(
        !root.join(destino).exists(),
        "planificar no debe materializar «{destino}» en disco"
    );
}

/// Control de NO vacuidad: el mismo `create`, pero a un path DESCUBIERTO, sí planifica. Sin esto,
/// una implementación que rechazara todo pasaría los tres criterios.
fn control_un_plan_normal_si_funciona(app: &App) {
    // E23-H02: el `create` ya no lleva `type`/`title` (parámetros privilegiados retirados); aquí
    // eran incidentales, así que se retiran sin sustituto.
    let ops = json!([
        { "op": "create", "path": "beta.md", "body": "# Resumen\n\ncuerpo visible\n" },
    ]);
    app.change_plan(None, &ops, policy_permisiva()).expect(
        "control de no vacuidad: crear un documento en una ruta DESCUBIERTA debe seguir \
         planificándose sin problemas",
    );
}

/// **Criterio 1** (`no_se_escribe_en_el_plano_de_control`) — **Dado** un `change_plan` que crea
/// `.lodestar/colado.md`, **Cuando** se planifica, **Entonces** se rechaza con `PERMISSION_DENIED`.
///
/// `.lodestar/**` es el **suelo duro** del descubrimiento (`CONTROL_PLANE_EXCLUDE`, E15-H07/H08):
/// la config puede añadir exclusiones pero nunca quitar esta. Es además el caso más grave, porque
/// `workspace_revision` excluye `.lodestar/` por decisión D5: un `.md` colado ahí sería
/// estructuralmente **ciego al control optimista**.
#[test]
fn no_se_escribe_en_el_plano_de_control() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    semilla(root);

    let app = App::open(root).expect("el workspace temporal debe abrir");

    let ops = json!([
        { "op": "create", "path": ".lodestar/colado.md",
          "body": "# Colado\n\ndocumento fuera del inventario\n" },
    ]);
    asevera_rechazo_al_planificar(&app, root, &ops, ".lodestar/colado.md");

    control_un_plan_normal_si_funciona(&app);
}

/// **Criterio 2** (`no_se_escribe_en_lo_ignorado`) — **Dado** un `.gitignore` con `vendor/` y un
/// plan que crea `vendor/oculto.md`, **Cuando** se planifica, **Entonces** se rechaza.
///
/// La exclusión aquí no viene de la config de Lodestar sino del **árbol** (`respect_gitignore`,
/// `true` por defecto): el guard tiene que consultar la política EFECTIVA de descubrimiento, no
/// una lista estática de globs.
#[test]
fn no_se_escribe_en_lo_ignorado() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    semilla(root);
    // `.gitignore` del usuario ANTES de abrir: `Workspace::open` (`ensure_gitignore`) preserva el
    // contenido propio y solo añade su bloque gestionado.
    escribe(root, ".gitignore", "vendor/\n");
    // Un `.md` real bajo `vendor/`: ancla que la exclusión es efectiva (ver aserción de abajo).
    escribe(
        root,
        "vendor/existente.md",
        "---\ntype: Nota\ntitle: Vendorizado\ndescription: de un tercero\n---\n\n# V\n\ncuerpo\n",
    );

    let app = App::open(root).expect("el workspace temporal debe abrir");

    // Precondición (test no vacuo): `vendor/` está REALMENTE fuera del inventario.
    let inventario = app
        .workspace()
        .document_set()
        .expect("el workspace debe cargarse")
        .files()
        .keys()
        .map(|p| p.as_str().to_string())
        .collect::<Vec<_>>();
    assert!(
        !inventario.iter().any(|p| p.starts_with("vendor/")),
        "precondición: `.gitignore` con `vendor/` debe dejar `vendor/` fuera del inventario; \
         inventario: {inventario:?}"
    );

    let ops = json!([
        { "op": "create", "path": "vendor/oculto.md",
          "body": "# Oculto\n\ndocumento fuera del inventario\n" },
    ]);
    asevera_rechazo_al_planificar(&app, root, &ops, "vendor/oculto.md");

    control_un_plan_normal_si_funciona(&app);
}

/// **Criterio 3** (`move_a_excluido_se_rechaza`) — **Dado** un `move_document` cuyo destino cae en
/// una ruta excluida, **Cuando** se planifica, **Entonces** se rechaza.
///
/// Cubre el **destino** de una operación de estructura (no solo el `path` de un `create`): un
/// `move` normaliza a varias operaciones y su `to` es una escritura como cualquier otra. La
/// exclusión se declara aquí por `discovery.exclude` del `config.yaml` (E15-H08), el tercer
/// mecanismo de exclusión, distinto del suelo duro y del `.gitignore` de los otros dos criterios.
///
/// Un `move` a un destino excluido es, además, la variante más destructiva del defecto: el origen
/// `alfa.md` SÍ se borraría del inventario, así que el documento no quedaría «solo invisible»,
/// desaparecería del workspace.
#[test]
fn move_a_excluido_se_rechaza() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    semilla(root);
    escribe(
        root,
        ".lodestar/config.yaml",
        "discovery:\n  exclude: [\"archivo/**\"]\n",
    );

    let app = App::open(root).expect("el workspace temporal debe abrir");

    // Precondición: la política efectiva excluye `archivo/**`.
    assert!(
        app.workspace()
            .discovery_policy()
            .exclude
            .iter()
            .any(|g| g == "archivo/**"),
        "precondición: la política efectiva debe llevar la exclusión declarada `archivo/**`; era: \
         {:?}",
        app.workspace().discovery_policy().exclude
    );

    let ops = json!([
        { "op": "move", "from": "alfa.md", "to": "archivo/alfa.md", "rewriteInboundLinks": true },
    ]);
    asevera_rechazo_al_planificar(&app, root, &ops, "archivo/alfa.md");

    // El origen sigue en disco: un move rechazado no puede haber empezado a moverse.
    assert!(
        root.join("alfa.md").is_file(),
        "un `move` rechazado debe dejar el documento de origen intacto en disco"
    );

    control_un_plan_normal_si_funciona(&app);
}

/// **Defensa en profundidad (apply)** — el descubrimiento es estado del ÁRBOL, no config de
/// sesión: puede cambiar entre el plan y el apply.
///
/// **Dado** un plan válido que crea `vendor/oculto.md` (sin exclusión en el momento de planificar),
/// **Cuando** aparece un `.gitignore` con `vendor/` y se aplica, **Entonces** el apply se rechaza
/// con `PERMISSION_DENIED` y no escribe nada.
///
/// Ni el control optimista ni el `planHash` cazan esto: un `.gitignore` no es un `.md`, así que
/// añadirlo no mueve la `WorkspaceRevision` y el plan sigue «vigente». El único punto donde se
/// puede parar es el guard del único escritor (paso 5 de `Workspace::apply_transaction`) — por eso
/// el «Alcance» de la historia sitúa la consulta a la `DiscoveryPolicy` dentro de
/// `assert_writable`, y no solo en la fachada de plan.
#[test]
fn plan_valido_no_escribe_en_lo_ignorado_sobrevenido() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    semilla(root);

    let app = App::open(root).expect("el workspace temporal debe abrir");

    // (1) `vendor/` todavía NO está excluido: el plan es legítimo y se acepta.
    let ops = json!([
        { "op": "create", "path": "vendor/oculto.md", "body": "# Oculto\n\ncuerpo\n" },
    ]);
    let plan = app
        .change_plan(None, &ops, policy_permisiva())
        .expect("sin exclusión vigente, planificar `vendor/oculto.md` debe funcionar");

    // (2) Entre el plan y el apply aparece la exclusión. No mueve la `WorkspaceRevision` (no es un
    //     `.md`), así que el plan sigue formalmente vigente.
    escribe(root, ".gitignore", "vendor/\n");

    // (3) El apply tiene que rechazarlo por el guard del único escritor.
    let err = match app.change_apply(&plan.change_set_id, None) {
        Err(e) => e,
        Ok(resultado) => panic!(
            "aplicar un plan cuyo destino quedó EXCLUIDO del descubrimiento debe rechazarse con \
             PERMISSION_DENIED, pero publicó: changedPaths={:?}",
            resultado.changed_paths
        ),
    };
    assert_eq!(
        err.code,
        ErrorCode::PermissionDenied,
        "el guard del único escritor debe rechazar un destino excluido con PERMISSION_DENIED; \
         era: {} ({err:?})",
        err.code.as_str()
    );

    // (4) Y no escribió nada: ni el `.md` colado, ni el directorio que lo contendría.
    assert!(
        !root.join("vendor/oculto.md").exists(),
        "un apply rechazado por PERMISSION_DENIED no debe materializar el documento excluido"
    );
}

// ===========================================================================
// E23-H02 — `create` sin residuo OKF, POR LA FACHADA Y HASTA EL DISCO
// (`ARCHITECTURE.md §20.2` invariante 3, `§20.4`; `requirements/epica-23-cierre-migracion.md`).
// Fase ROJA.
//
// El síntoma de la historia es lo que queda ESCRITO en disco tras `change_plan` + `change_apply`:
//
//     ---
//     title: nueva
//     type: ''
//     ---
//
//     # Nueva
//
// Los tests de firma pura (`plan::normalize_create`) viven en
// `crates/lodestar-core/tests/documento.rs`; estos tres cierran el camino completo por
// `App::change_plan`/`change_apply`, que es donde `normalize_raw_op` lee hoy `type`/`title` como
// campos privilegiados y donde el `frontmatter` arbitrario de la op debe empezar a viajar.
//
// ROJO esperado HOY: por ASERCIÓN (el `.md` escrito trae `title:`/`type: ''`, el `frontmatter`
// pedido se ignora y el título derivado sale del nombre de fichero en vez del H1).
// ===========================================================================

/// Aplica un `create` (una sola op) y devuelve el `.md` **tal como quedó en disco**. Falla con un
/// mensaje explícito si el plan o el apply se rechazan: un `create` de estos no tiene por qué
/// fallar, y confundir «se rechazó» con «se escribió mal» arruinaría el diagnóstico.
fn crea_y_lee(app: &App, root: &Path, op: Value) -> String {
    let plan = app
        .change_plan(None, &json!([op]), policy_permisiva())
        .expect("planificar un `create` bien formado no debe fallar");
    let ruta = plan
        .semantic_diff
        .created
        .first()
        .cloned()
        .unwrap_or_else(|| panic!("el plan de un `create` debe declarar el documento creado"));
    app.change_apply(&plan.change_set_id, None)
        .expect("aplicar un `create` bien formado no debe fallar");
    std::fs::read_to_string(root.join(ruta.as_str()))
        .unwrap_or_else(|e| panic!("el apply debe materializar «{}»: {e}", ruta.as_str()))
}

/// **Criterio 1** (`create_sin_frontmatter_no_inyecta`) — **Dado** un `create` sin `frontmatter`,
/// **Cuando** se aplica, **Entonces** el `.md` en disco **no** contiene `type:` ni `title:` ni un
/// bloque `---` vacío.
#[test]
fn create_sin_frontmatter_no_inyecta() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    semilla(root);
    let app = App::open(root).expect("el workspace temporal debe abrir");

    let raw = crea_y_lee(
        &app,
        root,
        json!({ "op": "create", "path": "notas/nueva.md", "body": "# Nueva\n" }),
    );

    assert!(
        !raw.contains("type:"),
        "crear un documento no puede inyectar la clave OKF `type` (§20.2 invariante 3: las claves \
         del frontmatter NO tienen semántica impuesta); disco =\n{raw}",
    );
    assert!(
        !raw.contains("title:"),
        "crear un documento no puede inyectar `title`: el título se DERIVA (§20.4), no se \
         materializa como metadata que el usuario no pidió; disco =\n{raw}",
    );
    assert!(
        !raw.starts_with("---"),
        "sin `frontmatter` el `.md` sale SIN bloque de frontmatter, ni siquiera uno vacío; \
         disco =\n{raw}",
    );
    assert_eq!(
        raw, "# Nueva\n",
        "el `.md` publicado debe ser EXACTAMENTE el cuerpo pedido, sin residuo",
    );
}

/// **Criterio 2** (`create_frontmatter_arbitrario`) — **Dado** un `create` con
/// `frontmatter: {estado: "borrador", tags: [a, b]}`, **Cuando** se aplica, **Entonces** el `.md`
/// lleva exactamente esas claves con sus tipos YAML.
///
/// Es el otro lado del criterio 1: retirar `type`/`title` no puede significar «el `create` ya no
/// escribe frontmatter», sino «escribe el que le pidan, arbitrario y sin claves de propina».
#[test]
fn create_frontmatter_arbitrario() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    semilla(root);
    let app = App::open(root).expect("el workspace temporal debe abrir");

    let raw = crea_y_lee(
        &app,
        root,
        json!({ "op": "create", "path": "notas/nueva.md",
                "frontmatter": { "estado": "borrador", "tags": ["a", "b"] },
                "body": "# Nueva\n" }),
    );

    let pf = lodestar_core::model::parse_frontmatter(&raw).unwrap_or_else(|| {
        panic!("un `create` CON `frontmatter` debe escribir su bloque; disco =\n{raw}")
    });

    // 1) Claves EXACTAMENTE las pedidas (ni falta ninguna ni sobra `type`/`title`).
    let claves: Vec<String> = pf
        .mapping()
        .keys()
        .filter_map(|k| k.as_str().map(str::to_string))
        .collect();
    assert_eq!(
        claves,
        vec!["estado".to_string(), "tags".to_string()],
        "el frontmatter escrito debe llevar exactamente las claves pedidas; disco =\n{raw}",
    );

    // 2) Con sus tipos YAML reales (`§20.4`: sin coerción) — `tags` sigue siendo lista.
    assert_eq!(
        pf.get_key("estado").and_then(|v| v.as_str()),
        Some("borrador"),
        "`estado` debe escribirse tal cual, como string; disco =\n{raw}",
    );
    let tags = pf
        .get_key("tags")
        .unwrap_or_else(|| panic!("`tags` debe estar en el frontmatter; disco =\n{raw}"));
    assert_eq!(
        tags.as_sequence()
            .map(|s| s.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()),
        Some(vec!["a", "b"]),
        "`tags` debe seguir siendo una LISTA con `a` y `b`, no un escalar aplanado ({tags:?}); \
         disco =\n{raw}",
    );

    // 3) Y el cuerpo pedido cierra el documento.
    assert!(
        raw.ends_with("# Nueva\n"),
        "el cuerpo pedido debe cerrar el documento; disco =\n{raw}",
    );
}

/// **Criterio 3** (`create_sin_frontmatter_titulo_derivado`) — **Dado** el documento creado sin
/// frontmatter, **Cuando** se consulta el motor, **Entonces** el título derivado sale del primer H1
/// o del nombre del fichero (`§20.4`).
///
/// **Nota del autor de tests**: el criterio nombra `knowledge_get`, pero `DocumentView`
/// (`lodestar-app`) **no proyecta `title`** — no hay tal campo en su forma de wire. La superficie
/// que sí expone el título derivado es `knowledge_search` (`SearchResult::title`, que llama a
/// `model::derived_title`), así que el criterio se verifica por ahí y por `knowledge_get` se
/// asevera lo que sí le corresponde: que el documento no tiene frontmatter que consultar. Si la
/// historia quisiera el título en `knowledge_get`, sería una ampliación de `DocumentView` que su
/// «Alcance» no pide (queda reportado).
///
/// Discriminador: el H1 (`Título del H1`) es DISTINTO del stem del fichero (`nueva`), y hoy el
/// `title: nueva` inyectado gana la cadena de `derived_title` — o sea que el rojo es visible.
#[test]
fn create_sin_frontmatter_titulo_derivado() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    semilla(root);
    let app = App::open(root).expect("el workspace temporal debe abrir");

    // (a) Con H1: el título derivado es el H1, no el nombre del fichero.
    crea_y_lee(
        &app,
        root,
        json!({ "op": "create", "path": "notas/nueva.md",
                "body": "# Título del H1\n\ncuerpo\n" }),
    );
    // (b) Sin cuerpo ni frontmatter: el título derivado cae al nombre del fichero.
    crea_y_lee(
        &app,
        root,
        json!({ "op": "create", "path": "notas/sin-cuerpo.md" }),
    );

    let resultados = app
        .knowledge_search("", None, None, &[], Some(100), None)
        .expect("buscar sobre el workspace no debe fallar")
        .results;
    let titulo = |path: &str| -> String {
        resultados
            .iter()
            .find(|r| r.path.as_str() == path)
            .unwrap_or_else(|| {
                panic!(
                    "«{path}» debe aparecer en el inventario tras crearlo; resultados = {:?}",
                    resultados
                        .iter()
                        .map(|r| r.path.as_str())
                        .collect::<Vec<_>>()
                )
            })
            .title
            .clone()
    };

    assert_eq!(
        titulo("notas/nueva.md"),
        "Título del H1",
        "sin frontmatter, el título derivado debe salir del primer H1 del cuerpo (§20.4), no de un \
         `title` inyectado ni del nombre del fichero",
    );
    assert_eq!(
        titulo("notas/sin-cuerpo.md"),
        "sin-cuerpo",
        "sin frontmatter y sin H1, el título derivado cae al nombre del fichero (§20.4)",
    );

    // Y `knowledge_get` confirma que no hay metadata que nadie pidió: el documento no trae
    // frontmatter (o trae uno vacío), nunca `title`/`type`.
    let vista = app
        .knowledge_get(
            &serde_json::from_value(json!({ "path": "notas/nueva.md" })).unwrap(),
            &["frontmatter".to_string()],
            None,
        )
        .expect("`knowledge_get` del documento creado no debe fallar");
    let claves: Vec<String> = vista
        .frontmatter
        .as_ref()
        .and_then(|v| v.as_mapping())
        .map(|m| {
            m.keys()
                .filter_map(|k| k.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    assert!(
        claves.is_empty(),
        "el documento creado sin `frontmatter` no debe exponer ni una clave de metadata; \
         claves = {claves:?}",
    );
}

// ===========================================================================
// E25-H01 — El `WRITE_CONFLICT` de la ventana `[T1, T3)` llega a la fachada
// (`requirements/epica-25-endurecimiento-escritura.md`, bloque A). Fase ROJA.
//
// `Workspace::apply_transaction` computa canónico/resultado/afectados en T1 y `publish_result`
// vuelve a leer el canónico en T3 (`crates/lodestar-workspace/src/publish.rs:104`), publicando la
// diferencia recomputada: cualquier cambio del canónico dentro de esa ventana se pisa o se borra
// sin guard, sin copia y sin journal. El arreglo aborta con `WorkspaceError::WriteConflict` antes
// del primer rename; este test fija que ese aborto **atraviesa la fachada** con su código de wire
// (`workspace_error_code` ya mapea `WriteConflict -> ErrorCode::WriteConflict`,
// `crates/lodestar-app/src/lib.rs:168`) — o sea, que el agente recibe `WRITE_CONFLICT`, terminal,
// y replanifica; no un `INTERNAL_IO_ERROR` ni, peor, un `applied: true` que no es verdad.
//
// REQUISITO DE BUILD (lo añade el implementador): `lodestar-app` **no** propaga hoy la feature
// `test-failpoints`, así que este módulo NO SE COMPILA y el criterio queda sin ejercer. Hace falta
// el passthrough de Cargo
//
//     [features]
//     test-failpoints = ["lodestar-workspace/test-failpoints"]
//
// y ampliar al crate el step de CI que hoy corre
// `cargo test -p lodestar-workspace --features test-failpoints`. (E25-H04 pide ese mismo
// passthrough para armar sus failpoints post-publicación desde `App::change_apply`.)
// ===========================================================================

#[cfg(feature = "test-failpoints")]
mod ventana_de_publicacion {
    use super::*;
    use lodestar_workspace::failpoints::{self, PuntoDeGancho};

    /// **E25-H01** · Criterio de fachada — **Dado** un `change_apply` en curso, **Cuando** otro
    /// proceso modifica dentro de la ventana `[T1, T3)` un `.md` que la transacción iba a
    /// sustituir, **Entonces** la fachada devuelve `WRITE_CONFLICT` y no publica nada.
    #[test]
    fn write_conflict_de_la_ventana_llega_a_la_fachada() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        semilla(root);

        let app = App::open(root).expect("el workspace temporal debe abrir");

        let ops = json!([
            { "op": "replace_body", "path": "alfa.md", "body": "# Resumen\n\ncuerpo del plan\n" },
        ]);
        let plan = app
            .change_plan(None, &ops, policy_permisiva())
            .expect("planificar una modificación normal debe funcionar");

        // El gancho hace de «otro proceso»: edita el `.md` afectado dentro de la ventana, cuando el
        // guard, las copias y el journal ya se ejercieron sobre el estado de T1.
        let ruta_alfa = root.join("alfa.md");
        let edicion_externa =
            "---\ntype: Nota\ntitle: Alfa\ndescription: Primer documento\n---\n\n# Resumen\n\nEDICIÓN EXTERNA\n";
        {
            let ruta = ruta_alfa.clone();
            failpoints::armar_gancho(PuntoDeGancho::AntesDePublicar, move || {
                std::fs::write(&ruta, edicion_externa)
                    .expect("el gancho debe poder editar alfa.md");
            });
        }
        let resultado = app.change_apply(&plan.change_set_id, None);
        failpoints::desarmar_ganchos();

        let err = match resultado {
            Err(e) => e,
            Ok(aplicado) => panic!(
                "aplicar sobre un canónico que cambió dentro de la ventana `[T1, T3)` debe \
                 rechazarse con WRITE_CONFLICT, pero la fachada informó de una publicación: \
                 applied={}, changedPaths={:?}",
                aplicado.applied, aplicado.changed_paths
            ),
        };
        assert_eq!(
            err.code,
            ErrorCode::WriteConflict,
            "el conflicto de la ventana debe llegar a la fachada con su código estable \
             WRITE_CONFLICT (terminal: el agente replanifica); era: {} ({err:?})",
            err.code.as_str()
        );
        assert_eq!(
            err.code.as_str(),
            "WRITE_CONFLICT",
            "y con esa cadena exacta en el wire"
        );

        // Y no publicó: el `.md` conserva la edición externa, no el cuerpo del plan.
        assert_eq!(
            std::fs::read_to_string(&ruta_alfa).unwrap(),
            edicion_externa,
            "un apply rechazado por WRITE_CONFLICT no puede haber pisado la edición de otro proceso"
        );
    }
}

// ===========================================================================
// E25-H04 — Publicar implica recibo: nada se pierde después del punto de no retorno
// (`requirements/epica-25-endurecimiento-escritura.md`, bloque B). Fase ROJA.
//
// EL DEFECTO (S5)
//
// Tras `publish_result` (`crates/lodestar-workspace/src/transaction.rs:198`) el disco YA está
// cambiado, pero quedan pasos que salen por `?`: el sellado (`transaction.rs:224-238`) y, en ESTA
// fachada, `write_receipt` (`crates/lodestar-app/src/lib.rs:1704-1706`), `gc_receipts` (`:1707-1709`)
// y `analyze` (`:1712-1715`). Cualquiera de ellos convierte una transacción **publicada** en un `Err`
// **sin recibo**: el agente concluye que no se aplicó nada y el workspace dice lo contrario. Y no hay
// salida — `change_revert` carga el recibo primero y, al no encontrarlo, responde `PLAN_EXPIRED`
// (`:1787-1790`) **para siempre**; un segundo `change_apply` del mismo plan muere con `PLAN_STALE`
// (`:1676-1681`) porque la base cambió.
//
// Los failpoints `TrasPublicarSinSellar` y `AntesDeSellar` dejan exactamente ese estado —canónico
// publicado, recibo inexistente— desde E24-H13, pero **nadie los ejerce a través de
// `App::change_apply`**: los dos se arman desde `lodestar-workspace`, por debajo de la capa que
// escribe los recibos, así que ningún test ha mirado nunca qué le pasa a `change_revert` después.
// Estos tests los ejercen por la fachada, y añaden el punto que faltaba (ver más abajo).
//
// QUÉ FIJAN ESTOS TESTS — la superficie observable, no el mecanismo
//
// La spec deja el mecanismo abierto («el recibo —o su registro durable equivalente— se escribe con el
// journal, y la recuperación por la vía COMPLETAR lo da por bueno»), así que aquí NO se asevera nada
// del layout de `.lodestar/runtime/receipts/`. Se asevera lo que un agente puede observar:
//
//   1. que el canónico está publicado (los bytes del `.md`);
//   2. que **existe un recibo** para ese `changeSetId` —visible en `workspace_status.receipts`
//      (E23-H11), que es como un agente lo encuentra si perdió el `receiptId`— y que ese recibo es
//      **coherente**: su `resultRevision` es la revisión actual del workspace, que es justo lo que
//      `change_revert` re-verifica antes de restaurar;
//   3. que `change_revert` con ese `receiptId` **funciona** y devuelve los bytes originales — la
//      única prueba de que el recibo es utilizable y no un registro decorativo;
//   4. que ningún fallo POSTERIOR a la publicación (sellado, limpieza de staging, GC) convierte el
//      apply en `Err`;
//   5. y el control anti-vacuo: un apply que falla ANTES del primer rename no deja recibo.
//
// EL SEAM NUEVO (declarado en la salida de la fase roja; aquí solo se USA)
//
// Los seis failpoints existentes viven dentro de `apply_transaction`. Falta el punto de la FACHADA
// —entre el retorno de `apply_transaction` y el recibo—, que es donde `write_receipt`/`gc_receipts`/
// `analyze` pueden perder una publicación. Se añade a la taxonomía:
//
// ```rust
// pub enum FailPoint {
//     …,
//     /// En `App::change_apply`, entre el retorno de `apply_transaction` y el recibo.
//     TrasLaTransaccionAntesDelRecibo,   // E25-H04
// }
// ```
//
// El macro `failpoint!` es interno de `lodestar-workspace`, así que la fachada lo consulta con la API
// pública del módulo (`failpoints::disparado(..)`, que **autodesarma** el punto al dispararse) — la
// forma exacta está documentada en el rustdoc de la variante. Los tests se apoyan en ese autodesarme
// para comprobar que el punto **se ejerció de verdad**: si tras el `change_apply` sigue armado, nadie
// lo consultó y el escenario habría pasado vacuamente.
//
// ROJO ESPERADO HOY
// - `publicar_implica_recibo`: ROJO en los tres puntos. Con los dos de `apply_transaction` la
//   transacción publica y sale por `Err` sin que el recibo se escriba nunca; con el de la fachada,
//   además, hoy NADIE lo consulta (el `assert` de «se ejerció» lo destapa).
// - `tras_fallo_post_publicacion_el_revert_funciona`: ROJO, `PLAN_EXPIRED`.
// - `el_cierre_no_convierte_un_apply_publicado_en_error`: ROJO, el apply publicado devuelve `Err`.
// - `el_gc_de_otro_proceso_no_impide_revertir_tras_el_fallo_post_publicacion`: ROJO (el hueco que
//   dejó anotado el juez de E25-H03: la ventana `[sellado, recibo)` no está cubierta por el lock).
// - `un_apply_no_publicado_no_deja_recibo`: **control anti-vacuo**, PASA YA HOY y tiene que seguir
//   pasando. No es vacuo: prohíbe que el arreglo consista en escribir recibos siempre. Con el recibo
//   persistido junto al journal, los dos escenarios que ejerce —caída con journal `prepared` y cero
//   renames, y aborto de ventana de E25-H01/H02— pasan por el punto donde el recibo YA existiría, así
//   que el arreglo tiene que retirarlo (o no darlo por bueno) cuando la publicación no llega a
//   ocurrir: si no, `change_revert` restauraría las copias de T1 sobre una edición externa.
// ===========================================================================

#[cfg(feature = "test-failpoints")]
mod recibo_tras_el_punto_de_no_retorno {
    use super::*;
    use lodestar_app::{Profile, ReceiptSummary};
    use lodestar_core::types::{ChangeSetId, ReceiptId};
    use lodestar_workspace::failpoints::{self, FailPoint, PuntoDeGancho};
    use lodestar_workspace::{transaction_id, Workspace};

    /// El cuerpo que publica el plan de estos tests (una sola ruta afectada: `alfa.md`).
    const CUERPO_NUEVO: &str = "# Resumen\n\ncuerpo del plan\n";

    /// Los tres puntos de caída **posteriores a la publicación** que hay que ejercer por la fachada.
    /// Los dos primeros existen desde E24-H13 y nadie los había armado desde `App::change_apply`; el
    /// tercero es el que faltaba (ver la cabecera del módulo).
    const PUNTOS_POST_PUBLICACION: &[FailPoint] = &[
        FailPoint::TrasPublicarSinSellar,
        FailPoint::AntesDeSellar,
        FailPoint::TrasLaTransaccionAntesDelRecibo,
    ];

    /// Semilla + plan listo para aplicar. Devuelve la fachada, el `changeSetId` del plan y el
    /// contenido ORIGINAL de `alfa.md` — el estado exacto al que tiene que volver un `change_revert`.
    ///
    /// `config_yaml`, si viene, se escribe **antes** de abrir: `WorkspaceConfig` se lee una sola vez,
    /// al abrir, así que es lo único que la hace efectiva.
    fn app_con_plan(root: &Path, config_yaml: Option<&str>) -> (App, ChangeSetId, String) {
        semilla(root);
        if let Some(yaml) = config_yaml {
            escribe(root, ".lodestar/config.yaml", yaml);
        }
        let original = std::fs::read_to_string(root.join("alfa.md"))
            .expect("la semilla debe haber escrito alfa.md");

        let app = App::open(root).expect("el workspace temporal debe abrir");
        let ops = json!([
            { "op": "replace_body", "path": "alfa.md", "body": CUERPO_NUEVO },
        ]);
        let plan = app
            .change_plan(None, &ops, policy_permisiva())
            .expect("planificar una modificación normal debe funcionar");
        (app, plan.change_set_id, original)
    }

    /// El recibo de `cs` tal y como lo ve un agente: `workspace_status.receipts` (E23-H11), la vía
    /// por la que se recupera un `receiptId` que no se apuntó. Deliberadamente NO se mira el disco:
    /// lo que la historia exige es un recibo **utilizable**, no un fichero en un sitio concreto.
    fn recibo_del_plan(app: &App, cs: &ChangeSetId) -> Option<ReceiptSummary> {
        app.workspace_status(Profile::Standard)
            .expect("workspace_status debe responder")
            .receipts
            .into_iter()
            .find(|r| r.change_set_id == *cs)
    }

    /// La revisión actual del workspace, por la misma fachada.
    fn revision_actual(app: &App) -> String {
        app.workspace_status(Profile::Standard)
            .expect("workspace_status debe responder")
            .workspace_revision
            .0
    }

    /// Aplica el plan con `fp` armado y devuelve el resultado de la fachada, comprobando que el punto
    /// **se ejerció**: `failpoints::disparado` autodesarma el punto al dispararse, así que si sigue
    /// armado después del `change_apply` es que nadie lo consultó — y el escenario no ha reproducido
    /// nada.
    fn aplica_cayendo_en(
        app: &App,
        cs: &ChangeSetId,
        fp: FailPoint,
    ) -> Result<(), lodestar_app::AppError> {
        failpoints::armar(fp);
        let resultado = app.change_apply(cs, None).map(|_| ());
        let seguia_armado = failpoints::disparado(fp);
        failpoints::desarmar();
        assert!(
            !seguia_armado,
            "el punto de caída {fp:?} no se ejerció: nadie lo consulta en el camino de \
             `App::change_apply`. Sin eso el escenario de esta historia no se reproduce y el test \
             pasaría vacuamente — ver la cabecera del módulo para la API esperada del punto de la \
             fachada"
        );
        resultado
    }

    /// Asevera lo que la historia llama «el canónico está publicado»: `alfa.md` tiene el cuerpo del
    /// plan (y por tanto los renames ocurrieron: se cruzó el punto de no retorno). `contexto` nombra
    /// el fallo que se inyectó, que es lo que tiene que ser POSTERIOR a la publicación.
    fn asevera_publicado(root: &Path, contexto: &str) {
        let alfa = std::fs::read_to_string(root.join("alfa.md")).expect("alfa.md debe existir");
        assert!(
            alfa.contains("cuerpo del plan"),
            "precondición del escenario: con {contexto} —posterior a la publicación— el canónico \
             tiene que estar ya publicado; si no, el test no está mirando la ventana de esta \
             historia. alfa.md = {alfa:?}"
        );
    }

    /// **Criterio 1** (`publicar_implica_recibo`) — **Dado** un failpoint armado **después** de la
    /// publicación y **antes** del recibo, **Cuando** se llama a `App::change_apply`, **Entonces** el
    /// canónico está publicado **y** existe un recibo válido.
    ///
    /// Los tres puntos de `PUNTOS_POST_PUBLICACION` describen la misma ventana con distinta
    /// profundidad, y ninguno de los tres puede satisfacerse escribiendo el recibo *después*: con
    /// `TrasPublicarSinSellar`/`AntesDeSellar` la transacción sale por `Err` **desde dentro** de
    /// `apply_transaction`, así que la fachada no llega nunca a su paso (5). La única forma de que
    /// exista recibo es persistirlo **antes del punto de no retorno**, que es lo que pide el alcance
    /// de la historia (las revisiones que lo componen ya se conocen: `previous` en el paso (3) y
    /// `result_rev` en `transaction.rs:161`, la que estampa el journal).
    #[test]
    fn publicar_implica_recibo() {
        for fp in PUNTOS_POST_PUBLICACION {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let (app, cs, _original) = app_con_plan(root, None);

            let resultado = aplica_cayendo_en(&app, &cs, *fp);

            // (1) El disco ya cambió: se cruzó el punto de no retorno.
            asevera_publicado(root, &format!("el punto de caída {fp:?}"));

            // (2) …luego el recibo tiene que existir, sea cual sea el veredicto que la fachada
            //     devuelva (la historia no fija que este punto artificial deba devolver `Ok`: eso lo
            //     fija `el_cierre_no_convierte_un_apply_publicado_en_error` para los fallos REALES de
            //     cierre. Lo que no puede pasar es que la publicación quede sin registro).
            let recibo = recibo_del_plan(&app, &cs).unwrap_or_else(|| {
                panic!(
                    "con {fp:?} el canónico quedó PUBLICADO y sin recibo: la transacción es \
                     irreversible para siempre (`change_revert` → PLAN_EXPIRED) y un segundo \
                     `change_apply` moriría con PLAN_STALE. change_apply devolvió: {resultado:?}"
                )
            });

            // (3) Y tiene que ser un recibo COHERENTE, no un registro decorativo: `change_revert`
            //     exige que `resultRevision` sea la revisión actual del workspace, así que un recibo
            //     con revisiones inventadas sería inservible aunque exista.
            assert_eq!(
                recibo.result_revision.0,
                revision_actual(&app),
                "el recibo de una transacción publicada tiene que declarar como `resultRevision` la \
                 revisión que dejó el apply (es la que `change_revert` re-verifica antes de \
                 restaurar); {fp:?}"
            );
            assert_eq!(
                recibo.changed_path_count, 1,
                "y las rutas que tocó: el plan afecta exactamente a `alfa.md`; {fp:?}"
            );
        }
    }

    /// **Criterio 2** (`tras_fallo_post_publicacion_el_revert_funciona`) — **Dado** ese mismo
    /// workspace, **Cuando** se llama a `change_revert` con ese `receiptId`, **Entonces** revierte
    /// correctamente. Hoy: `PLAN_EXPIRED` para siempre.
    ///
    /// Esta es la prueba de que el recibo es **utilizable**, y con los dos puntos de
    /// `apply_transaction` exige algo más que persistirlo temprano: al no haberse sellado la
    /// transacción, el journal queda `applied` en disco, así que `revert_transaction` recupera primero
    /// (su paso 2) y la vía **COMPLETAR** de `recovery.rs` pasa hoy por `finish_recovery`, que
    /// **borra las copias de recuperación** (`discard_recovery_copies`) — y sin copias no hay
    /// reversión posible. «La recuperación por la vía COMPLETAR lo da por bueno» (alcance de la
    /// historia) incluye, por tanto, conservar el plano de reversión de una transacción que sí
    /// publicó y sí tiene recibo. Es el mismo requisito que el criterio del `SIGKILL`
    /// (`crash_tras_publicar_deja_transaccion_reversible`, en
    /// `crates/lodestar-mcp/tests/crash_senal.rs`), aquí en forma determinista.
    #[test]
    fn tras_fallo_post_publicacion_el_revert_funciona() {
        for fp in PUNTOS_POST_PUBLICACION {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let (app, cs, original) = app_con_plan(root, None);

            let resultado = aplica_cayendo_en(&app, &cs, *fp);
            asevera_publicado(root, &format!("el punto de caída {fp:?}"));

            let recibo = recibo_del_plan(&app, &cs).unwrap_or_else(|| {
                panic!(
                    "con {fp:?} no hay recibo que revertir (criterio 1); change_apply devolvió: \
                     {resultado:?}"
                )
            });

            let revert = app.change_revert(&recibo.receipt_id, None);
            let salida = match revert {
                Ok(salida) => salida,
                Err(e) => panic!(
                    "tras un fallo posterior a la publicación ({fp:?}) la transacción TIENE que ser \
                     reversible: `change_revert` con el receiptId {:?} devolvió {} ({e:?}). \
                     PLAN_EXPIRED es el síntoma del defecto (recibo ausente); un fallo de IO es el \
                     del plano de reversión purgado por la recuperación",
                    recibo.receipt_id.0,
                    e.code.as_str()
                ),
            };
            assert!(
                salida.reverted,
                "la reversión debe declararse hecha; {fp:?}: {salida:?}"
            );
            assert_eq!(
                std::fs::read_to_string(root.join("alfa.md")).unwrap(),
                original,
                "revertir tiene que devolver `alfa.md` a sus bytes ORIGINALES (frontmatter incluido); \
                 {fp:?}"
            );
        }
    }

    /// Deja `dir` en solo-lectura (`r-x`): con el bit de escritura del **directorio** apagado no se
    /// pueden crear ni **borrar** entradas dentro de él, que es exactamente el fallo de IO que hay que
    /// inyectar (la limpieza de staging y el purgado del GC son borrados).
    #[cfg(unix)]
    fn solo_lectura(dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o500))
            .expect("dejar el directorio en solo-lectura");
    }

    /// Devuelve `dir` a escritura (si no, ni el `TempDir` puede limpiarse al final).
    #[cfg(unix)]
    fn con_escritura(dir: &Path) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700));
    }

    /// `true` si en este entorno un directorio `r-x` de verdad impide borrar dentro. Corriendo como
    /// **root** los bits de permiso no deniegan nada, y el escenario no se podría inyectar: el test
    /// lo dice y se salta en vez de dar un verde falso.
    #[cfg(unix)]
    fn los_permisos_deniegan() -> bool {
        let sonda = tempfile::tempdir().unwrap();
        let dentro = sonda.path().join("sub");
        std::fs::create_dir(&dentro).unwrap();
        std::fs::write(dentro.join("x"), b"x").unwrap();
        solo_lectura(&dentro);
        let denegado = std::fs::remove_file(dentro.join("x")).is_err();
        con_escritura(&dentro);
        denegado
    }

    /// **Criterio 3** (`el_cierre_no_convierte_un_apply_publicado_en_error`) — **Dado** un fallo de
    /// sellado, de limpieza de staging o del GC, **Cuando** ocurre después de publicar, **Entonces**
    /// `change_apply` devuelve **éxito**.
    ///
    /// Aquí el fallo es **real**, no un failpoint, y eso es deliberado: `failpoint!` hace `return
    /// Err(...)` desde el orquestador para modelar un proceso que MUERE, y
    /// `seam_real::recovery_sin_parciales_por_el_orquestador_real` (E24-H13) exige que siga abortando.
    /// Lo que esta historia degrada a *best-effort con aviso por stderr* son los pasos de cierre
    /// cuando fallan **de verdad** (`transaction.rs:233-238`, `lib.rs:1707-1709`), y eso solo se
    /// prueba haciendo que fallen de verdad:
    ///
    /// - **(a) limpieza de staging** —parte del sellado, paso (11)—: se deja
    ///   `.lodestar/runtime/staging/` en solo-lectura dentro de la ventana `[T1, T3)`, cuando el
    ///   staging ya está materializado y validado y lo único que queda por hacer con él es borrarlo.
    ///   El `remove_dir_all` del sellado falla con `EACCES`.
    /// - **(b) GC de retención**: con `maximumReceipts: 1` y un recibo viejo plantado, el barrido
    ///   posterior al apply tiene que purgar y su `remove_dir_all(recovery/<viejo>)` falla porque
    ///   `.lodestar/runtime/recovery/` queda en solo-lectura.
    ///
    /// El borrado del **fichero de journal** —el otro paso del sellado— no se inyecta por separado a
    /// propósito: dejar `journal/` en solo-lectura rompería `mark_applied`, que re-persiste el journal
    /// **durante** los renames, y el fallo dejaría de ser posterior a la publicación. Su camino de
    /// error es el mismo `?` que el de (a), en la línea siguiente.
    #[test]
    #[cfg(unix)]
    fn el_cierre_no_convierte_un_apply_publicado_en_error() {
        if !los_permisos_deniegan() {
            eprintln!(
                "AVISO (E25-H04): este entorno no deniega escritura por permisos (¿root?); \
                 `el_cierre_no_convierte_un_apply_publicado_en_error` no puede inyectar el fallo de \
                 IO y se salta"
            );
            return;
        }

        // (a) Fallo de la limpieza de staging (sellado, paso 11).
        {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let (app, cs, _original) = app_con_plan(root, None);

            let staging = root.join(".lodestar").join("runtime").join("staging");
            {
                let staging = staging.clone();
                failpoints::armar_gancho(PuntoDeGancho::AntesDePublicar, move || {
                    assert!(
                        staging.is_dir(),
                        "precondición: en la ventana `[T1, T3)` el staging ya está materializado"
                    );
                    solo_lectura(&staging);
                });
            }
            let resultado = app.change_apply(&cs, None);
            failpoints::desarmar_ganchos();
            con_escritura(&staging);

            let aplicado = resultado.unwrap_or_else(|e| {
                panic!(
                    "un fallo al limpiar el staging ocurre DESPUÉS de publicar: el canónico ya \
                     cambió, así que el apply no puede devolver {} ({e:?}) — sería un agente \
                     convencido de que no se aplicó nada y sin recibo con el que deshacerlo. El \
                     cierre es best-effort con aviso por stderr",
                    e.code.as_str()
                )
            });
            assert!(
                aplicado.applied,
                "y se declara aplicado: {:?}",
                aplicado.changed_paths
            );
            asevera_publicado(root, "el fallo al limpiar el staging");
            assert!(
                recibo_del_plan(&app, &cs).is_some(),
                "un apply que devuelve éxito tiene que dejar su recibo (si no, `change_revert` \
                 responde PLAN_EXPIRED sobre algo que sí se aplicó)"
            );
        }

        // (b) Fallo del GC de retención.
        {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            // `maximumReceipts: 1` obliga al barrido posterior al apply a purgar el recibo más
            // antiguo (el plantado) y, con él, su copia de recuperación.
            let (app, cs, _original) = app_con_plan(
                root,
                Some("transactions:\n  retainReceiptsFor: \"24h\"\n  maximumReceipts: 1\n"),
            );
            escribe(root, ".lodestar/runtime/receipts/viejo.json", "{}");
            escribe(root, ".lodestar/runtime/recovery/viejo/uno.md", "respaldo");

            let recovery = root.join(".lodestar").join("runtime").join("recovery");
            {
                let recovery = recovery.clone();
                failpoints::armar_gancho(PuntoDeGancho::AntesDePublicar, move || {
                    // Las copias de esta transacción ya están escritas (paso 8) y nada vuelve a
                    // escribir aquí: el sellado las CONSERVA. Lo único que este candado rompe es el
                    // purgado del GC, que corre ya publicado el canónico.
                    solo_lectura(&recovery);
                });
            }
            let resultado = app.change_apply(&cs, None);
            failpoints::desarmar_ganchos();
            con_escritura(&recovery);

            let aplicado = resultado.unwrap_or_else(|e| {
                panic!(
                    "el GC de retención es best-effort por definición (`receipts.rs:238-248` ya lo \
                     dice para el lock) y corre DESPUÉS de publicar: un fallo suyo no puede \
                     convertir el apply en {} ({e:?})",
                    e.code.as_str()
                )
            });
            assert!(aplicado.applied, "y se declara aplicado");
            asevera_publicado(root, "el fallo del GC de retención");
        }
    }

    /// **Extra** (hueco declarado por el juez de E25-H03) — la ventana `[sellado, recibo)`:
    /// `apply_transaction` suelta el lock al sellar, y hasta que el recibo existe la transacción no
    /// aparece en ninguno de los dos conjuntos de «vivos» del GC (`journal/` ∪ `receipts/`), así que
    /// el GC de **otro proceso** puede purgar en ese hueco las copias que el recibo va a referenciar.
    ///
    /// El escenario cabe en este arnés sin retorcerlo porque el punto de la fachada deja el proceso
    /// **exactamente** en ese estado —publicado, sellado, lock soltado— y desde ahí basta con lanzar
    /// el GC de un segundo `Workspace` sobre la misma raíz (el «otro proceso» de E25-H03) antes de
    /// revertir. Persistir el recibo con el journal cierra el hueco por diseño: cuando el lock se
    /// suelta, la transacción ya está en `receipts/`.
    #[test]
    fn el_gc_de_otro_proceso_no_impide_revertir_tras_el_fallo_post_publicacion() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let (app, cs, original) = app_con_plan(root, None);

        let resultado = aplica_cayendo_en(&app, &cs, FailPoint::TrasLaTransaccionAntesDelRecibo);
        asevera_publicado(root, "el punto de caída de la fachada");

        // El «otro proceso»: un segundo handle sobre la misma raíz que barre el plano de control en
        // el hueco entre el sellado y el recibo.
        Workspace::open(root)
            .expect("el segundo handle debe abrir")
            .gc_receipts()
            .expect("el GC es best-effort: nunca devuelve Err por no poder barrer");

        let recibo = recibo_del_plan(&app, &cs).unwrap_or_else(|| {
            panic!(
                "el recibo tiene que sobrevivir al GC de otro proceso (y existir: criterio 1); \
                 change_apply devolvió: {resultado:?}"
            )
        });
        app.change_revert(&recibo.receipt_id, None)
            .unwrap_or_else(|e| {
                panic!(
                    "el GC de otro proceso no puede dejar irreversible una transacción publicada: \
                     `change_revert` devolvió {} ({e:?}). Si el recibo ya existe cuando se suelta el \
                     lock, el GC ve la transacción como viva y no toca sus copias",
                    e.code.as_str()
                )
            });
        assert_eq!(
            std::fs::read_to_string(root.join("alfa.md")).unwrap(),
            original,
            "y la reversión devuelve `alfa.md` a sus bytes originales"
        );
    }

    /// **Criterio 5** (`un_apply_no_publicado_no_deja_recibo`) — **Dado** un apply que falla **antes**
    /// del primer rename, **Cuando** termina, **Entonces** **no** hay recibo y `change_revert` sigue
    /// respondiendo `PLAN_EXPIRED`.
    ///
    /// Control anti-vacuo: el arreglo no puede consistir en escribir recibos siempre. Y no es un
    /// control barato, porque los dos escenarios que ejerce pasan por el instante en el que el recibo
    /// **ya existiría** si se persiste junto al journal:
    ///
    /// - **(a)** `FailPoint::TrasJournalPrepared`: journal `prepared` y copias listas, **cero**
    ///   renames.
    /// - **(b)** el **aborto de ventana** de E25-H01/H02: la transacción llega con journal y copias
    ///   hasta el último instante de `[T1, T3)`, detecta que el canónico cambió y aborta con
    ///   `WRITE_CONFLICT` sellando su propio journal. Si el recibo sobreviviera a ese sellado,
    ///   `change_revert` escribiría las copias de T1 **encima** de la edición externa que el aborto
    ///   existe para no pisar.
    ///
    /// En los dos casos el `receiptId` se deriva del `changeSetId` con la misma convención que usa la
    /// fachada (`transaction_id`), porque el escenario es justamente que ningún recibo se ha publicado
    /// y no hay de dónde leerlo.
    #[test]
    fn un_apply_no_publicado_no_deja_recibo() {
        // (a) Caída con journal `prepared` y cero renames.
        {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let (app, cs, original) = app_con_plan(root, None);

            let err = aplica_cayendo_en(&app, &cs, FailPoint::TrasJournalPrepared)
                .expect_err("el failpoint aborta la transacción antes del primer rename");
            assert_eq!(
                std::fs::read_to_string(root.join("alfa.md")).unwrap(),
                original,
                "precondición: la caída fue ANTES del primer rename, así que el canónico no se movió \
                 ni un byte (el apply devolvió {})",
                err.code.as_str()
            );

            assert!(
                recibo_del_plan(&app, &cs).is_none(),
                "una transacción que NO publicó no puede dejar recibo: `change_revert` restauraría \
                 las copias de un estado que nunca se sustituyó"
            );
            let derivado = ReceiptId(transaction_id(&cs));
            let revert = app.change_revert(&derivado, None);
            assert_eq!(
                revert.err().map(|e| e.code),
                Some(ErrorCode::PlanExpired),
                "y revertir lo que no se aplicó sigue siendo PLAN_EXPIRED (transacción no \
                 disponible), no un éxito"
            );
        }

        // (b) Aborto de la ventana `[T1, T3)` (E25-H01/H02): WRITE_CONFLICT antes del primer rename.
        {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let (app, cs, _original) = app_con_plan(root, None);

            let edicion_externa = "---\ntype: Nota\ntitle: Alfa\ndescription: Primer documento\n---\n\n# Resumen\n\nEDICIÓN EXTERNA\n";
            {
                let ruta = root.join("alfa.md");
                failpoints::armar_gancho(PuntoDeGancho::AntesDePublicar, move || {
                    std::fs::write(&ruta, edicion_externa)
                        .expect("el gancho debe poder editar alfa.md");
                });
            }
            let err = app.change_apply(&cs, None).map(|_| ());
            failpoints::desarmar_ganchos();
            assert_eq!(
                err.err().map(|e| e.code),
                Some(ErrorCode::WriteConflict),
                "precondición (E25-H01): el canónico cambió en la ventana, así que el apply aborta \
                 con WRITE_CONFLICT antes del primer rename"
            );
            assert_eq!(
                std::fs::read_to_string(root.join("alfa.md")).unwrap(),
                edicion_externa,
                "y no publicó nada"
            );

            assert!(
                recibo_del_plan(&app, &cs).is_none(),
                "el aborto de ventana sella su transacción (E25-H02): un recibo suyo sobreviviente \
                 haría que `change_revert` pisara la edición externa con las copias de T1"
            );
            let derivado = ReceiptId(transaction_id(&cs));
            let revert = app.change_revert(&derivado, None);
            assert_eq!(
                revert.err().map(|e| e.code),
                Some(ErrorCode::PlanExpired),
                "y revertir un aborto de ventana sigue siendo PLAN_EXPIRED"
            );
        }
    }
}

// ===========================================================================
// E25-H05 — Revertir re-verifica bajo el lock y deja recibo
// (`requirements/epica-25-endurecimiento-escritura.md`, bloque B). Fase ROJA.
//
// LOS DOS DEFECTOS QUE SE PRUEBAN AQUÍ (el tercero, el fsync del directorio, es un límite
// estructural del nivel de integración y vive como test unitario en
// `crates/lodestar-workspace/src/io.rs`, módulo `durabilidad_del_directorio`)
//
// **(b) Revert sin re-verificación bajo el lock.** `change_revert_uncounted` compara la revisión
// actual con `receipt.result_revision` en `crates/lodestar-app/src/lib.rs:1845-1857` — **antes** de
// tomar el lock, que lo toma `revert_transaction` (`recovery.rs:898`). En esa ventana otro escritor
// puede tocar un `.md` afectado y la reversión lo sobrescribe con la copia respaldada, en silencio.
// La simetría con el apply está rota: `apply_transaction` sí vuelve a comprobar la base bajo el lock
// (`reverify_base_revision`, `transaction.rs:195`) y `revert_transaction` no tiene equivalente.
//
// **(c) La reversión conserva la forma exacta de S5** (hallazgo MAYOR-2 del juez ciego de E25-H04).
// E25-H04 cerró «publicar implica recibo» solo en el camino del apply. En el espejo,
// `write_receipt(&revert_receipt)` sale por `?` (`lib.rs:1880-1882`) **después** de que
// `revert_transaction` haya publicado la inversa (`:1863-1866`), y `revert_transaction`
// (`recovery.rs:892`) no llama a `write_pending_receipt` (`receipts.rs:225`) — así que no hay
// registro durable que la vía COMPLETAR (`finish_recovery_completada:769`) pueda promover. Un
// `SIGKILL` o un `ENOSPC` entre el último rename de la inversa y su recibo devuelve `Err` sobre algo
// PUBLICADO y sin recibo: el agente cree que no se revirtió, el canónico dice lo contrario, y como el
// recibo es el criterio de «vivo» del GC (`journal/ ∪ receipts/`), el árbol `recovery/<txnId>-revert/`
// queda huérfano y se purga — deshacer el *undo* se vuelve imposible para siempre.
//
// LOS SEAMS (declarados en la salida de la fase roja; aquí solo se USAN)
//
// 1. `PuntoDeGancho::AntesDeRestaurar` — **variante nueva** (stub añadido a
//    `crates/lodestar-workspace/src/failpoints.rs`, sin lógica). El implementador la dispara con un
//    `ejecutar_gancho` cfg-gateado dentro de `revert_transaction`, entre su entrada y su primera
//    escritura y **antes** de la re-verificación bajo el lock — es el espejo de
//    `AntesDePublicar` (E25-H01) para el camino que deshace. Como el gancho **continúa**, reproduce
//    lo que un `FailPoint` no sabe: la edición ajena ocurre *y el flujo sigue*.
// 2. `FailPoint::TrasLaTransaccionAntesDelRecibo` — **la que ya existe** (E25-H04), consultada
//    también desde `App::change_revert`, entre el retorno de `revert_transaction` y el
//    `write_receipt` de la inversa. Mismo `failpoints::disparado(..)` y mismo autodesarme, que es lo
//    que permite comprobar que el punto **se ejerció** y que el escenario no pasó vacuamente.
// 3. `FailPoint::TrasJournalPrepared` — **la que ya existe** (E24-H13), ejercida también en
//    `revert_transaction` justo después de su journal (y del registro durable del recibo que la
//    historia le añade) y **antes** del primer rename de la inversa. Es lo que hace fuerte al control
//    anti-vacuo: sin ella, «no dejar recibo» se cumpliría trivialmente en escenarios que ni siquiera
//    llegan a escribir el registro.
//
// ROJO ESPERADO HOY
// - `revert_con_edicion_externa_en_la_ventana_da_write_conflict`: ROJO. Nadie dispara el gancho, así
//   que el testigo del test no se enciende y el escenario ni siquiera se reproduce (y si se
//   reprodujera, hoy la reversión pisa la edición ajena en silencio y devuelve `Ok`).
// - `revert_sin_interferencia_sigue_funcionando`: **control anti-vacuo, PASA YA HOY** y tiene que
//   seguir pasando. Es la mitad que impide que el arreglo consista en rechazar reversiones.
// - `revertir_implica_recibo`: ROJO. Nadie consulta el punto de la fachada en el camino del revert.
// - `un_revert_no_publicado_no_deja_recibo`: ROJO en (a) y (c) por falta de los seams; (b) —copias
//   ausentes— pasa ya hoy y es el control de que el arreglo no puede consistir en escribir recibos
//   siempre.
// ===========================================================================

mod reversion_re_verificada {
    use super::*;
    use lodestar_app::{Profile, ReceiptSummary};
    use lodestar_core::types::{ChangeSetId, ReceiptId};
    use lodestar_workspace::transaction_id;

    /// El cuerpo que publica el plan de estos tests (una sola ruta afectada: `alfa.md`).
    const CUERPO_NUEVO: &str = "# Resumen\n\ncuerpo del plan\n";

    /// Lo que escribe «el otro escritor» sobre `alfa.md` dentro de la ventana de la reversión.
    const EDICION_EXTERNA: &str = "---\ntype: Nota\ntitle: Alfa\ndescription: Primer documento\n---\n\n# Resumen\n\nEDICIÓN EXTERNA\n";

    /// Un workspace con una transacción **ya publicada**: el punto de partida de todo `change_revert`.
    struct Aplicada {
        app: App,
        cs: ChangeSetId,
        /// El recibo del apply, el que se le pasa a `change_revert`.
        receipt_id: ReceiptId,
        /// Bytes de `alfa.md` ANTES del apply: el estado exacto al que tiene que volver la reversión.
        original: String,
        /// Bytes de `alfa.md` DESPUÉS del apply: lo que la reversión deshace.
        publicado: String,
        /// Revisión del workspace antes del apply (== `resultRevision` de la inversa).
        revision_original: String,
        /// Revisión del workspace tras el apply (== `previousRevision` de la inversa).
        revision_publicada: String,
    }

    /// Semilla + plan + apply, con las precondiciones aseveradas: sin una publicación real no hay
    /// nada que revertir y los escenarios de esta historia no significan nada.
    fn app_aplicada(root: &Path) -> Aplicada {
        semilla(root);
        let original = std::fs::read_to_string(root.join("alfa.md"))
            .expect("la semilla debe haber escrito alfa.md");

        let app = App::open(root).expect("el workspace temporal debe abrir");
        let ops = json!([
            { "op": "replace_body", "path": "alfa.md", "body": CUERPO_NUEVO },
        ]);
        let plan = app
            .change_plan(None, &ops, policy_permisiva())
            .expect("planificar una modificación normal debe funcionar");
        let aplicado = app
            .change_apply(&plan.change_set_id, None)
            .expect("aplicar el plan debe funcionar: es la precondición de todo revert");

        let publicado =
            std::fs::read_to_string(root.join("alfa.md")).expect("alfa.md debe existir");
        assert!(
            publicado.contains("cuerpo del plan") && publicado != original,
            "precondición: el apply tiene que haber publicado de verdad; alfa.md = {publicado:?}"
        );
        Aplicada {
            app,
            cs: plan.change_set_id,
            receipt_id: aplicado.receipt_id,
            original,
            publicado,
            revision_original: aplicado.previous_workspace_revision.0,
            revision_publicada: aplicado.workspace_revision.0,
        }
    }

    /// El id con el que la fachada nombra el recibo y el material de la transacción **inversa**
    /// (`<txnId>-revert`, `lib.rs:1861`). Se deriva del `changeSetId` porque los escenarios de fallo
    /// son justamente aquellos en los que no hay `RevertResult` del que leerlo.
    ///
    /// La convención de nombre no es cosmética: es lo que hace que el journal, las copias, el
    /// registro durable y el recibo de la inversa se localicen todos con el mismo `txnId` (y lo que
    /// permite que la vía COMPLETAR de la recuperación promueva su recibo sin código nuevo).
    fn id_de_la_inversa(cs: &ChangeSetId) -> ReceiptId {
        ReceiptId(format!("{}-revert", transaction_id(cs)))
    }

    /// El recibo `id` tal y como lo ve un agente: `workspace_status.receipts` (E23-H11). No se mira
    /// el disco a propósito: lo que la historia exige es un recibo **utilizable**, no un fichero en
    /// un sitio concreto.
    fn recibo_por_id(app: &App, id: &ReceiptId) -> Option<ReceiptSummary> {
        app.workspace_status(Profile::Standard)
            .expect("workspace_status debe responder")
            .receipts
            .into_iter()
            .find(|r| r.receipt_id == *id)
    }

    /// La revisión actual del workspace, por la misma fachada.
    fn revision_actual(app: &App) -> String {
        app.workspace_status(Profile::Standard)
            .expect("workspace_status debe responder")
            .workspace_revision
            .0
    }

    /// Los bytes de `alfa.md` en disco.
    fn alfa(root: &Path) -> String {
        std::fs::read_to_string(root.join("alfa.md")).expect("alfa.md debe existir")
    }

    /// **Criterio 2** (`revert_sin_interferencia_sigue_funcionando`) — **Dado** un `change_revert`
    /// sin interferencia, **Cuando** se ejecuta, **Entonces** restaura exactamente igual que en
    /// v0.3.1.
    ///
    /// Control anti-vacuo del criterio 1: el arreglo es *re-verificar*, no *rechazar*. Una
    /// implementación que devolviera `WRITE_CONFLICT` de más —por ejemplo comparando contra una
    /// revisión mal calculada, o computándola después de tocar el canónico— rompería este test, que
    /// fija el camino feliz completo: bytes restaurados, revisiones intercambiadas, rutas y recibo de
    /// la inversa.
    #[test]
    fn revert_sin_interferencia_sigue_funcionando() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let a = app_aplicada(root);

        let salida = a
            .app
            .change_revert(&a.receipt_id, None)
            .unwrap_or_else(|e| {
                panic!(
                "un `change_revert` sin interferencia tiene que seguir revirtiendo: devolvió {} \
                 ({e:?})",
                e.code.as_str()
            )
            });

        assert!(salida.reverted, "se declara revertida: {salida:?}");
        assert_eq!(
            alfa(root),
            a.original,
            "revertir devuelve `alfa.md` a sus bytes ORIGINALES (frontmatter incluido)"
        );
        assert_eq!(
            salida.previous_workspace_revision.0, a.revision_publicada,
            "la `previousWorkspaceRevision` de la inversa es la `resultRevision` del apply"
        );
        assert_eq!(
            salida.workspace_revision.0, a.revision_original,
            "y su `workspaceRevision` es la `previousRevision` del apply: el estado restaurado"
        );
        assert_eq!(
            revision_actual(&a.app),
            a.revision_original,
            "que además es la revisión REAL del workspace tras revertir"
        );
        assert_eq!(
            salida
                .changed_paths
                .iter()
                .map(|p| p.as_str())
                .collect::<Vec<_>>(),
            vec!["alfa.md"],
            "y las rutas restauradas son exactamente las que tocó el apply"
        );
        assert_eq!(
            salida.receipt_id,
            id_de_la_inversa(&a.cs),
            "el recibo de la inversa conserva la convención de nombre `<txnId>-revert`, que es lo \
             que localiza su journal, sus copias y su registro durable con el mismo id"
        );
        assert!(
            recibo_por_id(&a.app, &id_de_la_inversa(&a.cs)).is_some(),
            "y queda persistido y visible para un agente (`workspace_status.receipts`)"
        );
    }

    #[cfg(feature = "test-failpoints")]
    mod con_seams {
        use super::*;
        use lodestar_workspace::failpoints::{self, FailPoint, PuntoDeGancho};
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::Arc;

        /// Arma el gancho de la ventana de la reversión: escribe [`EDICION_EXTERNA`] sobre `alfa.md`
        /// —haciendo de «otro escritor»— y enciende un testigo, de modo que el test pueda distinguir
        /// «el escenario se reprodujo y el motor lo dejó pasar» de «nadie ejerció el gancho».
        fn armar_edicion_externa(root: &Path) -> Arc<AtomicBool> {
            let testigo = Arc::new(AtomicBool::new(false));
            let ruta = root.join("alfa.md");
            let marca = Arc::clone(&testigo);
            failpoints::armar_gancho(PuntoDeGancho::AntesDeRestaurar, move || {
                std::fs::write(&ruta, EDICION_EXTERNA)
                    .expect("el gancho debe poder editar alfa.md");
                marca.store(true, Ordering::SeqCst);
            });
            testigo
        }

        /// Asevera que el gancho se ejerció de verdad. Sin esto el escenario no se reproduce y el
        /// test pasaría vacuamente (o fallaría por una razón que no es la del criterio).
        fn asevera_gancho_ejercido(testigo: &Arc<AtomicBool>) {
            assert!(
                testigo.load(Ordering::SeqCst),
                "el gancho `PuntoDeGancho::AntesDeRestaurar` no se ejerció: nadie lo dispara en el \
                 camino de `Workspace::revert_transaction`. Sin él, la edición externa de la ventana \
                 `[comprobación de la fachada, primera escritura de la inversa)` no ocurre y este \
                 escenario no prueba nada — ver el rustdoc de la variante para dónde debe vivir el \
                 `ejecutar_gancho`"
            );
        }

        /// **Criterio 1** (`revert_con_edicion_externa_en_la_ventana_da_write_conflict`) — **Dado**
        /// un `change_revert` en curso, **Cuando** otro escritor modifica un `.md` afectado entre la
        /// comprobación de la fachada y la toma del lock, **Entonces** la reversión falla con
        /// `WRITE_CONFLICT` y **no** escribe nada.
        ///
        /// Hoy la pisa en silencio: la fachada mira `receipt.result_revision` sin el lock y nadie
        /// vuelve a mirar después de tomarlo, así que la copia respaldada se escribe encima de la
        /// edición ajena y `change_revert` devuelve `Ok`. El arreglo re-verifica bajo el lock
        /// **reusando** `reverify_base_revision` (invariante #3: una sola verdad, no una segunda
        /// comprobación escrita a mano).
        #[test]
        fn revert_con_edicion_externa_en_la_ventana_da_write_conflict() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let a = app_aplicada(root);

            let testigo = armar_edicion_externa(root);
            let resultado = a.app.change_revert(&a.receipt_id, None);
            failpoints::desarmar_ganchos();

            asevera_gancho_ejercido(&testigo);

            let err = match resultado {
                Err(e) => e,
                Ok(salida) => panic!(
                    "revertir sobre un canónico que cambió dentro de la ventana tiene que \
                     rechazarse con WRITE_CONFLICT, pero la fachada informó de una reversión: \
                     reverted={}, changedPaths={:?}. El `.md` afectado quedó: {:?}",
                    salida.reverted,
                    salida.changed_paths,
                    alfa(root)
                ),
            };
            assert_eq!(
                err.code,
                ErrorCode::WriteConflict,
                "el conflicto de la ventana del revert debe llegar a la fachada con su código \
                 estable WRITE_CONFLICT (terminal: el agente vuelve a mirar y decide); era: {} \
                 ({err:?})",
                err.code.as_str()
            );
            assert_eq!(
                err.code.as_str(),
                "WRITE_CONFLICT",
                "y con esa cadena exacta en el wire"
            );

            // Y NO escribió nada: la edición ajena sigue intacta y la revisión no se movió.
            assert_eq!(
                alfa(root),
                EDICION_EXTERNA,
                "una reversión rechazada por WRITE_CONFLICT no puede haber pisado la edición de otro \
                 escritor con la copia respaldada: ese cambio no está respaldado en ningún sitio y se \
                 pierde para siempre"
            );
            assert!(
                recibo_por_id(&a.app, &id_de_la_inversa(&a.cs)).is_none(),
                "ni puede haber dejado recibo de una inversa que no se publicó"
            );
        }

        /// Arma `fp`, ejecuta el `change_revert` y comprueba que el punto **se ejerció**:
        /// `failpoints::disparado` autodesarma al dispararse, así que si sigue armado después es que
        /// nadie lo consultó en ese camino.
        fn revierte_cayendo_en(
            app: &App,
            receipt_id: &ReceiptId,
            fp: FailPoint,
            donde: &str,
        ) -> Result<(), lodestar_app::AppError> {
            failpoints::armar(fp);
            let resultado = app.change_revert(receipt_id, None).map(|_| ());
            let seguia_armado = failpoints::disparado(fp);
            failpoints::desarmar();
            assert!(
                !seguia_armado,
                "el punto de caída {fp:?} no se ejerció: nadie lo consulta {donde}. Sin eso el \
                 escenario de esta historia no se reproduce y el test pasaría vacuamente"
            );
            resultado
        }

        /// **Criterio 4** (`revertir_implica_recibo`) — **Dado** un failpoint armado **entre la
        /// publicación de la inversa y su recibo**, **Cuando** se llama a `change_revert`,
        /// **Entonces** la inversa está publicada **y** existe su recibo.
        ///
        /// Hoy devuelve `Err` sobre algo ya publicado y sin recibo: `write_receipt` sale por `?`
        /// (`lib.rs:1880-1882`) después de que `revert_transaction` haya restaurado el canónico, y
        /// `revert_transaction` no persiste ningún registro durable antes de su punto de no retorno.
        /// El arreglo no puede consistir en escribir el recibo *después* —el proceso puede no llegar
        /// ahí—, sino en persistirlo con el journal de la inversa y promoverlo al sellar, con la
        /// misma mecánica que E25-H04 dio al apply (`write_pending_receipt`/`promote_pending_receipt`,
        /// efectividad decidida por el estado `applied` del journal).
        #[test]
        fn revertir_implica_recibo() {
            let dir = tempfile::tempdir().unwrap();
            let root = dir.path();
            let a = app_aplicada(root);

            let resultado = revierte_cayendo_en(
                &a.app,
                &a.receipt_id,
                FailPoint::TrasLaTransaccionAntesDelRecibo,
                "en el camino de `App::change_revert`, entre el retorno de `revert_transaction` y el \
                 `write_receipt` de la inversa",
            );

            // (1) La inversa está PUBLICADA: el canónico ya volvió al estado anterior al apply.
            assert_eq!(
                alfa(root),
                a.original,
                "precondición del escenario: el punto de caída es POSTERIOR a la publicación de la \
                 inversa, así que el canónico tiene que estar ya restaurado; si no, el test no está \
                 mirando la ventana de esta historia"
            );

            // (2) …luego su recibo tiene que existir, sea cual sea el veredicto que la fachada
            //     devuelva por este punto artificial.
            let inversa = id_de_la_inversa(&a.cs);
            let recibo = recibo_por_id(&a.app, &inversa).unwrap_or_else(|| {
                panic!(
                    "la inversa quedó PUBLICADA y sin recibo: el agente cree que no se revirtió, el \
                     canónico dice lo contrario, y como el recibo es el criterio de «vivo» del GC \
                     (`journal/ ∪ receipts/`) el árbol `recovery/{}/` queda huérfano y se purga — \
                     deshacer el *undo* se vuelve imposible para siempre. change_revert devolvió: \
                     {resultado:?}",
                    inversa.0
                )
            });

            // (3) Y tiene que ser COHERENTE, no un registro decorativo.
            assert_eq!(
                recibo.result_revision.0,
                revision_actual(&a.app),
                "el recibo de la inversa declara como `resultRevision` la revisión que dejó la \
                 reversión (la que se re-verifica antes de volver a tocar nada)"
            );
            assert_eq!(
                recibo.changed_path_count, 1,
                "y las rutas que tocó: la reversión afecta exactamente a `alfa.md`"
            );
        }

        /// **Criterio 5** (`un_revert_no_publicado_no_deja_recibo`) — **Dado** un `change_revert` que
        /// falla **antes** del primer rename de la inversa, **Cuando** termina, **Entonces** **no**
        /// queda recibo de la inversa.
        ///
        /// Control anti-vacuo: el arreglo no puede consistir en escribir recibos siempre, que es
        /// justo lo que `pending_receipt_efectivo` (`receipts.rs:259`) existe para impedir. Los tres
        /// escenarios cubren las tres formas de no publicar:
        ///
        /// - **(a)** la base re-verificada que no casa (el `WRITE_CONFLICT` del criterio 1). Un
        ///   recibo suyo haría que un revert posterior escribiera las copias respaldadas **encima**
        ///   de la edición externa que este rechazo existe para no pisar.
        /// - **(b)** copias de recuperación ausentes: falla antes de tocar nada (pasa ya hoy).
        /// - **(c)** caída con el journal de la inversa `prepared` y **cero** renames — el instante
        ///   en el que el registro durable del recibo YA existe. Es el escenario que da fuerza al
        ///   control: obliga a que la efectividad del registro la decida el estado del journal, no su
        ///   mera presencia.
        #[test]
        fn un_revert_no_publicado_no_deja_recibo() {
            // (a) WRITE_CONFLICT de la ventana.
            {
                let dir = tempfile::tempdir().unwrap();
                let root = dir.path();
                let a = app_aplicada(root);

                let testigo = armar_edicion_externa(root);
                let resultado = a.app.change_revert(&a.receipt_id, None).map(|_| ());
                failpoints::desarmar_ganchos();
                asevera_gancho_ejercido(&testigo);

                assert_eq!(
                    resultado.err().map(|e| e.code),
                    Some(ErrorCode::WriteConflict),
                    "precondición (criterio 1): la base re-verificada bajo el lock ya no casa, así \
                     que la reversión aborta antes del primer rename"
                );
                assert_eq!(
                    alfa(root),
                    EDICION_EXTERNA,
                    "y no publicó nada: la edición ajena sigue en pie"
                );
                assert!(
                    recibo_por_id(&a.app, &id_de_la_inversa(&a.cs)).is_none(),
                    "una reversión que NO publicó no puede dejar recibo: un revert posterior con él \
                     escribiría las copias respaldadas encima de la edición externa"
                );
            }

            // (b) Copias de recuperación ausentes (transacción ya no reversible).
            {
                let dir = tempfile::tempdir().unwrap();
                let root = dir.path();
                let a = app_aplicada(root);

                let recovery = root.join(".lodestar").join("runtime").join("recovery");
                std::fs::remove_dir_all(&recovery)
                    .expect("el apply tiene que haber dejado su árbol de recuperación");

                let err = a
                    .app
                    .change_revert(&a.receipt_id, None)
                    .map(|_| ())
                    .expect_err("sin copias de recuperación no se puede revertir");
                assert_eq!(
                    alfa(root),
                    a.publicado,
                    "precondición: el rechazo ocurre antes de tocar el canónico (devolvió {})",
                    err.code.as_str()
                );
                assert!(
                    recibo_por_id(&a.app, &id_de_la_inversa(&a.cs)).is_none(),
                    "ni deja recibo de una inversa que no llegó a existir"
                );
            }

            // (c) Caída con el journal de la inversa `prepared` y cero renames.
            {
                let dir = tempfile::tempdir().unwrap();
                let root = dir.path();
                let a = app_aplicada(root);

                let err = revierte_cayendo_en(
                    &a.app,
                    &a.receipt_id,
                    FailPoint::TrasJournalPrepared,
                    "en `Workspace::revert_transaction`, tras crear el journal de la inversa (y su \
                     registro durable de recibo) y ANTES del primer rename",
                )
                .expect_err("el failpoint aborta la reversión antes del primer rename");
                assert_eq!(
                    alfa(root),
                    a.publicado,
                    "precondición: la caída fue ANTES del primer rename, así que el canónico no se \
                     movió ni un byte (el revert devolvió {})",
                    err.code.as_str()
                );
                assert!(
                    recibo_por_id(&a.app, &id_de_la_inversa(&a.cs)).is_none(),
                    "el registro durable del recibo de la inversa ya está escrito en este punto, \
                     pero su journal declara `prepared`: no es efectivo, así que ni se lista ni se \
                     carga. Si lo fuera, un revert posterior restauraría copias de un estado que \
                     nunca se sustituyó"
                );
            }
        }
    }
}

// ===========================================================================
// E28-H01 — Round-trip de revisiones y receiptIds al deshacer el *undo* (M-01 del testbench)
//
// EL DEFECTO
//
// `App::change_revert_uncounted` (`crates/lodestar-app/src/lib.rs` ~L2168) deriva la identidad de la
// transacción inversa del `changeSetId` que el recibo lleva dentro:
//
//     let orig_txn_id = transaction_id(&receipt.change_set_id);
//     let revert_txn_id = format!("{orig_txn_id}-revert");
//
// Un recibo `X-revert` **hereda** el `changeSetId` de la transacción original, así que revertirlo
// recalcula `orig_txn_id = X` y `revert_txn_id = X-revert`: se restaura desde `recovery/X/` —el árbol
// pre-apply, que ya es el estado vigente: no-op— y la inversa colisiona consigo misma, sobrescribiendo
// `recovery/X-revert/` (que guardaba el estado redo) y `receipts/X-revert.json`.
//
// QUÉ FIJAN ESTOS TESTS, Y POR QUÉ AQUÍ
//
// Los cuatro criterios observables (estado restaurado, identidad propia, material previo intacto,
// composición) los fija el e2e de `crates/lodestar-mcp/tests/e2e_ciclo_vida.rs`, sobre la sesión viva
// que reprodujo el testbench. Este módulo cubre lo que solo se ve desde la fachada: el **round-trip de
// revisiones y receiptIds** —que la inversa de la inversa es un recibo coherente, no un registro
// degenerado `A→A`— y el **crash a mitad**, con los failpoints que solo `lodestar-app` puede armar
// porque atraviesan sus dos capas (`FailPoint::TrasJournalPrepared`, dentro de la transacción inversa;
// `FailPoint::TrasLaTransaccionAntesDelRecibo`, en la fachada).
//
// El camino de un `revert` sobre una transacción NORMAL no cambia: lo custodia
// `reversion_re_verificada::revert_sin_interferencia_sigue_funcionando`, que sigue verde sin tocarse.
// ===========================================================================

mod revertir_la_reversion {
    use super::*;
    use lodestar_app::{Profile, ReceiptSummary, RevertResult};
    use lodestar_core::types::ReceiptId;

    /// El cuerpo que publica el apply de estos tests: el estado **B**, al que hay que poder volver.
    const CUERPO_B: &str = "# Resumen\n\ncuerpo del plan\n";

    /// Un workspace con `plan → apply → revert` ya hechos: el punto de partida de deshacer el *undo*.
    struct Revertida {
        app: App,
        /// Bytes de `alfa.md` antes del apply (estado **A**, vigente tras el primer revert).
        estado_a: String,
        /// Bytes de `alfa.md` tras el apply (estado **B**, el redo que la segunda reversión restaura).
        estado_b: String,
        /// Recibo del primer `revert` — el `-revert` que estos tests revierten.
        recibo_revert: ReceiptId,
        /// `RevertResult` del primer revert, para casar revisiones en el round-trip.
        revert1: RevertResult,
    }

    /// Semilla + plan + apply + revert, con cada precondición aseverada: sin una reversión real
    /// publicada no hay ningún `-revert` que revertir y los escenarios no significan nada.
    fn app_revertida(root: &Path) -> Revertida {
        semilla(root);
        let estado_a = std::fs::read_to_string(root.join("alfa.md"))
            .expect("la semilla debe escribir alfa.md");

        let app = App::open(root).expect("el workspace temporal debe abrir");
        let ops = json!([{ "op": "replace_body", "path": "alfa.md", "body": CUERPO_B }]);
        let plan = app
            .change_plan(None, &ops, policy_permisiva())
            .expect("planificar una modificación normal debe funcionar");
        let aplicado = app
            .change_apply(&plan.change_set_id, None)
            .expect("aplicar el plan es la precondición de todo revert");
        let estado_b = std::fs::read_to_string(root.join("alfa.md")).expect("alfa.md debe existir");
        assert_ne!(
            estado_b, estado_a,
            "precondición: el apply tiene que haber publicado de verdad"
        );

        let revert1 = app
            .change_revert(&aplicado.receipt_id, None)
            .expect("revertir un apply normal debe funcionar (camino ya probado)");
        assert_eq!(
            std::fs::read_to_string(root.join("alfa.md")).expect("alfa.md debe existir"),
            estado_a,
            "precondición: el primer revert devuelve `alfa.md` al estado A"
        );

        Revertida {
            app,
            estado_a,
            estado_b,
            recibo_revert: revert1.receipt_id.clone(),
            revert1,
        }
    }

    /// Los bytes de `alfa.md` en disco.
    fn alfa(root: &Path) -> String {
        std::fs::read_to_string(root.join("alfa.md")).expect("alfa.md debe existir")
    }

    /// Los recibos que un agente ve (`workspace_status.receipts`, E23-H11), por id.
    fn recibo_por_id(app: &App, id: &ReceiptId) -> Option<ReceiptSummary> {
        app.workspace_status(Profile::Standard)
            .expect("workspace_status debe responder")
            .receipts
            .into_iter()
            .find(|r| r.receipt_id == *id)
    }

    /// La revisión actual del workspace, por la misma fachada.
    fn revision_actual(app: &App) -> String {
        app.workspace_status(Profile::Standard)
            .expect("workspace_status debe responder")
            .workspace_revision
            .0
    }

    /// **Round-trip de revisiones y receiptIds** — **Dado** un `-revert` publicado, **Cuando** se
    /// revierte, **Entonces** la inversa de la inversa es una transacción completa: identidad propia,
    /// revisiones intercambiadas respecto al recibo que deshace, y un recibo coherente y utilizable.
    ///
    /// Hoy devuelve el mismo `receiptId` que revierte, con `previousRevision == resultRevision`: un
    /// recibo degenerado (`A→A`) escrito **encima** del que describía la primera reversión.
    #[test]
    fn revertir_un_revert_deja_un_recibo_coherente() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let r = app_revertida(root);

        let salida = r
            .app
            .change_revert(&r.recibo_revert, None)
            .unwrap_or_else(|e| {
                panic!(
                    "deshacer el *undo* tiene que ser una reversión como cualquier otra: \
                     `change_revert` del recibo {:?} devolvió {} ({e:?})",
                    r.recibo_revert.0,
                    e.code.as_str()
                )
            });

        assert!(salida.reverted, "se declara revertida: {salida:?}");
        assert_ne!(
            salida.receipt_id, r.recibo_revert,
            "la transacción inversa gana identidad PROPIA: reusar el id del recibo que se revierte \
             la hace colisionar consigo misma y sobrescribir su `recovery/`/`receipts/`"
        );
        assert_eq!(
            alfa(root),
            r.estado_b,
            "y restaura el estado que ese recibo dejó atrás: el redo (B), no el estado vigente (A)"
        );
        assert_eq!(
            salida.previous_workspace_revision.0, r.revert1.workspace_revision.0,
            "la `previousWorkspaceRevision` de la inversa es la `workspaceRevision` que dejó el \
             recibo revertido"
        );
        assert_eq!(
            salida.workspace_revision.0, r.revert1.previous_workspace_revision.0,
            "y su `workspaceRevision` es la `previousWorkspaceRevision` de aquel: el round-trip \
             cierra (INVERSO al recibo que deshace, `contracts/mcp.yml`)"
        );
        assert_ne!(
            salida.previous_workspace_revision.0, salida.workspace_revision.0,
            "que además son distintas entre sí: iguales es la firma del no-op silencioso"
        );
        assert_eq!(
            revision_actual(&r.app),
            salida.workspace_revision.0,
            "y esa es la revisión REAL del workspace tras deshacer el *undo*"
        );

        let recibo = recibo_por_id(&r.app, &salida.receipt_id).unwrap_or_else(|| {
            panic!(
                "el recibo de la segunda reversión tiene que quedar persistido y visible: sin él la \
                 cadena no se puede seguir deshaciendo. change_revert devolvió {salida:?}"
            )
        });
        assert_eq!(
            recibo.result_revision.0, salida.workspace_revision.0,
            "y es coherente con lo publicado, no un registro decorativo"
        );
        assert_eq!(
            recibo.changed_path_count, 1,
            "declarando la ruta que restauró"
        );

        let previo = recibo_por_id(&r.app, &r.recibo_revert).unwrap_or_else(|| {
            panic!(
                "el recibo del PRIMER revert tiene que seguir existiendo: describe una transacción \
                 distinta y es lo que hace reversible la cadena"
            )
        });
        assert_eq!(
            previo.result_revision.0, r.revert1.workspace_revision.0,
            "y sin reescribirse: sigue declarando la revisión que aquella reversión dejó ({}), no \
             la de ahora",
            r.revert1.workspace_revision.0
        );
        assert_eq!(
            r.estado_a,
            std::fs::read_to_string(
                root.join(".lodestar/runtime/recovery")
                    .join(format!("{}/alfa.md", salida.receipt_id.0))
            )
            .unwrap_or_default(),
            "y la segunda reversión respalda SU estado previo (A) bajo su propia identidad: es lo \
             que permite deshacerla a su vez"
        );
    }

    #[cfg(feature = "test-failpoints")]
    mod con_seams {
        use super::*;
        use lodestar_workspace::failpoints::{self, FailPoint};

        /// Arma `fp`, intenta deshacer el *undo* y comprueba que el punto **se ejerció**
        /// (`failpoints::disparado` autodesarma al dispararse: si sigue armado, nadie lo consultó y
        /// el escenario no se reprodujo).
        fn revierte_cayendo_en(
            app: &App,
            receipt_id: &ReceiptId,
            fp: FailPoint,
            donde: &str,
        ) -> Result<RevertResult, lodestar_app::AppError> {
            failpoints::armar(fp);
            let resultado = app.change_revert(receipt_id, None);
            let seguia_armado = failpoints::disparado(fp);
            failpoints::desarmar();
            assert!(
                !seguia_armado,
                "el punto de caída {fp:?} no se ejerció: nadie lo consulta {donde}. Sin eso el \
                 escenario no se reproduce y el test pasaría vacuamente"
            );
            resultado
        }

        /// **Criterio 5** (`crash_a_mitad_de_revertir_un_revert_no_deja_parciales`) — **Dado** un
        /// crash a mitad del `revert` de un `-revert`, **Cuando** se reabre el workspace, **Entonces**
        /// el canónico converge a uno de los dos bordes (nunca un parcial) y la siguiente operación
        /// funciona al primer intento.
        ///
        /// Los dos puntos cubren los dos lados del punto de no retorno de la inversa: antes del primer
        /// rename (el canónico no se movió) y con la inversa ya publicada pero sin recibo escrito por
        /// la fachada. En ambos, tras reabrir y recuperar, `alfa.md` tiene que ser exactamente `A` o
        /// exactamente `B`, y un `change_revert` posterior del mismo `-revert` tiene que poder dejar
        /// el workspace en `B` **al primer intento** — que es donde hoy la colisión de identidad
        /// arrastra el fallo: la segunda transacción escribe sobre el material de la primera, así que
        /// lo que sobreviva a la caída ya no describe una transacción sino dos mezcladas.
        #[test]
        fn crash_a_mitad_de_revertir_un_revert_no_deja_parciales() {
            let puntos = [
                (
                    FailPoint::TrasJournalPrepared,
                    "en `Workspace::revert_transaction_con_recibo`, tras el journal de la inversa y \
                     ANTES de su primer rename",
                ),
                (
                    FailPoint::TrasLaTransaccionAntesDelRecibo,
                    "en `App::change_revert`, entre el retorno de la transacción inversa y su recibo",
                ),
            ];

            for (fp, donde) in puntos {
                let dir = tempfile::tempdir().unwrap();
                let root = dir.path();
                let r = app_revertida(root);

                let caida = revierte_cayendo_en(&r.app, &r.recibo_revert, fp, donde);
                assert!(
                    caida.is_err(),
                    "precondición: con {fp:?} armado la reversión no puede reportar éxito: {caida:?}"
                );

                // Se «reabre» el workspace, como haría el proceso siguiente.
                drop(r.app);
                let app2 = App::open(root).expect("reabrir el workspace tras la caída");

                let tras_recuperar = alfa(root);
                assert!(
                    tras_recuperar == r.estado_a || tras_recuperar == r.estado_b,
                    "desde {fp:?}, el canónico debe converger a UNO de los dos bordes de la \
                     reversión de la reversión, jamás a un parcial.\nen disco: \
                     {tras_recuperar:?}\nA: {:?}\nB: {:?}",
                    r.estado_a,
                    r.estado_b
                );

                // Y la siguiente operación funciona AL PRIMER INTENTO, dejando el workspace en B.
                if tras_recuperar == r.estado_a {
                    let reintento = app2
                        .change_revert(&r.recibo_revert, None)
                        .unwrap_or_else(|e| {
                            panic!(
                                "tras la caída en {fp:?} y la recuperación, deshacer el *undo* tiene \
                                 que funcionar al PRIMER intento: devolvió {} ({e:?})",
                                e.code.as_str()
                            )
                        });
                    assert!(reintento.reverted, "y publicar: {reintento:?}");
                }
                assert_eq!(
                    alfa(root),
                    r.estado_b,
                    "el estado final tras {fp:?} es el redo (B): o lo dejó la transacción que \
                     sobrevivió a la caída, o lo dejó el reintento"
                );
                assert_eq!(
                    revision_actual(&app2),
                    r.revert1.previous_workspace_revision.0,
                    "y la revisión que reporta la fachada es la que tenía el workspace en B, la \
                     misma que el apply original dejó: el estado converge, no queda a medias"
                );
            }
        }
    }
}
