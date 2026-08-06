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
//!   workspace que ya tenía un enlace roto se rechaza con `INVALID_RESULT`. El test espera que
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
use lodestar_core::types::{CheckCode, ErrorCode, RelPath, Severity};
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
        require_valid_result: false,
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
        report.valid,
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
        !report.valid,
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
fn planifica_y_aplica(
    app: &App,
    ops: &Value,
) -> Result<lodestar_app::ApplyResult, lodestar_app::AppError> {
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
/// (`valid == errors == 0`). Como `roto.md` sigue roto en el árbol resultante, hoy el apply se
/// rechaza con `INVALID_RESULT`; este test espera `Ok`.
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
            e.code.as_str()
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
        { "op": "create", "path": "nuevo.md",
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
        err.code,
        ErrorCode::InvalidResult,
        "introducir un error nuevo debe rechazarse con INVALID_RESULT; era: {} ({err:?})",
        err.code.as_str()
    );

    // El documento nuevo no se publicó (el apply rechazado no toca el canónico).
    assert!(
        !root.join("nuevo.md").exists(),
        "un apply rechazado por introducir un error nuevo no debe materializar `nuevo.md`"
    );
}

// ===========================================================================
// E23-H01 — Una sola verdad de validación (mitad `lodestar-app`)
//
// El criterio de aceptación de la historia se ejercita por las DOS superficies en
// `crates/lodestar-cli/tests/cli.rs` (`check_y_knowledge_check_coinciden_con_ignore` /
// `…_con_error` / `check_ve_diagnosticos_de_descubrimiento`): allí se compara el exit code del
// binario real contra el veredicto de `knowledge_check`.
//
// Este test es su complemento en esta capa y fija DÓNDE vive la corrección: en
// `App::full_analysis()` —el punto que comparten `lodestar check` y cualquier futura fachada—, no en
// `commands.rs`. El alcance de la historia lo dice explícitamente: «`commands.rs` no cambia de
// forma: sigue derivando `valid` de la ausencia de `Err`, pero sobre el análisis correcto».
// Sin este test, un parche que recompusiera la política dentro de la CLI pasaría los tests de
// `cli.rs` y dejaría la asimetría intacta para el siguiente consumidor.
// ===========================================================================

/// **E23-H01** (complemento de `check_y_knowledge_check_coinciden_con_ignore`):
/// **Dado** un workspace con un enlace roto y un YAML ilegible y una config que pone ambas familias
/// a `ignore`, **Cuando** se pide `App::full_analysis()`, **Entonces** su `Analysis` ya no contiene
/// ningún diagnóstico `Err` — el mismo veredicto que da `App::knowledge_check` sobre el mismo
/// workspace (invariante #3 de `CLAUDE.md`: una sola verdad computada).
///
/// ROJO hoy: `full_analysis` hace `document_set().analyze()` a secas, así que devuelve las
/// severidades **intrínsecas** (`LINK-TARGET-MISSING`/`FM-YAML-INVALID` como `Err`) e ignora la
/// sección `validation`, mientras que `knowledge_check` los suprime.
#[test]
fn full_analysis_aplica_la_politica_de_validacion() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    escribe(
        root,
        "notas/rota.md",
        "# Rota\n\nEnlace a un documento inexistente: [falta](inexistente.md).\n",
    );
    escribe(
        root,
        "notas/ilegible.md",
        "---\ntitulo: [sin cerrar\notra: \"comilla\n---\n\n# Ilegible\n\nCuerpo.\n",
    );
    escribe(
        root,
        ".lodestar/config.yaml",
        "validation:\n  danglingDocumentLinks: ignore\n  malformedFrontmatter: ignore\n",
    );

    let app = App::open(root).expect("el workspace temporal debe abrir");

    // Referencia: el veredicto del servicio que SÍ aplica la política (E20-H04).
    let report = check_workspace(&app);
    assert_eq!(
        report.summary.errors,
        0,
        "precondición: con las dos familias a `ignore`, knowledge_check no cuenta errores. \
         Diagnósticos: [{}]",
        resumen(&report)
    );

    // El análisis que alimenta a `lodestar check` debe decir LO MISMO.
    let analysis = app
        .full_analysis()
        .expect("full_analysis debe computar el análisis del working tree");
    let errores: Vec<String> = analysis
        .diagnostics
        .iter()
        .flat_map(|(path, checks)| {
            checks
                .iter()
                .filter(|c| c.level == Severity::Err)
                .map(move |c| format!("{}:{}", path.as_str(), c.code.as_str()))
        })
        .collect();
    assert!(
        errores.is_empty(),
        "`full_analysis` debe aplicar la misma política de severidad que `knowledge_check` \
         (`ValidationSection::effective_severity`): con las dos familias a `ignore` no puede quedar \
         ningún diagnóstico de error. Errores encontrados: {errores:?}"
    );
}

