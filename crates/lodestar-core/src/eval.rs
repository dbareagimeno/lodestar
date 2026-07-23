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

use std::cmp::Ordering;

use crate::types::{
    Analysis, ComparisonOperator, Expression, FieldPath, FunctionName, ParsedFrontmatter,
    QueryValue, RelPath, TypeError, ValueType,
};

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
// `analysis` hoy solo viaja por la recursión de los conectores lógicos (ningún operando hoja lo
// lee): es deliberado —lo consumirá el evaluador de `graph.*` en E19-H04 sin cambiar esta firma,
// que es el contrato `evaluate(expr, doc, analysis)`—, así que se silencia el lint en vez de
// mutilar la firma con un `_analysis`.
#[allow(clippy::only_used_in_recursion)]
pub fn evaluate(
    expr: &Expression,
    doc: &EvalDocument<'_>,
    analysis: &Analysis,
) -> Result<bool, TypeError> {
    match expr {
        Expression::Comparison {
            field,
            operator,
            value,
        } => eval_comparison(field, *operator, value, doc),
        Expression::Function { name, arguments } => Ok(eval_function(*name, arguments, doc)),
        // Los conectores lógicos evalúan con cortocircuito: `And` se detiene en la primera rama
        // falsa y `Or` en la primera verdadera, de modo que un error de tipo de una rama posterior
        // no se propaga si una anterior ya decidió el veredicto. Ningún test de H01 los ejercita
        // (los cubrirá E19-H02/H03), pero forman parte del AST de `§20.8`, así que se implementan.
        Expression::And(ramas) => {
            for rama in ramas {
                if !evaluate(rama, doc, analysis)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Expression::Or(ramas) => {
            for rama in ramas {
                if evaluate(rama, doc, analysis)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Expression::Not(interna) => Ok(!evaluate(interna, doc, analysis)?),
    }
}

/// Evalúa una comparación `campo operador valor` sobre el frontmatter de `doc`.
///
/// El acceso va **siempre** por [`ParsedFrontmatter::get`] (el `Value` con su tipo), nunca por
/// `get_text` (aviso rector de `§20.8`). Un campo **inexistente** cortocircuita a `Ok(false)`
/// **antes** de tipar: no se puede errar sobre un tipo que no se tiene.
fn eval_comparison(
    field: &FieldPath,
    operator: ComparisonOperator,
    literal: &QueryValue,
    doc: &EvalDocument<'_>,
) -> Result<bool, TypeError> {
    let Some(valor) = doc.frontmatter.and_then(|fm| fm.get(field)) else {
        return Ok(false);
    };
    let valor = untag(valor);

    use ComparisonOperator as C;
    match operator {
        C::Eq => Ok(valores_iguales(valor, literal)),
        C::Ne => Ok(!valores_iguales(valor, literal)),
        C::Gt | C::Ge | C::Lt | C::Le => eval_orden(field, operator, valor, literal),
        C::Contains => eval_contains(field, operator, valor, literal),
        C::StartsWith | C::EndsWith => Ok(eval_afijo(operator, valor, literal)),
        C::ContainsAny | C::ContainsAll => eval_contains_lista(field, operator, valor, literal),
    }
}

/// `has(x)`/`missing(x)`: existencia de la propiedad, **nunca** error de tipo. El argumento nombra
/// la propiedad como [`QueryValue::String`] (la forma del AST de `§20.8`); se reinterpreta como
/// [`FieldPath`] (parsea la dot-notation). La existencia se juzga con [`ParsedFrontmatter::get`],
/// así que una clave a `null`/`""`/`[]` cuenta como **presente**.
fn eval_function(name: FunctionName, arguments: &[QueryValue], doc: &EvalDocument<'_>) -> bool {
    let presente = propiedad_presente(arguments, doc);
    match name {
        FunctionName::Has => presente,
        FunctionName::Missing => !presente,
    }
}

/// `true` si el primer argumento nombra una propiedad presente en el frontmatter. Un argumento
/// ausente, no-string o con dot-notation inválida no direcciona ninguna propiedad → ausente.
fn propiedad_presente(arguments: &[QueryValue], doc: &EvalDocument<'_>) -> bool {
    let Some(QueryValue::String(nombre)) = arguments.first() else {
        return false;
    };
    let Ok(path) = FieldPath::parse(nombre) else {
        return false;
    };
    doc.frontmatter.and_then(|fm| fm.get(&path)).is_some()
}

/// Igualdad por **valor e igualdad de tipo** (`=`/`!=`). El cruce de tipos es `false`, **nunca**
/// error: `priority = "2"` sobre `priority: 2` (número) es `false` porque un string y un número
/// nunca son iguales, no porque comparar sea ilegal (eso es el ORDEN, ver [`eval_orden`]).
fn valores_iguales(valor: &serde_yaml::Value, literal: &QueryValue) -> bool {
    use serde_yaml::Value as V;
    match (valor, literal) {
        (V::Null, QueryValue::Null) => true,
        (V::Bool(a), QueryValue::Bool(b)) => a == b,
        (V::Number(a), QueryValue::Number(b)) => a == b,
        (V::String(a), QueryValue::String(b)) => a == b,
        (V::Sequence(items), QueryValue::List(lits)) => {
            items.len() == lits.len()
                && items
                    .iter()
                    .zip(lits)
                    .all(|(el, lit)| valores_iguales(untag(el), lit))
        }
        // Cruce de tipos (o mapa, que el lenguaje no compara por igualdad): distintos → false.
        _ => false,
    }
}

/// Comparación de **orden** (`> >= < <=`). Solo está definida entre dos números o entre dos
/// strings (lexicográfico); cualquier otro cruce —número vs string, o un tipo no ordenable como
/// booleano/`null`/lista/mapa a cualquier lado— es [`TypeError::OrderNotDefined`], **no** `false`.
fn eval_orden(
    field: &FieldPath,
    operator: ComparisonOperator,
    valor: &serde_yaml::Value,
    literal: &QueryValue,
) -> Result<bool, TypeError> {
    use serde_yaml::Value as V;
    let orden = match (valor, literal) {
        (V::Number(a), QueryValue::Number(b)) => comparar_numeros(a, b),
        (V::String(a), QueryValue::String(b)) => Some(a.as_str().cmp(b.as_str())),
        _ => None,
    };
    let Some(orden) = orden else {
        return Err(TypeError::OrderNotDefined {
            field: field.clone(),
            operator,
            field_type: ValueType::of(valor),
            value_type: tipo_de_literal(literal),
        });
    };
    use ComparisonOperator as C;
    Ok(match operator {
        C::Gt => orden == Ordering::Greater,
        C::Ge => orden != Ordering::Less,
        C::Lt => orden == Ordering::Less,
        C::Le => orden != Ordering::Greater,
        _ => unreachable!("eval_orden solo se invoca con > >= < <="),
    })
}

/// `contains`: el **tipo del campo decide** su significado. Sobre un **string** es subcadena; sobre
/// una **lista** es pertenencia; sobre cualquier otro escalar (número, booleano, `null`) o un mapa
/// es [`TypeError::NotAList`]. (`contains_any`/`contains_all` son exclusivos de listas; ver
/// [`eval_contains_lista`].)
fn eval_contains(
    field: &FieldPath,
    operator: ComparisonOperator,
    valor: &serde_yaml::Value,
    literal: &QueryValue,
) -> Result<bool, TypeError> {
    use serde_yaml::Value as V;
    match valor {
        // Subcadena: solo tiene sentido con un literal string (sin coerción, otro literal no casa).
        V::String(texto) => Ok(match literal {
            QueryValue::String(aguja) => texto.contains(aguja),
            _ => false,
        }),
        // Pertenencia: el literal es un elemento de la lista (comparado por valor e igualdad de tipo).
        V::Sequence(items) => Ok(items.iter().any(|el| valores_iguales(untag(el), literal))),
        otro => Err(TypeError::NotAList {
            field: field.clone(),
            operator,
            found: ValueType::of(otro),
        }),
    }
}

/// `starts_with`/`ends_with`: operadores de texto exclusivos de strings. Con un campo no-string o un
/// literal no-string no hay prefijo/sufijo que comprobar → `false` (no hay una variante de
/// [`TypeError`] «no es string», y H01 no la introduce; ningún test lo ejercita).
fn eval_afijo(
    operator: ComparisonOperator,
    valor: &serde_yaml::Value,
    literal: &QueryValue,
) -> bool {
    let (serde_yaml::Value::String(texto), QueryValue::String(aguja)) = (valor, literal) else {
        return false;
    };
    match operator {
        ComparisonOperator::StartsWith => texto.starts_with(aguja),
        ComparisonOperator::EndsWith => texto.ends_with(aguja),
        _ => unreachable!("eval_afijo solo se invoca con starts_with/ends_with"),
    }
}

/// `contains_any`/`contains_all`: **exclusivos de listas**. Sobre cualquier no-lista —incluido un
/// string, que `contains` a secas sí trata como texto— es [`TypeError::NotAList`]. `contains_any`
/// pide compartir al menos un elemento con el literal; `contains_all`, contenerlos todos.
fn eval_contains_lista(
    field: &FieldPath,
    operator: ComparisonOperator,
    valor: &serde_yaml::Value,
    literal: &QueryValue,
) -> Result<bool, TypeError> {
    let serde_yaml::Value::Sequence(items) = valor else {
        return Err(TypeError::NotAList {
            field: field.clone(),
            operator,
            found: ValueType::of(valor),
        });
    };
    // El operando es un literal lista; un literal escalar se trata como singleton (defensivo: el
    // parser de E19-H02/H03 garantiza la lista, ningún test de H01 pasa un escalar aquí).
    let agujas: Vec<&QueryValue> = match literal {
        QueryValue::List(lits) => lits.iter().collect(),
        otro => vec![otro],
    };
    let contiene = |aguja: &QueryValue| items.iter().any(|el| valores_iguales(untag(el), aguja));
    Ok(match operator {
        ComparisonOperator::ContainsAny => agujas.iter().any(|a| contiene(a)),
        ComparisonOperator::ContainsAll => agujas.iter().all(|a| contiene(a)),
        _ => unreachable!("eval_contains_lista solo se invoca con contains_any/contains_all"),
    })
}

/// El [`ValueType`] de un literal de consulta (el reflejo de [`ValueType::of`] para el operando
/// derecho), para poblar el `value_type` de un [`TypeError::OrderNotDefined`].
fn tipo_de_literal(literal: &QueryValue) -> ValueType {
    match literal {
        QueryValue::Null => ValueType::Null,
        QueryValue::Bool(_) => ValueType::Bool,
        QueryValue::Number(_) => ValueType::Number,
        QueryValue::String(_) => ValueType::String,
        QueryValue::List(_) => ValueType::List,
    }
}

/// Orden entre dos números YAML. Compara como enteros cuando ambos lo son (sin pérdida de
/// precisión) y cae a `f64` en otro caso; `None` si algún operando no es un número finito
/// comparable (p. ej. `NaN`).
fn comparar_numeros(a: &serde_yaml::Number, b: &serde_yaml::Number) -> Option<Ordering> {
    if let (Some(x), Some(y)) = (a.as_i64(), b.as_i64()) {
        return Some(x.cmp(&y));
    }
    if let (Some(x), Some(y)) = (a.as_u64(), b.as_u64()) {
        return Some(x.cmp(&y));
    }
    a.as_f64()?.partial_cmp(&b.as_f64()?)
}

/// Deshace un `!Tag valor` de YAML devolviendo el valor interno, de forma consistente con
/// [`ValueType::of`] (que también clasifica un [`serde_yaml::Value::Tagged`] por su interior).
fn untag(valor: &serde_yaml::Value) -> &serde_yaml::Value {
    match valor {
        serde_yaml::Value::Tagged(t) => untag(&t.value),
        otro => otro,
    }
}
