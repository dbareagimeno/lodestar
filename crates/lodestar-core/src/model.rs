//! Primitivas de modelo: parseo y serialización del documento (`ARCHITECTURE.md §4`, `§20.4`).
//!
//! El frontmatter es **metadata arbitraria del usuario** (`§20.4`, E16-H01): se conserva íntegro,
//! con su tipo YAML real y su texto original. El resto del módulo sigue siendo el port de
//! `resolveLink`, `normalize`, `outLinks`, `rawRelLinks`, `isISO`, quirks incluidos.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_yaml::Value as Yaml;

use crate::types::{FmError, ParsedFrontmatter};

/// `[texto](href "title")` — el grupo 1 es el href. Global.
pub(crate) static LINK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#).unwrap());

/// Resultado de [`split_front`]: dónde está (si está) el bloque de frontmatter de un documento.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SplitFront {
    /// El documento no abre bloque de frontmatter: el cuerpo es el documento entero. Es un estado
    /// **válido** (`§20.4`), no un error.
    Sin,
    /// Bloque presente y cerrado. `span` es el rango de bytes de su TEXTO YAML (sin los
    /// delimitadores `---`); `body_start` es el offset donde empieza el cuerpo.
    Bloque {
        span: std::ops::Range<usize>,
        body_start: usize,
    },
    /// El documento abre `---` y nunca lo cierra: el cuerpo es el documento entero.
    SinCerrar,
}

impl SplitFront {
    /// El cuerpo del documento `raw` según este corte.
    pub fn body<'a>(&self, raw: &'a str) -> &'a str {
        match self {
            SplitFront::Bloque { body_start, .. } => &raw[*body_start..],
            _ => raw,
        }
    }

    /// El texto YAML del bloque (sin delimitadores), o `None` si no hay bloque cerrado.
    pub fn fm_text<'a>(&self, raw: &'a str) -> Option<&'a str> {
        match self {
            SplitFront::Bloque { span, .. } => Some(&raw[span.clone()]),
            _ => None,
        }
    }
}

/// Separa el bloque de frontmatter del cuerpo, **por bytes** (para poder devolver el `span` que
/// necesitan el patch quirúrgico y los rangos de diagnóstico, `§20.4`/`§20.9`).
///
/// El bloque abre con `---` en la primera línea y cierra con la primera línea posterior que
/// empieza por `---`. Un bloque **vacío** (`---\n---\n`) es un bloque presente con texto vacío —
/// no un bloque sin cerrar, que era el veredicto del port del prototipo.
pub fn split_front(raw: &str) -> SplitFront {
    if !raw.starts_with("---") {
        return SplitFront::Sin;
    }
    // Tras el `---` de apertura debe venir un salto de línea; si no, no hay bloque bien formado.
    let after_open = if raw[3..].starts_with("\r\n") {
        5
    } else if raw[3..].starts_with('\n') {
        4
    } else {
        return SplitFront::SinCerrar;
    };

    // El cierre puede venir inmediatamente (bloque vacío) o tras una o más líneas de contenido.
    let (span, close_start) = if raw[after_open..].starts_with("---") {
        (after_open..after_open, after_open)
    } else {
        let Some(nl) = raw[after_open..]
            .match_indices('\n')
            .map(|(i, _)| after_open + i)
            .find(|i| raw[i + 1..].starts_with("---"))
        else {
            return SplitFront::SinCerrar;
        };
        // El `\r` de un CRLF pertenece al delimitador, no al texto del bloque.
        let end = if raw[..nl].ends_with('\r') {
            nl - 1
        } else {
            nl
        };
        (after_open..end, nl + 1)
    };

    // Tras el `---` de cierre se consume el salto de línea (CRLF o LF) si lo hay.
    let mut body_start = close_start + 3;
    if raw[body_start..].starts_with('\r') {
        body_start += 1;
    }
    if raw[body_start..].starts_with('\n') {
        body_start += 1;
    }
    SplitFront::Bloque { span, body_start }
}

