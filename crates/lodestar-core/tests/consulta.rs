//! Tests del **lenguaje de consulta tipado** (épica E19, `ARCHITECTURE.md §20.8`).
//!
//! Fase ROJA de **E19-H01** — el AST ([`Expression`]) y su evaluador tipado
//! ([`lodestar_core::eval::evaluate`]): las comparaciones respetan los tipos YAML **sin coerción**,
//! sobre [`ParsedFrontmatter::get`] (el `Value` con su tipo), **nunca** sobre `get_text` (que
//! renderiza a `String`). Sustituye la DSL de subcadena de `query.rs`, que se retira en E19-H05.
//!
//! Fichero propio (no `documento.rs` ni `core.rs`) por tres motivos, los mismos que aislaron
//! `documento.rs` en E16:
//!   1. Estos tests **no pasan** hasta que exista el evaluador ([`evaluate`] es hoy `todo!()`):
//!      aislados, su rojo no arrastra a los ~329 tests verdes de los demás binarios.
//!   2. E19-H02 (parser textual) y E19-H04 (namespaces) aportan la misma familia —el lenguaje de
//!      consulta— y tienen aquí su hogar natural.
//!   3. El estilo del repo es «un fichero de integración por familia» (`enlaces.rs`, `grafo.rs`,
//!      `diagnosticos.rs`); `consulta.rs` es esa familia.
//!
//! ---
//!
//! ## La asimetría que estos tests clavan (el contrato que hereda toda E19)
//!
//! **Sin coerción implícita, y el cruce de ORDEN es un error de tipo, no `false`:**
//!
//! | Caso | Resultado |
//! |---|---|
//! | `priority = "2"` sobre `priority: 2` (número) | `Ok(false)` — igualdad cruzada = **false** |
//! | `priority >= "high"` sobre `priority: 2` | `Err(TypeError)` — orden cruzado = **error** |
//! | `contains` sobre un **string** | subcadena (es texto) |
//! | `contains`/`contains_any`/`contains_all` sobre un **escalar no string** | `Err(TypeError)` |
//! | un campo **inexistente** en una comparación | `Ok(false)` — nunca error |
//!
//! Los casos `priority = "2" → false` y `priority >= "high" → error` (mismo campo, mismo literal de
//! forma numérica) son también la **red contra la regresión a `get_text`** que `§20.8` teme: un
//! evaluador construido sobre `get_text` renderizaría `priority: 2` a `"2"` y daría `Ok(true)` a la
//! igualdad y `Ok(false)` al orden — justo lo que estos asserts prohíben.

use lodestar_core::eval::{evaluate, EvalDocument};
use lodestar_core::model;
use lodestar_core::parse::parse;
use lodestar_core::types::ComparisonOperator as Op;
use lodestar_core::types::{
    Analysis, Expression, FieldPath, FunctionName, ParsedFrontmatter, QueryValue, RelPath,
    TypeError, ValueType,
};
use lodestar_core::DocumentSet;

// --- Utilidades --------------------------------------------------------------

/// `RelPath` para rutas obviamente válidas (invariante #6: nunca un string crudo).
fn rp(p: &str) -> RelPath {
    RelPath::new(p).unwrap_or_else(|e| panic!("`{p}` debe ser un RelPath válido: {e:?}"))
}

/// `FieldPath` desde dot-notation.
fn fp(s: &str) -> FieldPath {
    FieldPath::parse(s).unwrap_or_else(|e| panic!("`{s}` debe ser un FieldPath válido: {e:?}"))
}

/// Construye un [`ParsedFrontmatter`] a partir del cuerpo YAML de un bloque (sin delimitadores):
/// lo envuelve en un documento mínimo y lo parsea con el modelo real, para que los tipos YAML
/// lleguen al evaluador tal como llegarían en producción.
fn fm(yaml: &str) -> ParsedFrontmatter {
    let raw = format!("---\n{yaml}\n---\n\n# doc\n");
    model::parse_file("doc.md", &raw)
        .frontmatter
        .expect("el fixture define un frontmatter YAML válido")
}

/// Evalúa `expr` contra un documento cuyo frontmatter es `f`. El `Analysis` es el vacío por
/// defecto: E19-H01 no consulta el grafo (eso es E19-H04).
fn eval(expr: &Expression, f: &ParsedFrontmatter) -> Result<bool, TypeError> {
    let path = rp("doc.md");
    let doc = EvalDocument {
        path: &path,
        frontmatter: Some(f),
        body: "",
    };
    evaluate(expr, &doc, &Analysis::default())
}

/// `campo operador valor`.
fn cmp(field: &str, operator: Op, value: QueryValue) -> Expression {
    Expression::Comparison {
        field: fp(field),
        operator,
        value,
    }
}

/// `has(field)` / `missing(field)`: el argumento nombra la propiedad como [`QueryValue::String`]
/// (la forma que impone el AST de `§20.8`).
fn func(name: FunctionName, field: &str) -> Expression {
    Expression::Function {
        name,
        arguments: vec![qstr(field)],
    }
}

/// Literal numérico (entero).
fn num(n: i64) -> QueryValue {
    QueryValue::Number(n.into())
}

/// Literal string.
fn qstr(s: &str) -> QueryValue {
    QueryValue::String(s.to_string())
}

/// Literal lista de strings — el operando de `contains_any`/`contains_all`.
fn qlist(items: &[&str]) -> QueryValue {
    QueryValue::List(items.iter().map(|s| qstr(s)).collect())
}

// =============================================================================
// E19-H01 — Igualdad, orden y booleanos
// =============================================================================

/// Criterio: igualdad de string (`eq_string`).
///
/// `=`/`!=` comparan por valor **e igualdad de tipo**. El caso clave es el **cruce**: comparar un
/// string literal contra un campo numérico o booleano es `false`, **no** `true` (sería la coerción
/// de `get_text`) y **no** error (el error es solo del orden, no de la igualdad).
#[test]
fn eq_string() {
    let f = fm("status: accepted\npriority: 2\nreviewed: true");

    assert_eq!(
        eval(&cmp("status", Op::Eq, qstr("accepted")), &f),
        Ok(true),
        "`status = \"accepted\"` casa el string igual"
    );
    assert_eq!(
        eval(&cmp("status", Op::Eq, qstr("draft")), &f),
        Ok(false),
        "`status = \"draft\"` no casa un string distinto"
    );
    assert_eq!(
        eval(&cmp("status", Op::Ne, qstr("draft")), &f),
        Ok(true),
        "`status != \"draft\"` es la negación de la igualdad"
    );
    assert_eq!(
        eval(&cmp("status", Op::Ne, qstr("accepted")), &f),
        Ok(false),
        "`status != \"accepted\"` es falso cuando son iguales"
    );

    // Cruce de tipos en IGUALDAD = `false` (no `true`, no error). `priority: 2` es un número; el
    // literal `"2"` es un string. Un evaluador sobre `get_text` renderizaría `priority` a `"2"` y
    // daría `true`: esto lo prohíbe.
    assert_eq!(
        eval(&cmp("priority", Op::Eq, qstr("2")), &f),
        Ok(false),
        "`priority = \"2\"` (string) sobre `priority: 2` (número) es FALSE: la igualdad no coerciona"
    );
    assert_eq!(
        eval(&cmp("priority", Op::Ne, qstr("2")), &f),
        Ok(true),
        "`priority != \"2\"` es verdadero: tipos distintos no son iguales"
    );
    assert_eq!(
        eval(&cmp("reviewed", Op::Eq, qstr("true")), &f),
        Ok(false),
        "`reviewed = \"true\"` (string) sobre `reviewed: true` (booleano) es FALSE: sin coerción"
    );
}

