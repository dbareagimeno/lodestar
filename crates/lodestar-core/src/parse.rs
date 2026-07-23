//! Parser **textual** del lenguaje de consulta `where` (`ARCHITECTURE.md §20.8`,
//! `REFACTOR_PHASE_2 §Fase 5 (Consultas básicas, Expresiones booleanas, Existencia, Namespaces)`,
//! E19-H02).
//!
//! Traduce la consulta textual (`type = "decision" and (status = "draft" or status = "review")`) al
//! [`crate::types::Expression`] de E19-H01 — el **mismo** AST al que el filtro JSON de E19-H03
//! deserializa, de modo que `where` y `filter` producen exactamente el mismo resultado. No es la DSL
//! de subcadena de [`crate::query`] (que se retira en E19-H05): aquí no hay tokens con semántica de
//! `contains`, sino literales tipados por su forma y operadores con precedencia.
//!
//! **STUB de la fase ROJA de E19-H02**: [`parse`] es `todo!()`. Su lógica es la fase verde; la forma
//! ([`ParseError`] + la firma `parse(&str) -> Result<Expression, ParseError>`) es lo único que esta
//! fase congela.

use crate::types::Expression;

/// El error de una consulta textual `where` malformada (`status =` sin valor, paréntesis sin
/// cerrar, operador desconocido…). Es un **dato del `Result`** de [`parse`], no un panic ni una
/// query vacía: E20/E21 lo mapearán a `ErrorCode::InvalidSchema`. Tipo propio (mismo espíritu que
/// [`crate::types::FmError`] / [`crate::types::TypeError`]): un `where` mal escrito es entrada del
/// agente, y quien lo traduce a protocolo decide su envoltorio.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError {
    /// Descripción legible del fallo de parseo.
    pub message: String,
}

/// Traduce la consulta textual `where` de `§20.8` al [`Expression`] unificado de E19-H01.
///
/// - Literales por **forma**: entrecomillado = string; sin comillas, número/booleano/`null` según su
///   escritura (`2` → número, `true` → booleano, `"2"` → string).
/// - Dot-notation (`service.tier`) → [`crate::types::FieldPath`] de varios segmentos.
/// - **Abreviatura de namespace**: `status = "x"` produce el mismo AST que `frontmatter.status =
///   "x"` (el prefijo `frontmatter.` se normaliza a la ruta desnuda). Los namespaces calculados
///   (`document.*`, `graph.*`) se conservan como primer segmento del `FieldPath` (su evaluación es
///   E19-H04).
/// - `and`/`or`/`not`, paréntesis y **precedencia** `not` > `and` > `or`.
/// - Una consulta malformada es `Err(ParseError)`, **nunca** un panic ni una query vacía.
///
/// **STUB de la fase ROJA**: `todo!()`. La implementación es la fase verde.
pub fn parse(input: &str) -> Result<Expression, ParseError> {
    let _ = input;
    todo!("E19-H02: parser textual del lenguaje de consulta (fase verde)")
}
