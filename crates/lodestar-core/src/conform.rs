//! Conformidad: los checks OKF que quedan vivos (`ARCHITECTURE.md §4.1`, `§13.6`). Port de
//! `validateFile`, menos lo que E16-H02 retiró (`OKF-IDX`/`OKF-LOG`, que dependían de la clase de
//! fichero, y `ORPHAN`, que era el aislamiento disfrazado de diagnóstico). La reducción al catálogo
//! mínimo de `§20.9` es E16-H05.
//!
//! Mensajes en español canónico (reproducen el prototipo). La externalización i18n keyed por
//! código es E8-H03; el core ya produce `code` + `targets`, que es lo que la UI necesita para localizar.

use std::collections::BTreeMap;

use once_cell::sync::Lazy;
use regex::Regex;
use serde_yaml::Value as Yaml;

use crate::model::{self, Parsed};
use crate::types::{Check, CheckCode, FileMap, FmError, RelPath, Severity};

static CONFLICT_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^(<{7}|={7}|>{7}|\|{7})").unwrap());
static HEADING_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"(?m)^#{1,6}\s").unwrap());

/// Contexto de análisis necesario para validar un fichero (datos ya computados por `analyze`).
pub(crate) struct ConformCtx<'a> {
    pub files: &'a FileMap,
    pub out: &'a BTreeMap<RelPath, Vec<RelPath>>,
}

/// Valida un fichero y devuelve sus `Check`. Port fiel de `validateFile` + `OKF-CONFLICT` (nuevo).
pub(crate) fn validate_file(
    path: &RelPath,
    parsed: &Parsed,
    raw: &str,
    ctx: &ConformCtx,
) -> Vec<Check> {
    let mut out: Vec<Check> = Vec::new();

    // OKF-CONFLICT (hard-fail): marcadores de merge en cuerpo o frontmatter, en cualquier tipo de fichero.
    if CONFLICT_RE.is_match(raw) {
        out.push(Check::new(
            Severity::Err,
            CheckCode::OkfConflict,
            "Hay marcadores de conflicto de merge sin resolver.",
            vec![path.clone()],
        ));
    }

    // E16-H02: sin ramas por nombre de fichero. `validate_index`/`validate_log` (y con ellas
    // `OKF-IDX`/`OKF-LOG`) desaparecieron: `index.md` y `log.md` se validan como cualquier otro
    // documento. La reducción del catálogo al mínimo de `§20.9` es E16-H05.

    // Errores de frontmatter (early-return como el prototipo).
    match &parsed.fm_err {
        Some(FmError::Unclosed) => {
            out.push(Check::new(
                Severity::Err,
                CheckCode::OkfFm02,
                "El bloque de metadatos no está cerrado.",
                vec![path.clone()],
            ));
            return out;
        }
        Some(FmError::Malformed(e)) => {
            out.push(Check::new(
                Severity::Err,
                CheckCode::OkfFm03,
                format!("Los metadatos tienen un error de formato: {e}"),
                vec![path.clone()],
            ));
            return out;
        }
        None => {}
    }

    // OKF-FM01: sin bloque de frontmatter. El modelo ya NO lo trata como error de parseo
    // (E16-H01: un documento sin frontmatter es válido); el check se deriva aquí de la ausencia,
    // y desaparece del catálogo con E16-H05.
    let Some(fm) = parsed.frontmatter.as_ref() else {
        out.push(Check::new(
            Severity::Err,
            CheckCode::OkfFm01,
            "Falta el bloque de metadatos al inicio de la página.",
            vec![path.clone()],
        ));
        return out;
    };

    // OKF-TYPE (única regla dura): falta `type` → err; presente → pass con el tipo.
    match fm
        .get_text("type")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
    {
        None => out.push(Check::new(
            Severity::Err,
            CheckCode::OkfType,
            "Falta indicar de qué tipo es esta página.",
            vec![path.clone()],
        )),
        Some(t) => out.push(Check::new(
            Severity::Pass,
            CheckCode::OkfType,
            format!("Es una página de tipo «{t}»."),
            vec![path.clone()],
        )),
    }

    // REC-TITLE / REC-DESC (info).
    if fm.get_text("title").map(|s| s.is_empty()).unwrap_or(true) {
        out.push(Check::new(
            Severity::Info,
            CheckCode::RecTitle,
            "Sin título: ponle un nombre legible.",
            vec![path.clone()],
        ));
    }
    if fm
        .get_text("description")
        .map(|s| s.is_empty())
        .unwrap_or(true)
    {
        out.push(Check::new(
            Severity::Info,
            CheckCode::RecDesc,
            "Sin descripción: ayuda a encontrarla y a previsualizarla.",
            vec![path.clone()],
        ));
    }

    // FMT-TAGS (warn): tags presente pero no es lista.
    if let Some(v) = fm.get_key("tags") {
        if yaml_truthy(v) && !matches!(v, Yaml::Sequence(_)) {
            out.push(Check::new(
                Severity::Warn,
                CheckCode::FmtTags,
                "Las etiquetas deberían ir como una lista.",
                vec![path.clone()],
            ));
        }
    }
    // FMT-TS (warn): timestamp presente pero no ISO.
    if let Some(v) = fm.get_key("timestamp") {
        if yaml_truthy(v) && !model::is_iso(v) {
            out.push(Check::new(
                Severity::Warn,
                CheckCode::FmtTs,
                "La fecha no está en el formato estándar.",
                vec![path.clone()],
            ));
        }
    }

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
    // consultable del grafo, no un diagnóstico (`§20.7`).

    // BODY-STRUCT (info): el cuerpo no tiene encabezados.
    if !HEADING_RE.is_match(&parsed.body) {
        out.push(Check::new(
            Severity::Info,
            CheckCode::BodyStruct,
            "El cuerpo no tiene apartados; añade encabezados para organizarlo.",
            vec![path.clone()],
        ));
    }

    out
}

/// `true` si un valor YAML es "truthy" al estilo JS (no null, no cadena vacía, no lista vacía, no false/0).
fn yaml_truthy(v: &Yaml) -> bool {
    match v {
        Yaml::Null => false,
        Yaml::Bool(b) => *b,
        Yaml::String(s) => !s.is_empty(),
        Yaml::Sequence(s) => !s.is_empty(),
        Yaml::Number(n) => n.as_f64().map(|x| x != 0.0).unwrap_or(true),
        _ => true,
    }
}