/// Parsea el texto de un bloque de frontmatter. `Ok` es **siempre** un `Value::Mapping`: un bloque
/// vacío (o cuyo YAML no es un mapa) produce el mapa vacío; `Err(msg)` solo si el YAML es
/// sintácticamente inválido.
///
/// **No** convierte tipos ni descarta claves: `type: 2` es el número 2 y `status: true` el
/// booleano `true` (E16-H01 retiró la coerción `String(v)` heredada del prototipo).
pub fn parse_yaml(text: &str) -> Result<Yaml, String> {
    if text.trim().is_empty() {
        return Ok(Yaml::Mapping(serde_yaml::Mapping::new()));
    }
    match serde_yaml::from_str::<Yaml>(text) {
        Ok(v @ Yaml::Mapping(_)) => Ok(v),
        // Un YAML válido que no es un mapa no describe propiedades: frontmatter vacío.
        Ok(_) => Ok(Yaml::Mapping(serde_yaml::Mapping::new())),
        Err(e) => Err(e.to_string()),
    }
}

/// Nombre de fichero (último segmento). Port de `basename`.
pub fn basename(p: &str) -> &str {
    p.rsplit('/').next().unwrap_or(p)
}

/// Directorio contenedor con la barra final, o `""` para el root. Port de `dirOf`.
pub fn dir_of(p: &str) -> String {
    match p.rfind('/') {
        Some(i) => p[..=i].to_string(),
        None => String::new(),
    }
}

/// Título **presentable** de un documento (`ARCHITECTURE.md §20.4`,
/// `REFACTOR_PHASE_2 §Fase 4`). La cadena es:
///
/// ```text
/// frontmatter.title  →  primer heading H1 del cuerpo  →  nombre del fichero (sin `.md`)
/// ```
///
/// Es **solo una heurística de presentación**: `title` no es una propiedad reservada — se lee
/// como cualquier otra clave del frontmatter y **nunca** se reescribe (un `title: 42` se presenta
/// como `"42"` pero sigue siendo el número 42 para la consulta).
///
/// Función **pura** y **total**: devuelve `String`, no `Option`, porque el último eslabón —el
/// nombre del fichero— existe siempre. Un `title` sin rendición textual (lista, mapa, `null`) o
/// vacío no es un título presentable: la cadena continúa, sin error.
///
/// Recibe las tres piezas por separado —y no un [`Parsed`]— para que un consumidor que ya tenga
/// el frontmatter y el cuerpo (la cache, p. ej.) no tenga que re-parsear el documento entero.
pub fn derived_title(
    fm: Option<&ParsedFrontmatter>,
    body: &str,
    path: &crate::types::RelPath,
) -> String {
    if let Some(t) = fm
        .and_then(|f| f.get_text("title"))
        .filter(|s| !s.is_empty())
    {
        return t;
    }
    if let Some(h1) = first_h1(body) {
        return h1.to_string();
    }
    path.stem().to_string()
}

/// Texto del **primer heading de nivel 1** del cuerpo, ya recortado, o `None` si no hay ninguno.
///
/// Reutiliza [`parse_headings`], que reconoce los bloques de código fenceados: un `#` dentro de
/// un ` ``` ` es contenido del bloque, no un heading.
fn first_h1(body: &str) -> Option<&str> {
    parse_headings(body)
        .into_iter()
        .find(|h| h.level == 1)
        .map(|h| h.title)
}

/// Port de `normalize`: colapsa `.`/`..`/segmentos vacíos.
pub fn normalize(p: &str) -> String {
    let mut parts: Vec<&str> = Vec::new();
    for seg in p.split('/') {
        match seg {
            "." | "" => continue,
            ".." => {
                parts.pop();
            }
            _ => parts.push(seg),
        }
    }
    parts.join("/")
}

