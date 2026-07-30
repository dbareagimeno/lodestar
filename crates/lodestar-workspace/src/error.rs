//! `WorkspaceError`: envuelve `CoreError` + errores de la cache/IO con códigos estables
//! (`§6`, `§12`).

use lodestar_core::types::ErrorCode;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("error del núcleo: {0}")]
    Core(String),
    #[error("error de IO: {0}")]
    Io(String),
    #[error("la cache incremental no está activada (usa open_live/enable_cache)")]
    NoCache,
    #[error("error de la cache: {0}")]
    Store(String),
    /// Escritura rechazada por caer bajo un `referenceRoot` (inmutable) o fuera de los
    /// `writableRoots` configurados (E11-H04, `Workspace::assert_writable`). El mensaje describe
    /// el motivo concreto.
    #[error("permiso denegado: {0}")]
    PermissionDenied(String),
    /// El resultado hipotético de un plan, materializado en staging (E13-H01,
    /// [`crate::Workspace::validate_staging`]), NO pasa la **política de cambios** (E20-H04,
    /// `§20.9`): con `rejectNewErrors` introduciría errores que el canónico no tenía, o con
    /// `allowExistingErrors: false` el resultado deja errores. La validación aborta sin tocar el
    /// canónico y limpia el staging; el `String` describe el motivo concreto (errores nuevos / total).
    /// Mapea al wire `INVALID_RESULT`.
    #[error("el resultado del plan no pasa la política de cambios: {0}")]
    InvalidResult(String),
    /// Conflicto de escritura optimista (E13-H02, [`crate::Workspace::reverify_base_revision`]):
    /// la [`lodestar_core::types::WorkspaceRevision`] del conocimiento escribible cambió entre que
    /// se planificó (`baseWorkspaceRevision`) y el momento del apply — otro escritor (humano,
    /// agente o `git pull`) tocó el workspace. Publicar sobre una base obsoleta arriesga pisar ese
    /// cambio, así que la re-verificación aborta sin tocar el canónico. El `String` describe el
    /// conflicto (revisión esperada vs. actual). Mapea al wire `WRITE_CONFLICT`.
    #[error("conflicto de escritura: {0}")]
    WriteConflict(String),
    /// Hay una recuperación de publicación PENDIENTE (E13-H06,
    /// [`crate::Workspace::recover`]): al abrir el workspace se detectó un write-ahead journal
    /// no-`done` (E13-H03/H05) — una transacción que se interrumpió a mitad. Mientras `recover` no
    /// **complete** (journal `applied`: renames hechos, solo falta sellar) o **restaure** (journal
    /// `prepared`/`applying`: deshacer los renames parciales desde las copias de H04) esa
    /// transacción, toda escritura del canónico se rechaza ANTES de tocarlo, para no publicar sobre
    /// un estado a medio recuperar. El `String` describe la transacción pendiente. Mapea al wire
    /// `WORKSPACE_RECOVERY_REQUIRED`.
    #[error("recuperación pendiente: {0}")]
    WorkspaceRecoveryRequired(String),
    /// La recuperación de una transacción interrumpida **no se pudo llevar a término** (E25-H02,
    /// [`crate::Workspace::recover`]): la copia de recuperación de algún path afectado no está, no se
    /// puede leer o **no verifica** contra el tamaño y el hash blake3 que se registraron al
    /// respaldarla. Una copia que no verifica no es un original, así que no se escribe **nada** a
    /// partir de ella.
    ///
    /// El material de esa transacción —su write-ahead journal y su árbol de copias— se **mueve** a
    /// `.lodestar/runtime/journal/quarantine/<txnId>/` (nada se borra: es material forense) y el
    /// mensaje nombra esa ruta. La recuperación **sigue** con los demás journals pendientes y el
    /// gate de escritura se levanta: la operación que disparó la recuperación falla una vez con este
    /// error y la siguiente ya no encuentra recuperación pendiente. Mapea al wire `RECOVERY_FAILED`.
    #[error("la recuperación no se pudo completar: {0}")]
    RecoveryFailed(String),
    /// La **entrada** del agente no es interpretable: una consulta `where`/`filter` malformada,
    /// un valor fuera del rango que declara el `inputSchema`… (E24-H10). Mapea al wire
    /// `INVALID_SCHEMA`.
    ///
    /// Existe porque hasta v0.3.0 estos errores se envolvían en [`WorkspaceError::Core`], que
    /// `workspace_error_code` mapea a `INTERNAL_IO_ERROR`: un typo del agente en su consulta se le
    /// reportaba como un error interno de I/O del motor, y la misma consulta malformada daba **dos
    /// códigos distintos** según entrara por `knowledge_search` o por la selección masiva de
    /// `change_plan`.
    #[error("entrada inválida: {0}")]
    InvalidSchema(String),
}

impl WorkspaceError {
    /// Código estable para mapear a exit code / `{code, message}` de las fachadas.
    pub fn code(&self) -> &'static str {
        match self {
            WorkspaceError::Core(_) => "CORE",
            WorkspaceError::Io(_) => "IO",
            WorkspaceError::NoCache => "NO_CACHE",
            WorkspaceError::Store(_) => "STORE",
            // E24-H17: los que SON códigos del catálogo se toman de `ErrorCode::as_str()`, nunca
            // se reescriben aquí. Antes esta tabla era una segunda verdad del catálogo de wire
            // —justo lo que el doc-comment de `core::types::ErrorCode` prohíbe— y el grep de CI que
            // debía impedirlo no existía.
            WorkspaceError::PermissionDenied(_) => ErrorCode::PermissionDenied.as_str(),
            WorkspaceError::InvalidResult(_) => ErrorCode::InvalidResult.as_str(),
            WorkspaceError::WriteConflict(_) => ErrorCode::WriteConflict.as_str(),
            WorkspaceError::WorkspaceRecoveryRequired(_) => {
                ErrorCode::WorkspaceRecoveryRequired.as_str()
            }
            WorkspaceError::RecoveryFailed(_) => ErrorCode::RecoveryFailed.as_str(),
            WorkspaceError::InvalidSchema(_) => ErrorCode::InvalidSchema.as_str(),
        }
    }
}

impl From<lodestar_store::StoreError> for WorkspaceError {
    fn from(e: lodestar_store::StoreError) -> Self {
        WorkspaceError::Store(e.to_string())
    }
}

impl From<lodestar_core::CoreError> for WorkspaceError {
    fn from(e: lodestar_core::CoreError) -> Self {
        WorkspaceError::Core(e.to_string())
    }
}

impl From<std::io::Error> for WorkspaceError {
    fn from(e: std::io::Error) -> Self {
        WorkspaceError::Io(e.to_string())
    }
}
