//! **E20-H04** — Política de validación y diagnósticos de descubrimiento cableados.
//!
//! Fase ROJA (TDD). Cierra la épica E20 con dos deudas que `REFACTOR_PHASE_2 §Fase 10` y
//! `ARCHITECTURE.md §20.9` dejaron pendientes:
//!
//! 1. **Los diagnósticos de descubrimiento llegan a las fachadas.** Hoy
//!    `lodestar_workspace::discovery::discover` computa `DOC-NOT-UTF8`, `DOC-TOO-LARGE`,
//!    `SYMLINK-UNSUPPORTED`, `PATH-NOT-UTF8` y `LINK-CASE-MISMATCH` (a nivel de inventario) y su
//!    único llamador productivo (`Workspace::document_set()` vía `discover_files()`) **los descarta**.
//!    Media tabla de `§20.9` es invisible para `knowledge_check`/`lodestar check`.
//! 2. **La política `validation`/`transactions` se aplica.** Las secciones `validation`
//!    (severidad por familia) y `transactions.rejectNewErrors`/`allowExistingErrors` de
//!    `.lodestar/config.yaml` (E15-H08) hoy **solo se cargan** — nadie las consulta.
//!
//! ## Superficie observable que fijan estos tests (mi criterio propio, `regla 3` del autor)
//!
//! No sobre-especifico *cómo* se incorporan los `Discovered::diagnostics` al `Analysis` (¿cambia la
//! firma de `document_set()`? ¿un método nuevo? es decisión del implementador). Fijo el **efecto
//! observable por la frontera**:
//!
//! - `descubrimiento_llega_a_check` / `severidad_configurable`: un diagnóstico de descubrimiento (y
//!   la severidad que le asigna la sección `validation`) es visible en `App::knowledge_check` con
//!   `CheckScope::Workspace` — el mismo motor que sirve `lodestar check` y la tool MCP
//!   `knowledge_check`.
//! - `apply_sobre_errores_previos` / `rechaza_errores_nuevos`: la política antes/después gobierna el
//!   veredicto de `App::change_apply` (que atraviesa el gate de `Workspace::validate_staging`). El
//!   gate compara el conjunto de diagnósticos de **error** del workspace pre-plan contra el
//!   post-plan y rechaza **solo** si el después tiene errores que el antes no tenía.
//!
//! ## Estado ROJO esperado por test (verificado con `cargo test`, no supuesto)
//!
//! - `descubrimiento_llega_a_check` — **ROJO**: hoy `document_set()` tira el `DOC-NOT-UTF8`, así que
//!   `knowledge_check` no lo ve. La aserción de presencia falla.
//! - `severidad_configurable` — **ROJO**: la mitad configurada (`caseMismatch: error`) falla porque
//!   hoy la sección `validation` se ignora y el `LINK-CASE-MISMATCH` sigue siendo `Warn`. La mitad
//!   por defecto (`Warn`) ya pasa hoy y es el control anti-vacuidad **dentro** del mismo test.
//! - `apply_sobre_errores_previos` — **ROJO**: hoy el gate rechaza **cualquier** resultado con
//!   errores (`validate_staging` exige `errors == 0`), así que una reparación parcial sobre un
//!   workspace que ya tenía un enlace roto se rechaza con `NONCONFORMANT_RESULT`. El test espera que
//!   el apply se **permita**.
//! - `rechaza_errores_nuevos` — **GUARDA (verde hoy)**: introducir un error nuevo debe rechazarse, y
//!   el gate absoluto de hoy ya lo rechaza (por la razón «hay errores», no por «hay errores
//!   *nuevos*»). Su valor es blindar la implementación futura contra una relajación excesiva de
//!   `allowExistingErrors` («si antes había errores, permite cualquier cosa»): se monta sobre un
//!   workspace que **ya** tiene un error preexistente y comprueba que aun así el error **nuevo** se
//!   rechaza. Es el par natural de `apply_sobre_errores_previos` (permitir lo existente / rechazar lo
//!   nuevo); se documenta como guarda porque, en aislamiento, no puede ir roja mientras el gate de
//!   hoy sea «rechaza cualquier error».
//!
//! Ningún stub de producción hace falta: los cuatro tests compilan contra la API pública actual
//! (`App::knowledge_check`, `App::change_plan`, `App::change_apply`, `CheckScope`, `CheckReport`,
//! `Severity`, `CheckCode`, `ErrorCode`) y fallan en tiempo de ejecución (aserción incumplida), que
//! es el rojo ideal.