/// Criterio: comparación numérica (`cmp_numerico`).
///
/// El orden entre números es numérico; entre strings, lexicográfico (mismo tipo). La igualdad
/// numérica con literal numérico casa.
#[test]
fn cmp_numerico() {
    let f = fm("priority: 2\nstatus: accepted");

    assert_eq!(
        eval(&cmp("priority", Op::Ge, num(2)), &f),
        Ok(true),
        "2 >= 2"
    );
    assert_eq!(
        eval(&cmp("priority", Op::Ge, num(3)), &f),
        Ok(false),
        "2 >= 3 es falso"
    );
    assert_eq!(
        eval(&cmp("priority", Op::Gt, num(1)), &f),
        Ok(true),
        "2 > 1"
    );
    assert_eq!(
        eval(&cmp("priority", Op::Gt, num(2)), &f),
        Ok(false),
        "2 > 2 es falso"
    );
    assert_eq!(
        eval(&cmp("priority", Op::Lt, num(5)), &f),
        Ok(true),
        "2 < 5"
    );
    assert_eq!(
        eval(&cmp("priority", Op::Le, num(2)), &f),
        Ok(true),
        "2 <= 2"
    );
    assert_eq!(
        eval(&cmp("priority", Op::Eq, num(2)), &f),
        Ok(true),
        "`priority = 2` (número) sobre `priority: 2` casa por valor y tipo"
    );
    assert_eq!(
        eval(&cmp("priority", Op::Ne, num(2)), &f),
        Ok(false),
        "`priority != 2` es falso cuando son iguales"
    );

    // Orden entre dos strings: lexicográfico y VÁLIDO (mismo tipo, no error).
    assert_eq!(
        eval(&cmp("status", Op::Ge, qstr("a")), &f),
        Ok(true),
        "\"accepted\" >= \"a\": el orden entre strings es lexicográfico"
    );
    assert_eq!(
        eval(&cmp("status", Op::Lt, qstr("b")), &f),
        Ok(true),
        "\"accepted\" < \"b\" lexicográficamente"
    );
    assert_eq!(
        eval(&cmp("status", Op::Gt, qstr("z")), &f),
        Ok(false),
        "\"accepted\" > \"z\" es falso lexicográficamente"
    );
}

/// Criterio: booleanos (`booleano`).
#[test]
fn booleano() {
    let f = fm("reviewed: true\narchived: false");

    assert_eq!(
        eval(&cmp("reviewed", Op::Eq, QueryValue::Bool(true)), &f),
        Ok(true),
        "`reviewed = true` casa el booleano"
    );
    assert_eq!(
        eval(&cmp("reviewed", Op::Eq, QueryValue::Bool(false)), &f),
        Ok(false),
        "`reviewed = false` sobre `reviewed: true` es falso"
    );
    assert_eq!(
        eval(&cmp("archived", Op::Eq, QueryValue::Bool(false)), &f),
        Ok(true),
        "`archived = false` casa"
    );
    assert_eq!(
        eval(&cmp("reviewed", Op::Ne, QueryValue::Bool(false)), &f),
        Ok(true),
        "`reviewed != false` es verdadero"
    );
}

// =============================================================================
// E19-H01 — contains: texto y listas (el tipo del campo decide)
// =============================================================================

/// Criterio: `contains` sobre un string es **subcadena** (`contains_string`).
///
/// `starts_with`/`ends_with` son los otros dos operadores de texto.
#[test]
fn contains_string() {
    let f = fm("title: authentication");

    assert_eq!(
        eval(&cmp("title", Op::Contains, qstr("auth")), &f),
        Ok(true),
        "`title contains \"auth\"`: subcadena sobre un string"
    );
    assert_eq!(
        eval(&cmp("title", Op::Contains, qstr("xyz")), &f),
        Ok(false),
        "`title contains \"xyz\"`: la subcadena no aparece"
    );
    assert_eq!(
        eval(&cmp("title", Op::StartsWith, qstr("auth")), &f),
        Ok(true),
        "`title starts_with \"auth\"`: prefijo"
    );
    assert_eq!(
        eval(&cmp("title", Op::StartsWith, qstr("hen")), &f),
        Ok(false),
        "`title starts_with \"hen\"`: no es prefijo"
    );
    assert_eq!(
        eval(&cmp("title", Op::EndsWith, qstr("cation")), &f),
        Ok(true),
        "`title ends_with \"cation\"`: sufijo"
    );
    assert_eq!(
        eval(&cmp("title", Op::EndsWith, qstr("auth")), &f),
        Ok(false),
        "`title ends_with \"auth\"`: no es sufijo"
    );
}

/// Criterio: `contains` sobre una lista es **pertenencia** (`contains_lista`).
#[test]
fn contains_lista() {
    let f = fm(concat!("owners:\n", "  - platform\n", "  - security"));

    assert_eq!(
        eval(&cmp("owners", Op::Contains, qstr("security")), &f),
        Ok(true),
        "`owners contains \"security\"`: pertenencia a la lista"
    );
    assert_eq!(
        eval(&cmp("owners", Op::Contains, qstr("legal")), &f),
        Ok(false),
        "`owners contains \"legal\"`: el elemento no está en la lista"
    );
}

/// Criterio: `contains_any` (`contains_any_ok`).
#[test]
fn contains_any_ok() {
    let f = fm(concat!("owners:\n", "  - platform\n", "  - security"));

    assert_eq!(
        eval(
            &cmp("owners", Op::ContainsAny, qlist(&["security", "legal"])),
            &f
        ),
        Ok(true),
        "`owners contains_any [\"security\", \"legal\"]`: comparte al menos un elemento"
    );
    assert_eq!(
        eval(
            &cmp("owners", Op::ContainsAny, qlist(&["legal", "finance"])),
            &f
        ),
        Ok(false),
        "`owners contains_any [\"legal\", \"finance\"]`: no comparte ninguno"
    );
}

/// Criterio: `contains_all` (`contains_all_ok`).
#[test]
fn contains_all_ok() {
    let f = fm(concat!("owners:\n", "  - platform\n", "  - security"));

    assert_eq!(
        eval(
            &cmp("owners", Op::ContainsAll, qlist(&["platform", "security"])),
            &f
        ),
        Ok(true),
        "`owners contains_all [\"platform\", \"security\"]`: contiene todos"
    );
    assert_eq!(
        eval(
            &cmp("owners", Op::ContainsAll, qlist(&["platform", "legal"])),
            &f
        ),
        Ok(false),
        "`owners contains_all [\"platform\", \"legal\"]`: falta `legal`"
    );
}

// =============================================================================
// E19-H01 — Existencia: has / missing
// =============================================================================

