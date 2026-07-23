//! Enlaces Markdown: extracción y resolución (`ARCHITECTURE.md §20.6`, épica E17).
//!
//! Sustituye a `model::LINK_RE`/`resolve_link`/`out_links`/`raw_rel_links`: solo **Markdown
//! estándar** (inline, con fragmento, de referencia, anchors propios y URIs externas), resuelto
//! **únicamente por path** y sin una sola heurística — nada de buscar por basename o título, añadir
//! `.md`, tratar un directorio como `index.md` ni interpretar wikilinks.
//!
//! El parser Markdown (`pulldown-cmark`) es un **detalle de implementación**: ningún tipo suyo
//! asoma en esta API. Lo que sale de aquí son [`RawLink`] (E17-H01) y [`ResolvedLink`] (E17-H02),
//! definidos una sola vez en [`crate::types`] (invariante #4).

use std::ops::Range;

use pulldown_cmark::{Event, LinkType, Options, Parser, RefDefs, Tag, TagEnd};

use crate::types::{Inventory, LinkKind, LinkTarget, RawLink, RelPath, ResolvedLink};

/// Extensiones de Markdown reconocidas al extraer enlaces.
///
/// Son las del render del preview (`crate::render`) más las **notas al pie**. Las dos primeras
/// están aquí para **no inventar enlaces**, que es justo lo que prohíbe `§20.6`:
///
/// - Sin `ENABLE_TASKLISTS`, el `[x]` de un `- [x] hecho` es un enlace corto — y con un
///   `[x]: …` en el documento, un enlace de verdad a un destino que nadie escribió.
/// - Sin `ENABLE_FOOTNOTES`, un `[^1]: nota al pie` es una **definición de referencia** y su
///   `[^1]` un enlace corto al texto de la nota.
/// - `ENABLE_TABLES`/`ENABLE_STRIKETHROUGH` mantienen la lectura del documento alineada con la
///   del preview.
fn opciones() -> Options {
    let mut opts = Options::empty();
    opts.insert(Options::ENABLE_TABLES);
    opts.insert(Options::ENABLE_STRIKETHROUGH);
    opts.insert(Options::ENABLE_TASKLISTS);
    opts.insert(Options::ENABLE_FOOTNOTES);
    opts
}

// ---------------------------------------------------------------------------
// E17-H01 — Extracción
// ---------------------------------------------------------------------------

/// Marco de un enlace abierto mientras se recorren los eventos del parser.
struct Marco {
    /// Rango del enlace **entero** en el cuerpo (`[texto](destino)`).
    rango: Range<usize>,
    /// Forma sintáctica según el parser.
    link_type: LinkType,
    /// Etiqueta de la referencia (`world` en `[hello][world]`); vacía en los inline.
    id: String,
    /// Texto visible acumulado.
    texto: String,
    /// Último byte cubierto por el contenido del enlace: por ahí anda su `]` de cierre.
    fin_texto: usize,
    /// Los enlaces que viven dentro del texto alternativo de una imagen no se emiten.
    emitir: bool,
}