// ===========================================================================
// E29-H05 — `knowledge_check` scope `paths` con un path inexistente responde `DOCUMENT_NOT_FOUND`
// (`requirements/epica-29-honestidad-superficie.md §E29-H05`, `decisiones §23/A-07`).
//
// Mitad de servicio (sin levantar proceso MCP) del criterio que `crates/lodestar-mcp/tests/mcp.rs`
// ejerce por el wire. Causa raíz: `App::scope_paths` (`crates/lodestar-app/src/lib.rs`, brazo
// `CheckScope::Paths`) es `Ok(paths.iter().cloned().collect())` — no comprueba que cada path exista
// en `analysis.documents`, a diferencia de los brazos `Document`/`Affected`, que resuelven con
// `self.resolve_ref(…)?`.
//
// ROJO esperado HOY: por ASERCIÓN (ningún stub — el brazo `Paths` ya existe y compila).
// ===========================================================================

/// `RelPath` de conveniencia para construir el argumento `paths` de `CheckScope::Paths`.
fn rp(p: &str) -> RelPath {
    RelPath::new(p).unwrap()
}

/// Workspace mínimo: un único documento real, sin enlaces ni frontmatter.
fn app_con_un_documento() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "notas/alfa.md",
        "# Alfa\n\nDocumento real, sin enlaces ni frontmatter.\n",
    );
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");
    (dir, app)
}

/// `check_scope_paths_con_path_inexistente_falla` (mitad de servicio): **Dado** un workspace con
/// `notas/alfa.md`, **Cuando** se llama a `knowledge_check` con `scope: paths{["notas/no-existe.md"]}`,
/// **Entonces** `Err(ErrorCode::DocumentNotFound)` que nombra el path.
#[test]
fn check_scope_paths_con_path_inexistente_falla() {
    let (_dir, app) = app_con_un_documento();
    let scope = CheckScope::Paths {
        paths: vec![rp("notas/no-existe.md")],
    };
    let resultado = app.knowledge_check(&scope, Some(Severity::Info), false, None, None);
    let err = resultado.expect_err(
        "un scope `paths` con un path inexistente debe fallar, no devolver un informe vacío",
    );
    assert_eq!(
        err.code,
        ErrorCode::DocumentNotFound,
        "el código debe ser DOCUMENT_NOT_FOUND, era {} ({err:?})",
        err.code.as_str()
    );
    assert!(
        err.message.contains("notas/no-existe.md"),
        "el mensaje debe nombrar el path que no resolvió: {err:?}"
    );
}

