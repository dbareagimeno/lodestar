//! Validación genérica: el **catálogo mínimo** de `ARCHITECTURE.md §20.9` (E16-H05).
//!
//! La pregunta que responde este módulo es *«¿puede Lodestar interpretar y modificar este
//! documento de forma consistente y segura?»*, **no** *«¿cumple una especificación documental?»*.
//! Con ese giro se retiró el catálogo OKF entero (`OKF-FM01` —la falta de frontmatter dejó de ser
//! un error—, `OKF-TYPE`, `REC-TITLE`, `REC-DESC`, `FMT-TAGS`, `FMT-TS`, `BODY-STRUCT`, `ORPHAN`,
//! `OKF-IDX`, `OKF-LOG`) y se renombró lo que sobrevive: `OKF-FM02` → `FM-UNCLOSED`,
//! `OKF-FM03` → `FM-YAML-INVALID`, `OKF-CONFLICT` → `DOC-CONFLICT-MARKER`.
//!
//! **E17-H03** retiró de aquí los dos códigos de enlace heredados del prototipo (`LINK-STUB`, el
//! destino inexistente, y `LINK-REL`, «usa la ruta completa /…»): los enlaces se diagnostican en
//! [`crate::links::diagnose`] a partir de la clasificación del destino (`§20.6`), con
//! `LINK-TARGET-MISSING`/`LINK-ESCAPES-WORKSPACE`/`LINK-CASE-MISMATCH`. Este módulo se queda con lo
//! que depende **solo del documento**, sin mirar el inventario.
//!
//! Mensajes en español canónico. El core produce `code` + `targets` + `range`, que es lo que una
//! fachada necesita para localizar y para señalar el sitio.

use once_cell::sync::Lazy;
use regex::Regex;

use crate::model::{self, Parsed};
use crate::types::{Check, CheckCode, FmError, Range, RelPath, Severity};

static CONFLICT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^(<{7}|={7}|>{7}|\|{7})").unwrap());

/// Valida un fichero y devuelve sus `Check` del catálogo mínimo de `§20.9` que dependen **solo de
/// su contenido**. Los de enlace (que necesitan el inventario del workspace) los añade
/// [`crate::links::diagnose`] desde `DocumentSet::analyze`.
pub(crate) fn validate_file(path: &RelPath, parsed: &Parsed, raw: &str) -> Vec<Check> {
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

    // E17-H03: los diagnósticos de enlace (`LINK-TARGET-MISSING`/`LINK-ESCAPES-WORKSPACE`/
    // `LINK-CASE-MISMATCH`) los emite `links::diagnose`: dependen del INVENTARIO del workspace, no
    // del documento, y se derivan de la clasificación del destino, no de un segundo algoritmo.

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