/// Port de `resolveLink`: resuelve un href a un path del bundle, o `None` si no aplica.
pub fn resolve_link(href: &str, from_path: &str) -> Option<String> {
    // Esquema (http:, mailto:, …) → no es enlace interno.
    if Regex::new(r"^[a-z]+:")
        .unwrap()
        .is_match(&href.to_ascii_lowercase())
    {
        return None;
    }
    if href.starts_with('#') {
        return None;
    }
    let mut h = href
        .split('#')
        .next()
        .unwrap_or("")
        .split('?')
        .next()
        .unwrap_or("")
        .to_string();
    if h.is_empty() {
        return None;
    }
    if h.ends_with('/') {
        h.push_str("index.md");
    }
    if !h.ends_with(".md") {
        return None;
    }
    let target = if let Some(stripped) = h.strip_prefix('/') {
        stripped.to_string()
    } else {
        let base = dir_of(from_path);
        normalize(&format!("{base}{h}"))
    };
    Some(target)
}

/// Port de `outLinks`: destinos salientes únicos del cuerpo (excluyendo el propio path).
pub fn out_links(path: &str, body: &str) -> Vec<String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for cap in LINK_RE.captures_iter(body) {
        if let Some(href) = cap.get(1) {
            if let Some(t) = resolve_link(href.as_str(), path) {
                if t != path && seen.insert(t.clone()) {
                    result.push(t);
                }
            }
        }
    }
    result
}

/// Como [`out_links`], pero conserva el href **crudo** junto al destino resuelto.
/// Mismo criterio (destinos únicos, excluye el propio path); útil para materializar `links` en la cache.
pub fn out_links_with_href(path: &str, body: &str) -> Vec<(String, String)> {
    let mut seen = std::collections::BTreeSet::new();
    let mut result = Vec::new();
    for cap in LINK_RE.captures_iter(body) {
        if let Some(href) = cap.get(1) {
            if let Some(t) = resolve_link(href.as_str(), path) {
                if t != path && seen.insert(t.clone()) {
                    result.push((href.as_str().to_string(), t));
                }
            }
        }
    }
    result
}

/// Port de `rawRelLinks`: hrefs salientes que son relativos (`./` o `../`) y apuntan a `.md`.
pub fn raw_rel_links(body: &str) -> Vec<String> {
    let rel = Regex::new(r"^\.{1,2}/").unwrap();
    let mut res = Vec::new();
    for cap in LINK_RE.captures_iter(body) {
        if let Some(href) = cap.get(1) {
            let h = href.as_str();
            if rel.is_match(h) && h.contains(".md") {
                res.push(h.to_string());
            }
        }
    }
    res
}

/// Port de `isISO`: `true` si es una fecha ISO **válida entera** (`Date.parse` + regex).
/// La validación cubre el string completo: `2024-01-15hello` o `…T99:99` son NaN en
/// `Date.parse` → FMT-TS, no silencio.
pub fn is_iso(v: &serde_yaml::Value) -> bool {
    static ISO_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"^(\d{4})-(\d{2})-(\d{2})([T ](\d{2}):(\d{2})(:(\d{2})(\.\d+)?)?(Z|[+-]\d{2}:?\d{2})?)?$",
        )
        .unwrap()
    });
    let s = match v {
        serde_yaml::Value::String(s) => s,
        // serde_yaml tipa una fecha sin comillas como String; otros tipos no son ISO.
        _ => return false,
    };
    let Some(c) = ISO_RE.captures(s) else {
        return false;
    };
    let num = |i: usize| c.get(i).and_then(|m| m.as_str().parse::<u32>().ok());
    let (Some(y), Some(m), Some(d)) = (num(1), num(2), num(3)) else {
        return false;
    };
    if y < 1 || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return false;
    }
    match (num(5), num(6), num(8)) {
        (None, _, _) => true,
        // `24:00` es válido en ISO (y en `Date.parse`); 25+ no.
        (Some(h), Some(min), sec) => h <= 24 && min <= 59 && sec.map(|x| x <= 59).unwrap_or(true),
        _ => false,
    }
}

