//! Primitivas de modelo: parseo y serialización 1:1 del prototipo (`ARCHITECTURE.md §4`).
//!
//! Port fiel de `splitFront`, `parseYAML`, `dumpYAML`, `parseFile`, `buildRaw`, `resolveLink`,
//! `normalize`, `outLinks`, `rawRelLinks`, `isISO`. Quirks incluidos.

use once_cell::sync::Lazy;
use regex::Regex;
use serde_yaml::Value as Yaml;

use crate::types::{FileKind, FmError, Frontmatter, KNOWN_FM};

static SPLIT_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r"(?s)^---\r?\n(.*?)\r?\n---\r?\n?(.*)$").unwrap());

/// `[texto](href "title")` — el grupo 1 es el href. Global.
pub(crate) static LINK_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\[[^\]]*\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#).unwrap());

/// Resultado de `split_front`. `fm_text == Some("")` = sin frontmatter; `None` = sin cierre.
pub struct SplitFront {
    pub fm_text: Option<String>,
    pub body: String,
}

/// Port de `splitFront`: separa frontmatter del cuerpo.
pub fn split_front(raw: &str) -> SplitFront {
    if raw.starts_with("---") {
        if let Some(c) = SPLIT_RE.captures(raw) {
            return SplitFront {
                fm_text: Some(c.get(1).map_or("", |m| m.as_str()).to_string()),
                body: c.get(2).map_or("", |m| m.as_str()).to_string(),
            };
        }
        // Empieza por `---` pero no cierra → sin cierre.
        return SplitFront {
            fm_text: None,
            body: raw.to_string(),
        };
    }
    SplitFront {
        fm_text: Some(String::new()),
        body: raw.to_string(),
    }
}

/// Port de `parseYAML`: parsea el texto del frontmatter a un mapa. Devuelve `Err(msg)` si es inválido.
///
/// La conversión mapa→`Frontmatter` es **manual** (no serde-derive): el prototipo acepta
/// cualquier escalar YAML en los campos tipados (`type: 123` → página de tipo "123" vía
/// `String(v)` en los puntos de uso), mientras que el derive fallaba el fichero ENTERO con
/// `OKF-FM03` (hard-fail) — invirtiendo el veredicto de la puerta de CI. También conserva
/// los `null` explícitos (presentes en JS) y stringifica claves no-string (`1: x` → `"1"`).
pub fn parse_yaml(text: &str) -> Result<Frontmatter, String> {
    if text.trim().is_empty() {
        return Ok(Frontmatter::default());
    }
    match serde_yaml::from_str::<Yaml>(text) {
        // Solo los mapeos producen frontmatter; cualquier otra cosa → frontmatter vacío (como el proto).
        Ok(Yaml::Mapping(m)) => Ok(frontmatter_from_mapping(m)),
        Ok(_) => Ok(Frontmatter::default()),
        Err(e) => Err(e.to_string()),
    }
}

