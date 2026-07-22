//! Errores puros del núcleo (`ARCHITECTURE.md §6`, `§12`).
//!
//! `CoreError` NO incluye variantes de DB/git/runtime: esas viven en `store`/`vcs`/`workspace`.
//! `WorkspaceError` (en `lodestar-workspace`) lo envuelve.

use thiserror::Error;

use crate::types::RelPath;

/// Error del núcleo. Recuperable y serializable a un código estable por las fachadas.
#[derive(Debug, Error)]
pub enum CoreError {
    /// Ruta relativa inválida (absoluta, con `..`, o vacía). Único chokepoint de path-traversal.
    #[error("ruta relativa inválida: {0}")]
    InvalidRelPath(String),

    /// Al normalizar un `delete` con la política por defecto `reject` (E12-H06), el concepto a
    /// borrar todavía tiene enlaces entrantes. Lleva el path del concepto referenciado. Mapea a
    /// `ErrorCode::InboundLinksExist` (wire `"INBOUND_LINKS_EXIST"`).
    #[error(
        "el concepto «{0}» tiene enlaces entrantes; no se puede borrar con la política «reject»"
    )]
    InboundLinksExist(RelPath),

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

    /// Al normalizar `add_relation`/`remove_relation` (E12-H07), la relación viola su
    /// [`crate::schema::RelationDef`]: el `type` del target no está en `target_types` (vacío =
    /// cualquier tipo), o la cardinalidad `"one"` se superaría. El payload es un mensaje legible
    /// en español con el detalle del incumplimiento (concepto origen, relación, target y motivo).
    /// Mapea a `ErrorCode::RelationConstraintViolation` (wire `"RELATION_CONSTRAINT_VIOLATION"`).
    #[error("restricción de relación violada: {0}")]
    RelationConstraintViolation(String),

    /// Al normalizar `transition_status` (E12-H07), el estado destino no está en los
    /// `allowed_statuses` del `DocType` del concepto (cuando esa lista no está vacía). El payload
    /// es un mensaje legible con el estado rechazado y los permitidos. Mapea a
    /// `ErrorCode::InvalidSchema` (precondición de lifecycle incumplida).
    #[error("transición de estado no permitida: {0}")]
    InvalidStatusTransition(String),

    /// Al normalizar `apply_fix` (E12-H07), el `fix_id` pedido no corresponde a ningún `Fix`
    /// `safe` de los diagnósticos recomputados del bundle (desconocido, ya resuelto, o no `safe`).
    /// El payload es el `fix_id` no encontrado. Mapea a `ErrorCode::ConceptNotFound`.
    #[error("fix no encontrado o no aplicable: {0}")]
    FixNotFound(String),

    /// Al materializar un plan en memoria (E12-H08, [`crate::plan::apply_normalized_ops`]) se
    /// recibió una [`crate::types::NormalizedOperation`] en forma NO terminal (una variante
    /// semántica/de contenido que los normalizadores de E12-H05/H06/H07 ya resuelven a
    /// `Create`/`PatchFrontmatter`/`ReplaceBody`/`Move`/`Delete`). Es una violación de invariante
    /// interno —el aplicador solo debe ver operaciones ya normalizadas—, no un error del agente;
    /// mapea a `ErrorCode::InternalIoError`. El payload nombra la variante recibida.
    #[error("operación no aplicable (no normalizada a forma terminal): {0}")]
    OperationNotApplicable(String),
}

/// Resultado de conveniencia del núcleo.
pub type Result<T> = std::result::Result<T, CoreError>;
