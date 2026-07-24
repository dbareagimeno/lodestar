//! Deserialización del **filtro JSON estructurado** (`filter`) al mismo
//! [`crate::types::Expression`] que produce el parser textual (`where`), de modo que ambas formas
//! generan exactamente el mismo resultado (`ARCHITECTURE.md §20.10`,
//! `REFACTOR_PHASE_2 §Fase 5 (Superficie MCP)`, E19-H03).
//!
//! > **STUB de la fase ROJA de E19-H03** (autor de tests). Solo declara la firma que fijan los
//! > tests (`crates/lodestar-core/tests/consulta.rs::{filtro_json_deserializa, equivalencia_ast,
//! > equivalencia_resultado}`). La lógica —el tipo wire intermedio, la normalización de la
//! > abreviatura `frontmatter.` (idéntica a la de [`crate::parse`]) y el mapeo de los nombres largos
//! > de operador— la escribe el implementador; aquí no hay nada de eso.

use crate::types::Expression;

/// El error de un filtro JSON malformado (operador desconocido, nodo sin forma reconocible,
/// `value` ausente en una comparación…). Es un **dato del `Result`** de [`from_json`], no un panic,
/// coherente con [`crate::parse::ParseError`] del `where` textual: ambos son entrada del agente y
/// acabarán mapeados a `INVALID_SCHEMA` por la fachada (E19-H05/E20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterError {
    /// Descripción legible del fallo de deserialización.
    pub message: String,
}

/// Traduce el filtro JSON estructurado de `§20.10` al [`Expression`] unificado de E19-H01 — el
/// **mismo** AST al que [`crate::parse::parse`] traduce la consulta textual, de modo que `where` y
/// `filter` producen exactamente el mismo resultado.
pub fn from_json(value: &serde_json::Value) -> Result<Expression, FilterError> {
    let _ = value;
    todo!("E19-H03: deserializar el filtro JSON de §20.10 a Expression (fase roja)")
}
