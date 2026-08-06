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

/// Parsea una expresión del lenguaje textual, o explota con su mensaje (E24-H07/H08).
fn parsea(expr: &str) -> Expression {
    lodestar_core::parse::parse(expr)
        .unwrap_or_else(|e| panic!("`{expr}` debe parsear: {}", e.message))
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
///
/// Su `_ => None` (que trata `Err(TypeError)` como «no casa») es una comodidad de ESTE helper, no el
/// contrato de la superficie: E19-H04/E21-H02 decidieron que un `TypeError` excluyera el documento
/// en silencio y **E26-H08 lo revisa** —en las fachadas, que ahora abortan la consulta con
/// `INVALID_SCHEMA`—. El core no cambia: sigue devolviendo `Result<bool, TypeError>` por documento,
/// que es lo que permite a la fachada elegir. Los fixtures de `equivalencia_resultado` están
/// tipados de forma homogénea, así que aquí no llega ningún `Err`.
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

// ---------------------------------------------------------------------------
// E24-H07/H08 (v0.4.0) — Los namespaces reservados dejan de responder a lo que no entienden
//
// Hasta v0.3.1, `graph.backlink` (con typo) devolvía `[]`: indistinguible de un resultado
// legítimamente vacío. Es peor que un error — una respuesta silenciosamente equivocada. Y el
// frontmatter propio del usuario llamado `graph:`/`document:` era INALCANZABLE, porque
// `build_field_path` descartaba el prefijo `frontmatter.` y el namespace lo capturaba, pese a que
// `metadata_inspect` lo anuncia en su catálogo.
//
// Esto REVISA el criterio de E19-H04 («una sub-clave de namespace desconocida es propiedad
// ausente»), y por eso va en v0.4.0 y no en el parche.
// ---------------------------------------------------------------------------

/// **E24-H07** — una propiedad desconocida bajo namespace reservado es un ERROR de consulta.
#[test]
fn namespace_reservado_rechaza_propiedad_desconocida() {
    for expr in [
        "graph.backlink = 0", // typo: falta la `s`
        "graph.foo = 1",
        "document.pathh = \"a.md\"",
        "document.foo = 1",
        "graph.backlinks.extra = 1", // demasiados segmentos
    ] {
        let e = lodestar_core::parse::parse(expr)
            .expect_err(&format!("`{expr}` debe RECHAZARSE, no devolver []"));
        assert!(
            e.message.contains("no existe"),
            "el error debe decir que la propiedad no existe: {}",
            e.message
        );
        assert!(
            e.message.contains("frontmatter."),
            "y debe indicar la salida (anclar con `frontmatter.`) para quien SÍ tenga una clave \
             con ese nombre en su metadata: {}",
            e.message
        );
    }
}

/// **E24-H07** — control anti-vacuo: las 7 propiedades válidas siguen funcionando.
#[test]
fn namespaces_validos_siguen_parseando() {
    for expr in [
        "document.path = \"a.md\"",
        "document.title = \"A\"",
        "document.has_frontmatter = true",
        "graph.backlinks = 0",
        "graph.outgoing_links > 1",
        "graph.dangling_links = 0",
        "graph.isolated = true",
    ] {
        lodestar_core::parse::parse(expr)
            .unwrap_or_else(|e| panic!("`{expr}` es válida y debe parsear: {}", e.message));
    }
}

/// **E24-H07** — un campo de frontmatter inexistente SIN namespace reservado sigue siendo ausencia.
///
/// El rechazo es solo bajo `document.`/`graph.`: el frontmatter es metadata arbitraria del usuario
/// y preguntar por una clave que no tiene es legítimo, no un error.
#[test]
fn campo_de_frontmatter_inexistente_sigue_siendo_ausencia() {
    lodestar_core::parse::parse("status_inventado = x")
        .expect("un campo de frontmatter que no existe NO es un error de consulta");
    lodestar_core::parse::parse("service.tier.profundo = 1").expect("ni uno anidado que no existe");
}

/// **E24-H07** — el `filter` JSON rechaza igual (comparten `build_field_path`).
#[test]
fn filtro_con_namespace_desconocido_es_error() {
    let f = serde_json::json!({"field": "graph.backlink", "operator": "equals", "value": 0});
    let e = lodestar_core::filter::from_json(&f)
        .expect_err("el filtro JSON debe rechazar lo mismo que el `where`: comparten constructor");
    assert!(
        e.message.contains("no existe"),
        "mismo mensaje por los dos caminos: {}",
        e.message
    );
}

/// **E24-H08** — `frontmatter.` es un ANCLAJE: alcanza una clave llamada como un namespace.
#[test]
fn anclaje_frontmatter_alcanza_clave_reservada() {
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[(
        "b.md",
        "---\ngraph:\n  backlinks: 7\ndocument:\n  path: falso.md\n---\n\n# B\n",
    )]));

    assert_eq!(
        eval_en(&ds, "b.md", &parsea("frontmatter.graph.backlinks = 7")),
        Ok(true),
        "`frontmatter.graph.backlinks` debe alcanzar la clave del USUARIO. Hasta v0.3.1 el prefijo \
         se descartaba y lo capturaba el namespace del grafo, así que este dato —que \
         `metadata_inspect` SÍ anuncia en su catálogo— era inalcanzable por cualquier consulta"
    );
    assert_eq!(
        eval_en(
            &ds,
            "b.md",
            &parsea("frontmatter.document.path = \"falso.md\"")
        ),
        Ok(true),
        "ídem para `document`"
    );
}

/// **E24-H08** — control anti-vacuo: SIN anclaje, el namespace sigue ganando.
///
/// El anclaje añade una vía, no cambia la que ya había: `graph.backlinks` sigue siendo el grafo.
#[test]
fn namespace_sigue_ganando_sin_anclaje() {
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[(
        "b.md",
        "---\ngraph:\n  backlinks: 7\n---\n\n# B\n",
    )]));

    assert_eq!(
        eval_en(&ds, "b.md", &parsea("graph.backlinks = 0")),
        Ok(true),
        "sin anclaje, `graph.backlinks` es el GRAFO (0 backlinks reales), no el 7 del frontmatter"
    );
    assert_eq!(
        eval_en(&ds, "b.md", &parsea("graph.backlinks = 7")),
        Ok(false),
        "y por tanto NO casa el valor del frontmatter"
    );
}

/// **E24-H08** — `has()`/`missing()` respetan los namespaces.
///
/// Antes hacían `FieldPath::parse` y consultaban el frontmatter directamente, así que
/// `has(graph.backlinks)` miraba una clave literalmente llamada `graph.backlinks`.
#[test]
fn has_respeta_los_namespaces() {
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[(
        "a.md",
        "---\nstatus: draft\n---\n\n# A\n",
    )]));

    assert_eq!(
        eval_en(&ds, "a.md", &parsea("has(graph.backlinks)")),
        Ok(true),
        "las propiedades calculadas existen SIEMPRE para todo documento: `has(graph.backlinks)` \
         es trivialmente cierto, no depende de que el frontmatter tenga esa clave"
    );
    assert_eq!(
        eval_en(&ds, "a.md", &parsea("has(document.path)")),
        Ok(true),
        "ídem para `document.path`"
    );
    assert_eq!(
        eval_en(&ds, "a.md", &parsea("has(status)")),
        Ok(true),
        "control anti-vacuo: una clave real del frontmatter sigue detectándose"
    );
    assert_eq!(
        eval_en(&ds, "a.md", &parsea("missing(inventada)")),
        Ok(true),
        "y una que no existe sigue estando ausente"
    );
}

