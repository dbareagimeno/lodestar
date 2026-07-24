//! Deserialización del **filtro JSON estructurado** (`filter`) al mismo
//! [`crate::types::Expression`] que produce el parser textual (`where`), de modo que ambas formas
//! generan exactamente el mismo resultado (`ARCHITECTURE.md §20.10`,
//! `REFACTOR_PHASE_2 §Fase 5 (Superficie MCP)`, E19-H03).
//!
//! La traducción se hace en **dos pasos**, no con un `impl Deserialize for Expression`:
//!   1. Un **tipo wire intermedio** (`WireNode`, `#[serde(untagged)]`) casa la forma del JSON de
//!      `§20.10` —las envolturas `and`/`or`/`not`/`has`/`missing` y la comparación
//!      `{field, operator, value}`— y deja que serde deserialice `value` y `operator` **gratis**: el
//!      primero por el `#[serde(untagged)]` de [`QueryValue`] y el segundo por los
//!      `#[serde(rename = "equals" | …)]` de [`ComparisonOperator`], que son la **única** tabla de
//!      nombres de wire (invariante #4).
//!   2. `to_expression` transforma ese árbol wire en el [`Expression`] final aplicando la lógica de
//!      forma que un `derive` no da: la abreviatura de `frontmatter.` sobre `field` y el mapeo de
//!      `has`/`missing` a [`Expression::Function`]. Esa normalización **reutiliza**
//!      `crate::parse::build_field_path` —la misma del parser textual—, que es lo que garantiza que
//!      `where` y `filter` produzcan el mismo AST y, por tanto, el mismo resultado.

use serde::Deserialize;

use crate::parse::build_field_path;
use crate::types::{ComparisonOperator, Expression, FunctionName, QueryValue};

/// El error de un filtro JSON malformado (operador desconocido, nodo sin forma reconocible,
/// `value` ausente en una comparación…). Es un **dato del `Result`** de [`from_json`], no un panic,
/// coherente con [`crate::parse::ParseError`] del `where` textual: ambos son entrada del agente y
/// acabarán mapeados a `INVALID_SCHEMA` por la fachada (E19-H05/E20).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilterError {
    /// Descripción legible del fallo de deserialización.
    pub message: String,
}

impl FilterError {
    /// Construye un [`FilterError`] con el mensaje dado.
    fn new(message: impl Into<String>) -> FilterError {
        FilterError {
            message: message.into(),
        }
    }
}

/// Un nodo del filtro JSON de `§20.10`, tal como llega por el wire, **antes** de normalizar la
/// abreviatura de `frontmatter.` y de mapear `has`/`missing` a [`Expression::Function`].
///
/// `#[serde(untagged)]` casa la envoltura por su forma: serde prueba las variantes en orden y elige
/// la primera que deserializa. Una comparación no tiene ninguna de las claves de envoltura
/// (`and`/`or`/`not`/`has`/`missing`), así que solo casa la última variante; y a la inversa. `value`
/// y `operator` se deserializan aquí mismo por sus derivas serde: no hay lógica de tipos que
/// escribir, solo lógica de forma (que vive en [`to_expression`]).
#[derive(Deserialize)]
#[serde(untagged)]
enum WireNode {
    /// `{"and": [nodo, …]}` → [`Expression::And`].
    And { and: Vec<WireNode> },
    /// `{"or": [nodo, …]}` → [`Expression::Or`].
    Or { or: Vec<WireNode> },
    /// `{"not": nodo}` → [`Expression::Not`].
    Not { not: Box<WireNode> },
    /// `{"has": {"field": "…"}}` → [`Expression::Function`] con [`FunctionName::Has`].
    Has { has: WireField },
    /// `{"missing": {"field": "…"}}` → [`Expression::Function`] con [`FunctionName::Missing`].
    Missing { missing: WireField },
    /// `{"field": "…", "operator": "…", "value": …}` → [`Expression::Comparison`].
    Comparison {
        field: String,
        operator: ComparisonOperator,
        value: QueryValue,
    },
}

/// El argumento objeto de `has`/`missing`: `{"field": "…"}`. Se elige la forma objeto (y no un string
/// suelto) por coherencia con la clave `field` de la comparación.
#[derive(Deserialize)]
struct WireField {
    field: String,
}

/// Traduce el filtro JSON estructurado de `§20.10` al [`Expression`] unificado de E19-H01 — el
/// **mismo** AST al que [`crate::parse::parse`] traduce la consulta textual, de modo que `where` y
/// `filter` producen exactamente el mismo resultado.
///
/// Un filtro malformado (operador fuera de la tabla de nombres de wire, nodo sin forma reconocible,
/// un JSON que no es un objeto de filtro) es `Err(FilterError)`, **nunca** un panic — coherente con el
/// `ParseError` del textual.
pub fn from_json(value: &serde_json::Value) -> Result<Expression, FilterError> {
    let node: WireNode = serde_json::from_value(value.clone())
        .map_err(|e| FilterError::new(format!("filtro JSON malformado: {e}")))?;
    to_expression(node)
}

/// Transforma el árbol wire en el [`Expression`] final. Aquí —y solo aquí— vive la lógica de forma
/// que serde no da: la abreviatura de `frontmatter.` sobre `field` (vía [`build_field_path`]) y el
/// mapeo de `has`/`missing` a [`Expression::Function`].
fn to_expression(node: WireNode) -> Result<Expression, FilterError> {
    match node {
        WireNode::And { and } => Ok(Expression::And(mapear_ramas(and)?)),
        WireNode::Or { or } => Ok(Expression::Or(mapear_ramas(or)?)),
        WireNode::Not { not } => Ok(Expression::Not(Box::new(to_expression(*not)?))),
        WireNode::Has { has } => funcion(FunctionName::Has, &has.field),
        WireNode::Missing { missing } => funcion(FunctionName::Missing, &missing.field),
        WireNode::Comparison {
            field,
            operator,
            value,
        } => Ok(Expression::Comparison {
            field: normalizar_campo(&field)?,
            operator,
            value,
        }),
    }
}

/// Traduce las ramas de un conector (`and`/`or`), propagando el primer error.
fn mapear_ramas(ramas: Vec<WireNode>) -> Result<Vec<Expression>, FilterError> {
    ramas.into_iter().map(to_expression).collect()
}

/// Construye una [`Expression::Function`] de existencia. El argumento se normaliza como un
/// [`FieldPath`](crate::types::FieldPath) —misma abreviatura de `frontmatter.` que una comparación— y
/// se guarda como [`QueryValue::String`] de su forma con puntos, **idéntico** a como lo hace
/// `parse::parse_function` para el textual (por eso `has(...)`/`missing(...)` producen el mismo AST en
/// ambas superficies).
fn funcion(name: FunctionName, field: &str) -> Result<Expression, FilterError> {
    let path = normalizar_campo(field)?;
    Ok(Expression::Function {
        name,
        arguments: vec![QueryValue::String(path.to_string())],
    })
}

/// Normaliza un `field` del wire a su [`FieldPath`](crate::types::FieldPath) desnudo reutilizando la
/// **misma** [`build_field_path`] del parser textual — la identidad de esta normalización es lo que
/// hace exacta la equivalencia `where` ↔ `filter`. Un `field` inválido (vacío, segmento vacío) es
/// `Err`, no panic.
fn normalizar_campo(field: &str) -> Result<crate::types::FieldPath, FilterError> {
    build_field_path(field).map_err(|e| FilterError::new(e.message))
}