use std::path::Path;

use lodestar_app::{App, CheckReport, CheckScope};
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::{CheckCode, ErrorCode, Severity};
use serde_json::{json, Value};

// ---------------------------------------------------------------------------
// Utilidades compartidas
// ---------------------------------------------------------------------------

/// Escribe un fichero dentro del workspace temporal, creando los directorios intermedios.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Política de plan permisiva: el veredicto de conformidad del plan (`canApply`) no debe confundir
/// estos tests — lo que se prueba es el **gate del apply** (E20-H04), no la advertencia de
/// `change_plan`. Sin esto un plan podría marcarse no-aplicable por una razón distinta de la fijada.
fn policy_permisiva() -> PlanPolicy {
    PlanPolicy {
        require_conformant_result: false,
        allow_warnings: true,
    }
}

/// Audita todo el workspace con `knowledge_check` (scope workspace, umbral `Info` para no filtrar
/// nada salvo los `Pass`, límite holgado para que la paginación no oculte diagnósticos).
fn check_workspace(app: &App) -> CheckReport {
    app.knowledge_check(
        &CheckScope::Workspace,
        Some(Severity::Info),
        false,
        Some(1000),
        None,
    )
    .expect("knowledge_check(workspace) debe responder")
}

/// El primer diagnóstico del reporte con este código (si lo hay).
fn diag_con_codigo(report: &CheckReport, code: CheckCode) -> Option<&lodestar_core::types::Check> {
    report.diagnostics.iter().find(|c| c.code == code)
}

/// Resumen legible de los diagnósticos, para los mensajes de fallo.
fn resumen(report: &CheckReport) -> String {
    report
        .diagnostics
        .iter()
        .map(|c| {
            let targets: Vec<&str> = c.targets.iter().map(|t| t.as_str()).collect();
            format!("{}/{:?} {:?}", c.code.as_str(), c.level, targets)
        })
        .collect::<Vec<_>>()
        .join(", ")
}

// ===========================================================================
// Criterio: `descubrimiento_llega_a_check`
// ===========================================================================

/// **Dado** un workspace con un `.md` no-UTF8, **Cuando** se corre `knowledge_check`, **Entonces**
/// el `DOC-NOT-UTF8` aparece en el reporte.
///
/// El fixture combina documentos válidos y mutuamente enlazados (sus enlaces resuelven todos, así
/// que no aportan diagnósticos que confundan) con un `binario.md` de bytes no-UTF8 escrito
/// directamente en disco (los mismos que usa `lodestar_fixtures::materialize_disk_only`). El
/// descubrimiento ya emite `DOC-NOT-UTF8` para ese fichero (ver `tests/discovery.rs` en
/// `lodestar-workspace`); lo que falta —y lo que fija este test— es que ese diagnóstico atraviese
/// `document_set()`/`analyze()` hasta el reporte de `knowledge_check`.
///
/// **Nota sobre la superficie**: `binario.md` **no** es un documento del inventario (no se pudo
/// interpretar), así que su diagnóstico no está indexado por un `RelPath` de `Analysis::documents`.
/// El test no asume por dónde lo cuela la implementación: solo exige que, con `CheckScope::Workspace`,
/// el `DOC-NOT-UTF8` esté en `report.diagnostics`. Ese es el contrato observable.
///
/// ROJO hoy: `Workspace::document_set()` descarta los `Discovered::diagnostics`, luego el reporte no
/// contiene `DOC-NOT-UTF8` y la aserción de presencia falla.
#[test]
fn descubrimiento_llega_a_check() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();

    // Documentos válidos y mutuamente enlazados (sin diagnósticos propios).
    escribe(
        root,
        "README.md",
        "# Proyecto\n\nEmpieza por [lo primero](docs/uno.md).\n",
    );
    escribe(
        root,
        "docs/uno.md",
        "# Uno\n\nVuelve al [inicio](../README.md).\n",
    );
    // El `.md` no-UTF8: 0xF0 abre una secuencia de 4 bytes y 0x28 no es continuación válida.
    std::fs::write(root.join("binario.md"), [0xF0, 0x28, 0x8C, 0xBC]).unwrap();

    let app = App::open(root).expect("el workspace temporal debe abrir");

    // Precondición no vacua: hay documentos que analizar (si no, el reporte estaría vacío por otra
    // razón y el test no probaría nada).
    let report = check_workspace(&app);
    assert!(
        !app.workspace()
            .document_set()
            .unwrap()
            .analyze()
            .documents
            .is_empty(),
        "precondición: el workspace debe tener documentos válidos que analizar"
    );

    let diag = diag_con_codigo(&report, CheckCode::DocNotUtf8);
    let diag = diag.unwrap_or_else(|| {
        panic!(
            "un `.md` no-UTF8 debe reportarse como DOC-NOT-UTF8 en knowledge_check(workspace): los \
             diagnósticos de descubrimiento aún no llegan a la fachada (hoy `document_set()` los \
             descarta). Diagnósticos vistos: [{}]",
            resumen(&report)
        )
    });
    assert!(
        diag.targets.iter().any(|t| t.as_str() == "binario.md") || diag.msg.contains("binario.md"),
        "el DOC-NOT-UTF8 debe señalar al fichero culpable `binario.md`: {diag:?}"
    );
}