// =============================================================================
// E26-H08 — La PREMISA del determinismo: el orden total y el primer `Err`
// =============================================================================
//
// E26-H08 hace que un `TypeError` de evaluación ABORTE la consulta (`INVALID_SCHEMA`) en vez de
// excluir el documento en silencio, y exige que el documento reportado sea el **primero del orden
// total ya existente** (`Analysis::documents`, ordenado por `RelPath`), no el primero que toque el
// consumidor. Ese cambio vive en la fachada (`lodestar-app`): el core NO cambia — sigue devolviendo
// `Result<bool, TypeError>` por documento, que es justo lo que permite a la fachada elegir.
//
// Este test es la premisa de la que dependen `el_type_error_reportado_es_determinista`
// (`lodestar-mcp/tests/mcp.rs`) y su gemelo de `seleccion.rs`: fija QUÉ documento es «el primero que
// yerra» en el fixture que ambos usan. **Verde por diseño** (el core ya se comporta así); su valor
// es que, si alguien cambiara el orden de `Analysis::documents` o la clasificación de tipos, el
// esperado de aquellos tests dejaría de ser arbitrario y este assert diría por qué.

/// **E26-H08** (premisa) — sobre una base heterogénea, el primer `Err` en el orden total de
/// `Analysis::documents` es `bravo.md`: ni `alfa.md` (primero del orden, pero no yerra) ni `zulu.md`
/// (yerra, pero el último).
#[test]
fn primer_type_error_en_el_orden_total() {
    // Se insertan DESORDENADOS a propósito: el orden lo pone el modelo (`RelPath`), no el fixture.
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[
        ("zulu.md", "---\npriority: 9\n---\n\n# Zulu\n"),
        ("bravo.md", "---\npriority: 2\n---\n\n# Bravo\n"),
        ("alfa.md", "---\npriority: high\n---\n\n# Alfa\n"),
        ("charlie.md", "---\npriority: urgent\n---\n\n# Charlie\n"),
    ]));
    let expr = parsea("priority >= \"high\"");
    let analysis = ds.analyze();

    assert_eq!(
        analysis
            .documents
            .iter()
            .map(RelPath::as_str)
            .collect::<Vec<_>>(),
        vec!["alfa.md", "bravo.md", "charlie.md", "zulu.md"],
        "`Analysis::documents` es el orden TOTAL por `RelPath` (§20.7): es el que hereda el \
         determinismo de E26-H08"
    );

    // Qué hace cada documento con la MISMA consulta (la heterogeneidad es el escenario real).
    assert_eq!(
        eval_en(&ds, "alfa.md", &expr),
        Ok(true),
        "`alfa.md` compara string con string: casa, y no yerra"
    );
    assert!(
        matches!(
            eval_en(&ds, "bravo.md", &expr),
            Err(TypeError::OrderNotDefined {
                field_type: ValueType::Number,
                value_type: ValueType::String,
                ..
            })
        ),
        "`bravo.md` tiene `priority` numérico: el orden cruzado es ERROR, con los dos tipos"
    );
    assert_eq!(
        eval_en(&ds, "charlie.md", &expr),
        Ok(true),
        "`charlie.md` también es string (`urgent` >= `high` lexicográfico)"
    );
    assert!(
        matches!(eval_en(&ds, "zulu.md", &expr), Err(TypeError::OrderNotDefined { .. })),
        "`zulu.md` yerra igual que `bravo.md`: hay MÁS de un candidato, y por eso «cuál se reporta» \
         tiene que estar decidido"
    );

    // La premisa: recorrido en el orden total, el primer `Err` es `bravo.md`.
    let primero = analysis
        .documents
        .iter()
        .find(|p| eval_en(&ds, p.as_str(), &expr).is_err());
    assert_eq!(
        primero.map(RelPath::as_str),
        Some("bravo.md"),
        "el primer documento del orden total que yerra es `bravo.md` — el esperado de \
         `el_type_error_reportado_es_determinista`"
    );

    // Y el `TypeError` YA lleva la información que el mensaje de la fachada necesita (campo,
    // operador y los dos tipos): E26-H08 sube un dato existente, no inventa un diagnóstico.
    let Err(TypeError::OrderNotDefined {
        field,
        operator,
        field_type,
        value_type,
    }) = eval_en(&ds, "bravo.md", &expr)
    else {
        panic!("`bravo.md` debe dar `OrderNotDefined`");
    };
    assert_eq!(field, fp("priority"), "el campo viaja en el error");
    assert_eq!(operator, Op::Ge, "el operador también");
    assert_eq!(field_type, ValueType::Number, "y el tipo del campo");
    assert_eq!(value_type, ValueType::String, "y el del literal");
}

// =============================================================================
// E29-H03 — `has(frontmatter)` / `missing(frontmatter)` responden la VERDAD
// =============================================================================
//
// `requirements/epica-29-honestidad-superficie.md §E29-H03` · `decisiones §19(a)` ·
// `ARCHITECTURE.md §20.8` (L1247-1249, la promesa literal: «existencia `has(x)` `missing(x)`
// (incluido `has(frontmatter)`)») · `CLAUDE.md` invariante #3.
//
// SÍNTOMA verificado hoy (v0.5.0) sobre un `DocumentSet` de 3 documentos —2 con bloque de
// frontmatter, 1 sin él—:
//
//   has(frontmatter)      -> Ok(false) para LOS TRES   (debería casar los 2 con bloque)
//   missing(frontmatter)  -> Ok(true)  para LOS TRES   (debería casar solo el 1 sin bloque)
//
// Es decir: la función devuelve el valor CONTRARIO al correcto para todo documento, sin error.
//
// CAUSA (verificada en código): `eval::resolver_campo` (`eval.rs:144`) detecta el anclaje por el
// primer segmento y llama a `field.sin_anclaje()?`; `FieldPath::sin_anclaje` (`types.rs:507`)
// devuelve `None` para el anclaje PELADO —`frontmatter` a secas—, porque `FieldPath::from_segments`
// sobre cero segmentos es inválido por construcción. El `?` propaga ese `None` y
// `propiedad_presente` lo lee como «propiedad ausente».
//
// DÓNDE se arregla (lo fija la historia, y estos tests NO lo relajan): en el camino de existencia,
// reconociendo el anclaje pelado ANTES de intentar desanclarlo. **No** ampliando `sin_anclaje` para
// que rinda un path vacío: un `FieldPath` sin segmentos es inválido por construcción y ese
// invariante sostiene el dialecto de dot-paths de E26-H09.
//
// LA VERDAD A LA QUE SE ATAN estos tests es `document.has_frontmatter` (invariante #3: no se computa
// dos veces). Por eso el criterio central no compara contra una lista escrita a mano, sino contra el
// conjunto que ya devuelve el camino largo — así la aserción sobrevive a un cambio futuro en la
// definición de «tiene frontmatter».
//
// ROJO esperado HOY: por ASERCIÓN en los cuatro primeros (ninguna API nueva, ningún stub); los dos
// últimos son controles anti-vacuo y están VERDES desde ya, y deben seguir verdes después.
// ---------------------------------------------------------------------------

