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
use lodestar_core::types::ComparisonOperator as Op;
use lodestar_core::types::{
    Analysis, Expression, FieldPath, FunctionName, ParsedFrontmatter, QueryValue, RelPath,
    TypeError, ValueType,
};

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
