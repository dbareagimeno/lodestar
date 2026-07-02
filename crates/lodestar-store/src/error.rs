//! Errores del store (taxonomía estable, `§12`).

use thiserror::Error;

/// Error del store (SQLite/FTS5 + watcher).
#[derive(Debug, Error)]
pub enum StoreError {
    /// Error de SQLite (apertura, DDL, consulta).
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    /// Error de I/O al leer el bundle o preparar `.lodestar/`.
    #[error("io: {0}")]
    Io(String),
    /// Error del watcher (`notify`).
    #[error("watcher: {0}")]
    Watch(String),
    /// Ruta inválida al reconstruir un `RelPath` (no debería ocurrir con datos de la cache).
    #[error("core: {0}")]
    Core(#[from] lodestar_core::CoreError),
}