/// El workspace de los criterios: 2 documentos **con** bloque de frontmatter y 1 **sin** él.
///
/// `con-claves.md` y `con-otra-clave.md` no comparten ninguna clave (`status` vs `owner`): así el
/// veredicto de `has(frontmatter)` no puede confundirse con el de `has(<una clave concreta>)`, que
/// es la única vía por la que hoy `has` responde algo cierto.
fn ds_con_y_sin_frontmatter() -> DocumentSet {
    DocumentSet::from_files(lodestar_fixtures::file_map(&[
        ("con-claves.md", "---\nstatus: accepted\n---\n\n# Con\n"),
        ("con-otra-clave.md", "---\nowner: ana\n---\n\n# Otra\n"),
        ("sin-bloque.md", "# Sin\n\nNi rastro de frontmatter.\n"),
    ]))
}

/// Los documentos de `ds` que casan `expr`, en el orden total de `Analysis::documents` (`§20.7`).
/// Un `Err` de tipo explota: ninguno de estos criterios comparara tipos, así que un `TypeError` aquí
/// sería una regresión y no puede pasar como «no casó».
fn casan(ds: &DocumentSet, expr: &Expression) -> Vec<String> {
    ds.analyze()
        .documents
        .iter()
        .filter(|p| {
            eval_en(ds, p.as_str(), expr).unwrap_or_else(|e| {
                panic!("`{p}` no debe dar TypeError en un `has`/`missing`: {e:?}")
            })
        })
        .map(|p| p.as_str().to_string())
        .collect()
}

/// E29-H03 · Criterio `has_frontmatter_pelado_casa_los_documentos_con_frontmatter`:
/// Dado un workspace con 3 documentos, 2 con bloque de frontmatter y 1 sin él, Cuando se consulta
/// `has(frontmatter)`, Entonces devuelve exactamente los 2 con frontmatter.
#[test]
fn has_frontmatter_pelado_casa_los_documentos_con_frontmatter() {
    let ds = ds_con_y_sin_frontmatter();

    assert_eq!(
        casan(&ds, &parsea("has(frontmatter)")),
        vec!["con-claves.md", "con-otra-clave.md"],
        "`has(frontmatter)` debe casar los documentos que TIENEN bloque de frontmatter. Hoy no casa \
         ninguno (0 de 3): `resolver_campo` desancla el path y `sin_anclaje` devuelve `None` para el \
         anclaje pelado, así que `propiedad_presente` lo lee como ausencia para TODO documento — la \
         respuesta contraria a la correcta, sin ningún error que lo delate"
    );
}

/// E29-H03 · Criterio `missing_frontmatter_pelado_casa_los_que_no_tienen`:
/// Dado ese mismo workspace, Cuando se consulta `missing(frontmatter)`, Entonces devuelve
/// exactamente el 1 sin frontmatter.
#[test]
fn missing_frontmatter_pelado_casa_los_que_no_tienen() {
    let ds = ds_con_y_sin_frontmatter();

    assert_eq!(
        casan(&ds, &parsea("missing(frontmatter)")),
        vec!["sin-bloque.md"],
        "`missing(frontmatter)` debe casar solo el documento SIN bloque. Hoy casa los 3, incluidos \
         los que tienen frontmatter con claves: es la negación de un `has` que siempre es `false`"
    );
}

/// E29-H03 · Criterio `has_frontmatter_coincide_con_document_has_frontmatter`:
/// Dado ese mismo workspace, Cuando se comparan los resultados de `has(frontmatter)` y de
/// `document.has_frontmatter = true`, Entonces son **el mismo conjunto**.
///
/// Es el criterio de invariante #3 —una sola verdad computada— y el que hace la aserción robusta a
/// cambios futuros en la definición de «tiene frontmatter»: no fija una lista, fija una IGUALDAD
/// entre el camino corto y el largo. `missing(frontmatter)` se ata simétricamente a `= false`, para
/// que el arreglo no pueda dejar los dos operadores desacoplados.
#[test]
fn has_frontmatter_coincide_con_document_has_frontmatter() {
    let ds = ds_con_y_sin_frontmatter();

    let camino_largo = casan(&ds, &parsea("document.has_frontmatter = true"));
    // Guarda de no vacuidad: el camino largo YA responde bien hoy, y responde algo distinto de
    // «todos» y de «ninguno». Sin esta guarda, un `has` roto podría igualar a un largo también roto.
    assert_eq!(
        camino_largo,
        vec!["con-claves.md", "con-otra-clave.md"],
        "premisa: `document.has_frontmatter = true` ya distingue hoy los 2 con bloque de los 3 \
         documentos — es la verdad a la que este criterio ata el camino corto"
    );

    assert_eq!(
        casan(&ds, &parsea("has(frontmatter)")),
        camino_largo,
        "`has(frontmatter)` y `document.has_frontmatter = true` deben ser EL MISMO conjunto \
         (invariante #3: la presencia del bloque no se computa dos veces con dos respuestas)"
    );
    assert_eq!(
        casan(&ds, &parsea("missing(frontmatter)")),
        casan(&ds, &parsea("document.has_frontmatter = false")),
        "…y `missing(frontmatter)` debe coincidir con `document.has_frontmatter = false`: los dos \
         operadores se atan a la MISMA verdad, no cada uno a la suya"
    );
}

/// E29-H03 · Criterio `has_frontmatter_vacio_coincide_con_el_camino_largo`:
/// Dado un documento con un frontmatter **vacío** (`---\n---`), Cuando se consulta
/// `has(frontmatter)`, Entonces el veredicto coincide con el de `document.has_frontmatter` sobre ese
/// mismo documento, sea cual sea.
///
/// Caso límite: el bloque existe pero no tiene claves. La historia **no** inventa una respuesta
/// propia — se ata a la única verdad existente. Hoy el modelo dice que un bloque vacío SÍ es
/// frontmatter (`parse_file` rinde `Some(ParsedFrontmatter)` vacío), y el test lo comprueba en vez
/// de asumirlo, para que si esa definición cambiase el criterio siga diciendo lo correcto.
#[test]
fn has_frontmatter_vacio_coincide_con_el_camino_largo() {
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[
        ("vacio.md", "---\n---\n\n# Bloque vacío\n"),
        ("con-claves.md", "---\nstatus: accepted\n---\n\n# Con\n"),
        ("sin-bloque.md", "# Sin\n\nNi rastro de frontmatter.\n"),
    ]));

    let largo = eval_en(&ds, "vacio.md", &parsea("document.has_frontmatter = true"));
    let corto = eval_en(&ds, "vacio.md", &parsea("has(frontmatter)"));
    assert_eq!(
        corto, largo,
        "sobre un bloque VACÍO (`---\\n---`), `has(frontmatter)` debe dar exactamente lo que da \
         `document.has_frontmatter`: la historia no inventa una tercera respuesta para el borde. \
         Camino largo = {largo:?}, camino corto = {corto:?}"
    );
    assert_eq!(
        eval_en(&ds, "vacio.md", &parsea("missing(frontmatter)")),
        largo.map(|v| !v),
        "y `missing(frontmatter)` es su negación exacta sobre el mismo documento"
    );

    // Guarda de no vacuidad: los otros dos documentos siguen dando respuestas OPUESTAS entre sí, así
    // que la igualdad de arriba no se cumple por un `has` degenerado que responda lo mismo a todo.
    assert_eq!(
        casan(&ds, &parsea("has(frontmatter)")),
        casan(&ds, &parsea("document.has_frontmatter = true")),
        "…y sobre el conjunto entero, corto y largo siguen coincidiendo con el bloque vacío dentro"
    );
}