/// Todos los enlaces del cuerpo, en orden de aparición, con su href crudo y el rango de bytes de
/// su destino **relativo a `body`** (E17-H01).
///
/// No cuentan los enlaces dentro de bloques de código (fence o indentado) ni de spans de código,
/// ni los wikilinks/embeds de Obsidian, ni las imágenes (no son enlaces de navegación).
///
/// El `href` se toma **del cuerpo**, no del destino que decodifica el parser: por construcción
/// `body[span] == href`, que es lo que necesitan la reescritura quirúrgica de `move_document`
/// (`§20.11`) y el `range` de los diagnósticos (`§20.9`). En un enlace de **referencia** el destino
/// no está en el sitio del uso sino en su definición, así que el rango cae dentro de ella
/// (`[spec]: ../x.md`) — que es exactamente el byte que hay que reescribir.
pub fn extract_links(body: &str) -> Vec<RawLink> {
    let mut iter = Parser::new_ext(body, opciones()).into_offset_iter();
    // Los eventos se materializan antes de consultar las definiciones de referencia: iterar pide
    // `&mut` y `reference_definitions` pide `&`. Los `Event` toman prestado `body`, no el iterador.
    let eventos: Vec<(Event, Range<usize>)> = iter.by_ref().collect();
    let defs = iter.reference_definitions();

    let mut pila: Vec<Marco> = Vec::new();
    let mut prof_imagen: usize = 0;
    let mut enlaces: Vec<RawLink> = Vec::new();

    for (ev, rango) in eventos {
        match ev {
            Event::Start(Tag::Link { link_type, id, .. }) => {
                pila.push(Marco {
                    fin_texto: rango.start + 1,
                    rango,
                    link_type,
                    id: id.to_string(),
                    texto: String::new(),
                    emitir: prof_imagen == 0,
                });
                continue;
            }
            Event::End(TagEnd::Link) => {
                // El evento de cierre abarca el enlace entero: no puede alimentar `fin_texto`.
                if let Some(marco) = pila.pop() {
                    if marco.emitir {
                        if let Some(enlace) = materializar(body, &marco, defs) {
                            enlaces.push(enlace);
                        }
                    }
                }
                continue;
            }
            _ => {}
        }

        if let Some(marco) = pila.last_mut() {
            marco.fin_texto = marco.fin_texto.max(rango.end);
            match &ev {
                Event::Text(t) | Event::Code(t) => marco.texto.push_str(t),
                Event::SoftBreak | Event::HardBreak => marco.texto.push(' '),
                _ => {}
            }
        }

        match ev {
            Event::Start(Tag::Image { .. }) => prof_imagen += 1,
            Event::End(TagEnd::Image) => prof_imagen = prof_imagen.saturating_sub(1),
            _ => {}
        }
    }

    enlaces
}

/// Convierte un marco cerrado en un [`RawLink`], o `None` si no se puede localizar su destino en
/// el cuerpo (enlace mal formado o definición ausente): la extracción nunca inventa un enlace.
fn materializar(body: &str, marco: &Marco, defs: &RefDefs<'_>) -> Option<RawLink> {
    let span = match marco.link_type {
        LinkType::Inline => span_inline(body, &marco.rango, marco.fin_texto)?,
        LinkType::Autolink | LinkType::Email => span_autolink(body, &marco.rango),
        _ => {
            let def = defs.get(&marco.id)?;
            span_definicion(body, &def.span)?
        }
    };
    // Un rango que no cae en frontera de carácter no es indexable: se descarta el enlace antes de
    // que pueda hacer pánico.
    if !body.is_char_boundary(span.start) || !body.is_char_boundary(span.end) {
        return None;
    }
    Some(RawLink {
        href: body[span.clone()].to_string(),
        text: marco.texto.clone(),
        span,
        kind: clase(marco.link_type),
    })
}

/// El `link_type` del parser traducido al contrato de [`LinkKind`].
///
/// Las variantes `…Unknown` solo las produce un *broken link callback*, que aquí no se instala
/// (una referencia sin definición es texto plano); se traducen a su forma base por totalidad.
fn clase(t: LinkType) -> LinkKind {
    match t {
        LinkType::Inline => LinkKind::Inline,
        LinkType::Reference | LinkType::ReferenceUnknown => LinkKind::Reference,
        LinkType::Collapsed | LinkType::CollapsedUnknown => LinkKind::Collapsed,
        LinkType::Shortcut | LinkType::ShortcutUnknown => LinkKind::Shortcut,
        LinkType::Autolink | LinkType::Email => LinkKind::Autolink,
    }
}

/// Rango del destino de un enlace inline: `[texto](destino "título")`.
///
/// Parte del final del contenido del enlace (que el parser ya delimitó por eventos, así que un
/// `]` dentro de un span de código o de una imagen anidada no confunde a nadie), salta el `](` y
/// aplica las reglas de CommonMark para el destino: forma `<…>` o secuencia sin espacios con
/// paréntesis balanceados.
fn span_inline(body: &str, rango: &Range<usize>, fin_texto: usize) -> Option<Range<usize>> {
    let b = body.as_bytes();
    let fin = rango.end.min(b.len());

    let mut i = fin_texto.min(fin);
    while i < fin && b[i] != b']' {
        i += 1;
    }
    if i >= fin {
        return None;
    }
    i += 1;
    if i >= fin || b[i] != b'(' {
        return None;
    }
    i += 1;
    destino_desde(b, i, fin)
}

/// Rango del destino de un autolink `<https://example.com>`: lo de dentro de los ángulos.
fn span_autolink(body: &str, rango: &Range<usize>) -> Range<usize> {
    let s = &body[rango.clone()];
    if s.len() >= 2 && s.starts_with('<') && s.ends_with('>') {
        (rango.start + 1)..(rango.end - 1)
    } else {
        rango.clone()
    }
}