/// `check_scope_paths_falla_aunque_haya_paths_validos` (mitad de servicio): una lista mixta (un path
/// real + uno inexistente) falla entera — no devuelve el informe parcial del real.
#[test]
fn check_scope_paths_falla_aunque_haya_paths_validos() {
    let (_dir, app) = app_con_un_documento();
    let scope = CheckScope::Paths {
        paths: vec![rp("notas/alfa.md"), rp("notas/typo.md")],
    };
    let err = app
        .knowledge_check(&scope, Some(Severity::Info), false, None, None)
        .expect_err(
            "una lista mixta con al menos un path inexistente debe fallar entera, no devolver el \
             informe parcial del path real",
        );
    assert_eq!(err.code, ErrorCode::DocumentNotFound, "era {err:?}");
    assert!(
        err.message.contains("notas/typo.md"),
        "el mensaje debe nombrar el path inexistente, no el real: {err:?}"
    );
}

/// `check_scope_paths_reporta_el_primer_path_inexistente` (mitad de servicio): con dos paths
/// inexistentes, el mensaje nombra el PRIMERO de la lista recibida, no el que ordenaría antes en un
/// `BTreeSet`.
#[test]
fn check_scope_paths_reporta_el_primer_path_inexistente() {
    let (_dir, app) = app_con_un_documento();
    let scope = CheckScope::Paths {
        paths: vec![rp("zzz-no-existe.md"), rp("aaa-no-existe.md")],
    };
    let err = app
        .knowledge_check(&scope, Some(Severity::Info), false, None, None)
        .expect_err("dos paths inexistentes deben fallar");
    assert_eq!(err.code, ErrorCode::DocumentNotFound, "era {err:?}");
    assert!(
        err.message.contains("zzz-no-existe.md"),
        "el mensaje debe nombrar el PRIMERO de la lista recibida: {err:?}"
    );
    assert!(
        !err.message.contains("aaa-no-existe.md"),
        "el mensaje NO debe nombrar el segundo path inexistente: {err:?}"
    );
}

/// `check_scope_paths_trata_lo_excluido_como_inexistente` (mitad de servicio): un path excluido por
/// `.lodestarignore` no está en el inventario y por tanto cuenta como inexistente — mismo criterio
/// que ya aplica `resolve_ref`.
#[test]
fn check_scope_paths_trata_lo_excluido_como_inexistente() {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "notas/alfa.md",
        "# Alfa\n\nDocumento real, sin enlaces ni frontmatter.\n",
    );
    escribe(
        dir.path(),
        "borradores/wip.md",
        "# WIP\n\nExcluido por .lodestarignore.\n",
    );
    escribe(dir.path(), ".lodestarignore", "borradores/\n");
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");

    let scope = CheckScope::Paths {
        paths: vec![rp("borradores/wip.md")],
    };
    let err = app
        .knowledge_check(&scope, Some(Severity::Info), false, None, None)
        .expect_err(
            "un path excluido por .lodestarignore (fuera del inventario) debe dar \
             DOCUMENT_NOT_FOUND, no un informe vacío",
        );
    assert_eq!(err.code, ErrorCode::DocumentNotFound, "era {err:?}");
    assert!(
        err.message.contains("borradores/wip.md"),
        "el mensaje debe nombrar el path excluido: {err:?}"
    );
}

/// `check_scope_paths_valido_sigue_funcionando` (control anti-vacuo, mitad de servicio): un scope
/// con paths que TODOS existen sigue funcionando exactamente como hoy.
#[test]
fn check_scope_paths_valido_sigue_funcionando() {
    let (_dir, app) = app_con_un_documento();
    let scope = CheckScope::Paths {
        paths: vec![rp("notas/alfa.md")],
    };
    let report = app
        .knowledge_check(&scope, Some(Severity::Info), false, None, None)
        .expect("un scope `paths` con paths que TODOS existen no debe fallar");
    assert!(
        report.valid,
        "el único documento del scope no tiene diagnósticos: el informe debe ser válido: {}",
        resumen(&report)
    );
}