/// E29-H03 · Criterio `has_con_anclaje_y_sufijo_no_cambia` (control anti-vacuo):
/// Dado una consulta `has(frontmatter.status)` (anclaje **con** sufijo), Cuando se evalúa, Entonces
/// responde igual que antes del arreglo.
///
/// El arreglo toca el camino del anclaje, que es exactamente el que resuelve `frontmatter.<clave>`:
/// si reconocer el anclaje pelado se llevara por delante el anclaje con sufijo, este test lo vería.
/// Incluye el caso reservado (`frontmatter.graph.backlinks`, E24-H08), que es el que justifica que
/// el anclaje exista.
#[test]
fn has_con_anclaje_y_sufijo_no_cambia() {
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[
        (
            "a.md",
            "---\nstatus: draft\ngraph:\n  backlinks: 7\n---\n\n# A\n",
        ),
        ("b.md", "# B\n\nSin frontmatter.\n"),
    ]));

    assert_eq!(
        eval_en(&ds, "a.md", &parsea("has(frontmatter.status)")),
        Ok(true),
        "`has(frontmatter.status)` con la clave presente sigue siendo `true`"
    );
    assert_eq!(
        eval_en(&ds, "a.md", &parsea("has(frontmatter.inventada)")),
        Ok(false),
        "…y con una clave que no existe sigue siendo `false`: el anclaje CON sufijo direcciona la \
         clave, no el bloque"
    );
    assert_eq!(
        eval_en(&ds, "b.md", &parsea("has(frontmatter.status)")),
        Ok(false),
        "…y sobre un documento sin bloque tampoco casa"
    );
    assert_eq!(
        eval_en(&ds, "a.md", &parsea("missing(frontmatter.inventada)")),
        Ok(true),
        "`missing` con anclaje y sufijo sigue siendo la negación de `has`"
    );
    // E24-H08: la clave del usuario llamada como un namespace reservado sigue alcanzándose anclada.
    assert_eq!(
        eval_en(&ds, "a.md", &parsea("has(frontmatter.graph.backlinks)")),
        Ok(true),
        "el anclaje sobre un namespace reservado (E24-H08) sigue alcanzando la clave del USUARIO: \
         reconocer el anclaje pelado no puede cortocircuitar el camino que lo justifica"
    );
}

/// E29-H03 · Criterio `has_de_clave_y_de_namespace_no_cambia` (control anti-vacuo):
/// Dado `has(status)` y `has(graph.backlinks)`, Cuando se evalúan, Entonces responden igual que
/// antes del arreglo.
///
/// El resto del operador no se toca: una clave de frontmatter sigue juzgándose por presencia
/// (`ParsedFrontmatter::get`, con `null`/`""`/`[]` como PRESENTES) y un namespace calculado sigue
/// existiendo trivialmente para todo documento —incluido uno **sin** frontmatter—, que es lo que la
/// historia declara fuera de alcance.
#[test]
fn has_de_clave_y_de_namespace_no_cambia() {
    let ds = DocumentSet::from_files(lodestar_fixtures::file_map(&[
        (
            "a.md",
            "---\nstatus: draft\nvacia: \"\"\nnula: null\n---\n\n# A\n",
        ),
        ("b.md", "# B\n\nSin frontmatter.\n"),
    ]));

    assert_eq!(
        eval_en(&ds, "a.md", &parsea("has(status)")),
        Ok(true),
        "una clave real del frontmatter sigue detectándose"
    );
    assert_eq!(
        eval_en(&ds, "a.md", &parsea("has(inventada)")),
        Ok(false),
        "…y una que no existe sigue ausente"
    );
    assert_eq!(
        eval_en(&ds, "a.md", &parsea("has(vacia)")),
        Ok(true),
        "la cadena vacía sigue contando como valor PRESENTE"
    );
    assert_eq!(
        eval_en(&ds, "a.md", &parsea("has(nula)")),
        Ok(true),
        "…y una clave a `null` también"
    );
    assert_eq!(
        eval_en(&ds, "b.md", &parsea("has(status)")),
        Ok(false),
        "sobre un documento sin bloque, preguntar por una clave sigue siendo ausencia"
    );
    assert_eq!(
        eval_en(&ds, "a.md", &parsea("has(graph.backlinks)")),
        Ok(true),
        "los namespaces calculados siguen existiendo SIEMPRE (fuera de alcance de esta historia)"
    );
    assert_eq!(
        eval_en(&ds, "b.md", &parsea("has(graph.backlinks)")),
        Ok(true),
        "…incluido sobre un documento SIN frontmatter: `has(graph.*)` no habla del bloque"
    );
    assert_eq!(
        eval_en(&ds, "b.md", &parsea("has(document.path)")),
        Ok(true),
        "…y `document.path` igual"
    );
    assert_eq!(
        eval_en(&ds, "a.md", &parsea("missing(inventada)")),
        Ok(true),
        "`missing` de una clave inexistente sigue siendo `true`"
    );
}

/// E29-H03 · El `filter` JSON responde lo MISMO que el `where` textual (`§20.10`).
///
/// Los dos caminos comparten `parse::build_field_path`, así que el anclaje pelado llega igual por
/// los dos; esta equivalencia es la que garantiza que arreglar `where` no deje `filter` mintiendo
/// (el defecto de `§19(a)` es del evaluador, pero la superficie tiene dos puertas).
#[test]
fn has_frontmatter_pelado_es_igual_por_where_y_por_filter() {
    let ds = ds_con_y_sin_frontmatter();

    let por_filter = lodestar_core::filter::from_json(&serde_json::json!({
        "has": { "field": "frontmatter" }
    }))
    .expect("`{\"has\":{\"field\":\"frontmatter\"}}` es un filtro JSON válido (§20.10)");

    assert_eq!(
        casan(&ds, &por_filter),
        vec!["con-claves.md", "con-otra-clave.md"],
        "el `filter` JSON debe casar los mismos 2 documentos con bloque que el `where` textual"
    );
    assert_eq!(
        casan(&ds, &por_filter),
        casan(&ds, &parsea("has(frontmatter)")),
        "`where` y `filter` producen el mismo `Expression` y por tanto el mismo conjunto (§20.10): \
         una sola verdad por las dos puertas del wire"
    );
}