/// Rango del destino dentro de una **definición** de referencia (`[id]: ../x.md "título"`).
///
/// Es el rango que necesitan los enlaces de referencia, colapsados y cortos: su destino no está
/// donde se usa el enlace sino aquí.
fn span_definicion(body: &str, def: &Range<usize>) -> Option<Range<usize>> {
    let b = body.as_bytes();
    let fin = def.end.min(b.len());
    let mut i = def.start;

    if b.get(i) != Some(&b'[') {
        return None;
    }
    i += 1;
    // La etiqueta acaba en el primer `]` sin escapar (CommonMark).
    while i < fin {
        match b[i] {
            b'\\' => i += 2,
            b']' => break,
            _ => i += 1,
        }
    }
    if i >= fin {
        return None;
    }
    i += 1;
    if b.get(i) != Some(&b':') {
        return None;
    }
    i += 1;
    destino_desde(b, i, fin)
}

/// Destino que empieza en `i` (tras saltar el espacio en blanco que le preceda), acotado por `fin`.
fn destino_desde(b: &[u8], mut i: usize, fin: usize) -> Option<Range<usize>> {
    while i < fin && b[i].is_ascii_whitespace() {
        i += 1;
    }
    if i < fin && b[i] == b'<' {
        let inicio = i + 1;
        let mut j = inicio;
        while j < fin {
            match b[j] {
                b'\\' => j += 2,
                b'>' => return Some(inicio..j),
                _ => j += 1,
            }
        }
        return None;
    }

    let inicio = i;
    let mut prof: usize = 0;
    let mut j = i;
    while j < fin {
        match b[j] {
            b'\\' => {
                j += 2;
                continue;
            }
            b'(' => prof += 1,
            b')' => {
                if prof == 0 {
                    break;
                }
                prof -= 1;
            }
            c if c.is_ascii_whitespace() => break,
            _ => {}
        }
        j += 1;
    }
    Some(inicio..j.min(fin))
}

// ---------------------------------------------------------------------------
// E17-H02 — Resolución y clasificación
// ---------------------------------------------------------------------------

/// Resuelve y clasifica un enlace crudo con los 10 pasos de `§20.6` (E17-H02).
///
/// `from` es el documento **origen** (los paths relativos se resuelven contra su directorio) y
/// `inventory` es lo que el motor sabe que existe. Función total y pura: no toca el disco, así que
/// el veredicto depende solo de sus tres argumentos.
///
/// Decisiones fijadas en E17-H02 (ver `ARCHITECTURE.md §20.6`):
///
/// - Un href **raíz-absoluto** (`/beta.md`) se resuelve contra la **raíz del workspace**.
/// - El **fragmento** vive siempre en [`ResolvedLink::fragment`], nunca dentro del
///   [`LinkTarget`] — también en un anchor propio y en una URI externa.
/// - La **query** (`?v=1`) no se modela: se descarta al resolver el path y sobrevive en el href.
/// - Contener **no** es recortar: si en algún punto de la normalización el destino sube por encima
///   de la raíz, escapa — aunque los `..` sobrantes lo devolvieran a un path que existe.
pub fn resolve(raw: &RawLink, from: &RelPath, inventory: &Inventory) -> ResolvedLink {
    let href = raw.href.as_str();
    let (sin_fragmento, fragment) = match href.find('#') {
        Some(i) => (&href[..i], Some(href[i + 1..].to_string())),
        None => (href, None),
    };

    let target = if tiene_esquema(href) {
        // Paso 3: una URI con esquema se registra entera, fragmento y query incluidos.
        LinkTarget::ExternalUri(href.to_string())
    } else if href.starts_with('#') {
        // Paso 4: anchor del propio documento, sin pasar por el inventario.
        LinkTarget::SelfAnchor(fragment.clone().unwrap_or_default())
    } else {
        let ruta = sin_fragmento.split('?').next().unwrap_or("");
        clasificar(ruta, from, inventory)
    };

    ResolvedLink {
        href: raw.href.clone(),
        text: raw.text.clone(),
        span: raw.span.clone(),
        kind: raw.kind,
        target,
        fragment,
    }
}