/// Criterio: `has(x)` (`has_ok`).
///
/// La existencia se juzga con [`ParsedFrontmatter::get`]: una clave presente cuenta **aunque su
/// valor sea `null`, `""` o `[]`** — al revés que la vieja `fmPresent` de `query.rs`, que trataba
/// la cadena y la lista vacías como ausencia. Esa diferencia es la red anti-regresión de esta
/// historia.
#[test]
fn has_ok() {
    let f = fm(concat!(
        "status: accepted\n",
        "nota_vacia: \"\"\n",
        "sin_duenos: []\n",
        "deprecated_field: null\n",
        "service:\n",
        "  tier: critical",
    ));

    assert_eq!(
        eval(&func(FunctionName::Has, "status"), &f),
        Ok(true),
        "`has(status)`: la clave existe"
    );
    assert_eq!(
        eval(&func(FunctionName::Has, "no_existe"), &f),
        Ok(false),
        "`has(no_existe)`: la clave no existe"
    );
    assert_eq!(
        eval(&func(FunctionName::Has, "nota_vacia"), &f),
        Ok(true),
        "`has(nota_vacia)`: la cadena vacía es un valor PRESENTE (no como la vieja fmPresent)"
    );
    assert_eq!(
        eval(&func(FunctionName::Has, "sin_duenos"), &f),
        Ok(true),
        "`has(sin_duenos)`: la lista vacía es un valor PRESENTE"
    );
    assert_eq!(
        eval(&func(FunctionName::Has, "deprecated_field"), &f),
        Ok(true),
        "`has(deprecated_field)`: una clave a `null` está presente"
    );
    assert_eq!(
        eval(&func(FunctionName::Has, "service.tier"), &f),
        Ok(true),
        "`has(service.tier)`: la dot-notation desciende hasta la clave anidada"
    );
    assert_eq!(
        eval(&func(FunctionName::Has, "service.ausente"), &f),
        Ok(false),
        "`has(service.ausente)`: la clave anidada no existe"
    );
}

/// Criterio: `missing(x)` (`missing_ok`) — la negación exacta de `has`.
#[test]
fn missing_ok() {
    let f = fm(concat!(
        "status: accepted\n",
        "deprecated_field: null\n",
        "service:\n",
        "  tier: critical",
    ));

    assert_eq!(
        eval(&func(FunctionName::Missing, "reviewed_at"), &f),
        Ok(true),
        "`missing(reviewed_at)`: la clave no existe"
    );
    assert_eq!(
        eval(&func(FunctionName::Missing, "status"), &f),
        Ok(false),
        "`missing(status)`: la clave existe"
    );
    assert_eq!(
        eval(&func(FunctionName::Missing, "deprecated_field"), &f),
        Ok(false),
        "`missing(deprecated_field)`: una clave a `null` está presente, así que NO falta"
    );
    assert_eq!(
        eval(&func(FunctionName::Missing, "service.tier"), &f),
        Ok(false),
        "`missing(service.tier)`: la clave anidada existe"
    );
    assert_eq!(
        eval(&func(FunctionName::Missing, "service.ausente"), &f),
        Ok(true),
        "`missing(service.ausente)`: la clave anidada no existe"
    );
}

// =============================================================================
// E19-H01 — Ausencia y errores de tipo (la asimetría rectora)
// =============================================================================

/// Criterio: un campo inexistente en una comparación es `false`, no error (`campo_inexistente`).
///
/// La ausencia **cortocircuita antes** de comprobar tipos: por eso `nonexistent >= 2` y
/// `nonexistent contains "x"` son `Ok(false)` y **no** `TypeError`, aunque un `>=`/`contains` sobre
/// ese mismo valor si existiera pudiera serlo. No se puede errar sobre un tipo que no se tiene.
#[test]
fn campo_inexistente() {
    let f = fm("status: accepted");

    assert_eq!(
        eval(&cmp("no_existe", Op::Eq, qstr("x")), &f),
        Ok(false),
        "`no_existe = \"x\"`: un campo ausente no casa la igualdad"
    );
    assert_eq!(
        eval(&cmp("no_existe", Op::Ne, qstr("x")), &f),
        Ok(false),
        "`no_existe != \"x\"`: un campo ausente tampoco casa el `!=` (no es error ni true)"
    );
    assert_eq!(
        eval(&cmp("no_existe", Op::Ge, num(2)), &f),
        Ok(false),
        "`no_existe >= 2`: la ausencia es FALSE, no un error de orden — se corta antes de tipar"
    );
    assert_eq!(
        eval(&cmp("no_existe", Op::Contains, qstr("x")), &f),
        Ok(false),
        "`no_existe contains \"x\"`: la ausencia es FALSE, no un error de lista"
    );
    assert_eq!(
        eval(&cmp("service.tier", Op::Eq, qstr("x")), &f),
        Ok(false),
        "`service.tier = \"x\"`: descender por un mapa inexistente también es ausencia = FALSE"
    );
}

/// Criterio: `priority >= "high"` sobre `priority: 2` es un `TypeError` (`error_de_tipo_orden_cruzado`).
///
/// Es el corazón del lenguaje. Este test clava la asimetría **dentro de un mismo fixture**: el
/// **orden** cruzado es error; la **igualdad** cruzada es `false`. Y remata la red anti-`get_text`:
/// `priority > "2"` —cuyo literal *parece* numérico— sigue siendo error, porque no hay coerción.
#[test]
fn error_de_tipo_orden_cruzado() {
    let f = fm("priority: 2\nstatus: accepted\nreviewed: true");

    // Orden cruzado número-vs-string: ERROR, con los tipos de ambos operandos.
    let r = eval(&cmp("priority", Op::Ge, qstr("high")), &f);
    assert!(
        matches!(
            r,
            Err(TypeError::OrderNotDefined {
                field_type: ValueType::Number,
                value_type: ValueType::String,
                ..
            })
        ),
        "`priority >= \"high\"` debe ser OrderNotDefined{{number, string}}, no {r:?}"
    );
    assert_ne!(r, Ok(false), "el orden cruzado es ERROR, nunca `false`");
    assert_ne!(r, Ok(true), "y desde luego nunca `true`");

    // El literal `"2"` *parece* un número pero es un string: sin coerción, sigue siendo error.
    assert!(
        matches!(
            eval(&cmp("priority", Op::Gt, qstr("2")), &f),
            Err(TypeError::OrderNotDefined { .. })
        ),
        "`priority > \"2\"` es error: `\"2\"` es string, y no se coerciona a número"
    );

    // Simétrico: campo string, literal número.
    assert!(
        matches!(
            eval(&cmp("status", Op::Ge, num(2)), &f),
            Err(TypeError::OrderNotDefined {
                field_type: ValueType::String,
                value_type: ValueType::Number,
                ..
            })
        ),
        "`status >= 2` es error simétrico: string frente a número"
    );

    // Orden sobre un tipo no ordenable (booleano), aunque ambos lados sean del mismo tipo.
    assert!(
        matches!(
            eval(&cmp("reviewed", Op::Gt, QueryValue::Bool(false)), &f),
            Err(TypeError::OrderNotDefined {
                field_type: ValueType::Bool,
                ..
            })
        ),
        "`reviewed > false` es error: el orden no está definido sobre booleanos"
    );

    // El CONTRASTE que separa este lenguaje de un grep: misma forma, operador de IGUALDAD →
    // `false`, no error.
    assert_eq!(
        eval(&cmp("priority", Op::Eq, qstr("high")), &f),
        Ok(false),
        "`priority = \"high\"`: la igualdad cruzada es FALSE, no error (solo el ORDEN yerra)"
    );
    assert_eq!(
        eval(&cmp("priority", Op::Eq, qstr("2")), &f),
        Ok(false),
        "`priority = \"2\"`: igualdad cruzada número/string = FALSE (misma forma, distinto veredicto)"
    );
}