// =============================================================================
// E29-H04 — `starts_with`/`ends_with` sobre un campo no-string es TYPE ERROR
// =============================================================================
//
// `requirements/epica-29-honestidad-superficie.md §E29-H04` · `decisiones §23/A-04` (criterio
// **ratificado por el usuario el 2026-08-06: type error ruidoso**) · caso G1-20 del testbench
// homelab (`docs/qa/informe-homelab-2026-08-06.md §3`) · `ARCHITECTURE.md §20.8` («sin coerción
// implícita»: el lenguaje es tipado) · E26-H08 (el precedente: el type error del orden aborta en vez
// de saltarse el documento).
//
// SÍNTOMA medido hoy (v0.5.0), sobre un documento con `priority: 3` (número), `tags: [uno]` (lista),
// `mapa: {k: v}` y `flag: true`:
//
//   priority starts_with "3"   -> Ok(false)      (debería ser Err de tipo)
//   priority ends_with "3"     -> Ok(false)      (ídem)
//   tags starts_with "x"       -> Ok(false)      (ídem)
//   mapa starts_with "x"       -> Ok(false)      (ídem)
//   flag starts_with "t"       -> Ok(false)      (ídem)
//   status starts_with 3       -> Ok(false)      (literal no-string; el parser HOY lo acepta)
//
// En el homelab eso es el caso G1-20: 7 documentos con `priority: 3` y `priority starts_with "3"`
// devolviendo 0 resultados sin un solo aviso — una lista recortada indistinguible de una lista
// legítimamente vacía, que es exactamente lo que E26-H08 declaró cerrado para el ORDEN.
//
// CAUSA (reconocida en el propio código): `eval::eval_afijo` (`eval.rs:362-378`) devuelve `bool`, no
// `Result`, y su doc-comment declara el hueco: «Con un campo no-string o un literal no-string no hay
// prefijo/sufijo que comprobar → `false` (no hay una variante de `TypeError` «no es string», y H01 no
// la introduce; ningún test lo ejercita)». Este bloque es el «ningún test lo ejercita».
//
// LA FORMA QUE ESTOS TESTS EXIGEN (y la historia fija): una TERCERA variante de `TypeError` en
// `core::types`, `NotAString { field, operator, found }`, con la MISMA forma que `NotAList` —los dos
// son «el campo no es del tipo que el operador exige»— para que el wire de los type errors sea
// uniforme. `eval_afijo` pasa a `Result<bool, TypeError>` y `eval_comparison` propaga.
//
// LO QUE NO CAMBIA (controles anti-vacuo, hoy VERDES y que deben seguir verdes):
//   · un campo AUSENTE sigue siendo `Ok(false)`: la ausencia cortocircuita en `eval_comparison`
//     (`eval.rs:108-110`) antes de mirar tipos — mismo contrato que rige para `NotAList`;
//   · `starts_with`/`ends_with` sobre un STRING siguen casando prefijo/sufijo como siempre;
//   · `=`/`!=` con cruce de tipos siguen siendo `false` (no error) y `contains` sobre string sigue
//     siendo subcadena: la historia NO amplía el rechazo a los demás operadores.
//
// ROJO esperado HOY: por ASERCIÓN. `NotAString` no existe todavía, así que los tests **no fijan la
// variante por patrón** en el criterio principal —eso no compilaría y borraría la evidencia del
// rojo—: aseveran que el resultado es `Err(..)` y no `Ok(false)`, que es el cambio de comportamiento
// que la historia decide, y `starts_with_produce_la_variante_de_tipo_de_string` (abajo, ignorado
// hasta el verde) fija la FORMA exacta en un test propio para que el implementador no pueda elegir
// otra sin tocarlo.
// ---------------------------------------------------------------------------

/// El workspace del caso G1-20, con el **mismo nombre de campo** tomando tipos distintos en
/// documentos distintos (el escenario real del homelab) y un campo por cada familia no-string que el
/// operador de afijo puede encontrarse: número, lista, mapa y booleano.
///
/// `g1-20-numerico.md` va PRIMERO en el orden total (`§20.7`) a propósito y `z-textual.md` último:
/// así el conjunto que hoy devuelve `priority starts_with "3"` (vacío) y el que devolvería una
/// implementación que solo mirase el primer documento se distinguen del correcto.
fn ds_afijo_heterogeneo() -> DocumentSet {
    DocumentSet::from_files(lodestar_fixtures::file_map(&[
        (
            "g1-20-numerico.md",
            "---\npriority: 3\nstatus: active\n---\n\n# El caso G1-20\n",
        ),
        (
            "lista.md",
            "---\ntags:\n  - uno\n  - dos\nstatus: draft\n---\n\n# Lista\n",
        ),
        (
            "mapa.md",
            "---\nservice:\n  name: api\nstatus: draft\n---\n\n# Mapa\n",
        ),
        (
            "booleano.md",
            "---\nreviewed: true\nstatus: draft\n---\n\n# Booleano\n",
        ),
        (
            "z-textual.md",
            "---\npriority: \"3-alta\"\nstatus: activo\n---\n\n# Textual\n",
        ),
    ]))
}

/// Juzga que `expr` sobre `path` es un **error de tipo** y no la respuesta silenciosa de hoy.
///
/// No fija la variante por patrón a propósito: `TypeError::NotAString` **no existe todavía**, y
/// nombrarla aquí impediría compilar el fichero entero —el rojo dejaría de ser una aserción y pasaría
/// a ser un fallo de build que taparía los controles anti-vacuo—. La forma exacta la fija
/// `starts_with_produce_la_variante_de_tipo_de_string`.
fn es_type_error(ds: &DocumentSet, path: &str, expr: &str, porque: &str) {
    let r = eval_en(ds, path, &parsea(expr));
    assert!(
        r.is_err(),
        "`{expr}` sobre `{path}` debe ser un TypeError: {porque}. Hoy devuelve {r:?} — la respuesta \
         silenciosamente equivocada que `decisiones §23/A-04` cerró (caso G1-20) y que E26-H08 ya \
         declaró inaceptable para el operador de orden"
    );
    assert_ne!(
        r,
        Ok(false),
        "…y en particular NO puede seguir siendo `Ok(false)`: es lo que hace la lista recortada \
         indistinguible de la vacía (`{expr}` sobre `{path}`)"
    );
}

/// E29-H04 · Criterio `starts_with_sobre_numero_es_type_error`:
/// Dado un workspace con documentos cuyo `priority` es un **número**, Cuando se busca con
/// `priority starts_with "3"`, Entonces es un error de tipo que nombra campo, operador y tipo
/// encontrado.
///
/// Es el caso **G1-20 literal**: en el homelab 7 documentos con `priority: 3` desaparecían de esta
/// consulta sin error. El fixture reproduce la heterogeneidad real (`z-textual.md` tiene `priority`
/// string y **sí** casa), de modo que hoy la respuesta no es «vacía» sino **recortada**: es el
/// veneno concreto que la historia quita.
#[test]
fn starts_with_sobre_numero_es_type_error() {
    let ds = ds_afijo_heterogeneo();

    es_type_error(
        &ds,
        "g1-20-numerico.md",
        "priority starts_with \"3\"",
        "`priority: 3` es un número y `starts_with` es un operador de TEXTO; el lenguaje no coerce \
         (§20.8), así que no hay prefijo que comprobar",
    );

    // La premisa que hace el defecto peligroso: el documento textual SÍ casa, así que hoy la
    // consulta devuelve una lista NO vacía y perfectamente creíble, a la que le faltan documentos.
    assert_eq!(
        eval_en(&ds, "z-textual.md", &parsea("priority starts_with \"3\"")),
        Ok(true),
        "premisa: sobre `priority: \"3-alta\"` (string) la MISMA consulta casa. Por eso el defecto \
         produce una lista recortada y no una lista vacía: el agente no tiene forma de sospechar"
    );
}

