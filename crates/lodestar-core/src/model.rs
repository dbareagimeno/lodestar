//! Primitivas de modelo: parseo y serialización del documento (`ARCHITECTURE.md §4`, `§20.4`).
//!
//! El frontmatter es **metadata arbitraria del usuario** (`§20.4`, E16-H01): se conserva íntegro,
//! con su tipo YAML real y su texto original. El resto del módulo sigue siendo el port de
//! `resolveLink`, `normalize`, `outLinks`, `rawRelLinks`, quirks incluidos (`isISO` murió con
//! `FMT-TS` en E16-H05).

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

/// Port de `resolveLink`: resuelve un href a un path del workspace, o `None` si no aplica.
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

/// El documento resultante de aplicar un [`crate::types::FrontmatterPatch`]
/// (`ARCHITECTURE.md §20.4`, E16-H04).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PatchedDocument {
    /// El `.md` **completo** resultante (frontmatter + cuerpo).
    pub raw: String,
    /// `true` si el bloque de frontmatter se **reserializó entero** en vez de editarse in situ —
    /// es decir, si se ha perdido el texto original del bloque (formato, comillas, comentarios
    /// YAML). Lo consume `change_plan` (E21) para declararlo en el plan.
    ///
    /// **Crear** un bloque donde no había ninguno **no** es reserialización: no se pierde texto
    /// del usuario porque no había texto que perder.
    pub reserialized: bool,
}

/// Aplica un `FrontmatterPatch` sobre el texto crudo de **un** documento (`§20.4`, E16-H04).
///
/// Función **pura**: ni toca disco ni necesita el resto del workspace. Recibe el `raw` entero
/// (y no un `&DocumentSet`) porque la edición quirúrgica necesita el `span` del bloque **dentro** del
/// documento.
///
/// # Los tres caminos
///
/// 1. **Quirúrgico** (`reserialized: false`) — se sustituyen o borran solo las líneas de las
///    claves tocadas, y las claves nuevas se añaden al final del bloque. Se toma cuando **cada**
///    clave del patch, o bien no existe en el bloque, o bien existe en el primer nivel con su
///    valor escrito **en una sola línea** (`clave: escalar`, `clave: [a, b]` en flow style,
///    `clave:` vacío). Las líneas no tocadas llegan al resultado **byte a byte**: el flow style
///    sigue en flow, las comillas siguen como estaban y los comentarios YAML sobreviven.
/// 2. **Reserialización** (`reserialized: true`) — se vuelca el mapa entero con `serde_yaml`,
///    perdiendo el texto original del bloque pero **ningún dato**: se conservan todas las claves,
///    su orden de aparición y sus tipos. Se toma cuando alguna clave tocada ocupa varias líneas
///    (mapa o lista en block style, block scalar `|`/`>`) o cuando el bloque tiene una forma que
///    impide localizar líneas con seguridad (claves duplicadas, anchors/alias, líneas de primer
///    nivel que no son `clave: valor`).
/// 3. **Creación** (`reserialized: false`) — el documento no tiene bloque: se antepone uno con las
///    claves del patch y el documento original queda intacto como cuerpo.
///
/// El **cuerpo queda intacto byte a byte** en los tres casos: la operación solo reemplaza el rango
/// de bytes del bloque (o antepone uno nuevo).
///
/// # Errores
/// [`crate::CoreError::UnreadableFrontmatter`] si el documento **tiene** bloque pero Lodestar no
/// puede interpretarlo (sin cerrar, o YAML inválido). Es deliberado y no un detalle: `parse_file`
/// devuelve `frontmatter: None` tanto para «no hay bloque» como para «hay un bloque ilegible», así
/// que una implementación guiada por `frontmatter.is_none()` reconstruiría el bloque **encima** del
/// ilegible y borraría la metadata del usuario.
pub fn patch_frontmatter(
    raw: &str,
    patch: &crate::types::FrontmatterPatch,
) -> Result<PatchedDocument, crate::CoreError> {
    let span = match split_front(raw) {
        SplitFront::SinCerrar => {
            return Err(crate::CoreError::UnreadableFrontmatter(
                "el bloque abre «---» y nunca cierra".to_string(),
            ));
        }
        // Sin bloque: se crea uno con las claves que el patch escribe (`§20.4`).
        SplitFront::Sin => {
            let mut map = serde_yaml::Mapping::new();
            crate::document_set::apply_patch(&mut map, patch.clone());
            if map.is_empty() {
                // Un patch que solo borra claves inexistentes no toca el documento.
                return Ok(PatchedDocument {
                    raw: raw.to_string(),
                    reserialized: false,
                });
            }
            let bloque = dump_mapping(&map);
            return Ok(PatchedDocument {
                raw: format!("---\n{bloque}\n---\n\n{raw}"),
                reserialized: false,
            });
        }
        SplitFront::Bloque { span, .. } => span,
    };

    let texto = &raw[span.clone()];
    let valor = parse_yaml(texto).map_err(crate::CoreError::UnreadableFrontmatter)?;
    let mapa = valor
        .as_mapping()
        .cloned()
        .unwrap_or_else(serde_yaml::Mapping::new);

    // Claves de primer nivel del mapa parseado, rendidas a texto y en orden de aparición: es la
    // referencia contra la que se valida el escaneo por líneas.
    let claves: Vec<String> = mapa.keys().filter_map(yaml_key_text).collect();
    let escaneo = scan_top_level(texto).filter(|entradas| {
        entradas.len() == mapa.len()
            && entradas.len() == claves.len()
            && entradas.iter().map(|e| &e.clave).eq(claves.iter())
    });

    match plan_surgical(patch, escaneo.as_deref(), &claves) {
        // Sin ediciones el documento no cambia: se devuelve byte a byte.
        Some(edits) if edits.is_empty() => Ok(PatchedDocument {
            raw: raw.to_string(),
            reserialized: false,
        }),
        Some(edits) => Ok(PatchedDocument {
            raw: splice(raw, &span, &apply_line_edits(texto, &edits)),
            reserialized: false,
        }),
        None => {
            let mut mapa = mapa;
            crate::document_set::apply_patch(&mut mapa, patch.clone());
            Ok(PatchedDocument {
                raw: splice(raw, &span, &dump_mapping(&mapa)),
                reserialized: true,
            })
        }
    }
}

