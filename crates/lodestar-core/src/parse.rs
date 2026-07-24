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
//! **Descenso recursivo escrito a mano** (sin `nom`/`pest`: el core es puro y minimalista, y una
//! gramática de este tamaño no justifica una dependencia). El flujo es tokenizar → parsear con dos
//! niveles de precedencia (`or` < `and` < `not`) sobre el flujo de tokens.

use crate::types::{ComparisonOperator, Expression, FieldPath, FunctionName, QueryValue};

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

impl ParseError {
    /// Construye un [`ParseError`] con el mensaje dado.
    fn new(message: impl Into<String>) -> ParseError {
        ParseError {
            message: message.into(),
        }
    }
}

/// Traduce la consulta textual `where` de `§20.8` al [`Expression`] unificado de E19-H01.
///
/// - Literales por **forma**: entrecomillado = string; sin comillas, número/booleano/`null` según su
///   escritura (`2` → número, `true` → booleano, `"2"` → string). Una palabra desnuda que no sea
///   número/booleano/`null` es un string (como el escalar plano de YAML), de modo que `status =
///   draft` equivale a `status = "draft"`.
/// - Dot-notation (`service.tier`) → [`crate::types::FieldPath`] de varios segmentos.
/// - **Abreviatura de namespace**: `status = "x"` produce el mismo AST que `frontmatter.status =
///   "x"` (el prefijo `frontmatter.` se normaliza a la ruta desnuda). Los namespaces calculados
///   (`document.*`, `graph.*`) se conservan como primer segmento del `FieldPath` (su evaluación es
///   E19-H04).
/// - Operadores: `= != > >= < <=`, `contains`/`starts_with`/`ends_with`, `contains_any`/
///   `contains_all` (estos dos con un literal lista `["a", "b"]`), y `has(x)`/`missing(x)`.
/// - `and`/`or`/`not`, paréntesis y **precedencia** `not` > `and` > `or`.
/// - Una consulta malformada es `Err(ParseError)`, **nunca** un panic ni una query vacía.
pub fn parse(input: &str) -> Result<Expression, ParseError> {
    let tokens = tokenize(input)?;
    let mut parser = Parser {
        tokens: &tokens,
        pos: 0,
    };
    let expr = parser.parse_or()?;
    if let Some(sobrante) = parser.peek() {
        return Err(ParseError::new(format!(
            "tokens sobrantes tras la expresión: {sobrante:?}"
        )));
    }
    Ok(expr)
}

// --- Tokenizador -------------------------------------------------------------

/// Un token de la consulta textual. Los operadores simbólicos (`=`, `>=`, …) se resuelven ya a su
/// [`ComparisonOperator`]; las palabras (`Word`) las clasifica el parser según su posición
/// (campo/función/keyword lógica/operador de texto/valor desnudo).
#[derive(Debug, Clone, PartialEq)]
enum Token {
    LParen,
    RParen,
    LBracket,
    RBracket,
    Comma,
    /// Un operador simbólico (`=`, `!=`, `>`, `>=`, `<`, `<=`).
    Op(ComparisonOperator),
    /// Un literal string entrecomillado (ya sin comillas ni escapes).
    Str(String),
    /// Una palabra desnuda: campo, keyword lógica, operador de texto o literal sin comillas.
    Word(String),
}

/// `true` si `c` puede formar parte de una palabra desnuda (campo, keyword o literal sin comillas):
/// letras/dígitos Unicode, `_`, `.` (dot-notation) y `-` (fechas ISO, números negativos, claves con
/// guion).
fn is_word_char(c: char) -> bool {
    c.is_alphanumeric() || c == '_' || c == '.' || c == '-'
}

