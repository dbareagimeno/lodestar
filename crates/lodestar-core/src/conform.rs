//! Validación genérica: el **catálogo mínimo** de `ARCHITECTURE.md §20.9` (E16-H05).
//!
//! La pregunta que responde este módulo es *«¿puede Lodestar interpretar y modificar este
//! documento de forma consistente y segura?»*, **no** *«¿cumple una especificación documental?»*.
//! Con ese giro se retiró el catálogo OKF entero (`OKF-FM01` —la falta de frontmatter dejó de ser
//! un error—, `OKF-TYPE`, `REC-TITLE`, `REC-DESC`, `FMT-TAGS`, `FMT-TS`, `BODY-STRUCT`, `ORPHAN`,
//! `OKF-IDX`, `OKF-LOG`) y se renombró lo que sobrevive: `OKF-FM02` → `FM-UNCLOSED`,
//! `OKF-FM03` → `FM-YAML-INVALID`, `OKF-CONFLICT` → `DOC-CONFLICT-MARKER`.
//!
//! Siguen aquí `LINK-STUB`/`LINK-REL` hasta E17, que los sustituye por
//! `LINK-TARGET-MISSING`/`LINK-ESCAPES-WORKSPACE`/`LINK-CASE-MISMATCH`.
//!
//! Mensajes en español canónico. El core produce `code` + `targets` + `range`, que es lo que una
//! fachada necesita para localizar y para señalar el sitio.

use std::collections::BTreeMap;

use once_cell::sync::Lazy;
use regex::Regex;

use crate::model::{self, Parsed};
use crate::types::{Check, CheckCode, FileMap, FmError, Range, RelPath, Severity};

static CONFLICT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^(<{7}|={7}|>{7}|\|{7})").unwrap());

/// Contexto de análisis necesario para validar un fichero (datos ya computados por `analyze`).
pub(crate) struct ConformCtx<'a> {
    pub files: &'a FileMap,
    pub out: &'a BTreeMap<RelPath, Vec<RelPath>>,
}

/// Valida un fichero y devuelve sus `Check` del catálogo mínimo de `§20.9`.
pub(crate) fn validate_file(
    path: &RelPath,
    parsed: &Parsed,
    raw: &str,
    ctx: &ConformCtx,
) -> Vec<Check> {
    let mut out: Vec<Check> = Vec::new();

    // DOC-CONFLICT-MARKER (hard-fail): marcadores de merge en cuerpo o frontmatter.
    if CONFLICT_RE.is_match(raw) {
        out.push(Check::new(
            Severity::Err,
            CheckCode::DocConflictMarker,
            "Hay marcadores de conflicto de merge sin resolver.",
            vec![path.clone()],
        ));
    }

    // Frontmatter no interpretable: es lo único que impide leer la metadata del documento, así que
    // corta el resto de la validación (como hacía el port del prototipo).
    match &parsed.fm_err {
        Some(FmError::Unclosed) => {
            out.push(Check::new(
                Severity::Err,
                CheckCode::FmUnclosed,
                "El bloque de metadatos no está cerrado.",
                vec![path.clone()],
            ));
            return out;
        }
        Some(FmError::Malformed(e)) => {
            let mut check = Check::new(
                Severity::Err,
                CheckCode::FmYamlInvalid,
                format!("Los metadatos tienen un error de formato: {e}"),
                vec![path.clone()],
            );
            check.range = block_line_range(raw);
            out.push(check);
            return out;
        }
        None => {}
    }

    // E16-H05: la AUSENCIA de frontmatter ya no diagnostica nada (`OKF-FM01` retirado), ni su
    // contenido (`OKF-TYPE`/`REC-*`/`FMT-*` retirados: el frontmatter es metadata arbitraria del
    // usuario, no un formato de Lodestar).

    // LINK-STUB (info): destinos salientes que no existen como fichero.
    let empty = Vec::new();
    let outs = ctx.out.get(path).unwrap_or(&empty);
    let dang: Vec<RelPath> = outs
        .iter()
        .filter(|t| !ctx.files.contains_key(*t))
        .cloned()
        .collect();
    if !dang.is_empty() {
        // El destino no existe: no hay frontmatter ni cuerpo del que derivar título, así que la
        // cadena de `derived_title` se resuelve por su último eslabón (el nombre del fichero).
        let titles: Vec<String> = dang
            .iter()
            .map(|t| model::derived_title(None, "", t))
            .collect();
        let verb = if dang.len() == 1 {
            "enlace lleva"
        } else {
            "enlaces llevan"
        };
        out.push(Check::new(
            Severity::Info,
            CheckCode::LinkStub,
            format!(
                "{} {} a páginas que aún no existen: {}",
                dang.len(),
                verb,
                titles.join(", ")
            ),
            dang,
        ));
    }

    // LINK-REL (info): enlaces relativos.
    if !model::raw_rel_links(&parsed.body).is_empty() {
        out.push(Check::new(
            Severity::Info,
            CheckCode::LinkRel,
            "Hay enlaces relativos; es mejor usar la ruta completa /…",
            vec![path.clone()],
        ));
    }

    // E16-H02: `ORPHAN` ya NO se emite. El aislamiento (`Analysis::isolated`) es una propiedad
    // consultable del grafo, no un diagnóstico (`§20.7`). E16-H05: `BODY-STRUCT` tampoco — la
    // estructura de encabezados es cosa del usuario.

    out
}

/// Rango de líneas (1-based, ambas inclusive) del **contenido** del bloque de frontmatter de
/// `raw`, con los delimitadores `---` excluidos: la traducción a líneas del `span` de
/// [`crate::types::ParsedFrontmatter`] que `§20.9` pide para `FM-YAML-INVALID`.
///
/// `None` si el documento no tiene bloque cerrado o si el bloque está vacío (no hay línea de
/// contenido que señalar).
fn block_line_range(raw: &str) -> Option<Range> {
    let model::SplitFront::Bloque { span, .. } = model::split_front(raw) else {
        return None;
    };
    if span.is_empty() {
        return None;
    }
    let linea_de = |offset: usize| (raw[..offset].matches('\n').count() + 1) as u32;
    Some(Range {
        start_line: linea_de(span.start),
        end_line: linea_de(span.end),
    })
}