/// `check_scope_paths_vacio_no_es_error` (control anti-vacuo del borde, mitad de servicio): un
/// scope `paths` vacío sigue devolviendo un informe vacío sin error.
#[test]
fn check_scope_paths_vacio_no_es_error() {
    let (_dir, app) = app_con_un_documento();
    let scope = CheckScope::Paths { paths: vec![] };
    let report = app
        .knowledge_check(&scope, Some(Severity::Info), false, None, None)
        .expect("un scope `paths` VACÍO no debe ser un error");
    assert!(
        report.diagnostics.is_empty(),
        "un scope `paths` vacío no puede aportar ningún diagnóstico: {}",
        resumen(&report)
    );
    assert!(report.valid, "un scope vacío es trivialmente válido");
}

// ===========================================================================
// E29-H06 — Workspace vacío: aviso `WORKSPACE-EMPTY` en vez de silencio
// (`requirements/epica-29-honestidad-superficie.md §E29-H06`, `decisiones §16(f)`).
//
// Mitad de análisis (sin binario ni proceso MCP): fija que `App::full_analysis()` —el punto que
// alimenta `lodestar check`— y `App::knowledge_check(scope: workspace)` publican el MISMO
// diagnóstico `WORKSPACE-EMPTY` cuando el inventario queda vacío, con severidad `Warn` y sin tocar
// el veredicto `valid`/`hard_fail` (invariante #3: una sola verdad computada).
//
// PUERTA DE DECISIÓN DE ANCLAJE (declarada en la spec, resuelta aquí): un `WORKSPACE-EMPTY` no
// describe un fichero, así que no tiene `target`. La spec propone dos resoluciones y deja que la
// fase roja elija:
//   (a) anclarlo a la RAÍZ como target, si existe un `RelPath` válido para ella;
//   (b) extender el indexado de `full_analysis` (`lib.rs` ~L1306-1316) para los diagnósticos SIN
//       target, que hoy se descartan con `let Some(anchor) = check.targets.first().cloned() else
//       { continue; }`.
//
// Verificado en código: `RelPath::new("")` es un `Err` explícito (types.rs, «rechaza… la cadena
// vacía»), y la raíz del workspace no tiene ningún segmento relativo a sí misma que sobreviva a
// `from_segments` (todo `""`/"."` se descarta antes de comprobar `parts.is_empty()`). La opción (a)
// es por tanto INVIABLE sin relajar el invariante #6 (chokepoint de `RelPath`), que la propia
// historia prohíbe tocar. **Se elige la opción (b)**: `full_analysis` deja de descartar los
// diagnósticos de descubrimiento sin `target`. La forma concreta de la clave con la que entran en
// `Analysis::diagnostics: BTreeMap<RelPath, Vec<Check>>` la decide el implementador (p. ej. una
// clave sentinela estable); estos tests NO la asumen — solo exigen el efecto observable: que el
// código `WORKSPACE-EMPTY` esté presente en `analysis.diagnostics.values().flatten()` y en
// `report.diagnostics`, por las DOS fachadas.
//
// ROJO esperado HOY: por ASERCIÓN. No hay productor de `WORKSPACE-EMPTY` en ninguna parte (el
// stub de `CheckCode::WorkspaceEmpty` es solo la firma, sin lógica), así que la búsqueda del
// código en los diagnósticos falla siempre.
// ===========================================================================

/// Workspace temporal sin ni un solo `.md`: el caso más simple del síntoma.
fn app_vacio() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    // Un fichero que NO es Markdown no debe cambiar nada: el inventario de documentos sigue vacío.
    escribe(dir.path(), "LEEME.txt", "esto no es un documento OKF\n");
    let app = App::open(dir.path())
        .expect("un directorio sin `.md` sigue siendo workspace válido (§20.1)");
    (dir, app)
}

/// Busca `WORKSPACE-EMPTY` en `analysis.diagnostics` (indexados por lo que decida el implementador
/// para la clave sin target — ver la nota de anclaje arriba): recorre TODOS los valores del mapa,
/// no una clave concreta.
fn tiene_workspace_empty(analysis: &lodestar_core::types::Analysis) -> bool {
    analysis
        .diagnostics
        .values()
        .flatten()
        .any(|c| c.code == CheckCode::WorkspaceEmpty)
}