/// E29-H04 · Criterio `ends_with_sobre_numero_es_type_error`:
/// Dado ese mismo workspace, Cuando se busca con `ends_with` sobre el mismo campo, Entonces el mismo
/// error.
///
/// Los dos operadores de afijo comparten `eval_afijo`: si uno se arreglara sin el otro, el hueco
/// seguiría abierto por la mitad.
#[test]
fn ends_with_sobre_numero_es_type_error() {
    let ds = ds_afijo_heterogeneo();

    es_type_error(
        &ds,
        "g1-20-numerico.md",
        "priority ends_with \"3\"",
        "`ends_with` es el gemelo de `starts_with` y comparte evaluador: el número tampoco tiene \
         sufijo",
    );
    assert_eq!(
        eval_en(&ds, "z-textual.md", &parsea("priority ends_with \"alta\"")),
        Ok(true),
        "premisa simétrica: sobre el documento textual `ends_with` sigue casando el sufijo"
    );
}

/// E29-H04 · Criterio `starts_with_sobre_lista_es_type_error`:
/// Dado un documento cuyo campo `tags` es una **lista**, Cuando se evalúa `tags starts_with "x"`,
/// Entonces es type error (no `false`).
///
/// Se cubren de paso las otras dos familias no-string que un frontmatter real produce —**mapa** y
/// **booleano**—, porque un arreglo que solo mirase `Value::Number` dejaría tres cuartas partes del
/// hueco abiertas y ningún otro test lo vería.
#[test]
fn starts_with_sobre_lista_es_type_error() {
    let ds = ds_afijo_heterogeneo();

    es_type_error(
        &ds,
        "lista.md",
        "tags starts_with \"uno\"",
        "una lista no tiene prefijo de texto; que su PRIMER elemento sí lo tenga es justo la \
         coerción que §20.8 prohíbe",
    );
    es_type_error(
        &ds,
        "mapa.md",
        "service starts_with \"api\"",
        "un mapa tampoco: `service` es un objeto, y su clave `name` es otro campo con su propio \
         field path",
    );
    es_type_error(
        &ds,
        "booleano.md",
        "reviewed starts_with \"t\"",
        "un booleano tampoco: `true` no es el texto «true» (misma regla que hace `reviewed = \
         \"true\"` un `false` desde E19-H01)",
    );
    es_type_error(
        &ds,
        "lista.md",
        "tags ends_with \"dos\"",
        "y el gemelo `ends_with` yerra igual sobre la lista",
    );
}

/// E29-H04 · El afijo sobre el **anclaje pelado** (`frontmatter starts_with "x"`) también yerra.
///
/// Cierra el cabo suelto del efecto colateral de E29-H03 que esta historia declara decidido (ver la
/// sección de abajo): el anclaje pelado se resuelve al **booleano** de presencia, y un booleano no es
/// string, así que cae por la misma regla que `reviewed starts_with "t"` — sin caso especial ni
/// excepción. Hoy es `Ok(false)`, el último silencio que queda en esa esquina.
#[test]
fn starts_with_sobre_el_anclaje_pelado_es_type_error() {
    let ds = ds_con_y_sin_frontmatter();

    es_type_error(
        &ds,
        "con-claves.md",
        "frontmatter starts_with \"x\"",
        "el anclaje pelado vale el booleano de presencia (E29-H03) y un booleano no tiene prefijo \
         de texto: misma regla que cualquier otro campo no-string, sin excepción",
    );
    // Y sobre un documento SIN bloque sigue siendo ausencia: `false` sin error (frontera intacta).
    assert_eq!(
        eval_en(
            &ds,
            "sin-bloque.md",
            &parsea("frontmatter starts_with \"x\"")
        ),
        Ok(false),
        "sin bloque no hay campo que tipar: la ausencia cortocircuita antes, como en \
         `starts_with_sobre_campo_ausente_sigue_siendo_false`"
    );
}

/// E29-H04 · Criterio `starts_with_sobre_campo_ausente_sigue_siendo_false` (control anti-vacuo y
/// frontera semántica):
/// Dado un workspace donde **ningún** documento tiene el campo `inexistente`, Cuando se evalúa
/// `inexistente starts_with "x"`, Entonces devuelve 0 resultados **sin error**.
///
/// La ausencia se cortocircuita en `eval_comparison` **antes** de tipar (`eval.rs:108-110`) y ese
/// contrato no cambia: es el mismo que ya rige para `NotAList` («un campo inexistente no llega
/// aquí»). Sin este control, el arreglo podría convertir en error toda consulta sobre un frontmatter
/// heterogéneo —que es la norma, no la excepción— y el remedio sería peor que la enfermedad.
///
/// **Verde hoy**, y debe seguir verde.
#[test]
fn starts_with_sobre_campo_ausente_sigue_siendo_false() {
    let ds = ds_afijo_heterogeneo();

    for path in [
        "g1-20-numerico.md",
        "lista.md",
        "mapa.md",
        "booleano.md",
        "z-textual.md",
    ] {
        assert_eq!(
            eval_en(&ds, path, &parsea("inexistente starts_with \"x\"")),
            Ok(false),
            "sobre `{path}`, un campo que NO existe excluye el documento sin error: no se puede \
             errar sobre un tipo que no se tiene (E19-H01, `campo_inexistente`)"
        );
        assert_eq!(
            eval_en(&ds, path, &parsea("inexistente ends_with \"x\"")),
            Ok(false),
            "…y lo mismo con `ends_with` sobre `{path}`"
        );
    }
    // Y el caso mixto, que es el realista: el campo existe en unos documentos y no en otros. El
    // documento SIN `tags` no puede errar; el que lo tiene como lista, sí.
    assert_eq!(
        eval_en(&ds, "g1-20-numerico.md", &parsea("tags starts_with \"u\"")),
        Ok(false),
        "`g1-20-numerico.md` no tiene `tags`: ausencia, no error"
    );
}

