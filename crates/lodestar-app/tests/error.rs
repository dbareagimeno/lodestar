//! Tests de integración de E10-H02: mapeo de errores del núcleo/workspace a `ErrorCode` estable.
//!
//! Criterio de aceptación `mapeo_core_error`: un `CoreError::InvalidRelPath` (el que produce
//! `RelPath::new("../x")` al rechazar un traversal) se mapea al código estable `PERMISSION_DENIED`
//! — `RelPath` es el único chokepoint de path-traversal (invariante #6), y un intento de salir del
//! workspace es semánticamente un permiso denegado.
//!
//! Fase ROJA: NI `ErrorCode` (core) NI la función de mapeo del app existen todavía. Este test hace
//! ROJO por símbolos ausentes hasta que E10-H02 los implemente.
//!
//! API objetivo asumida (el implementador debe crearla con ESTE nombre/firma; el contrato aseverado
//! es el `ErrorCode` resultante, no la mecánica interna):
//!
//! ```ignore
//! // en `lodestar-app`: mapea un error del núcleo a su código estable de protocolo.
//! pub fn error_code(err: &lodestar_core::CoreError) -> lodestar_core::types::ErrorCode
//! ```
//!
//! (La misma capa mapeará también `WorkspaceError → ErrorCode`; el orphan rule impide un
//! `impl From<&WorkspaceError> for ErrorCode` en `lodestar-app`, así que un mapeo por FUNCIÓN LIBRE
//! es la forma natural. Si el implementador prefiere `From<&CoreError> for ErrorCode` o
//! `App::map_error`, deberá exponer además esta función libre para no romper este test.)

use lodestar_core::types::{ErrorCode, RelPath};
use lodestar_core::CoreError;

/// `mapeo_core_error` — Dado un `CoreError::InvalidRelPath` (RelPath inválido por traversal),
/// Cuando se mapea con la función del app, Entonces el código estable es `PERMISSION_DENIED`.
#[test]
fn mapeo_core_error() {
    // Error REAL producido hoy por el chokepoint de path-traversal: `RelPath::new` devuelve
    // `Err(CoreError::InvalidRelPath(_))` ante un `..`.
    let err: CoreError = RelPath::new("../x").expect_err("`../x` debe rechazarse como traversal");
    assert!(
        matches!(err, CoreError::InvalidRelPath(_)),
        "el error real de un RelPath con `..` debe ser InvalidRelPath, es {err:?}"
    );

    // El contrato: un RelPath inválido → PERMISSION_DENIED (o el código documentado).
    let code = lodestar_app::error_code(&err);
    assert!(
        matches!(code, ErrorCode::PermissionDenied),
        "un CoreError::InvalidRelPath debe mapear a PERMISSION_DENIED, mapea a {code:?}"
    );
}

// ===========================================================================
// E25-H02 — `RECOVERY_FAILED` gana su primer emisor real y llega a la fachada
// (`requirements/epica-25-endurecimiento-escritura.md`, bloque B). Fase ROJA.
//
// Hoy, si la copia de recuperación de una transacción interrumpida es ilegible, `restore_backups`
// devuelve `Err` y `recover()` lo propaga con `?` (`crates/lodestar-workspace/src/recovery.rs:271`,
// `:275-276`, `:297-298`). Consecuencias en la fachada:
//   1. el error llega como `WorkspaceError::Io`, que `workspace_error_code` mapea a
//      `INTERNAL_IO_ERROR` (`crates/lodestar-app/src/lib.rs:163`) — un código que no le dice al
//      agente nada de lo que ha pasado ni de lo que puede hacer;
//   2. el journal sigue pendiente, así que **toda** operación futura muere igual: el workspace queda
//      cerrado a la escritura para siempre (no hay cuarentena, ni descarte, ni `--force`).
//
// E25-H02 manda el material a `.lodestar/runtime/journal/quarantine/<txnId>/` (donde nada se borra),
// reporta el fallo **una vez** con `RECOVERY_FAILED` y deja que la siguiente operación proceda. Con
// eso `RECOVERY_FAILED` sale de `codigos_sin_emisor` (`contracts/mcp.yml:665`) sin añadir ninguna
// fila al catálogo de 16.
//
// LÍMITE DE LA API (declarado): `App::change_plan`/`change_apply` devuelven `Err(ErrorCode)` — el
// mensaje del `WorkspaceError` no atraviesa la fachada, así que «el mensaje nombra la ruta de
// cuarentena» solo es aseverable en `lodestar-workspace`
// (`tests/transactions.rs::journal_irrecuperable_no_encalla_el_workspace`). Aquí se fija lo que la
// fachada SÍ expone: el código estable del wire, y que el material está donde ese mensaje dice.
// ===========================================================================

