//! Errores puros del núcleo (`ARCHITECTURE.md §6`, `§12`).
//!
//! `CoreError` NO incluye variantes de DB/git/runtime: esas viven en `store`/`vcs`/`workspace`.
//! `WorkspaceError` (en `lodestar-workspace`) lo envuelve.

use thiserror::Error;

/// Error del núcleo. Recuperable y serializable a un código estable por las fachadas.
#[derive(Debug, Error)]
pub enum CoreError {
    /// Ruta relativa inválida (absoluta, con `..`, o vacía). Único chokepoint de path-traversal.
    #[error("ruta relativa inválida: {0}")]
    InvalidRelPath(String),

    /// SHA de git con formato inválido (no hexadecimal o longitud incorrecta).
    #[error("sha inválido: {0}")]
    InvalidSha(String),

    /// El contenido excede la guarda de tamaño de una operación (p. ej. diff/LCS).
    #[error("excedida la guarda de tamaño: {0}")]
    SizeGuardExceeded(String),

    /// Error de escritura/serialización al exportar (p. ej. al construir el zip).
    #[error("error de export/IO: {0}")]
    Export(String),

    /// Al normalizar `replace_text` (E12-H05), el número de coincidencias de la cadena buscada no
    /// casa con el `expected_occurrences` declarado. Lleva `(esperadas, encontradas)`.
    #[error("replace_text: se esperaban {0} coincidencias pero se encontraron {1}")]
    ReplaceTextMismatch(usize, usize),

    /// Al normalizar una operación de contenido (E12-H05), el concepto o la sección referida no
    /// existe en el bundle (path sin fichero, o `heading_path` que no casa con ningún heading).
    #[error("objetivo de normalización no encontrado: {0}")]
    NormalizeTargetNotFound(String),
}

/// Resultado de conveniencia del núcleo.
pub type Result<T> = std::result::Result<T, CoreError>;
