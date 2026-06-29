//! Errores de `lodestar-vcs`.

use thiserror::Error;

#[derive(Debug, Error)]
pub enum VcsError {
    #[error("error de git: {0}")]
    Git(String),
    #[error("error de IO: {0}")]
    Io(String),
    #[error("el repo no tiene HEAD (sin commits)")]
    NoHead,
    #[error("error del núcleo: {0}")]
    Core(String),
}

impl From<git2::Error> for VcsError {
    fn from(e: git2::Error) -> Self {
        VcsError::Git(e.to_string())
    }
}

impl From<lodestar_core::CoreError> for VcsError {
    fn from(e: lodestar_core::CoreError) -> Self {
        VcsError::Core(e.to_string())
    }
}