/// Port de `sortPaths` = `a.localeCompare(b, undefined, {numeric:true})`: orden natural con
/// reconocimiento de números (`doc-2` < `doc-10`). Las tiras de dígitos se comparan por valor;
/// el resto, por code-point. La paridad exacta con la colación ICU para mayúsculas/acentos es un
/// no-goal documentado: para paths kebab-case en minúscula (el caso real) coincide.
pub fn sort_paths_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (mut i, mut j) = (0usize, 0usize);
    while i < a.len() && j < b.len() {
        let (ca, cb) = (a[i], b[j]);
        if ca.is_ascii_digit() && cb.is_ascii_digit() {
            let si = i;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            let sj = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let na: String = a[si..i].iter().collect();
            let nb: String = b[sj..j].iter().collect();
            let ta = na.trim_start_matches('0');
            let tb = nb.trim_start_matches('0');
            // Mismo valor numérico ⇒ compara por magnitud (longitud sin ceros, luego dígitos),
            // y como desempate la tira más corta (menos ceros a la izquierda) va primero.
            let ord = ta
                .len()
                .cmp(&tb.len())
                .then_with(|| ta.cmp(tb))
                .then_with(|| na.len().cmp(&nb.len()));
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            match ca.cmp(&cb) {
                Ordering::Equal => {
                    i += 1;
                    j += 1;
                }
                ord => return ord,
            }
        }
    }
    (a.len() - i).cmp(&(b.len() - j))
}

/// Aproximación de `a.localeCompare(b)` (colación ICU por defecto): primaria = letras base
/// en minúscula (NFD sin marcas), desempate = minúscula antes que mayúscula, luego code-point.
/// Para el catálogo real (tags en español) coincide con V8; la paridad ICU exacta es no-goal.
pub fn locale_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    use unicode_normalization::char::is_combining_mark;
    use unicode_normalization::UnicodeNormalization;
    let fold = |s: &str| -> String {
        s.nfd()
            .filter(|c| !is_combining_mark(*c))
            .flat_map(char::to_lowercase)
            .collect()
    };
    match fold(a).cmp(&fold(b)) {
        Ordering::Equal => {}
        ord => return ord,
    }
    // Desempate por caso: la minúscula ordena antes ("foo" < "Foo", como localeCompare).
    for (ca, cb) in a.chars().zip(b.chars()) {
        if ca == cb {
            continue;
        }
        let (la, lb): (String, String) = (ca.to_lowercase().collect(), cb.to_lowercase().collect());
        if la == lb {
            return if ca.is_lowercase() {
                Ordering::Less
            } else {
                Ordering::Greater
            };
        }
        return ca.cmp(&cb);
    }
    a.len().cmp(&b.len())
}

/// Parsea **solo** el frontmatter de un documento, sin necesitar su path: para las utilidades que
/// no tienen más que el raw (el diff, p. ej.). `None` si el documento no tiene bloque cerrado o su
/// YAML es inválido.
pub fn parse_frontmatter(raw: &str) -> Option<ParsedFrontmatter> {
    let SplitFront::Bloque { span, .. } = split_front(raw) else {
        return None;
    };
    let texto = &raw[span.clone()];
    parse_yaml(texto).ok().map(|value| ParsedFrontmatter {
        value,
        raw: texto.to_string(),
        span,
    })
}

