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

    /// Ruta a propiedad de frontmatter inválida (vacía, o con algún segmento vacío como en
    /// `"service."`). La construye [`crate::types::FieldPath`] — es un dato de entrada del agente,
    /// no un panic.
    #[error("ruta de campo inválida: {0}")]
    InvalidFieldPath(String),

    /// Al normalizar un `delete` con la política por defecto `reject` (E12-H06), el documento a
    /// borrar todavía tiene enlaces entrantes. Lleva el path del documento referenciado. Mapea a
    /// `ErrorCode::InboundLinksExist` (wire `"INBOUND_LINKS_EXIST"`).
    #[error(
        "el documento «{0}» tiene enlaces entrantes; no se puede borrar con la política «reject»"
    )]
    InboundLinksExist(RelPath),

    /// El contenido excede la guarda de tamaño de una operación (p. ej. diff/LCS).
    #[error("excedida la guarda de tamaño: {0}")]
    SizeGuardExceeded(String),

    /// Al normalizar `replace_text` (E12-H05), el número de coincidencias de la cadena buscada no
    /// casa con el `expected_occurrences` declarado. Lleva `(esperadas, encontradas)`.
    #[error("replace_text: se esperaban {0} coincidencias pero se encontraron {1}")]
    ReplaceTextMismatch(usize, usize),

    /// Al normalizar una operación de contenido (E12-H05), el documento o la sección referida no
    /// existe en el workspace (path sin fichero, o `heading_path` que no casa con ningún heading).
    #[error("objetivo de normalización no encontrado: {0}")]
    NormalizeTargetNotFound(String),

    /// Restricción de relación violada. **Sin productor desde E20-H03**: con el retiro de
    /// `core::schema` (`§20.10`, modelo universal) `add_relation`/`remove_relation` dejan de validar
    /// contra tipos/cardinalidad, así que esta variante ya no se construye. Se conserva para no
    /// cambiar el catálogo de [`crate::CoreError`] (mapea a `ErrorCode::RelationConstraintViolation`)
    /// hasta que Fase 12 retire las operaciones de relación por completo. El payload es un mensaje
    /// legible con el detalle del incumplimiento.
    #[error("restricción de relación violada: {0}")]
    RelationConstraintViolation(String),

    /// Transición de estado no permitida. **Sin productor desde E20-H03**: `transition_status` deja
    /// de validar `to` contra ninguna lista de estados permitidos (`status` es una propiedad de
    /// frontmatter arbitraria, `§20.10`). Se conserva por la misma razón que
    /// [`Self::RelationConstraintViolation`]; mapea a `ErrorCode::InvalidSchema`.
    #[error("transición de estado no permitida: {0}")]
    InvalidStatusTransition(String),

    /// Al normalizar `apply_fix` (E12-H07), el `fix_id` pedido no corresponde a ningún `Fix`
    /// `safe` de los diagnósticos recomputados del workspace (desconocido, ya resuelto, o no `safe`).
    /// El payload es el `fix_id` no encontrado. Mapea a `ErrorCode::DocumentNotFound`.
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

    /// Al parchear el frontmatter de un documento (E16-H04,
    /// [`crate::model::patch_frontmatter`]) el bloque existe pero Lodestar **no puede
    /// interpretarlo**: abre `---` y nunca cierra, o su YAML es sintácticamente inválido.
    ///
    /// No hay mapa sobre el que aplicar el merge-patch, y reconstruir el bloque desde cero
    /// **borraría la metadata del usuario** — la operación falla y el documento queda intacto.
    /// El payload es un mensaje legible con el motivo, para que el agente sepa qué reparar.
    /// Mapea a `ErrorCode::InvalidSchema`.
    #[error("el frontmatter del documento no es interpretable: {0}")]
    UnreadableFrontmatter(String),
}

/// Resultado de conveniencia del núcleo.
pub type Result<T> = std::result::Result<T, CoreError>;
