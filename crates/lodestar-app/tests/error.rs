//! Tests de integración de E10-H02: mapeo de errores del núcleo/workspace a `ErrorCode` estable.
//!
//! Criterio de aceptación `mapeo_core_error`: un `CoreError::InvalidRelPath` (el que produce
//! `RelPath::new("../x")` al rechazar un traversal) se mapea al código estable `PERMISSION_DENIED`
//! — `RelPath` es el único chokepoint de path-traversal (invariante #6), y un intento de salir del
//! bundle es semánticamente un permiso denegado.
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
