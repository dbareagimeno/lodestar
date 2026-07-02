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
pub fn parse_yaml(text: &str) -> Result<Frontmatter, String> {
    if text.trim().is_empty() {
        return Ok(Frontmatter::default());
    }
    match serde_yaml::from_str::<Yaml>(text) {
        Ok(v) => {
            // Solo los mapeos producen frontmatter; cualquier otra cosa → frontmatter vacío (como el proto).
            if let Yaml::Mapping(_) = v {
                serde_yaml::from_value::<Frontmatter>(v).map_err(|e| e.to_string())
            } else {
                Ok(Frontmatter::default())
            }
        }
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

/// Id de concepto: la ruta sin `.md`. Port de `conceptId`.
pub fn concept_id(p: &str) -> String {
    p.strip_suffix(".md").unwrap_or(p).to_string()
}

/// `true` si el basename es reservado.
pub fn is_reserved(p: &str) -> bool {
    matches!(basename(p), "index.md" | "log.md")
}

/// Port de `titleFromPath`: deriva un título legible del path (Title Case sobre el concept id).
pub fn title_from_path(p: &str) -> String {
    let base = concept_id(basename(p)).replace(['-', '_'], " ");
    let mut out = String::with_capacity(base.len());
    let mut at_word_start = true;
    for ch in base.chars() {
        if at_word_start && ch.is_alphanumeric() {
            out.extend(ch.to_uppercase());
        } else {
            out.push(ch);
        }
        at_word_start = ch.is_whitespace();
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

/// Port de `isISO`: `true` si parece una fecha ISO (`AAAA-MM-DD…`).
pub fn is_iso(v: &serde_yaml::Value) -> bool {
    let s = match v {
        serde_yaml::Value::String(s) => s.clone(),
        // serde_yaml puede tipar una fecha sin comillas como String; otros tipos no son ISO.
        _ => return false,
    };
    let date_re = Regex::new(r"\d{4}-\d{2}-\d{2}").unwrap();
    date_re.is_match(&s) && parse_iso_date(&s)
}

/// Validación laxa de fecha: `AAAA-MM-DD` con rangos plausibles (sustituye a `Date.parse`).
fn parse_iso_date(s: &str) -> bool {
    let head = &s.get(0..10).unwrap_or("");
    let bytes = head.as_bytes();
    if head.len() != 10 || bytes[4] != b'-' || bytes[7] != b'-' {
        return false;
    }
    let (y, m, d) = (&head[0..4], &head[5..7], &head[8..10]);
    let (y, m, d) = match (y.parse::<u32>(), m.parse::<u32>(), d.parse::<u32>()) {
        (Ok(y), Ok(m), Ok(d)) => (y, m, d),
        _ => return false,
    };
    y >= 1 && (1..=12).contains(&m) && (1..=31).contains(&d)
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

/// Construye la representación YAML canónica de un frontmatter ordenado (known fields primero).
fn dump_frontmatter(fm: &Frontmatter) -> String {
    let mut map = serde_yaml::Mapping::new();
    let push = |map: &mut serde_yaml::Mapping, k: &str, v: Yaml| {
        if !yaml_is_empty(&v) {
            map.insert(Yaml::String(k.to_string()), v);
        }
    };
    // Orden KNOWN_FM.
    if let Some(v) = &fm.r#type {
        push(&mut map, "type", Yaml::String(v.clone()));
    }
    if let Some(v) = &fm.title {
        push(&mut map, "title", Yaml::String(v.clone()));
    }
    if let Some(v) = &fm.description {
        push(&mut map, "description", Yaml::String(v.clone()));
    }
    if let Some(v) = &fm.resource {
        push(&mut map, "resource", Yaml::String(v.clone()));
    }
    if let Some(v) = &fm.tags {
        push(&mut map, "tags", v.clone());
    }
    if let Some(v) = &fm.timestamp {
        push(&mut map, "timestamp", v.clone());
    }
    if let Some(v) = &fm.status {
        push(&mut map, "status", Yaml::String(v.clone()));
    }
    // Extras (claves de productor), ordenadas por el BTreeMap.
    for (k, v) in &fm.extra {
        if !KNOWN_FM.contains(&k.as_str()) {
            push(&mut map, k, v.clone());
        }
    }
    serde_yaml::to_string(&Yaml::Mapping(map))
        .unwrap_or_default()
        .trim_end()
        .to_string()
}

/// `true` si un valor YAML cuenta como "vacío" para `build_raw`. El filtro de `buildRaw`
/// (`fm[k]!==undefined && fm[k]!=="" && !lista-vacía`) solo descarta cadena/lista vacías: un
/// `null` YAML SÍ se serializa (p. ej. `tags: null`), así que NO cuenta como vacío.
fn yaml_is_empty(v: &Yaml) -> bool {
    match v {
        Yaml::String(s) => s.is_empty(),
        Yaml::Sequence(s) => s.is_empty(),
        _ => false,
    }
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
