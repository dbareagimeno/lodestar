//! `WorkspaceError`: envuelve `CoreError` + errores de la cache/IO con códigos estables
//! (`§6`, `§12`).

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
    /// [`crate::Workspace::validate_staging`]), NO cumple la política de conformidad estricta
    /// (`hard_fail > 0`): publicarlo dejaría el conocimiento canónico no conforme. La validación
    /// aborta sin tocar el canónico y limpia el staging; el `String` describe el motivo concreto
    /// (recuento de fallos duros). Mapea al wire `NONCONFORMANT_RESULT`.
    #[error("el resultado del plan no es conforme: {0}")]
    NonconformantResult(String),
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
}

impl WorkspaceError {
    /// Código estable para mapear a exit code / `{code, message}` de las fachadas.
    pub fn code(&self) -> &'static str {
        match self {
            WorkspaceError::Core(_) => "CORE",
            WorkspaceError::Io(_) => "IO",
            WorkspaceError::NoCache => "NO_CACHE",
            WorkspaceError::Store(_) => "STORE",
            WorkspaceError::PermissionDenied(_) => "PERMISSION_DENIED",
            WorkspaceError::NonconformantResult(_) => "NONCONFORMANT_RESULT",
            WorkspaceError::WriteConflict(_) => "WRITE_CONFLICT",
            WorkspaceError::WorkspaceRecoveryRequired(_) => "WORKSPACE_RECOVERY_REQUIRED",
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