/// Reconstruye el `.md` a partir de su frontmatter y su cuerpo.
///
/// Sin frontmatter (`None`) el documento **es** su cuerpo: no se inventa un bloque vacío. Con
/// frontmatter se serializa su `value`, que preserva el orden de aparición de las claves (el
/// `Mapping` de `serde_yaml` es un `IndexMap`) y **no descarta ninguna**: ni la cadena vacía, ni
/// la lista vacía, ni el `null` explícito — todos son valores del usuario (`§20.4`).
///
/// > La edición **quirúrgica** del bloque (reutilizar `raw`/`span` en vez de reserializar) es
/// > E16-H04; aquí siempre se reserializa el `value`.
pub fn build_raw(fm: Option<&ParsedFrontmatter>, body: &str) -> String {
    let Some(fm) = fm else {
        return body.to_string();
    };
    let y = serde_yaml::to_string(&fm.value)
        .unwrap_or_default()
        .trim_end()
        .to_string();
    let body_trimmed = body.trim_start_matches('\n');
    format!("---\n{y}\n---\n\n{body_trimmed}")
}

/// Resultado del parseo de un documento (sin el `raw`, que ya tiene el llamante).
pub struct Parsed {
    /// El frontmatter del documento, o `None` si no tiene bloque (estado **válido**, `§20.4`).
    pub frontmatter: Option<ParsedFrontmatter>,
    pub fm_err: Option<FmError>,
    pub body: String,
}