/// Criterio: `contains` sobre un escalar es un `TypeError` (`error_de_tipo_contains_escalar`).
///
/// El operador de lista exige una lista. Sobre un escalar **no string** es error; sobre un string,
/// `contains` es texto (subcadena, ver `contains_string`) — pero `contains_any`/`contains_all`, que
/// son exclusivos de listas, son error también sobre un string. El tipo del campo decide.
#[test]
fn error_de_tipo_contains_escalar() {
    let f = fm("priority: 2\ntitle: authentication");

    // `contains` sobre un número: error de lista, con el tipo hallado.
    let r = eval(&cmp("priority", Op::Contains, qstr("2")), &f);
    assert!(
        matches!(
            r,
            Err(TypeError::NotAList {
                found: ValueType::Number,
                ..
            })
        ),
        "`priority contains \"2\"` debe ser NotAList{{number}}, no {r:?}"
    );
    assert_ne!(
        r,
        Ok(false),
        "`contains` sobre un escalar es ERROR, no `false`"
    );

    // `contains_any` sobre un número: error.
    assert!(
        matches!(
            eval(&cmp("priority", Op::ContainsAny, qlist(&["2"])), &f),
            Err(TypeError::NotAList {
                found: ValueType::Number,
                ..
            })
        ),
        "`priority contains_any [\"2\"]` es error: `priority` no es lista"
    );

    // `contains_all` sobre un STRING: error, porque `contains_all` es exclusivo de listas (un
    // string no cuenta, aunque `contains` a secas sí lo trate como texto).
    assert!(
        matches!(
            eval(&cmp("title", Op::ContainsAll, qlist(&["auth"])), &f),
            Err(TypeError::NotAList {
                found: ValueType::String,
                ..
            })
        ),
        "`title contains_all [\"auth\"]` es error: `contains_all` no acepta un string como lista"
    );
}

/// Criterio: una propiedad con tipos distintos en dos documentos se evalúa según el tipo de **su**
/// documento (`tipos_heterogeneos`).
///
/// Es la prueba viva de que el evaluador lee el `Value` real de cada documento (no un texto
/// aplanado común): el **mismo** `priority >= 2` es `Ok(true)` sobre el documento numérico y
/// `Err(TypeError)` sobre el documento donde `priority` es un string.
#[test]
fn tipos_heterogeneos() {
    let numerico = fm("priority: 2"); // número
    let textual = fm("priority: high"); // string (bare scalar YAML)

    // `priority >= 2` respeta el tipo de cada documento.
    assert_eq!(
        eval(&cmp("priority", Op::Ge, num(2)), &numerico),
        Ok(true),
        "sobre el documento numérico, `priority >= 2` compara números"
    );
    assert!(
        matches!(
            eval(&cmp("priority", Op::Ge, num(2)), &textual),
            Err(TypeError::OrderNotDefined {
                field_type: ValueType::String,
                value_type: ValueType::Number,
                ..
            })
        ),
        "sobre el documento textual, el mismo `priority >= 2` es error: string vs número"
    );

    // `priority = "high"` (string): casa el textual, no el numérico (igualdad cruzada = false).
    assert_eq!(
        eval(&cmp("priority", Op::Eq, qstr("high")), &textual),
        Ok(true),
        "sobre el documento textual, `priority = \"high\"` casa por valor y tipo"
    );
    assert_eq!(
        eval(&cmp("priority", Op::Eq, qstr("high")), &numerico),
        Ok(false),
        "sobre el documento numérico, `priority = \"high\"` es FALSE (número vs string)"
    );

    // `priority = 2` (número): al revés.
    assert_eq!(
        eval(&cmp("priority", Op::Eq, num(2)), &numerico),
        Ok(true),
        "`priority = 2` casa el documento numérico"
    );
    assert_eq!(
        eval(&cmp("priority", Op::Eq, num(2)), &textual),
        Ok(false),
        "`priority = 2` sobre el documento textual es FALSE (string \"high\" vs número 2)"
    );
}

// =============================================================================
// Utilidades añadidas para E19-H02/H04
// =============================================================================

/// Conjunción `And(ramas)`.
fn and(ramas: Vec<Expression>) -> Expression {
    Expression::And(ramas)
}

/// Disyunción `Or(ramas)`.
fn or(ramas: Vec<Expression>) -> Expression {
    Expression::Or(ramas)
}

/// Negación `Not(inner)`.
fn not(inner: Expression) -> Expression {
    Expression::Not(Box::new(inner))
}

/// Literal booleano.
fn qbool(b: bool) -> QueryValue {
    QueryValue::Bool(b)
}

/// El literal `null`.
fn qnull() -> QueryValue {
    QueryValue::Null
}

// =============================================================================
// E19-H02 — El parser textual (`where`)
// =============================================================================
//
// La firma que fija esta fase: `lodestar_core::parse::parse(&str) -> Result<Expression, ParseError>`
// (módulo NUEVO `parse`, no la DSL de subcadena de `query.rs`, que se retira en E19-H05). El parser
// traduce la consulta textual al MISMO `Expression` de H01, sin coerción: el tipo de un literal nace
// de su forma sintáctica (comillas → string; sin comillas → número/booleano/`null` por su escritura).
//
// **Decisión de criterio (abreviatura de namespace)**: `frontmatter.X` y `X` a secas producen el
// `FieldPath` DESNUDO (`["X"]`), NO `["frontmatter", "X"]` — el prefijo `frontmatter.` se normaliza
// fuera. Es la única forma consistente con el evaluador YA VERDE de H01 (que va directo a
// `ParsedFrontmatter::get(field)`) y con el reparto de H04 (primer segmento `document`/`graph` =
// namespace; cualquier otro = frontmatter). Los tests `abreviatura_de_namespace` y
// `dot_notation_textual` clavan esa forma.