/// Convierte un mapping YAML a `Frontmatter` con la coerción del prototipo.
fn frontmatter_from_mapping(m: serde_yaml::Mapping) -> Frontmatter {
    let mut fm = Frontmatter::default();
    for (k, v) in m {
        let key = js_string(&k);
        // Los 5 knowns string: null explícito se registra (presente); el resto se coerce a string.
        let mut set = |slot: &mut Option<String>, name: &str, v: Yaml| match v {
            Yaml::Null => fm_mark_null(&mut fm.known_null, name),
            other => *slot = Some(js_string(&other)),
        };
        match key.as_str() {
            "type" => set(&mut fm.r#type, "type", v),
            "title" => set(&mut fm.title, "title", v),
            "description" => set(&mut fm.description, "description", v),
            "resource" => set(&mut fm.resource, "resource", v),
            "status" => set(&mut fm.status, "status", v),
            // tags/timestamp se guardan RAW (incluido `null`: presente, y FMT-TAGS lo ve).
            "tags" => fm.tags = Some(v),
            "timestamp" => fm.timestamp = Some(v),
            _ => {
                fm.extra.insert(key, v);
            }
        }
    }
    fm
}

fn fm_mark_null(nulls: &mut Vec<String>, name: &str) {
    if !nulls.iter().any(|n| n == name) {
        nulls.push(name.to_string());
    }
}

/// Port de `String(v)` de JS para valores YAML: números/bools a texto, arrays `join(",")`
/// (con `null` → `""` dentro del join, como `Array.prototype.toString`), mapas `[object Object]`.
pub(crate) fn js_string(v: &Yaml) -> String {
    match v {
        Yaml::String(s) => s.clone(),
        Yaml::Bool(b) => b.to_string(),
        Yaml::Number(n) => n.to_string(),
        Yaml::Null => "null".to_string(),
        Yaml::Sequence(items) => items
            .iter()
            .map(|x| match x {
                Yaml::Null => String::new(),
                other => js_string(other),
            })
            .collect::<Vec<_>>()
            .join(","),
        Yaml::Mapping(_) => "[object Object]".to_string(),
        Yaml::Tagged(t) => js_string(&t.value),
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

/// Id de concepto: la ruta sin `.md`. Port de `conceptId`.
pub fn concept_id(p: &str) -> String {
    p.strip_suffix(".md").unwrap_or(p).to_string()
}

/// `true` si el basename es reservado.
pub fn is_reserved(p: &str) -> bool {
    matches!(basename(p), "index.md" | "log.md")
}

/// Port de `titleFromPath`: `replace(/\b\w/g, c=>c.toUpperCase())`. El boundary `\b` de JS es
/// «char `\w` precedido de no-`\w`» con `\w = [A-Za-z0-9_]`: un acento o un punto también abren
/// palabra (`año` → `AñO`, `foo.bar` → `Foo.Bar`) — quirk incluido, es la spec.
pub fn title_from_path(p: &str) -> String {
    let base = concept_id(basename(p)).replace(['-', '_'], " ");
    let mut out = String::with_capacity(base.len());
    let mut prev_is_word = false;
    for ch in base.chars() {
        let is_word = ch.is_ascii_alphanumeric() || ch == '_';
        if is_word && !prev_is_word {
            out.extend(ch.to_uppercase());
        } else {
            out.push(ch);
        }
        prev_is_word = is_word;
    }
    out
}

/// Clase del fichero a partir del path.
pub fn file_kind(p: &str) -> FileKind {
    match basename(p) {
        "index.md" => FileKind::Index,
        "log.md" => FileKind::Log,
        _ => FileKind::Concept,
    }
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

/// Construye la representación YAML canónica de un frontmatter ordenado (known fields primero).
///
/// Filtros de `buildRaw` (fieles al proto): los KNOWN descartan cadena vacía Y lista vacía;
/// los extras SOLO cadena vacía (una lista vacía de productor se conserva). Un `null` explícito
/// se serializa en ambos casos (`tags: null`).
fn dump_frontmatter(fm: &Frontmatter) -> String {
    let mut map = serde_yaml::Mapping::new();
    let push_known = |map: &mut serde_yaml::Mapping, k: &str, v: Yaml| {
        let empty = matches!(&v, Yaml::String(s) if s.is_empty())
            || matches!(&v, Yaml::Sequence(s) if s.is_empty());
        if !empty {
            map.insert(Yaml::String(k.to_string()), v);
        }
    };
    let is_null = |k: &str| fm.known_null.iter().any(|n| n == k);
    // Orden KNOWN_FM (los null explícitos también se emiten, en su posición canónica).
    match (&fm.r#type, is_null("type")) {
        (Some(v), _) => push_known(&mut map, "type", Yaml::String(v.clone())),
        (None, true) => push_known(&mut map, "type", Yaml::Null),
        _ => {}
    }
    match (&fm.title, is_null("title")) {
        (Some(v), _) => push_known(&mut map, "title", Yaml::String(v.clone())),
        (None, true) => push_known(&mut map, "title", Yaml::Null),
        _ => {}
    }
    match (&fm.description, is_null("description")) {
        (Some(v), _) => push_known(&mut map, "description", Yaml::String(v.clone())),
        (None, true) => push_known(&mut map, "description", Yaml::Null),
        _ => {}
    }
    match (&fm.resource, is_null("resource")) {
        (Some(v), _) => push_known(&mut map, "resource", Yaml::String(v.clone())),
        (None, true) => push_known(&mut map, "resource", Yaml::Null),
        _ => {}
    }
    if let Some(v) = &fm.tags {
        push_known(&mut map, "tags", v.clone());
    }
    if let Some(v) = &fm.timestamp {
        push_known(&mut map, "timestamp", v.clone());
    }
    match (&fm.status, is_null("status")) {
        (Some(v), _) => push_known(&mut map, "status", Yaml::String(v.clone())),
        (None, true) => push_known(&mut map, "status", Yaml::Null),
        _ => {}
    }
    // Extras (claves de productor), en orden de aparición; solo se filtra la cadena vacía.
    for (k, v) in &fm.extra {
        if !KNOWN_FM.contains(&k.as_str()) && !matches!(v, Yaml::String(s) if s.is_empty()) {
            map.insert(Yaml::String(k.clone()), v.clone());
        }
    }
    serde_yaml::to_string(&Yaml::Mapping(map))
        .unwrap_or_default()
        .trim_end()
        .to_string()
}

/// Port de `buildRaw`: reconstrucción canónica del `.md` (frontmatter ordenado + cuerpo).
/// Es LA canonicalización del modelo OKF.
pub fn build_raw(fm: &Frontmatter, body: &str) -> String {
    let y = dump_frontmatter(fm);
    let body_trimmed = body.trim_start_matches('\n');
    format!("---\n{y}\n---\n\n{body_trimmed}")
}

/// Resultado interno del parseo de un fichero (sin el `raw`, que añade el llamante).
pub struct Parsed {
    pub kind: FileKind,
    pub fm: Option<Frontmatter>,
    pub fm_err: Option<FmError>,
    pub body: String,
}

/// Port de `parseFile`: NUNCA falla por contenido (FM01/02/03 son datos, no `Result`).
pub fn parse_file(path: &str, raw: &str) -> Parsed {
    let kind = file_kind(path);
    let sf = split_front(raw);
    if kind != FileKind::Concept {
        // Reservados: el cuerpo es el raw entero, sin frontmatter tipado.
        return Parsed {
            kind,
            fm: None,
            fm_err: None,
            body: raw.to_string(),
        };
    }
    match sf.fm_text {
        None => Parsed {
            kind,
            fm: None,
            fm_err: Some(FmError::Unclosed),
            body: sf.body,
        },
        Some(t) if t.is_empty() => Parsed {
            kind,
            fm: None,
            fm_err: Some(FmError::Missing),
            body: sf.body,
        },
        Some(t) => match parse_yaml(&t) {
            Ok(fm) => Parsed {
                kind,
                fm: Some(fm),
                fm_err: None,
                body: sf.body,
            },
            Err(e) => Parsed {
                kind,
                fm: None,
                fm_err: Some(FmError::Malformed(e)),
                body: sf.body,
            },
        },
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