/// Parsea un documento. NUNCA falla por contenido: los errores de frontmatter son datos
/// ([`FmError`]), no un `Result`.
///
/// Un documento **sin** frontmatter es válido: `frontmatter: None`, `fm_err: None` y el cuerpo es
/// el fichero entero.
///
/// **No ramifica por nombre de fichero** (E16-H02, `REFACTOR_PHASE_2 §Principio 4`): `index.md`,
/// `log.md`, `README.md` y `docs/decisions/auth.md` se parsean exactamente igual. El `path` solo
/// se conserva en la firma porque es la identidad del documento para los llamantes.
pub fn parse_file(_path: &str, raw: &str) -> Parsed {
    let sf = split_front(raw);
    let body = sf.body(raw).to_string();
    match &sf {
        SplitFront::Sin => Parsed {
            frontmatter: None,
            fm_err: None,
            body,
        },
        SplitFront::SinCerrar => Parsed {
            frontmatter: None,
            fm_err: Some(FmError::Unclosed),
            body,
        },
        SplitFront::Bloque { span, .. } => {
            let texto = &raw[span.clone()];
            match parse_yaml(texto) {
                Ok(value) => Parsed {
                    frontmatter: Some(ParsedFrontmatter {
                        value,
                        raw: texto.to_string(),
                        span: span.clone(),
                    }),
                    fm_err: None,
                    body,
                },
                Err(e) => Parsed {
                    frontmatter: None,
                    fm_err: Some(FmError::Malformed(e)),
                    body,
                },
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Localización de secciones por `headingPath` (movido de `lodestar-app`, E10-H10;
// reusado por `knowledge_get` y por la normalización de `edit_section`, E12-H05).
// ---------------------------------------------------------------------------

/// Un heading Markdown detectado en un `body`, con el rango de bytes de la sección que abarca:
/// desde el final de su propia línea de heading hasta el siguiente heading de nivel **menor o
/// igual** al suyo (o el final del cuerpo). Ese rango contiene exactamente sus subsecciones
/// anidadas (nivel estrictamente mayor) y nada de sus hermanas ni de secciones de nivel superior —
/// la propiedad que usa [`locate_section`] para no necesitar validar jerarquía explícitamente.
///
/// Tipo opaco: los campos son privados del módulo (solo [`parse_headings`]/[`locate_section`] los
/// tocan); los llamantes externos lo manejan como un `Vec<Heading>` sin inspeccionarlo.
pub struct Heading<'a> {
    /// Nivel ATX del heading (1..=6). Lo necesita [`derived_title`] para quedarse con el primer
    /// **H1** (no con el primer heading a secas).
    level: usize,
    /// Texto del heading, recortado.
    title: &'a str,
    /// Offset de byte donde empieza la línea del heading (para comprobar pertenencia a un rango).
    line_start: usize,
    /// Offset de byte donde empieza el contenido de su sección (justo tras su línea).
    content_start: usize,
    /// Offset de byte donde termina el contenido de su sección (exclusivo).
    content_end: usize,
}

/// Detecta los headings ATX (`#` a `######`) de `body` línea a línea y calcula el rango de
/// contenido de cada uno.
///
/// **Reconoce los bloques de código fenceados** (` ``` `): una línea cuyo texto recortado empieza
/// por ` ``` ` (con o sin lenguaje) abre o cierra un bloque de código, y los `#` que aparezcan
/// DENTRO de ese bloque NO se tratan como headings (serían texto/comentarios del código). Esto
/// evita truncar el rango de una sección real en un `#` espurio (E12-H05, cierra la reserva
/// documentada de E10-H10).
pub fn parse_headings(body: &str) -> Vec<Heading<'_>> {
    let mut raw: Vec<(usize, &str, usize, usize)> = Vec::new();
    let mut offset = 0usize;
    let mut in_fence = false;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        // Un fence de código (```) abre/cierra el bloque; la propia línea del fence nunca es un
        // heading, y mientras el bloque está abierto los `#` internos se ignoran.
        if trimmed.trim_start().starts_with("```") {
            in_fence = !in_fence;
            offset += line.len();
            continue;
        }
        if !in_fence {
            let hashes = trimmed.chars().take_while(|c| *c == '#').count();
            if (1..=6).contains(&hashes) {
                let rest = &trimmed[hashes..];
                if rest.starts_with(' ') || rest.starts_with('\t') {
                    raw.push((hashes, rest.trim(), offset, offset + line.len()));
                }
            }
        }
        offset += line.len();
    }
    let body_len = body.len();
    raw.iter()
        .enumerate()
        .map(|(i, &(level, title, line_start, content_start))| {
            let content_end = raw[i + 1..]
                .iter()
                .find(|&&(l, ..)| l <= level)
                .map(|&(_, _, ls, _)| ls)
                .unwrap_or(body_len);
            Heading {
                level,
                title,
                line_start,
                content_start,
                content_end,
            }
        })
        .collect()
}

/// Localiza el rango de bytes del contenido de la subsección apuntada por un `heading_path` (p. ej.
/// `["Security","Token rotation"]`): recorre el path segmento a segmento, en cada paso busca el
/// primer heading cuyo título coincida (comparación exacta, recortada) **dentro del rango actual**
/// y estrecha el rango a su sección. Como el rango de una sección solo contiene a sus
/// descendientes (ver [`Heading`]), no hace falta comprobar niveles explícitamente: el segundo
/// segmento del path solo puede casar con un heading anidado bajo el primero. `None` si algún
/// segmento no casa (heading_path inexistente). El rango devuelto es `(content_start, content_end)`
/// — el contenido de la sección SIN su línea de heading.
pub fn locate_section(
    headings: &[Heading<'_>],
    body_len: usize,
    path: &[String],
) -> Option<(usize, usize)> {
    let mut range = (0usize, body_len);
    for segment in path {
        let found = headings
            .iter()
            .find(|h| h.line_start >= range.0 && h.line_start < range.1 && h.title == *segment)?;
        range = (found.content_start, found.content_end);
    }
    Some(range)
}

/// Extrae y concatena (separadas por una línea en blanco) las subsecciones apuntadas por cada
/// `heading_path` de `sections`, en el orden pedido. Un `heading_path` que no casa con ningún
/// heading se omite silenciosamente (sin `sections` no vacío, el llamante ya filtra este caso).
pub fn extract_sections(body: &str, sections: &[Vec<String>]) -> String {
    let headings = parse_headings(body);
    sections
        .iter()
        .filter(|path| !path.is_empty())
        .filter_map(|path| locate_section(&headings, body.len(), path))
        .map(|(start, end)| body[start..end].to_string())
        .collect::<Vec<_>>()
        .join("\n\n")
}