// ===========================================================================
// Criterio: `severidad_configurable`
// ===========================================================================

/// Monta un workspace mínimo con **exactamente** una colisión de capitalización y nada más:
/// - `docs/auth.md`: un documento real, sin frontmatter ni enlaces (silencioso).
/// - `indice.md`: enlaza a `docs/Auth.md` (capitalización errónea). El core lo resuelve a un destino
///   ausente que el inventario tiene *salvo capitalización* → `LINK-CASE-MISMATCH` (familia
///   `caseMismatch`).
///
/// Se usa la colisión **por enlace** (`links::diagnose`) y no la colisión de inventario del
/// descubrimiento (`case_collisions`) por portabilidad: en un volumen case-insensitive (APFS/NTFS,
/// dos de las tres plataformas del CI) no se pueden materializar dos ficheros que solo difieran en
/// capitalización — colapsan en uno. La colisión por enlace es un solo fichero real más texto, así
/// que el escenario es determinista en las tres plataformas. Ambos productores emiten el **mismo
/// código** `LINK-CASE-MISMATCH`, y la severidad de la sección `validation` se asigna **por
/// familia/código**: `caseMismatch` gobierna cualquier `LINK-CASE-MISMATCH`, venga del inventario o
/// de un enlace. (Ese es el criterio que fija este test.)
fn semilla_case_mismatch(root: &Path) {
    escribe(
        root,
        "docs/auth.md",
        "# Auth\n\nDocumento real, en minúsculas.\n",
    );
    escribe(
        root,
        "indice.md",
        "# Índice\n\nVer la [autenticación](docs/Auth.md).\n",
    );
}

/// **Dado** el default (sin `validation` en la config), **Cuando** hay una colisión de
/// capitalización, **Entonces** es `warning` (mitad de control anti-vacuidad: ya pasa hoy).
///
/// Va junta con la mitad configurada en el mismo criterio de la épica, pero se separa en dos tests
/// para que el rojo sea inequívoco: esta mitad ancla el comportamiento por defecto y **no** debe
/// romperse cuando se aplique la política.
#[test]
fn severidad_configurable_por_defecto_es_warning() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    semilla_case_mismatch(root);
    // Sin `.lodestar/config.yaml`: defaults de `§20.9` (`caseMismatch: warning`).

    let app = App::open(root).expect("el workspace temporal debe abrir");
    let report = check_workspace(&app);

    let diag = diag_con_codigo(&report, CheckCode::LinkCaseMismatch).unwrap_or_else(|| {
        panic!(
            "el enlace con capitalización errónea debe producir un LINK-CASE-MISMATCH. \
             Diagnósticos: [{}]",
            resumen(&report)
        )
    });
    assert_eq!(
        diag.level,
        Severity::Warn,
        "por defecto (`caseMismatch: warning`) una colisión de capitalización es un aviso, no un \
         error: {diag:?}"
    );
    // Sin más diagnósticos de error, el workspace es conforme por defecto.
    assert_eq!(
        report.summary.errors,
        0,
        "por defecto la colisión de capitalización no debe contar como error: {}",
        resumen(&report)
    );
    assert!(
        report.conformant,
        "por defecto (colisión = aviso) el workspace es conforme"
    );
}

