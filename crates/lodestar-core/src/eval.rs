//! Evaluador **tipado** del lenguaje de consulta (`ARCHITECTURE.md §20.8`,
//! `REFACTOR_PHASE_2 §Fase 5`, E19-H01).
//!
//! Evalúa una [`crate::types::Expression`] contra un documento **respetando los tipos YAML sin
//! coerción**: `priority >= 2` funciona, `priority >= "high"` es un [`TypeError`], y la igualdad
//! entre tipos distintos es `false` (no error). Sustituye la semántica de subcadena de
//! [`crate::query`] (la DSL vieja, que se retira en E19-H05).
//!
//! **Aviso rector (`§20.8`, heredado de E16-H01)**: las comparaciones van **siempre** sobre
//! [`crate::types::ParsedFrontmatter::get`] (que devuelve el `serde_yaml::Value` con su tipo),
//! **nunca** sobre `get_text` (que renderiza a `String` y reintroduciría la coerción).
//!
//! **STUB de la fase ROJA de E19-H01**: [`evaluate`] es `todo!()`. Su lógica es la fase verde.

use crate::types::{Analysis, Expression, ParsedFrontmatter, RelPath, TypeError};

/// La vista de un documento que el evaluador necesita para resolver una [`Expression`].
///
/// Es una **vista prestada** (no posee nada): se construye por documento al filtrar un workspace.
/// Lleva de entrada lo justo para E19-H01 (el `frontmatter`, sobre el que van las comparaciones) y
/// deja la puerta abierta a E19-H04 sin cambiar la firma de [`evaluate`]: `path` y `body` alimentan
/// el namespace `document.*` (`document.path`/`document.title`/`document.has_frontmatter`) y, junto
/// al [`Analysis`], el namespace `graph.*` — que **no** se evalúan aquí.
pub struct EvalDocument<'a> {
    /// La ruta del documento (identidad para cruzar con [`Analysis`] en E19-H04).
    pub path: &'a RelPath,
    /// El frontmatter parseado, o `None` si el documento no tiene bloque (estado válido, `§20.4`).
    pub frontmatter: Option<&'a ParsedFrontmatter>,
    /// El cuerpo del documento (para `document.title` en E19-H04).
    pub body: &'a str,
}

/// Evalúa `expr` contra `doc` y devuelve si el documento **casa** con la consulta.
///
/// - Las comparaciones de orden (`> >= < <=`) exigen operandos del mismo tipo numérico o ambos
///   string (lexicográfico); un cruce es `Err(TypeError::OrderNotDefined)`, **no** `Ok(false)`.
/// - `=`/`!=` comparan por valor **e igualdad de tipo**: el cruce de tipos es `Ok(false)`, nunca
///   error.
/// - `contains`/`contains_any`/`contains_all` exigen que el campo sea lista (excepción: `contains`
///   sobre un string es subcadena); sobre un escalar no string es `Err(TypeError::NotAList)`.
/// - Un campo **inexistente** en una comparación es `Ok(false)`, nunca error.
///
/// `analysis` queda en la firma para el namespace `graph.*` de E19-H04; E19-H01 no lo consulta.
pub fn evaluate(
    _expr: &Expression,
    _doc: &EvalDocument<'_>,
    _analysis: &Analysis,
) -> Result<bool, TypeError> {
    todo!("E19-H01 (fase verde): evaluador tipado sobre ParsedFrontmatter::get")
}