/// `full_analysis_de_workspace_vacio_avisa` (mitad de servicio del criterio
/// `check_en_workspace_vacio_avisa_con_exit_0`): **Dado** un workspace sin ningún `.md`, **Cuando**
/// se pide `App::full_analysis()`, **Entonces** el `Analysis` contiene un diagnóstico
/// `WORKSPACE-EMPTY` de severidad `Warn`, y el veredicto (`hard_fail() == 0`) no cambia.
#[test]
fn full_analysis_de_workspace_vacio_avisa() {
    let (_dir, app) = app_vacio();

    let analysis = app
        .full_analysis()
        .expect("full_analysis debe computar el análisis de un workspace vacío sin error");

    assert!(
        analysis.documents.is_empty(),
        "precondición: el inventario debe estar vacío de verdad (0 documentos), o el test no \
         prueba el síntoma de la historia"
    );
    assert!(
        tiene_workspace_empty(&analysis),
        "un workspace con 0 documentos debe llevar el diagnóstico WORKSPACE-EMPTY en \
         `full_analysis` (el punto que alimenta `lodestar check`): diagnósticos vistos = {:?}",
        analysis
            .diagnostics
            .values()
            .flatten()
            .map(|c| c.code.as_str())
            .collect::<Vec<_>>()
    );
    let severidad = analysis
        .diagnostics
        .values()
        .flatten()
        .find(|c| c.code == CheckCode::WorkspaceEmpty)
        .map(|c| c.level);
    assert_eq!(
        severidad,
        Some(Severity::Warn),
        "WORKSPACE-EMPTY debe ser un AVISO, no un error: un repo vacío sigue siendo válido (§20.1)"
    );
    assert_eq!(
        analysis.hard_fail(),
        0,
        "el aviso NO puede convertirse en hard-fail: el exit code de `lodestar check` sobre un \
         workspace vacío debe seguir siendo 0"
    );
}

/// `knowledge_check_de_workspace_vacio_avisa` (mitad de servicio del criterio
/// `knowledge_check_en_workspace_vacio_avisa`): el mismo diagnóstico, visible por
/// `App::knowledge_check(scope: workspace)`, sin tumbar `report.valid`.
#[test]
fn knowledge_check_de_workspace_vacio_avisa() {
    let (_dir, app) = app_vacio();

    let report = check_workspace(&app);

    assert!(
        report
            .diagnostics
            .iter()
            .any(|c| c.code == CheckCode::WorkspaceEmpty),
        "knowledge_check(scope: workspace) sobre un inventario vacío debe reportar \
         WORKSPACE-EMPTY: {}",
        resumen(&report)
    );
    assert!(
        report
            .diagnostics
            .iter()
            .find(|c| c.code == CheckCode::WorkspaceEmpty)
            .is_some_and(|c| c.level == Severity::Warn),
        "el diagnóstico debe ser Warn: {}",
        resumen(&report)
    );
    assert!(
        report.valid,
        "un workspace vacío SIN otros diagnósticos sigue siendo `valid: true`: el aviso no puede \
         tumbar el veredicto: {}",
        resumen(&report)
    );
}