/// Serializa un mapa YAML al texto de un bloque de frontmatter (**sin** los delimitadores ni el
/// salto final: el `span` del bloque tampoco los incluye). El mapa vacío da la cadena vacía.
fn dump_mapping(map: &serde_yaml::Mapping) -> String {
    if map.is_empty() {
        return String::new();
    }
    serde_yaml::to_string(&Yaml::Mapping(map.clone()))
        .unwrap_or_default()
        .trim_end()
        .to_string()
}

/// Sustituye el rango de bytes `span` de `raw` por `bloque`, añadiendo el salto de línea que el
/// delimitador de cierre necesita si el bloque original estaba vacío (su `span` es el hueco
/// `---\n|---`, sin el `\n` que ahora hace falta).
fn splice(raw: &str, span: &std::ops::Range<usize>, bloque: &str) -> String {
    let cola = &raw[span.end..];
    let sep = if bloque.is_empty() || cola.starts_with('\n') || cola.starts_with("\r\n") {
        ""
    } else {
        "\n"
    };
    format!("{}{bloque}{sep}{cola}", &raw[..span.start])
}

/// Una clave de primer nivel localizada en el TEXTO del bloque: su nombre y el rango de líneas
/// (semiabierto) que ocupa su entrada, ya sin las líneas en blanco ni los comentarios de cola.
struct TopLevelEntry {
    clave: String,
    inicio: usize,
    fin: usize,
}

