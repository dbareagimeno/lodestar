//! `WorkspaceError`: envuelve `CoreError` + errores de vcs/IO con códigos estables (`§6`, `§12`).

use thiserror::Error;

#[derive(Debug, Error)]
pub enum WorkspaceError {
    #[error("error del núcleo: {0}")]
    Core(String),
    #[error("error de git: {0}")]
    Vcs(String),
    #[error("error de IO: {0}")]
    Io(String),
    #[error("el bundle no tiene git inicializado")]
    NoVcs,
    #[error("la cache incremental no está activada (usa open_live/enable_cache)")]
    NoCache,
    #[error("error de la cache: {0}")]
    Store(String),
    #[error("hay un merge/rebase en curso: resuelve el conflicto antes de commitear")]
    RepoBusy,
    /// Escritura rechazada por caer bajo un `referenceRoot` (inmutable) o fuera de los
    /// `writableRoots` configurados (E11-H04, `Workspace::assert_writable`). El mensaje describe
    /// el motivo concreto.
    #[error("permiso denegado: {0}")]
    PermissionDenied(String),
}

impl WorkspaceError {
    /// Código estable para mapear a exit code / `{code, message}` de las fachadas.
    pub fn code(&self) -> &'static str {
        match self {
            WorkspaceError::Core(_) => "CORE",
            WorkspaceError::Vcs(_) => "VCS",
            WorkspaceError::Io(_) => "IO",
            WorkspaceError::NoVcs => "NO_VCS",
            WorkspaceError::NoCache => "NO_CACHE",
            WorkspaceError::Store(_) => "STORE",
            WorkspaceError::RepoBusy => "REPO_BUSY",
            WorkspaceError::PermissionDenied(_) => "PERMISSION_DENIED",
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

impl From<lodestar_vcs::VcsError> for WorkspaceError {
    fn from(e: lodestar_vcs::VcsError) -> Self {
        WorkspaceError::Vcs(e.to_string())
    }
}

impl From<std::io::Error> for WorkspaceError {
    fn from(e: std::io::Error) -> Self {
        WorkspaceError::Io(e.to_string())
    }
}