/// `workspace_con_todo_excluido_tambien_avisa` (mitad de servicio): un workspace CON `.md` pero
/// cuya `discovery.include` los excluye a todos es el mismo caso engañoso — «no hay inventario», no
/// solo «no hay ficheros» — y también debe avisar.
#[test]
fn workspace_con_todo_excluido_tambien_avisa() {
    let dir = tempfile::tempdir().unwrap();
    escribe(dir.path(), "notas/alfa.md", "# Alfa\n\ncontenido real.\n");
    // `include` restrictivo: solo casaría con una carpeta que no existe. `alfa.md` sobrevive al
    // descubrimiento (existe, es `.md`) pero el filtro `include` lo descarta del inventario.
    escribe(
        dir.path(),
        ".lodestar/config.yaml",
        "discovery:\n  include: [\"solo-esto/**/*.md\"]\n",
    );
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");

    let analysis = app
        .full_analysis()
        .expect("full_analysis debe computar el análisis de un workspace con todo excluido");
    assert!(
        analysis.documents.is_empty(),
        "precondición: con `include` restrictivo el inventario de DOCUMENTOS debe quedar vacío, \
         aunque el `.md` exista en disco"
    );
    assert!(
        tiene_workspace_empty(&analysis),
        "un `discovery.include` que excluye TODO también es un inventario vacío y debe avisar \
         (el caso engañoso no es solo «no hay ficheros»): diagnósticos vistos = {:?}",
        analysis
            .diagnostics
            .values()
            .flatten()
            .map(|c| c.code.as_str())
            .collect::<Vec<_>>()
    );
}

/// `workspace_con_documentos_no_avisa` (control anti-vacuo, mitad de servicio): un workspace con AL
/// MENOS un documento no lleva `WORKSPACE-EMPTY`, ni en `full_analysis` ni en `knowledge_check`.
#[test]
fn workspace_con_documentos_no_avisa() {
    let (_dir, app) = app_con_un_documento();

    let analysis = app
        .full_analysis()
        .expect("full_analysis debe computar el análisis de un workspace con documentos");
    assert!(
        !analysis.documents.is_empty(),
        "precondición: el workspace debe tener al menos un documento"
    );
    assert!(
        !tiene_workspace_empty(&analysis),
        "un workspace con documentos NO debe llevar WORKSPACE-EMPTY (full_analysis): {:?}",
        analysis
            .diagnostics
            .values()
            .flatten()
            .map(|c| c.code.as_str())
            .collect::<Vec<_>>()
    );

    let report = check_workspace(&app);
    assert!(
        !report
            .diagnostics
            .iter()
            .any(|c| c.code == CheckCode::WorkspaceEmpty),
        "un workspace con documentos NO debe llevar WORKSPACE-EMPTY (knowledge_check): {}",
        resumen(&report)
    );
}

/// `el_aviso_de_vacio_respeta_block_warnings` (mitad de servicio): con `gate.blockWarnings: true`,
/// el `Analysis` de un workspace vacío debe contar como aviso a efectos de la puerta configurable
/// (`WorkspaceConfig::gate_blocked`) — la interacción que la historia declara explícitamente para
/// que no quede a interpretación de quien la descubra.
#[test]
fn el_aviso_de_vacio_respeta_block_warnings() {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        ".lodestar/config.yaml",
        "gate:\n  blockWarnings: true\n",
    );
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");

    let analysis = app
        .full_analysis()
        .expect("full_analysis debe computar el análisis de un workspace vacío con blockWarnings");
    assert!(
        analysis.documents.is_empty(),
        "precondición: el inventario debe estar vacío"
    );
    assert_eq!(
        analysis.hard_fail(),
        0,
        "el aviso en sí NO es un hard-fail: si esto no fuera 0, el bloqueo vendría del aviso \
         mismo, no de la política del usuario"
    );
    assert!(
        app.workspace().config().gate_blocked(&analysis),
        "con `gate.blockWarnings: true` un workspace vacío (que SOLO tiene el aviso \
         WORKSPACE-EMPTY) debe bloquear la puerta por la POLÍTICA del usuario, no por el aviso en \
         sí — que es justo el criterio `el_aviso_de_vacio_respeta_block_warnings` de la historia. \
         Si WORKSPACE-EMPTY no cuenta como Warn en `analysis.diagnostics`, `gate_blocked` no lo ve \
         y este assert falla."
    );
}