/// ¿El href lleva esquema (`https:`, `mailto:`…)?
///
/// Regla de RFC 3986: letra seguida de letras/dígitos/`+`/`-`/`.` y un `:`, **antes** de cualquier
/// `/`, `?` o `#`. Así `notas/a:b.md` sigue siendo un path relativo.
fn tiene_esquema(href: &str) -> bool {
    let mut chars = href.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() => {}
        _ => return false,
    }
    for c in chars {
        match c {
            ':' => return true,
            c if c.is_ascii_alphanumeric() || c == '+' || c == '-' || c == '.' => {}
            _ => return false,
        }
    }
    false
}

/// Pasos 5–9: resolver contra el directorio del origen, normalizar, verificar contención,
/// decodificar y buscar en el inventario.
fn clasificar(ruta: &str, from: &RelPath, inventory: &Inventory) -> LinkTarget {
    // Un href sin path (`` o `?v=1`) designa el propio documento origen.
    if ruta.is_empty() {
        return pertenencia(from.clone(), inventory);
    }

    // Un href raíz-absoluto arranca de la raíz del workspace; el resto, del directorio del origen.
    let (mut partes, segmentos): (Vec<&str>, &str) = match ruta.strip_prefix('/') {
        Some(resto) => (Vec::new(), resto),
        None => {
            let dir = from.as_str();
            let base = match dir.rfind('/') {
                Some(i) => dir[..i].split('/').collect(),
                None => Vec::new(),
            };
            (base, ruta)
        }
    };

    for seg in segmentos.split('/') {
        match seg {
            "" | "." => continue,
            // Contener es CONTAR PROFUNDIDAD, no recortar: si el nivel baja de cero el destino
            // está fuera del workspace, aunque los segmentos siguientes lo devuelvan dentro.
            ".." => {
                if partes.pop().is_none() {
                    return LinkTarget::EscapesWorkspace;
                }
            }
            s => partes.push(s),
        }
    }

    // El percent-decoding se aplica DESPUÉS de interpretar `.`/`..` (RFC 3986): así un `%2e%2e`
    // es el nombre literal de un segmento y no una subida de directorio encubierta.
    let destino = partes
        .iter()
        .map(|s| decodificar_segmento(s))
        .collect::<Vec<_>>()
        .join("/");

    match RelPath::new(&destino) {
        Ok(p) => pertenencia(p, inventory),
        // Solo queda un destino no nombrable como `RelPath`: la **raíz** del workspace (`../` desde
        // un subdirectorio, `./` desde la raíz). No es un documento y no hay path que reportar, así
        // que se clasifica con la única variante sin path. Caso degenerado y consciente.
        Err(_) => LinkTarget::EscapesWorkspace,
    }
}

/// Paso 9: clasificar un destino ya contenido y normalizado contra el inventario.
fn pertenencia(path: RelPath, inventory: &Inventory) -> LinkTarget {
    if inventory.contains_document(&path) {
        LinkTarget::Document(path)
    } else if inventory.contains_file(&path) {
        LinkTarget::WorkspaceFile(path)
    } else {
        LinkTarget::Missing(path)
    }
}

/// Percent-decoding de un segmento **que no puede dejar de ser un segmento**: si lo decodificado
/// introduce un separador o vuelve a ser `.`/`..`, el segmento se deja codificado.
///
/// Sin esta guarda, `docs/%2Fa.md` acabaría siendo `docs/a.md` (un documento que existe) en vez
/// del destino inexistente que es: decodificar resuelve nombres, nunca cambia la estructura del
/// path ya normalizado.
fn decodificar_segmento(seg: &str) -> String {
    let d = percent_decode(seg);
    if d.is_empty() || d == "." || d == ".." || d.contains('/') || d.contains('\\') {
        seg.to_string()
    } else {
        d
    }
}

/// Percent-decoding de un **segmento** de path. Una secuencia `%XX` mal formada, o un resultado
/// que no es UTF-8, se deja tal cual: decodificar nunca puede perder el nombre original.
fn percent_decode(seg: &str) -> String {
    if !seg.contains('%') {
        return seg.to_string();
    }
    let b = seg.as_bytes();
    let mut out: Vec<u8> = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            let hi = (b[i + 1] as char).to_digit(16);
            let lo = (b[i + 2] as char).to_digit(16);
            if let (Some(h), Some(l)) = (hi, lo) {
                out.push((h * 16 + l) as u8);
                i += 3;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8(out).unwrap_or_else(|_| seg.to_string())
}
