//! **E15-H09** — La política de escritura respeta el descubrimiento.
//!
//! Fase ROJA (TDD). Origen: hallazgo MAYOR-1 del juez ciego de E15-H07. La fuente normativa es
//! `docs/REFACTOR_PHASE_2.md §Principio 8 (Seguridad de escritura)`: *«Ninguna operación debe: …
//! **escribir sobre archivos excluidos** …»*.
//!
//! ## El defecto que fijan estos tests
//!
//! `Workspace::assert_writable` (`crates/lodestar-workspace/src/external_refs.rs:130-157`) solo
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
        require_conformant_result: false,
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
        err,
        ErrorCode::PermissionDenied,
        "el rechazo de una escritura sobre «{destino}» debe llevar el código estable \
         PERMISSION_DENIED (`REFACTOR_PHASE_2 §Principio 8`); era: {err:?} ({})",
        err.as_str()
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
    let ops = json!([
        { "op": "create", "path": "beta.md", "type": "Nota", "title": "Beta",
          "body": "# Resumen\n\ncuerpo visible\n" },
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
        { "op": "create", "path": ".lodestar/colado.md", "type": "Nota", "title": "Colado",
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
        { "op": "create", "path": "vendor/oculto.md", "type": "Nota", "title": "Oculto",
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
        { "op": "create", "path": "vendor/oculto.md", "type": "Nota", "title": "Oculto",
          "body": "# Oculto\n\ncuerpo\n" },
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
        err,
        ErrorCode::PermissionDenied,
        "el guard del único escritor debe rechazar un destino excluido con PERMISSION_DENIED; \
         era: {} ({err:?})",
        err.as_str()
    );

    // (4) Y no escribió nada: ni el `.md` colado, ni el directorio que lo contendría.
    assert!(
        !root.join("vendor/oculto.md").exists(),
        "un apply rechazado por PERMISSION_DENIED no debe materializar el documento excluido"
    );
}