/// Parte `input` en [`Token`]s. Un carácter inesperado o un string sin cerrar es `Err`.
fn tokenize(input: &str) -> Result<Vec<Token>, ParseError> {
    let chars: Vec<char> = input.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        match c {
            c if c.is_whitespace() => i += 1,
            '(' => {
                tokens.push(Token::LParen);
                i += 1;
            }
            ')' => {
                tokens.push(Token::RParen);
                i += 1;
            }
            '[' => {
                tokens.push(Token::LBracket);
                i += 1;
            }
            ']' => {
                tokens.push(Token::RBracket);
                i += 1;
            }
            ',' => {
                tokens.push(Token::Comma);
                i += 1;
            }
            '=' => {
                tokens.push(Token::Op(ComparisonOperator::Eq));
                i += 1;
            }
            '!' => {
                if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token::Op(ComparisonOperator::Ne));
                    i += 2;
                } else {
                    return Err(ParseError::new("`!` suelto: ¿querías `!=`?"));
                }
            }
            '>' => {
                if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token::Op(ComparisonOperator::Ge));
                    i += 2;
                } else {
                    tokens.push(Token::Op(ComparisonOperator::Gt));
                    i += 1;
                }
            }
            '<' => {
                if chars.get(i + 1) == Some(&'=') {
                    tokens.push(Token::Op(ComparisonOperator::Le));
                    i += 2;
                } else {
                    tokens.push(Token::Op(ComparisonOperator::Lt));
                    i += 1;
                }
            }
            '"' => {
                i += 1;
                let mut s = String::new();
                loop {
                    match chars.get(i) {
                        None => return Err(ParseError::new("string sin cerrar")),
                        Some('"') => {
                            i += 1;
                            break;
                        }
                        // Escape mínimo: `\x` inserta `x` literal (cubre `\"` y `\\`).
                        Some('\\') => match chars.get(i + 1) {
                            Some(&esc) => {
                                s.push(esc);
                                i += 2;
                            }
                            None => return Err(ParseError::new("escape `\\` sin carácter")),
                        },
                        Some(&ch) => {
                            s.push(ch);
                            i += 1;
                        }
                    }
                }
                tokens.push(Token::Str(s));
            }
            c if is_word_char(c) => {
                let inicio = i;
                while i < chars.len() && is_word_char(chars[i]) {
                    i += 1;
                }
                tokens.push(Token::Word(chars[inicio..i].iter().collect()));
            }
            otro => {
                return Err(ParseError::new(format!("carácter inesperado: {otro:?}")));
            }
        }
    }
    Ok(tokens)
}

// --- Parser de descenso recursivo -------------------------------------------

struct Parser<'t> {
    tokens: &'t [Token],
    pos: usize,
}