/// **Dado** `validation.caseMismatch: error` en la config, **Cuando** hay una colisión de
/// capitalización, **Entonces** es **error** (no el warning por defecto) → `severidad_configurable`.
///
/// ROJO hoy: la sección `validation` se carga pero no se aplica (`config.rs`: «Solo se carga»), así
/// que el `LINK-CASE-MISMATCH` sigue siendo `Warn` y las aserciones de error fallan.
#[test]
fn severidad_configurable() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    semilla_case_mismatch(root);
    // La config eleva la familia `caseMismatch` a `error`.
    escribe(
        root,
        ".lodestar/config.yaml",
        "validation:\n  caseMismatch: error\n",
    );

    let app = App::open(root).expect("el workspace temporal debe abrir");
    let report = check_workspace(&app);

    let diag = diag_con_codigo(&report, CheckCode::LinkCaseMismatch).unwrap_or_else(|| {
        panic!(
            "el enlace con capitalización errónea debe producir un LINK-CASE-MISMATCH. \
             Diagnósticos: [{}]",
            resumen(&report)
        )
    });
    assert_eq!(
        diag.level,
        Severity::Err,
        "con `validation.caseMismatch: error` la colisión de capitalización debe ser un ERROR, no \
         el aviso por defecto: la política `validation` no se está aplicando. {diag:?}"
    );
    assert!(
        report.summary.errors >= 1,
        "la colisión elevada a error debe contar en el resumen de errores: {}",
        resumen(&report)
    );
    assert!(
        !report.conformant,
        "con la colisión elevada a error, el workspace deja de ser conforme"
    );
}

// ===========================================================================
// Criterios: `apply_sobre_errores_previos` / `rechaza_errores_nuevos`
// ===========================================================================

/// Semilla común de los dos tests de política antes/después: un workspace que **ya** tiene un enlace
/// roto (error preexistente `LINK-TARGET-MISSING` sobre `roto.md`) más un documento limpio
/// (`limpio.md`).
///
/// Escribe además una config **explícita** con la política por defecto de `§Fase 10`
/// (`rejectNewErrors: true`, `allowExistingErrors: true`) — coincide con
/// `TransactionsSection::default`, pero declararla documenta la intención y ejercita la vía de carga.
fn semilla_con_error_preexistente(root: &Path) {
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
    escribe(
        root,
        ".lodestar/config.yaml",
        "transactions:\n  rejectNewErrors: true\n  allowExistingErrors: true\n",
    );
}

/// Aplica `ops` como un ciclo `change_plan` → `change_apply` completo (política de plan permisiva
/// para no confundir el veredicto del plan con el gate del apply). Devuelve el resultado del apply.
fn planifica_y_aplica(app: &App, ops: &Value) -> Result<lodestar_app::ApplyResult, ErrorCode> {
    let plan = app
        .change_plan(None, ops, policy_permisiva())
        .expect("el change_plan debe producir un plan");
    app.change_apply(&plan.change_set_id, None)
}