/// Escribe un `.md` dentro del workspace temporal, creando los directorios intermedios.
fn escribe_md(root: &std::path::Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// **E25-H02** · Criterio 2 en la fachada — **Dado** un journal `prepared` cuya copia de
/// recuperación es ilegible, **Cuando** un agente llama a una operación de `App`, **Entonces**
/// recibe el código estable `RECOVERY_FAILED`, el material queda en la cuarentena que el mensaje
/// nombra, y **la siguiente llamada procede**.
///
/// Hoy: `INTERNAL_IO_ERROR` en la primera llamada y en todas las siguientes, para siempre.
#[test]
fn recovery_failed_llega_a_la_fachada() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    escribe_md(
        root,
        "alfa.md",
        "---\ntype: Nota\ntitle: Alfa\n---\n\n# Alfa\n\ncuerpo original\n",
    );
    escribe_md(
        root,
        "beta.md",
        "---\ntype: Nota\ntitle: Beta\n---\n\n# Beta\n\ncuerpo original\n",
    );

    // Estado durable de una transacción interrumpida, en el orden real del orquestador (copias →
    // journal → renames): respalda `alfa.md` y `beta.md`, publica el rename de `beta.md` y lo anota.
    let txn = "e25-h02-fachada-roto";
    let ws = lodestar_workspace::Workspace::open(root).expect("abrir el workspace");
    let alfa = RelPath::new("alfa.md").unwrap();
    let beta = RelPath::new("beta.md").unwrap();
    let afectados = vec![alfa.clone(), beta.clone()];
    let base = ws.workspace_revision().unwrap();
    ws.backup_originals(txn, &afectados)
        .expect("copias de recuperación");
    let mut journal = ws
        .create_journal(txn, &afectados, &base, &base)
        .expect("write-ahead journal");
    std::fs::write(
        root.join("beta.md"),
        "---\ntype: Nota\ntitle: Beta\n---\n\n# Beta\n\ncuerpo PUBLICADO\n",
    )
    .unwrap();
    journal.mark_applied(&beta).expect("anotar el rename");
    drop(journal);
    drop(ws);

    // La copia de recuperación de `beta.md` —la que la restauración necesita— queda ILEGIBLE (bytes
    // que no son UTF-8: `read_to_string` falla igual que con un permiso denegado, sin depender de
    // `chmod` ni del usuario que corre los tests).
    let recovery = root
        .join(".lodestar")
        .join("runtime")
        .join("recovery")
        .join(txn);
    std::fs::write(recovery.join("beta.md"), [0xff, 0xfe, 0x00, 0x01]).unwrap();

    let app = lodestar_app::App::open(root).expect("la fachada debe abrir el workspace");
    let ops = serde_json::json!([
        { "op": "replace_body", "path": "alfa.md", "body": "# Alfa\n\ncuerpo del plan\n" },
    ]);
    let policy = lodestar_core::plan::PlanPolicy {
        require_valid_result: false,
        allow_warnings: true,
    };

    // PRIMERA operación: recupera antes de leer nada (`App::change_plan` paso (0), E24-H03) y hereda
    // el fallo de recuperación.
    let err = match app.change_plan(None, &ops, policy) {
        Err(e) => e,
        Ok(plan) => panic!(
            "una operación que arrastra una recuperación irrecuperable no puede reportar éxito: \
             changeSetId={:?}",
            plan.change_set_id
        ),
    };
    assert_eq!(
        err.code,
        ErrorCode::RecoveryFailed,
        "el fallo de recuperación tiene código propio en el catálogo de 16 y E25-H02 le da su primer \
         emisor real: el agente no puede recibir un INTERNAL_IO_ERROR genérico. Era: {} ({err:?})",
        err.code.as_str()
    );
    assert_eq!(
        err.code.as_str(),
        "RECOVERY_FAILED",
        "y con esa cadena exacta en el wire"
    );

    // El material forense está donde el mensaje del motor dice que está.
    let cuarentena = root
        .join(".lodestar")
        .join("runtime")
        .join("journal")
        .join("quarantine")
        .join(txn);
    assert!(
        cuarentena.is_dir(),
        "el journal y sus copias se MUEVEN a la cuarentena (nada se borra): {}",
        cuarentena.display()
    );

    // SEGUNDA operación: ya no hay journal pendiente, así que el workspace vuelve a ser usable de
    // punta a punta (planificar + aplicar).
    let plan = app.change_plan(None, &ops, policy).unwrap_or_else(|e| {
        panic!(
            "la segunda operación debe proceder: {} ({e:?})",
            e.code.as_str()
        )
    });
    let aplicado = app
        .change_apply(&plan.change_set_id, None)
        .unwrap_or_else(|e| panic!("y el apply también: {} ({e:?})", e.code.as_str()));
    assert!(
        aplicado.applied,
        "un journal irrecuperable no puede cerrar el workspace a la escritura para siempre"
    );
    assert!(
        std::fs::read_to_string(root.join("alfa.md"))
            .unwrap()
            .contains("cuerpo del plan"),
        "y el resultado del plan tiene que estar en el canónico"
    );
}