impl<'t> Parser<'t> {
    /// El token actual sin consumirlo.
    fn peek(&self) -> Option<&'t Token> {
        self.tokens.get(self.pos)
    }

    /// Consume y devuelve el token actual (o `None` en fin de entrada).
    fn advance(&mut self) -> Option<&'t Token> {
        let token = self.tokens.get(self.pos);
        if token.is_some() {
            self.pos += 1;
        }
        token
    }

    /// `true` si el token actual es la palabra `kw` (una keyword lógica: `and`/`or`/`not`).
    fn peek_word(&self, kw: &str) -> bool {
        matches!(self.peek(), Some(Token::Word(w)) if w == kw)
    }

    /// `or_expr := and_expr ("or" and_expr)*` — el `or` es el conector que menos liga.
    fn parse_or(&mut self) -> Result<Expression, ParseError> {
        let mut ramas = vec![self.parse_and()?];
        while self.peek_word("or") {
            self.advance();
            ramas.push(self.parse_and()?);
        }
        Ok(colapsar(ramas, Expression::Or))
    }

    /// `and_expr := not_expr ("and" not_expr)*` — el `and` liga más que el `or`.
    fn parse_and(&mut self) -> Result<Expression, ParseError> {
        let mut ramas = vec![self.parse_not()?];
        while self.peek_word("and") {
            self.advance();
            ramas.push(self.parse_not()?);
        }
        Ok(colapsar(ramas, Expression::And))
    }

    /// `not_expr := "not" not_expr | primary` — el `not` es el que más liga (prefijo).
    fn parse_not(&mut self) -> Result<Expression, ParseError> {
        if self.peek_word("not") {
            self.advance();
            Ok(Expression::Not(Box::new(self.parse_not()?)))
        } else {
            self.parse_primary()
        }
    }

    /// `primary := "(" or_expr ")" | atom` — el paréntesis reagrupa contra la precedencia natural y
    /// no añade ningún nodo (devuelve exactamente su contenido).
    fn parse_primary(&mut self) -> Result<Expression, ParseError> {
        if matches!(self.peek(), Some(Token::LParen)) {
            self.advance();
            let interna = self.parse_or()?;
            match self.advance() {
                Some(Token::RParen) => Ok(interna),
                _ => Err(ParseError::new("paréntesis sin cerrar")),
            }
        } else {
            self.parse_atom()
        }
    }

    /// `atom := función | comparación`. Una palabra `has`/`missing` seguida de `(` es una función;
    /// cualquier otra palabra abre una comparación `campo operador valor`.
    fn parse_atom(&mut self) -> Result<Expression, ParseError> {
        let Some(Token::Word(palabra)) = self.peek() else {
            return Err(ParseError::new(format!(
                "se esperaba un campo o una función, se encontró {:?}",
                self.peek()
            )));
        };
        let palabra = palabra.clone();

        if (palabra == "has" || palabra == "missing")
            && matches!(self.tokens.get(self.pos + 1), Some(Token::LParen))
        {
            return self.parse_function(&palabra);
        }

        self.advance(); // el campo
        let field = build_field_path(&palabra)?;
        let operator = self.parse_operator()?;
        let value = self.parse_value()?;
        Ok(Expression::Comparison {
            field,
            operator,
            value,
        })
    }

    /// `has(x)` / `missing(x)`: un único argumento que nombra la propiedad. Se normaliza como un
    /// [`FieldPath`] (misma abreviatura de `frontmatter.` que una comparación) y se guarda como
    /// [`QueryValue::String`], la forma que impone el AST de `§20.8` para el argumento.
    fn parse_function(&mut self, nombre: &str) -> Result<Expression, ParseError> {
        self.advance(); // el nombre
        self.advance(); // `(` (ya verificado por `parse_atom`)
        let arg = match self.advance() {
            Some(Token::Word(w)) => w.clone(),
            otro => {
                return Err(ParseError::new(format!(
                    "`{nombre}(...)` espera un nombre de propiedad, se encontró {otro:?}"
                )));
            }
        };
        match self.advance() {
            Some(Token::RParen) => {}
            otro => {
                return Err(ParseError::new(format!(
                    "falta `)` en `{nombre}(...)`, se encontró {otro:?}"
                )));
            }
        }
        let field = build_field_path(&arg)?;
        let name = match nombre {
            "has" => FunctionName::Has,
            "missing" => FunctionName::Missing,
            _ => unreachable!("parse_function solo se invoca con has/missing"),
        };
        Ok(Expression::Function {
            name,
            arguments: vec![QueryValue::String(field.to_string())],
        })
    }

    /// El operador de una comparación: un símbolo (`=`, `>=`, …) o una palabra de texto/lista
    /// (`contains`, `starts_with`, `contains_any`, …).
    fn parse_operator(&mut self) -> Result<ComparisonOperator, ParseError> {
        match self.advance() {
            Some(Token::Op(op)) => Ok(*op),
            Some(Token::Word(w)) => word_to_operator(w)
                .ok_or_else(|| ParseError::new(format!("operador desconocido: `{w}`"))),
            otro => Err(ParseError::new(format!(
                "se esperaba un operador, se encontró {otro:?}"
            ))),
        }
    }

    /// El operando derecho: un string, un escalar desnudo (número/booleano/`null`/string) o un
    /// literal lista `["a", "b"]` (para `contains_any`/`contains_all`).
    fn parse_value(&mut self) -> Result<QueryValue, ParseError> {
        match self.advance() {
            Some(Token::Str(s)) => Ok(QueryValue::String(s.clone())),
            Some(Token::Word(w)) => Ok(word_to_value(w)),
            Some(Token::LBracket) => self.parse_list(),
            otro => Err(ParseError::new(format!(
                "se esperaba un valor, se encontró {otro:?}"
            ))),
        }
    }

    /// Un literal lista `["a", "b", …]` — el `[` ya fue consumido. Admite la lista vacía y elementos
    /// escalares separados por comas.
    fn parse_list(&mut self) -> Result<QueryValue, ParseError> {
        let mut items = Vec::new();
        if matches!(self.peek(), Some(Token::RBracket)) {
            self.advance();
            return Ok(QueryValue::List(items));
        }
        loop {
            items.push(self.parse_scalar()?);
            match self.advance() {
                Some(Token::Comma) => continue,
                Some(Token::RBracket) => break,
                otro => {
                    return Err(ParseError::new(format!(
                        "se esperaba `,` o `]` en la lista, se encontró {otro:?}"
                    )));
                }
            }
        }
        Ok(QueryValue::List(items))
    }

    /// Un elemento escalar de una lista (string o palabra desnuda; nunca otra lista).
    fn parse_scalar(&mut self) -> Result<QueryValue, ParseError> {
        match self.advance() {
            Some(Token::Str(s)) => Ok(QueryValue::String(s.clone())),
            Some(Token::Word(w)) => Ok(word_to_value(w)),
            otro => Err(ParseError::new(format!(
                "se esperaba un elemento de lista, se encontró {otro:?}"
            ))),
        }
    }
}