/// E29-H04 · Criterio `starts_with_sobre_string_sigue_funcionando` (control anti-vacuo):
/// Dado un workspace con campos string, Cuando se evalúa `status starts_with "act"`, Entonces casa
/// como siempre.
///
/// El arreglo no puede consistir en hacer ilegal el operador: sobre su tipo natural sigue casando el
/// prefijo/sufijo, con `true` **y** con `false` (un `false` legítimo de «no es prefijo» tiene que
/// seguir siendo `Ok(false)`, no un error, o el operador dejaría de ser usable).
///
/// **Verde hoy**, y debe seguir verde.
#[test]
fn starts_with_sobre_string_sigue_funcionando() {
    let ds = ds_afijo_heterogeneo();

    assert_eq!(
        eval_en(
            &ds,
            "g1-20-numerico.md",
            &parsea("status starts_with \"act\"")
        ),
        Ok(true),
        "`status: active` empieza por «act»: el operador sigue haciendo su trabajo sobre un string"
    );
    assert_eq!(
        eval_en(&ds, "lista.md", &parsea("status starts_with \"act\"")),
        Ok(false),
        "`status: draft` NO empieza por «act»: un `false` legítimo sigue siendo `false`, no un error \
         — si el arreglo convirtiera esto en `Err` el operador sería inservible"
    );
    assert_eq!(
        eval_en(
            &ds,
            "g1-20-numerico.md",
            &parsea("status ends_with \"ive\"")
        ),
        Ok(true),
        "…y `ends_with` casa el sufijo igual que siempre"
    );
    assert_eq!(
        eval_en(&ds, "lista.md", &parsea("status ends_with \"ive\"")),
        Ok(false),
        "…con su `false` legítimo correspondiente"
    );
    // `document.path` es un namespace calculado de tipo string (E19-H04): el arreglo tampoco puede
    // romper el operador sobre los valores SINTETIZADOS, que entran por la misma maquinaria.
    assert_eq!(
        eval_en(
            &ds,
            "g1-20-numerico.md",
            &parsea("document.path starts_with \"g1-\"")
        ),
        Ok(true),
        "`document.path` es un string sintetizado y sigue admitiendo prefijo (`namespace_document_path`)"
    );
}

/// E29-H04 · Un **literal** no-string también es type error (`status starts_with 3`).
///
/// La historia lo declara en su alcance: «si el parser ya lo rechaza antes, se declara y se fija con
/// un test, no se duplica la validación». **Medido hoy**: el parser lo ACEPTA
/// (`parse("status starts_with 3")` es `Ok`) y el evaluador devuelve `Ok(false)` — el mismo silencio
/// del campo, en el otro operando. El test acepta las dos salidas legítimas (rechazo en el parser o
/// error en el evaluador) porque la historia deja la elección abierta; lo que NO acepta es que siga
/// siendo `Ok(false)`.
#[test]
fn starts_with_con_literal_no_string_es_type_error() {
    let ds = ds_afijo_heterogeneo();

    for expr in [
        "status starts_with 3",
        "status ends_with 3",
        "status starts_with true",
        "status starts_with null",
    ] {
        // Salida A: el parser lo rechaza antes de llegar al evaluador.
        let Ok(ast) = lodestar_core::parse::parse(expr) else {
            continue;
        };
        // Salida B: el evaluador lo trata como error de tipo.
        let r = eval_en(&ds, "g1-20-numerico.md", &ast);
        assert!(
            r.is_err(),
            "`{expr}` compara un campo string con un literal que no es texto: sin coerción (§20.8) \
             no hay prefijo que comprobar, y callar es el mismo defecto que el del campo. Hoy el \
             parser lo acepta y el evaluador devuelve {r:?}. Vale rechazarlo en el parser o errar \
             aquí, pero no responder `false`"
        );
    }
}

/// E29-H04 · El `filter` JSON yerra igual que el `where` textual (`§20.10`).
///
/// Las dos puertas del wire producen el **mismo** `Expression` (`filter::from_json` reutiliza
/// `parse::build_field_path` y la tabla de nombres de wire de `ComparisonOperator`), así que el
/// arreglo del evaluador las cubre a las dos por construcción — pero solo si el veredicto se juzga
/// por las dos, porque `knowledge_search` acepta `filter` igual que `where` y un agente que use la
/// forma estructurada vería hoy el mismo silencio.
#[test]
fn starts_with_es_type_error_por_where_y_por_filter() {
    let ds = ds_afijo_heterogeneo();

    let por_filter = lodestar_core::filter::from_json(&serde_json::json!({
        "field": "priority", "operator": "starts_with", "value": "3"
    }))
    .expect("`starts_with` es la grafía de wire de `ComparisonOperator::StartsWith` (§20.10)");

    assert_eq!(
        por_filter,
        parsea("priority starts_with \"3\""),
        "premisa: `filter` y `where` producen EL MISMO `Expression`, que es lo que hace la \
         equivalencia exacta y no aproximada"
    );

    let r = eval_en(&ds, "g1-20-numerico.md", &por_filter);
    assert!(
        r.is_err(),
        "el `filter` JSON debe dar el mismo type error que el `where` textual sobre el mismo \
         documento: una sola verdad por las dos puertas del wire (§20.10, invariante #3). Hoy \
         devuelve {r:?}"
    );
    assert_eq!(
        r,
        eval_en(
            &ds,
            "g1-20-numerico.md",
            &parsea("priority starts_with \"3\"")
        ),
        "…y exactamente el mismo `Result`, error incluido: si divergieran, el agente aprendería a \
         corregir con una forma y se quedaría a ciegas con la otra"
    );
}

/// E29-H04 · La **forma** del error nuevo: variante propia, con campo, operador y tipo encontrado.
///
/// Fija el contrato de datos de la historia —`TypeError::NotAString { field, operator, found }`, con
/// la misma forma que `NotAList` para que el wire de los type errors sea uniforme— y por eso está
/// separado de los criterios de comportamiento, que solo exigen que el resultado sea `Err`.
///
/// Lo que este test impide, y ningún otro vería: que el afijo se disfrace de uno de los dos
/// diagnósticos que ya existían. `OrderNotDefined` diría «el orden no está definido» y `NotAList`
/// «esto no es una lista», y ninguna de las dos cosas es lo que le pasa a `priority starts_with
/// "3"`: le pasa que no es un string. El `let else` sobre `NotAString` excluye ambas por
/// construcción, así que no hace falta —ni debe añadirse— una aserción aparte que las descarte.
///
/// La otra mitad, igual de necesaria: los tres campos tienen que **llegar poblados**. Sin ellos,
/// `error_de_tipo` (`lodestar-app`) no tendría con qué redactar el mensaje que los criterios del
/// wire exigen (campo, operador y tipo hallado), y el type error nacería mudo — el defecto de vuelta
/// con otro nombre.
#[test]
fn starts_with_produce_la_variante_de_tipo_de_string() {
    let ds = ds_afijo_heterogeneo();
    let expr = parsea("priority starts_with \"3\"");
    let r = eval_en(&ds, "g1-20-numerico.md", &expr);

    let Err(TypeError::NotAString {
        field,
        operator,
        found,
    }) = r
    else {
        panic!(
            "`priority starts_with \"3\"` debe dar `NotAString`: el type error de un operador de \
             AFIJO necesita variante PROPIA (§E29-H04, con la forma de `NotAList`), no reusar \
             `OrderNotDefined` ni `NotAList`, que dirían al agente algo falso. Recibido: {r:?}"
        );
    };
    assert_eq!(
        field,
        fp("priority"),
        "el campo viaja en el error, como en las otras dos variantes"
    );
    assert_eq!(
        operator,
        Op::StartsWith,
        "y el operador, que distingue `starts_with` de `ends_with`"
    );
    assert_eq!(
        found,
        ValueType::Number,
        "y el tipo REAL del campo, que es lo que el agente necesita \
         para corregir la consulta (`metadata_inspect` habla ese mismo vocabulario)"
    );

    let Err(TypeError::NotAString {
        operator, found, ..
    }) = eval_en(&ds, "lista.md", &parsea("tags ends_with \"dos\""))
    else {
        panic!("la lista también da `NotAString`");
    };
    assert_eq!(
        operator,
        Op::EndsWith,
        "el operador distingue los dos afijos"
    );
    assert_eq!(
        found,
        ValueType::List,
        "y el tipo hallado es el real de cada campo"
    );
}