/// Texto de una clave YAML de primer nivel (solo escalares: las claves compuestas no son
/// direccionables por [`crate::types::FieldPath`] ni por [`crate::types::FrontmatterPatch`]).
fn yaml_key_text(k: &Yaml) -> Option<String> {
    match k {
        Yaml::String(s) => Some(s.clone()),
        Yaml::Number(n) => Some(n.to_string()),
        Yaml::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Localiza las claves de **primer nivel** en el texto crudo de un bloque, línea a línea.
///
/// `None` si alguna línea sin indentar no es un `clave: valor` reconocible (una lista de primer
/// nivel, un `...`, una clave entrecomillada con escapes) o si algún valor abre un anchor/alias
/// YAML: en esos casos no se puede editar por líneas con seguridad y el llamante reserializa.
fn scan_top_level(texto: &str) -> Option<Vec<TopLevelEntry>> {
    let lineas: Vec<&str> = texto.split('\n').collect();
    let mut entradas: Vec<TopLevelEntry> = Vec::new();
    for (i, linea) in lineas.iter().enumerate() {
        let l = linea.trim_end_matches('\r');
        // Blancos, comentarios y continuaciones indentadas pertenecen a la entrada anterior (o a
        // nadie): no abren una clave de primer nivel.
        if l.trim().is_empty() || l.starts_with([' ', '\t']) || l.starts_with('#') {
            continue;
        }
        let (clave, valor) = split_key_line(l)?;
        // Anchors y alias quedan fuera de alcance (`§20.4`): editar una línea puede dejar un alias
        // sin definición, así que se reserializa (que siempre es correcto).
        if valor.trim_start().starts_with(['&', '*']) {
            return None;
        }
        if let Some(previa) = entradas.last_mut() {
            previa.fin = i;
        }
        entradas.push(TopLevelEntry {
            clave,
            inicio: i,
            fin: lineas.len(),
        });
    }
    // La cola de cada entrada no incluye sus líneas en blanco ni sus comentarios finales: un
    // comentario entre dos claves flota, no pertenece a la de arriba.
    for e in &mut entradas {
        while e.fin > e.inicio + 1 {
            let l = lineas[e.fin - 1].trim_end_matches('\r');
            if l.trim().is_empty() || l.trim_start().starts_with('#') {
                e.fin -= 1;
            } else {
                break;
            }
        }
    }
    Some(entradas)
}

/// Parte una línea de primer nivel en `(clave, resto tras los dos puntos)`. `None` si no tiene la
/// forma `clave:` o `clave: valor` (la clave puede ir entrecomillada, sin escapes).
fn split_key_line(l: &str) -> Option<(String, &str)> {
    if let Some(comilla) = l.chars().next().filter(|c| *c == '"' || *c == '\'') {
        let cuerpo = &l[1..];
        let cierre = cuerpo.find(comilla)?;
        let clave = &cuerpo[..cierre];
        if clave.contains('\\') {
            return None;
        }
        let resto = &cuerpo[cierre + 1..];
        let valor = resto.strip_prefix(':')?;
        return Some((clave.to_string(), valor));
    }
    // Sin comillas: los dos puntos que separan clave de valor van seguidos de espacio o fin de
    // línea (`a: b`, `a:`), lo que deja intactos los `http://…` y los `12:30` de un valor.
    let corte = l
        .match_indices(':')
        .find(|(i, _)| l[i + 1..].is_empty() || l[i + 1..].starts_with([' ', '\t']))?;
    let clave = l[..corte.0].trim_end();
    if clave.is_empty() || clave.starts_with('-') {
        return None;
    }
    Some((clave.to_string(), &l[corte.0 + 1..]))
}

/// Una edición del bloque por líneas: sustituir `[inicio, fin)` por `lineas` (vacío = borrar).
struct LineEdit {
    inicio: usize,
    fin: usize,
    lineas: Vec<String>,
}

/// Decide si el patch se puede aplicar **quirúrgicamente** y, en tal caso, devuelve sus ediciones.
///
/// `None` = hay que reserializar el bloque entero: o el escaneo por líneas no es fiable
/// (`entradas` es `None`), o alguna clave tocada ocupa más de una línea.
fn plan_surgical(
    patch: &crate::types::FrontmatterPatch,
    entradas: Option<&[TopLevelEntry]>,
    claves: &[String],
) -> Option<Vec<LineEdit>> {
    let mut edits: Vec<LineEdit> = Vec::new();
    let mut anexos: Vec<String> = Vec::new();
    for (clave, valor) in &patch.0 {
        let existe = claves.iter().any(|k| k == clave);
        match (existe, valor) {
            // Borrar una clave que no está es un no-op: no necesita ni localizar ni reserializar.
            (false, None) => continue,
            // Clave nueva: se añade una línea al final del bloque.
            (false, Some(v)) => {
                entradas?;
                anexos.extend(render_entry(clave, v));
            }
            (true, _) => {
                let entrada = entradas?.iter().find(|e| &e.clave == clave)?;
                if entrada.fin != entrada.inicio + 1 {
                    // Valor multilínea (mapa/lista en block style, block scalar): tocarlo es
                    // tocar la estructura entera → reserialización.
                    return None;
                }
                edits.push(LineEdit {
                    inicio: entrada.inicio,
                    fin: entrada.fin,
                    lineas: valor
                        .as_ref()
                        .map(|v| render_entry(clave, v))
                        .unwrap_or_default(),
                });
            }
        }
    }
    if !anexos.is_empty() {
        let n = entradas.map_or(0, |e| e.iter().map(|e| e.fin).max().unwrap_or(0));
        edits.push(LineEdit {
            inicio: usize::MAX,
            fin: n,
            lineas: anexos,
        });
    }
    Some(edits)
}

/// Serializa un par `clave: valor` a las líneas YAML que le corresponden.
fn render_entry(clave: &str, valor: &Yaml) -> Vec<String> {
    let mut map = serde_yaml::Mapping::new();
    map.insert(Yaml::String(clave.to_string()), valor.clone());
    dump_mapping(&map).split('\n').map(str::to_string).collect()
}

/// Aplica las ediciones sobre el texto del bloque, de atrás hacia delante para que los índices de
/// línea sigan siendo válidos. El anexo (`inicio == usize::MAX`) se resuelve primero: va al final
/// de la última entrada, antes de los comentarios de cola.
fn apply_line_edits(texto: &str, edits: &[LineEdit]) -> String {
    let mut lineas: Vec<String> = if texto.trim().is_empty() {
        Vec::new()
    } else {
        texto.split('\n').map(str::to_string).collect()
    };
    let mut edits: Vec<&LineEdit> = edits.iter().collect();
    edits.sort_by_key(|e| std::cmp::Reverse(e.inicio));
    for e in edits {
        let inicio = e.inicio.min(e.fin).min(lineas.len());
        let fin = e.fin.min(lineas.len());
        lineas.splice(inicio..fin, e.lineas.iter().cloned());
    }
    lineas.join("\n")
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