/// Colapsa las ramas de un conector: una sola rama se devuelve **desnuda** (sin `And`/`Or` de un
/// elemento), de modo que una comparación suelta o un grupo entre paréntesis no ganan envoltura.
fn colapsar(mut ramas: Vec<Expression>, envolver: fn(Vec<Expression>) -> Expression) -> Expression {
    if ramas.len() == 1 {
        ramas.pop().expect("len == 1")
    } else {
        envolver(ramas)
    }
}

/// Mapea una palabra de operador de texto/lista a su [`ComparisonOperator`]. `None` si la palabra no
/// es un operador (entonces la comparación está malformada).
fn word_to_operator(w: &str) -> Option<ComparisonOperator> {
    Some(match w {
        "contains" => ComparisonOperator::Contains,
        "contains_any" => ComparisonOperator::ContainsAny,
        "contains_all" => ComparisonOperator::ContainsAll,
        "starts_with" => ComparisonOperator::StartsWith,
        "ends_with" => ComparisonOperator::EndsWith,
        _ => return None,
    })
}

/// Tipa un literal **desnudo** (sin comillas) por su forma: `true`/`false` → booleano, `null` →
/// nulo, una cifra → número (entero si cabe en `i64`, real si no), y cualquier otra palabra → string
/// (el escalar plano de YAML). El tipo nace de la **forma sintáctica**, sin coerción: `"2"` (string,
/// vía [`Token::Str`]) y `2` (número, aquí) son literales distintos.
fn word_to_value(w: &str) -> QueryValue {
    match w {
        "true" => QueryValue::Bool(true),
        "false" => QueryValue::Bool(false),
        "null" => QueryValue::Null,
        _ => {
            if let Ok(n) = w.parse::<i64>() {
                QueryValue::Number(n.into())
            } else if let Ok(f) = w.parse::<f64>() {
                QueryValue::Number(f.into())
            } else {
                QueryValue::String(w.to_string())
            }
        }
    }
}

/// Construye el [`FieldPath`] de una palabra con dot-notation, aplicando la **abreviatura de
/// namespace**: un prefijo `frontmatter.` se descarta (`frontmatter.status` → `["status"]`), de modo
/// que la ruta de frontmatter queda **desnuda** —la forma que el evaluador de H01 resuelve directo
/// con `ParsedFrontmatter::get`—; `document.*`/`graph.*` conservan su primer segmento (son
/// namespaces calculados, E19-H04). Un segmento vacío (`service.`, `a..b`) es `Err`.
///
/// `pub(crate)` porque el filtro JSON de E19-H03 ([`crate::filter::from_json`]) la **reutiliza** para
/// normalizar su campo `field` de forma idéntica al textual — esa identidad es lo que garantiza que
/// `where` y `filter` produzcan exactamente el mismo [`Expression`] (`§20.10`); reimplementarla
/// abriría la puerta a que las dos superficies divergieran.
pub(crate) fn build_field_path(word: &str) -> Result<FieldPath, ParseError> {
    let partes: Vec<&str> = word.split('.').collect();
    let partes: &[&str] = if partes.len() > 1 && partes[0] == "frontmatter" {
        &partes[1..]
    } else {
        partes.as_slice()
    };
    FieldPath::from_segments(partes.iter().copied())
        .map_err(|e| ParseError::new(format!("campo inválido `{word}`: {e:?}")))
}