/// **Dado** un workspace que **ya** tiene un enlace roto, **Cuando** se aplica un cambio que **no**
/// añade errores, **Entonces** el apply se permite (`allowExistingErrors`) →
/// `apply_sobre_errores_previos`.
///
/// El cambio toca **otro** documento (`patch_frontmatter` sobre `limpio.md`): la reparación es
/// parcial y el error preexistente de `roto.md` **sigue existiendo** tras el apply. El criterio no es
/// «el error desaparece» sino «el apply se **permite** pese al error preexistente».
///
/// ROJO hoy: `Workspace::validate_staging` rechaza cualquier resultado con `errors > 0`
/// (`conformant == errors == 0`). Como `roto.md` sigue roto en el árbol resultante, hoy el apply se
/// rechaza con `NONCONFORMANT_RESULT`; este test espera `Ok`.
#[test]
fn apply_sobre_errores_previos() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    semilla_con_error_preexistente(root);

    let app = App::open(root).expect("el workspace temporal debe abrir");

    // Precondición no vacua: el error preexistente EXISTE antes del cambio.
    let antes = check_workspace(&app);
    assert!(
        diag_con_codigo(&antes, CheckCode::LinkTargetMissing)
            .is_some_and(|c| c.level == Severity::Err),
        "precondición: `roto.md` debe aportar un LINK-TARGET-MISSING de nivel error antes del \
         cambio. Diagnósticos: [{}]",
        resumen(&antes)
    );

    // Cambio que NO añade errores: un patch inocuo sobre otro documento (limpio).
    let ops = json!([
        { "op": "patch_frontmatter", "ref": { "path": "limpio.md" },
          "patch": { "status": "revisado" } },
    ]);

    let resultado = planifica_y_aplica(&app, &ops);
    let apply = match resultado {
        Ok(a) => a,
        Err(e) => panic!(
            "una reparación parcial que NO introduce errores debe permitirse sobre un workspace que \
             ya los tiene (`allowExistingErrors`); el gate la rechazó con {e:?} ({}). El gate está \
             comparando el resultado contra «cero errores» en vez de contra el estado previo.",
            e.as_str()
        ),
    };
    assert!(
        apply.applied,
        "el apply permitido debe reportar applied: true"
    );

    // El error preexistente SIGUE ahí: no se reparó, solo se toleró (el criterio no pide que
    // desaparezca).
    let despues = check_workspace(&app);
    assert!(
        diag_con_codigo(&despues, CheckCode::LinkTargetMissing).is_some(),
        "el apply permitido no repara el error preexistente de `roto.md`; debe seguir presente tras \
         el cambio. Diagnósticos: [{}]",
        resumen(&despues)
    );
}

/// **Dado** un cambio que **introduciría** un enlace roto nuevo, **Cuando** se aplica con
/// `rejectNewErrors`, **Entonces** se rechaza → `rechaza_errores_nuevos`.
///
/// GUARDA (verde hoy): el gate absoluto de hoy ya rechaza cualquier resultado con errores, así que
/// este test **pasa** en la fase roja. Su función es blindar la implementación futura de la política
/// antes/después contra una relajación excesiva: se monta sobre un workspace que **ya** tiene un
/// error preexistente (`roto.md`) y comprueba que aun así el error **nuevo** (un `create` con un
/// enlace roto en `nuevo.md`) se rechaza. Una implementación ingenua de `allowExistingErrors` («si
/// antes había errores, permite cualquier cosa») haría pasar `apply_sobre_errores_previos` pero
/// rompería este test — que es justo lo que lo hace útil como par.
#[test]
fn rechaza_errores_nuevos() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    semilla_con_error_preexistente(root);

    let app = App::open(root).expect("el workspace temporal debe abrir");

    // Precondición: el workspace ya tiene un error preexistente (para que la guarda contra la
    // relajación excesiva sea significativa: el «antes» no está limpio).
    let antes = check_workspace(&app);
    assert!(
        diag_con_codigo(&antes, CheckCode::LinkTargetMissing).is_some(),
        "precondición: debe existir un error preexistente en `roto.md`. Diagnósticos: [{}]",
        resumen(&antes)
    );

    // Cambio que INTRODUCE un error NUEVO: crea un documento con un enlace a un `.md` inexistente
    // (destino distinto del enlace roto preexistente).
    let ops = json!([
        { "op": "create", "path": "nuevo.md", "type": "Nota", "title": "Nuevo",
          "body": "# Nuevo\n\nEnlace roto recién introducido: [otro](inexistente-nuevo.md).\n" },
    ]);

    let resultado = planifica_y_aplica(&app, &ops);
    let err = match resultado {
        Err(e) => e,
        Ok(apply) => panic!(
            "un cambio que introduce un enlace roto NUEVO debe rechazarse (`rejectNewErrors`), \
             incluso sobre un workspace que ya tenía errores; en su lugar se aplicó: changedPaths={:?}",
            apply.changed_paths
        ),
    };
    assert_eq!(
        err,
        ErrorCode::NonconformantResult,
        "introducir un error nuevo debe rechazarse con NONCONFORMANT_RESULT; era: {} ({err:?})",
        err.as_str()
    );

    // El documento nuevo no se publicó (el apply rechazado no toca el canónico).
    assert!(
        !root.join("nuevo.md").exists(),
        "un apply rechazado por introducir un error nuevo no debe materializar `nuevo.md`"
    );
}