// =============================================================================
// E29-H04 (efecto colateral de E29-H03) — el anclaje PELADO en una COMPARACIÓN
// =============================================================================
//
// E29-H03 hizo que `resolver_campo` reconociera el anclaje pelado (`frontmatter` a secas) y lo
// resolviera a `Some(Bool(true))` para los documentos con bloque. `resolver_campo` tiene DOS
// llamadores: `propiedad_presente` —el de `has`/`missing`, que era el objetivo de aquella historia— y
// `eval_comparison` (`eval.rs:108`), que no lo era. El efecto sobre el segundo quedó sin decidir ni
// testear, y el juez ciego de E29-H03 lo levantó; se decide AQUÍ, en E29-H04, por ser la historia que
// fija el criterio de tipos de los operadores.
//
// VEREDICTO (medido en el árbol y declarado decidido): el comportamiento actual es el correcto según
// el principio rector de la épica —type error ruidoso antes que respuesta silenciosa—, así que se
// FIJA tal cual:
//
//   frontmatter = true         -> Ok(true)  para los documentos CON bloque (y `false` para los que no)
//   frontmatter > 1            -> Err(OrderNotDefined { field_type: Bool, .. })
//   frontmatter contains "x"   -> Err(NotAList { found: Bool })
//
// Es decir: el anclaje pelado se comporta en una comparación como el **booleano de presencia** que
// es —el mismo dato que `document.has_frontmatter`, invariante #3—, y cualquier operador que no esté
// definido sobre un booleano yerra en voz alta en vez de devolver `false`. Coherente, además, con lo
// que esta misma historia fija para los afijos: tras el arreglo, `frontmatter starts_with "x"` pasará
// a ser también type error (hoy es `Ok(false)`), por la misma regla y sin caso especial.
//
// Estos tests nacen VERDES: no piden ningún cambio, declaran un veredicto para que deje de estar
// implícito y para que nadie lo cambie sin darse cuenta.
// ---------------------------------------------------------------------------

/// E29-H04 · El anclaje pelado en una comparación de **igualdad** es el booleano de presencia.
///
/// `frontmatter = true` casa exactamente los documentos con bloque —el mismo conjunto que
/// `has(frontmatter)` y que `document.has_frontmatter = true`—, y `frontmatter = false` es su
/// complemento. No es una capacidad que la épica añada: es la consecuencia, ahora declarada, de que
/// E29-H03 resolviera el anclaje pelado a un valor tipado en vez de a una ausencia.
#[test]
fn frontmatter_pelado_en_comparacion_es_el_booleano_de_presencia() {
    let ds = ds_con_y_sin_frontmatter();

    assert_eq!(
        casan(&ds, &parsea("frontmatter = true")),
        vec!["con-claves.md", "con-otra-clave.md"],
        "`frontmatter = true` casa los documentos CON bloque: el anclaje pelado se resuelve al mismo \
         booleano de presencia que alimenta `has(frontmatter)` (E29-H03)"
    );
    assert_eq!(
        casan(&ds, &parsea("frontmatter = true")),
        casan(&ds, &parsea("has(frontmatter)")),
        "…y por tanto al MISMO conjunto que la función de existencia: una sola verdad (invariante #3)"
    );
    assert_eq!(
        casan(&ds, &parsea("frontmatter = true")),
        casan(&ds, &parsea("document.has_frontmatter = true")),
        "…y que el camino largo, que es la verdad a la que E29-H03 ató el corto"
    );
    assert_eq!(
        casan(&ds, &parsea("frontmatter = false")),
        Vec::<String>::new(),
        "`frontmatter = false` no casa a NADIE: un documento sin bloque no resuelve el anclaje (es \
         ausencia), y la ausencia en una comparación es `false` — no el booleano `false`. Es la \
         asimetría con `missing(frontmatter)`, que sí casa `sin-bloque.md`"
    );
    assert_eq!(
        casan(&ds, &parsea("frontmatter != true")),
        Vec::<String>::new(),
        "…y `!=` tampoco, por la misma razón: la ausencia cortocircuita antes de comparar \
         (`campo_inexistente`, E19-H01)"
    );
}

/// E29-H04 · Un operador **no definido sobre un booleano** aplicado al anclaje pelado es type error.
///
/// Fija el efecto colateral de E29-H03 sobre `eval_comparison` en su mitad ruidosa: `frontmatter > 1`
/// y `frontmatter contains "x"` no responden `false`, yerran — con el `Bool` de la presencia como
/// tipo hallado, que es lo que le dice al agente qué es realmente el anclaje pelado.
///
/// Verde por diseño (el motor ya se comporta así desde E29-H03); su valor es que el veredicto quede
/// declarado y que un cambio futuro que lo devolviera al silencio tenga que romper este test.
#[test]
fn frontmatter_pelado_con_operador_no_definido_es_type_error() {
    let ds = ds_con_y_sin_frontmatter();

    let r_orden = eval_en(&ds, "con-claves.md", &parsea("frontmatter > 1"));
    assert!(
        matches!(
            r_orden,
            Err(TypeError::OrderNotDefined {
                field_type: ValueType::Bool,
                value_type: ValueType::Number,
                ..
            })
        ),
        "`frontmatter > 1` debe ser `OrderNotDefined{{bool, number}}`: el anclaje pelado vale el \
         BOOLEANO de presencia, y el orden no está definido sobre booleanos (E19-H01). Recibido: \
         {r_orden:?}"
    );

    let r_lista = eval_en(&ds, "con-claves.md", &parsea("frontmatter contains \"x\""));
    assert!(
        matches!(
            r_lista,
            Err(TypeError::NotAList {
                found: ValueType::Bool,
                ..
            })
        ),
        "`frontmatter contains \"x\"` debe ser `NotAList{{bool}}`: un operador de lista sobre el \
         booleano de presencia. Recibido: {r_lista:?}"
    );

    // Sobre un documento SIN bloque el anclaje no resuelve: es ausencia, y la ausencia cortocircuita
    // antes de tipar. Los mismos operadores son `Ok(false)` ahí — no una contradicción, sino el
    // contrato de E19-H01 aplicado a un campo que ese documento no tiene.
    assert_eq!(
        eval_en(&ds, "sin-bloque.md", &parsea("frontmatter > 1")),
        Ok(false),
        "sobre un documento sin bloque, el anclaje pelado es AUSENCIA: `false` sin error, como \
         cualquier campo que no está"
    );
    assert_eq!(
        eval_en(&ds, "sin-bloque.md", &parsea("frontmatter contains \"x\"")),
        Ok(false),
        "…y lo mismo con el operador de lista"
    );
}