/// Criterio: `and` (`and_ok`).
#[test]
fn and_ok() {
    assert_eq!(
        parse(r#"type = "decision" and status = "accepted""#).unwrap(),
        and(vec![
            cmp("type", Op::Eq, qstr("decision")),
            cmp("status", Op::Eq, qstr("accepted")),
        ]),
        "`a and b` es una conjunción de las dos comparaciones"
    );
}

/// Criterio: `or` (`or_ok`).
#[test]
fn or_ok() {
    assert_eq!(
        parse(r#"status = "draft" or status = "review""#).unwrap(),
        or(vec![
            cmp("status", Op::Eq, qstr("draft")),
            cmp("status", Op::Eq, qstr("review")),
        ]),
        "`a or b` es una disyunción de las dos comparaciones"
    );
}

/// Criterio: `not` (`not_ok`).
#[test]
fn not_ok() {
    assert_eq!(
        parse(r#"not tags contains "archived""#).unwrap(),
        not(cmp("tags", Op::Contains, qstr("archived"))),
        "`not` niega la comparación que le sigue"
    );
}

/// Criterio: paréntesis (`parentesis`).
#[test]
fn parentesis() {
    // (1) Un grupo entre paréntesis es exactamente su contenido, sin envoltura extra.
    assert_eq!(
        parse(r#"(status = "draft" or status = "review")"#).unwrap(),
        or(vec![
            cmp("status", Op::Eq, qstr("draft")),
            cmp("status", Op::Eq, qstr("review")),
        ]),
        "los paréntesis alrededor de una expresión no añaden ningún nodo"
    );
    // (2) Los paréntesis REAGRUPAN contra la precedencia natural: sin ellos el `and` ataría primero;
    //     con ellos, el `or` queda anidado bajo el `and`.
    assert_eq!(
        parse(r#"type = "decision" and (status = "draft" or status = "review")"#).unwrap(),
        and(vec![
            cmp("type", Op::Eq, qstr("decision")),
            or(vec![
                cmp("status", Op::Eq, qstr("draft")),
                cmp("status", Op::Eq, qstr("review")),
            ]),
        ]),
        "los paréntesis fuerzan el `or` bajo el `and`"
    );
}

/// Criterio: precedencia `not` > `and` > `or` (`precedencia`).
#[test]
fn precedencia() {
    // `a or b and c` = `a or (b and c)`: el `and` liga más fuerte que el `or`.
    assert_eq!(
        parse(r#"status = "a" or status = "b" and status = "c""#).unwrap(),
        or(vec![
            cmp("status", Op::Eq, qstr("a")),
            and(vec![
                cmp("status", Op::Eq, qstr("b")),
                cmp("status", Op::Eq, qstr("c")),
            ]),
        ]),
        "`and` liga más que `or`: `a or b and c` = `a or (b and c)`"
    );
    // `not a and b` = `(not a) and b`: el `not` liga más fuerte que el `and`.
    assert_eq!(
        parse(r#"not status = "a" and status = "b""#).unwrap(),
        and(vec![
            not(cmp("status", Op::Eq, qstr("a"))),
            cmp("status", Op::Eq, qstr("b")),
        ]),
        "`not` liga más que `and`: `not a and b` = `(not a) and b`"
    );
}

/// Criterio: dot-notation (`dot_notation_textual`).
#[test]
fn dot_notation_textual() {
    let expr = parse(r#"service.tier = "critical""#).unwrap();
    assert_eq!(
        expr,
        cmp("service.tier", Op::Eq, qstr("critical")),
        "`service.tier = \"critical\"` es una `Comparison` con `FieldPath` de dos segmentos"
    );
    // El `FieldPath` es EXACTAMENTE los dos segmentos (no una clave literal con punto).
    let Expression::Comparison { field, .. } = &expr else {
        panic!("un `campo = valor` parsea a una `Comparison`: {expr:?}");
    };
    assert_eq!(
        field.segments(),
        ["service", "tier"],
        "la dot-notation parte por puntos en dos segmentos"
    );
}

/// Criterio: abreviatura de namespace (`abreviatura_de_namespace`).
#[test]
fn abreviatura_de_namespace() {
    let abreviado = parse(r#"status = "accepted""#).unwrap();
    let explicito = parse(r#"frontmatter.status = "accepted""#).unwrap();

    assert_eq!(
        abreviado, explicito,
        "`status = ...` y `frontmatter.status = ...` producen el MISMO `Expression`"
    );
    // …y ese AST lleva la ruta de frontmatter DESNUDA: `frontmatter.` se normaliza fuera, para que
    // el evaluador de H01 —que va directo a `ParsedFrontmatter::get(field)`— la resuelva sin conocer
    // el prefijo, y `document`/`graph` queden como únicos primeros segmentos reservados (E19-H04).
    assert_eq!(
        abreviado,
        cmp("status", Op::Eq, qstr("accepted")),
        "la abreviatura resuelve a `FieldPath([\"status\"])`, no a `[\"frontmatter\", \"status\"]`"
    );
}

/// Criterio: literales por forma (`literales_por_forma`).
#[test]
fn literales_por_forma() {
    // Un número sin comillas es NÚMERO.
    assert_eq!(
        parse("priority = 2").unwrap(),
        cmp("priority", Op::Eq, num(2)),
        "`2` sin comillas es un literal numérico"
    );
    // Un booleano sin comillas es BOOLEANO.
    assert_eq!(
        parse("reviewed = true").unwrap(),
        cmp("reviewed", Op::Eq, qbool(true)),
        "`true` sin comillas es un literal booleano"
    );
    // `null` sin comillas es NULL.
    assert_eq!(
        parse("deprecated = null").unwrap(),
        cmp("deprecated", Op::Eq, qnull()),
        "`null` sin comillas es el literal nulo"
    );
    // Lo MISMO entre comillas es STRING: `"2"` no se coerciona al número 2 (la red anti-`get_text`
    // empieza ya en el parser — el tipo del literal nace de su forma sintáctica).
    assert_eq!(
        parse(r#"label = "2""#).unwrap(),
        cmp("label", Op::Eq, qstr("2")),
        "`\"2\"` entre comillas es un string, no el número 2"
    );
    // El contraste vivo: mismo campo, misma cifra, distinto tipo según las comillas.
    assert_ne!(
        parse("count = 2").unwrap(),
        parse(r#"count = "2""#).unwrap(),
        "`count = 2` (número) y `count = \"2\"` (string) son AST distintos: el parser no coerciona"
    );
}

/// Criterio: consulta malformada (`parseo_malformado_es_error`).
#[test]
fn parseo_malformado_es_error() {
    // El caso rector: un operador sin operando derecho es `Err`, NO un panic ni una query vacía.
    assert!(
        parse("status =").is_err(),
        "`status =` sin valor es un `Err`"
    );
    // Otras formas rotas, también `Err` sin panic: un paréntesis sin cerrar y la consulta vacía
    // (`§Fase 5`: los errores son `Result`, «no queries vacías»).
    assert!(
        parse(r#"(status = "a""#).is_err(),
        "un paréntesis sin cerrar es un `Err`"
    );
    assert!(parse("").is_err(), "la consulta vacía es un `Err`");
}

// =============================================================================
// E19-H04 — Namespaces calculados (`document.*`, `graph.*`)
// =============================================================================
//
// A diferencia de H01 (que dejaba `document.*`/`graph.*` sin resolver), aquí el evaluador SÍ los
// evalúa: `document.*` desde el propio [`EvalDocument`] y `graph.*` desde el [`Analysis`] (que ya
// viaja en la firma `evaluate(expr, doc, analysis)` de H01 — ningún cambio de firma es necesario).
//
// **Decisiones de criterio**:
//   - **Cómo se distingue namespace de frontmatter**: el PRIMER segmento del `FieldPath` decide.
//     `document`/`graph` son reservados; cualquier otro (incluida la abreviatura de `frontmatter.X`)
//     va al frontmatter. `namespace_graph_isolated` fuerza que esto sea correcto: una clave de
//     usuario `isolated` en el frontmatter NO puede colarse como `graph.isolated`.
//   - **Sin romper la regla de tipos de H01**: los valores calculados se comparan como su tipo
//     natural — `document.path` como string, `document.has_frontmatter`/`graph.isolated` como
//     booleanos, `graph.backlinks`/`graph.dangling_links` como números—, así que `graph.backlinks =
//     "0"` (string) seguiría siendo `false` y `graph.backlinks >= "x"` un error, igual que cualquier
//     número de frontmatter. Los tests comparan cada namespace con el `QueryValue` de su tipo.
//   - **`Analysis` de verdad**: se construye un `DocumentSet` con enlaces reales y se usa su
//     `analyze()`, no un `Analysis` fabricado a mano — así el test no miente sobre lo que el grafo
//     calcula (mismo enfoque que `grafo.rs`).

/// Evalúa `expr` sobre el documento `path` de un `DocumentSet` **real**: su frontmatter y su cuerpo
/// salen del `.md`, y el `Analysis` (backlinks/dangling/isolated) lo calcula el grafo de verdad —no
/// un `Analysis` fabricado—, de modo que el test no puede mentir sobre lo que el grafo computa. Es el
/// contraste con el helper `eval` de H01, que usa `Analysis::default()` porque H01 no toca el grafo.
fn eval_en(ds: &DocumentSet, path: &str, expr: &Expression) -> Result<bool, TypeError> {
    let p = rp(path);
    let raw = ds
        .files()
        .get(&p)
        .unwrap_or_else(|| panic!("`{path}` debe estar en el `DocumentSet`"));
    let parsed = model::parse_file(path, raw);
    let doc = EvalDocument {
        path: &p,
        frontmatter: parsed.frontmatter.as_ref(),
        body: &parsed.body,
    };
    evaluate(expr, &doc, ds.analyze())
}

/// Criterio: `document.path starts_with "docs/"` (`namespace_document_path`).
#[test]
fn namespace_document_path() {
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[
        ("docs/guia.md", "# Guía\n\nBajo docs.\n"),
        ("README.md", "# Readme\n\nEn la raíz.\n"),
    ]));
    let expr = cmp("document.path", Op::StartsWith, qstr("docs/"));

    assert_eq!(
        eval_en(&ds, "docs/guia.md", &expr),
        Ok(true),
        "`document.path` resuelve la ruta REAL del documento, no una clave de frontmatter"
    );
    // No vacuo: la ruta de la raíz no empieza por `docs/` — el namespace lee la ruta de verdad.
    assert_eq!(
        eval_en(&ds, "README.md", &expr),
        Ok(false),
        "`README.md` no empieza por `docs/`"
    );
}

/// Criterio: `document.has_frontmatter = false` (`namespace_has_frontmatter`).
#[test]
fn namespace_has_frontmatter() {
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[
        ("con-fm.md", "---\nstatus: accepted\n---\n\n# Con bloque\n"),
        ("sin-fm.md", "# Sin bloque\n\nNi rastro de frontmatter.\n"),
    ]));
    let sin_bloque = cmp("document.has_frontmatter", Op::Eq, qbool(false));

    // Selecciona los documentos SIN bloque.
    assert_eq!(
        eval_en(&ds, "sin-fm.md", &sin_bloque),
        Ok(true),
        "`document.has_frontmatter = false` casa el documento sin bloque"
    );
    // No vacuo: el que tiene bloque no casa.
    assert_eq!(
        eval_en(&ds, "con-fm.md", &sin_bloque),
        Ok(false),
        "…y NO el documento con bloque"
    );
    // El booleano calculado se compara con `QueryValue::Bool` como cualquier booleano de
    // frontmatter, sin romper la regla de tipos de H01.
    assert_eq!(
        eval_en(
            &ds,
            "con-fm.md",
            &cmp("document.has_frontmatter", Op::Eq, qbool(true)),
        ),
        Ok(true),
        "`document.has_frontmatter = true` casa el documento con bloque"
    );
}

/// Criterio: `graph.backlinks = 0` (`namespace_graph_backlinks`).
#[test]
fn namespace_graph_backlinks() {
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[
        ("target.md", "# Target\n\nMe enlazan.\n"),
        ("source.md", "# Source\n\nVer [target](target.md).\n"),
    ]));
    let sin_backlinks = cmp("graph.backlinks", Op::Eq, num(0));

    // `source.md` no recibe enlaces: 0 backlinks.
    assert_eq!(
        eval_en(&ds, "source.md", &sin_backlinks),
        Ok(true),
        "`graph.backlinks = 0` casa el documento no enlazado"
    );
    // No vacuo: `target.md` recibe un enlace desde `source.md`.
    assert_eq!(
        eval_en(&ds, "target.md", &sin_backlinks),
        Ok(false),
        "`target.md` recibe un enlace: no tiene 0 backlinks"
    );
    // El contador es un NÚMERO calculado: el orden funciona como con cualquier número de frontmatter.
    assert_eq!(
        eval_en(&ds, "target.md", &cmp("graph.backlinks", Op::Ge, num(1))),
        Ok(true),
        "`graph.backlinks >= 1` casa el documento enlazado: el namespace de grafo es numérico"
    );
}

/// Criterio: `graph.dangling_links > 0` (`namespace_graph_dangling`).
#[test]
fn namespace_graph_dangling() {
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[
        ("roto.md", "# Roto\n\nVer [lo que falta](no-existe.md).\n"),
        ("sano.md", "# Sano\n\nVer [roto](roto.md).\n"),
    ]));
    let con_rotos = cmp("graph.dangling_links", Op::Gt, num(0));

    // `roto.md` tiene un enlace a un destino inexistente.
    assert_eq!(
        eval_en(&ds, "roto.md", &con_rotos),
        Ok(true),
        "`graph.dangling_links > 0` casa el documento con un enlace roto"
    );
    // No vacuo: `sano.md` enlaza a `roto.md`, que existe → ningún colgante.
    assert_eq!(
        eval_en(&ds, "sano.md", &con_rotos),
        Ok(false),
        "`sano.md` enlaza a un documento que existe: 0 colgantes"
    );
}

/// Criterio: `graph.isolated = true` y la NO interferencia de una clave de frontmatter `isolated`
/// (`namespace_graph_isolated`).
#[test]
fn namespace_graph_isolated() {
    // `aislado.md` no enlaza ni es enlazado → aislado en el grafo. Su frontmatter lleva un DECOY:
    // una clave `isolated: false`, para forzar que `graph.isolated` NO se confunda con ella.
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[
        (
            "aislado.md",
            "---\nisolated: false\n---\n\n# Aislado\n\nNi enlazo ni me enlazan.\n",
        ),
        ("a.md", "# A\n\nVer [b](b.md).\n"),
        ("b.md", "# B\n\nMe enlazan.\n"),
    ]));

    // (1) El namespace de grafo dice la verdad del GRAFO: `aislado.md` está aislado.
    assert_eq!(
        eval_en(
            &ds,
            "aislado.md",
            &cmp("graph.isolated", Op::Eq, qbool(true))
        ),
        Ok(true),
        "`graph.isolated = true` casa el documento aislado en el grafo"
    );
    // (2) No vacuo: un documento conectado no está aislado.
    assert_eq!(
        eval_en(&ds, "a.md", &cmp("graph.isolated", Op::Eq, qbool(true))),
        Ok(false),
        "`a.md` participa en el grafo: no está aislado"
    );
    // (3) La clave de frontmatter `isolated` NO interfiere: el namespace es EXPLÍCITO. Bare
    //     `isolated` lee el frontmatter (`false`); `graph.isolated` lee el grafo (`true`). Dan
    //     respuestas OPUESTAS sobre el MISMO documento.
    assert_eq!(
        eval_en(&ds, "aislado.md", &cmp("isolated", Op::Eq, qbool(false))),
        Ok(true),
        "bare `isolated = false` lee la clave de frontmatter (que vale `false`)"
    );
    assert_eq!(
        eval_en(&ds, "aislado.md", &cmp("isolated", Op::Eq, qbool(true))),
        Ok(false),
        "bare `isolated = true` NO se cuela al grafo: la clave de frontmatter vale `false`"
    );
}

// =============================================================================
// E19-H03 — El filtro JSON y la equivalencia
// =============================================================================
//
// La firma que fija esta fase: `lodestar_core::filter::from_json(&serde_json::Value) ->
// Result<Expression, FilterError>` (módulo NUEVO `filter`). Deserializa el `filter` estructurado de
// `§20.10` al MISMO `Expression` de H01 al que `parse` traduce el `where` textual — que es lo que
// garantiza que ambas formas «producen exactamente el mismo resultado» (`§Fase 5`, «AST unificado»).
//
// **Por qué `from_json(&serde_json::Value)` y no `impl Deserialize for Expression`**: `Expression`
// NO es hoy `Deserialize` (su doc en `types.rs` lo difiere aquí), y la forma del wire NO es un
// reflejo mecánico del AST — necesita LÓGICA de traducción que un `derive` no da:
//   - `field: "frontmatter.status"` debe normalizar a la ruta DESNUDA `["status"]`, la MISMA
//     abreviatura que aplica el parser textual (`parse::build_field_path`); un `FieldPath:
//     Deserialize` genérico no la haría (no debe, para direccionar claves que contienen puntos).
//   - `has`/`missing` mapean a `Expression::Function { arguments: vec![QueryValue::String(campo)] }`
//     — una transformación de forma, no una deserialización directa.
// Los sub-campos SÍ están ya cableados en el contrato de wire de H01 y no hace falta tocarlos: el
// `value` deserializa por el `#[serde(untagged)]` de `QueryValue` (string/número/booleano/null/lista
// desnudos) y el `operator` por los `#[serde(rename = "equals" | "greater_than_or_equal" | …)]` de
// `ComparisonOperator` — esa es la TABLA de nombres largos del wire (fijada en H01), que estos tests
// reutilizan tal cual (`equals`, `not_equals`, `greater_than`, `greater_than_or_equal`, `less_than`,
// `less_than_or_equal`, `contains`, `contains_any`, `contains_all`, `starts_with`, `ends_with`).
//
// **Decisiones de criterio (autor de tests, documentadas y clavadas por los asserts)**:
//   - Envoltura del nodo: `{and:[…]}` / `{or:[…]}` (listas) → `And`/`Or`; `{not: <nodo>}` (un
//     objeto) → `Not`; `{field, operator, value}` → `Comparison`.
//   - Existencia: `{has: {field: "…"}}` / `{missing: {field: "…"}}` → `Function` — se elige la forma
//     objeto `{field: …}` por coherencia con la clave `field` de la comparación (y su campo también
//     aplica la abreviatura de `frontmatter.`).
//   - Malformado (`filtro_malformado_es_error`): operador desconocido, nodo sin forma o un JSON que
//     no es objeto → `Err`, nunca panic (coherente con el `ParseError` del textual).
//
// **Por qué `equivalencia_ast` NO es vacuo**: cada pareja se ancla contra un `Expression` construido
// a mano (`esperado`) y se exige que TANTO `parse(where)` COMO `from_json(filter)` sean iguales a él
// —y entre sí—. Las 8 parejas son no triviales (una con `not`, una con lista, una con `>=` numérico,
// una anidada con precedencia `and`/`or`) y comparan el AST COMPLETO por igualdad estructural: si el
// JSON no normalizara `frontmatter.`, o envolviera `has` distinto, o dejara el número como string,
// el AST diferiría del ancla y el assert mordería (algo que `equivalencia_resultado`, por sí solo,
// podría no notar si dos ASTs distintos seleccionaran lo mismo en el fixture).

use lodestar_core::filter::from_json;
use serde_json::json;

/// Afirma que la consulta textual `donde` (`where`) y el filtro JSON `filtro` producen el **mismo**
/// [`Expression`], y que ese AST es EXACTAMENTE `esperado`. El ancla `esperado` (construido a mano)
/// es lo que impide el test vacuo: si cualquiera de los dos caminos —el parser textual o el
/// deserializador JSON— derivara a otra estructura, el `assert_eq!` mordería.
fn equivalen(donde: &str, filtro: serde_json::Value, esperado: Expression) {
    let del_texto =
        parse(donde).unwrap_or_else(|e| panic!("el `where` `{donde}` debe parsear: {e:?}"));
    let del_json = from_json(&filtro)
        .unwrap_or_else(|e| panic!("el `filter` de `{donde}` debe deserializar: {e:?}"));
    assert_eq!(
        del_texto, esperado,
        "el `where` `{donde}` produce el AST esperado"
    );
    assert_eq!(
        del_json, esperado,
        "el `filter` de `{donde}` produce el MISMO AST que el `where`"
    );
    assert_eq!(
        del_texto, del_json,
        "`where` y `filter` producen el mismo Expression para `{donde}`"
    );
}

/// Selecciona, en orden de `RelPath`, los documentos de `ds` que casan `expr`. Evalúa cada documento
/// con su frontmatter real y el `Analysis` de verdad del grafo (mismo patrón que `eval_en`), de modo
/// que el conjunto seleccionado no puede mentir sobre lo que el evaluador computa.
fn seleccion(ds: &DocumentSet, expr: &Expression) -> Vec<RelPath> {
    ds.files()
        .iter()
        .filter_map(|(p, raw)| {
            let parsed = model::parse_file(p.as_str(), raw);
            let doc = EvalDocument {
                path: p,
                frontmatter: parsed.frontmatter.as_ref(),
                body: &parsed.body,
            };
            match evaluate(expr, &doc, ds.analyze()) {
                Ok(true) => Some(p.clone()),
                _ => None,
            }
        })
        .collect()
}

/// Criterio: un filtro JSON con `and`/comparación/lista deserializa al `Expression` correcto
/// (`filtro_json_deserializa`).
///
/// Cubre de una sola pieza: la envoltura `{and:[…]}`, la normalización de `frontmatter.` a la ruta
/// desnuda, el mapeo del nombre largo de operador (`equals`, `contains_any`) y el `value` JSON
/// desnudo tanto escalar (`"accepted"`) como lista (`["platform","security"]`).
#[test]
fn filtro_json_deserializa() {
    let filtro = json!({
        "and": [
            { "field": "frontmatter.status", "operator": "equals", "value": "accepted" },
            {
                "field": "frontmatter.owners",
                "operator": "contains_any",
                "value": ["platform", "security"]
            }
        ]
    });
    let esperado = and(vec![
        cmp("status", Op::Eq, qstr("accepted")),
        cmp("owners", Op::ContainsAny, qlist(&["platform", "security"])),
    ]);

    assert_eq!(
        from_json(&filtro).unwrap(),
        esperado,
        "el filtro JSON deserializa a `and` de una igualdad de string y un `contains_any` de lista, \
         con la ruta `frontmatter.X` normalizada a `[\"X\"]`"
    );
}

/// Criterio: para 6+ consultas de `§Fase 5`, `where` y `filter` dan el **mismo AST**
/// (`equivalencia_ast`).
///
/// Ocho parejas no triviales que cubren comparación, orden numérico, texto, lista, `has`, `missing`,
/// `not`, `and`, `or` y anidamiento con precedencia. Cada una se ancla contra un `Expression`
/// construido a mano (ver [`equivalen`]) — el AST se compara COMPLETO, no «ambos seleccionan algo».
#[test]
fn equivalencia_ast() {
    // (1) Comparación de igualdad + abreviatura de namespace (`frontmatter.status` → `["status"]`).
    equivalen(
        r#"status = "accepted""#,
        json!({ "field": "frontmatter.status", "operator": "equals", "value": "accepted" }),
        cmp("status", Op::Eq, qstr("accepted")),
    );

    // (2) Orden NUMÉRICO: nombre largo `greater_than_or_equal` → `Ge`, y `value: 2` (número JSON) →
    //     `QueryValue::Number`, NO string (la red anti-coerción llega también al filtro JSON).
    equivalen(
        "priority >= 2",
        json!({ "field": "frontmatter.priority", "operator": "greater_than_or_equal", "value": 2 }),
        cmp("priority", Op::Ge, num(2)),
    );

    // (3) Operador de LISTA `contains_any` con literal lista.
    equivalen(
        r#"owners contains_any ["platform", "security"]"#,
        json!({
            "field": "frontmatter.owners",
            "operator": "contains_any",
            "value": ["platform", "security"]
        }),
        cmp("owners", Op::ContainsAny, qlist(&["platform", "security"])),
    );

    // (4) Existencia `has(...)`: `{has:{field}}` → `Function{Has}`, con `frontmatter.` normalizado.
    equivalen(
        "has(status)",
        json!({ "has": { "field": "frontmatter.status" } }),
        func(FunctionName::Has, "status"),
    );

    // (5) Existencia `missing(...)` con dot-notation preservada en el argumento.
    equivalen(
        "missing(service.tier)",
        json!({ "missing": { "field": "service.tier" } }),
        func(FunctionName::Missing, "service.tier"),
    );

    // (6) `not` de una comparación de texto: `{not: <nodo>}` → `Not`.
    equivalen(
        r#"not tags contains "archived""#,
        json!({
            "not": { "field": "frontmatter.tags", "operator": "contains", "value": "archived" }
        }),
        not(cmp("tags", Op::Contains, qstr("archived"))),
    );

    // (7) `and` de dos comparaciones (el ejemplo canónico de `§20.10`).
    equivalen(
        r#"status = "accepted" and owners contains "platform""#,
        json!({ "and": [
            { "field": "frontmatter.status", "operator": "equals", "value": "accepted" },
            { "field": "frontmatter.owners", "operator": "contains", "value": "platform" }
        ]}),
        and(vec![
            cmp("status", Op::Eq, qstr("accepted")),
            cmp("owners", Op::Contains, qstr("platform")),
        ]),
    );

    // (8) Anidamiento con PRECEDENCIA: `and` de tres ramas con un `or` entre paréntesis y un `not`
    //     (`§Fase 5`, la consulta insignia). El textual aplana `a and b and c` a un `And` de tres;
    //     el JSON `{and:[x,y,z]}` debe producir el MISMO `And` de tres con el `Or` anidado dentro.
    equivalen(
        r#"type = "decision" and (status = "draft" or status = "review") and not tags contains "archived""#,
        json!({ "and": [
            { "field": "frontmatter.type", "operator": "equals", "value": "decision" },
            { "or": [
                { "field": "frontmatter.status", "operator": "equals", "value": "draft" },
                { "field": "frontmatter.status", "operator": "equals", "value": "review" }
            ]},
            { "not": { "field": "frontmatter.tags", "operator": "contains", "value": "archived" } }
        ]}),
        and(vec![
            cmp("type", Op::Eq, qstr("decision")),
            or(vec![
                cmp("status", Op::Eq, qstr("draft")),
                cmp("status", Op::Eq, qstr("review")),
            ]),
            not(cmp("tags", Op::Contains, qstr("archived"))),
        ]),
    );
}

/// Criterio: `where` y `filter` seleccionan el **mismo conjunto de documentos** sobre un workspace
/// real (`equivalencia_resultado`).
///
/// Sobre un `DocumentSet` de verdad, `evaluate(parse(where))` y `evaluate(from_json(filter))` deben
/// coincidir documento a documento. Cada caso exige además un subconjunto ESTRICTO y no vacío (la
/// selección discrimina), para que la igualdad no sea trivialmente cierta.
///
/// **Ubicación** (decisión de criterio): va en el core y no en `crates/lodestar-app/tests/` (donde
/// la sugería el campo *Pruebas* de la historia) porque el cableado a `knowledge_search` —la vía por
/// la que `App` filtraría— es **E19-H05, fuera del alcance de H03**. El core (`DocumentSet` +
/// `evaluate` + `from_json`) basta para probar la equivalencia de resultado sin anticipar ese
/// cableado.
#[test]
fn equivalencia_resultado() {
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[
        (
            "a.md",
            "---\ntype: decision\nstatus: accepted\nowners:\n  - platform\n  - security\npriority: 2\ntags:\n  - core\n---\n\n# A\n",
        ),
        (
            "b.md",
            "---\ntype: decision\nstatus: draft\nowners:\n  - platform\npriority: 1\ntags:\n  - wip\n---\n\n# B\n",
        ),
        (
            "c.md",
            "---\ntype: guide\nstatus: review\nowners:\n  - security\npriority: 3\ntags:\n  - archived\n---\n\n# C\n",
        ),
        (
            "d.md",
            "---\ntype: decision\nstatus: review\nowners:\n  - legal\npriority: 5\ntags:\n  - archived\n---\n\n# D\n",
        ),
    ]));
    let total = ds.files().len();

    // (where textual, filter JSON, conjunto esperado). El esperado es un subconjunto estricto y no
    // vacío en los tres casos.
    let casos: Vec<(&str, serde_json::Value, Vec<&str>)> = vec![
        (
            r#"status = "accepted""#,
            json!({ "field": "frontmatter.status", "operator": "equals", "value": "accepted" }),
            vec!["a.md"],
        ),
        (
            r#"owners contains "platform""#,
            json!({ "field": "frontmatter.owners", "operator": "contains", "value": "platform" }),
            vec!["a.md", "b.md"],
        ),
        (
            r#"type = "decision" and (status = "draft" or status = "review") and not tags contains "archived""#,
            json!({ "and": [
                { "field": "frontmatter.type", "operator": "equals", "value": "decision" },
                { "or": [
                    { "field": "frontmatter.status", "operator": "equals", "value": "draft" },
                    { "field": "frontmatter.status", "operator": "equals", "value": "review" }
                ]},
                { "not": { "field": "frontmatter.tags", "operator": "contains", "value": "archived" } }
            ]}),
            vec!["b.md"],
        ),
    ];

    for (donde, filtro, esperado) in casos {
        let esperado: Vec<RelPath> = esperado.iter().map(|p| rp(p)).collect();
        let sel_texto = seleccion(&ds, &parse(donde).unwrap());
        let sel_json = seleccion(
            &ds,
            &from_json(&filtro)
                .unwrap_or_else(|e| panic!("`filter` de `{donde}` no deserializa: {e:?}")),
        );

        assert_eq!(
            sel_texto, sel_json,
            "`where` y `filter` seleccionan el MISMO conjunto para `{donde}`"
        );
        assert_eq!(
            sel_json, esperado,
            "`{donde}` selecciona exactamente el conjunto esperado"
        );
        assert!(
            !sel_json.is_empty() && sel_json.len() < total,
            "`{donde}` debe seleccionar un subconjunto estricto y no vacío (sel={sel_json:?}, total={total})"
        );
    }
}

/// Guarda (no es criterio formal, sí decisión de criterio del autor): un filtro JSON malformado es
/// `Err`, nunca un panic — coherente con `parseo_malformado_es_error` del `where` textual.
#[test]
fn filtro_malformado_es_error() {
    // Operador desconocido: `like` no está en la tabla de nombres largos.
    assert!(
        from_json(&json!({ "field": "frontmatter.status", "operator": "like", "value": "x" }))
            .is_err(),
        "un operador desconocido es `Err`"
    );
    // Objeto sin forma reconocible (ni `and`/`or`/`not`/`has`/`missing` ni `field`).
    assert!(
        from_json(&json!({})).is_err(),
        "un nodo vacío/sin forma es `Err`"
    );
    // Un JSON que ni siquiera es un objeto de filtro.
    assert!(
        from_json(&json!("status = accepted")).is_err(),
        "un filtro que no es objeto es `Err`"
    );
}
