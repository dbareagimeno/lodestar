//! Motor de indexación: extracción de filas desde el core y upsert/delete transaccional
//! (`ARCHITECTURE.md §5`). El store **no reimplementa checks**: los diagnostics locales salen
//! de `core::analyze`; los de enlace (no locales) se **sintetizan** al leer (ver `synth`).

use rusqlite::{params, Transaction};

use lodestar_core::links;
use lodestar_core::model;
use lodestar_core::types::{CheckCode, FileMap, Inventory, RelPath, Severity};
use lodestar_core::DocumentSet;

use crate::error::StoreError;

/// ¿Es un diagnóstico de **enlace**? Esos no se materializan: dependen del inventario entero
/// (crear un fichero repara el enlace roto de otro documento), así que materializarlos obligaría a
/// invalidar en cascada. Se sintetizan al leer (`synth::link_diagnostics`, `§10` fila 10).
pub(crate) fn es_de_enlace(code: CheckCode) -> bool {
    matches!(
        code,
        CheckCode::LinkTargetMissing
            | CheckCode::LinkEscapesWorkspace
            | CheckCode::LinkCaseMismatch
    )
}

/// Diagnostics **locales** de un fichero (todos menos los de enlace, que se sintetizan al leer).
///
/// `ORPHAN` desapareció del filtro con E16-H02: el core ya no lo emite (el aislamiento es una
/// propiedad del grafo, no un diagnóstico). E17-H03 sustituyó el filtro de `LINK-STUB` por el de
/// la familia entera de enlaces.
///
/// Se computan con el core (autoridad) sobre un workspace de un solo fichero: como los checks
/// locales dependen solo del contenido propio, el resultado es idéntico al del workspace completo.
fn local_diagnostics(path: &RelPath, raw: &str) -> Vec<lodestar_core::types::Check> {
    let mut fm = FileMap::new();
    fm.insert(path.clone(), raw.to_string());
    let doc_set = DocumentSet::from_files(fm);
    doc_set
        .analyze()
        .diagnostics
        .get(path)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| !es_de_enlace(c.code))
        .collect()
}

fn level_str(level: Severity) -> &'static str {
    match level {
        Severity::Pass => "pass",
        Severity::Info => "info",
        Severity::Warn => "warn",
        Severity::Err => "err",
    }
}

/// Extrae las etiquetas (solo si `tags` es una lista; escalares → string).
fn extract_tags(fm: &lodestar_core::types::ParsedFrontmatter) -> Vec<String> {
    match fm.get_key("tags") {
        Some(serde_yaml::Value::Sequence(items)) => items
            .iter()
            .filter_map(|v| match v {
                serde_yaml::Value::String(s) => Some(s.clone()),
                serde_yaml::Value::Number(n) => Some(n.to_string()),
                serde_yaml::Value::Bool(b) => Some(b.to_string()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

/// Borra todas las filas materializadas de un path (en todas las tablas).
pub(crate) fn delete_file(tx: &Transaction, path: &RelPath) -> Result<(), StoreError> {
    let p = path.as_str();
    tx.execute("DELETE FROM files WHERE path = ?1", params![p])?;
    tx.execute("DELETE FROM links WHERE src = ?1", params![p])?;
    tx.execute("DELETE FROM tags WHERE path = ?1", params![p])?;
    tx.execute("DELETE FROM diagnostics WHERE path = ?1", params![p])?;
    tx.execute("DELETE FROM files_fts WHERE path = ?1", params![p])?;
    Ok(())
}

/// Upsert de un fichero: borra sus filas previas e inserta las nuevas (files/links/tags/diagnostics/fts).
pub(crate) fn upsert_file(
    tx: &Transaction,
    path: &RelPath,
    raw: &str,
    mtime: i64,
    size: i64,
) -> Result<(), StoreError> {
    delete_file(tx, path)?;

    let parsed = model::parse_file(path.as_str(), raw);
    let fm = parsed.frontmatter.clone().unwrap_or_default();
    let hash = blake3::hash(raw.as_bytes());
    // La cache materializa el frontmatter ARBITRARIO tal cual (E16-H01): el `value` YAML entero,
    // no una proyección de campos conocidos.
    let fm_json = serde_json::to_string(&fm.value).unwrap_or_else(|_| "{}".to_string());
    let p = path.as_str();

    tx.execute(
        r#"INSERT INTO files
           (path, type, title, description, status, resource,
            frontmatter_json, body, raw, hash, mtime, size)
           VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12)"#,
        params![
            p,
            fm.get_text("type"),
            fm.get_text("title"),
            fm.get_text("description"),
            fm.get_text("status"),
            fm.get_text("resource"),
            fm_json,
            parsed.body,
            raw,
            hash.as_bytes().as_slice(),
            mtime,
            size,
        ],
    )?;

    tx.execute(
        "INSERT INTO files_fts (path, title, description, body) VALUES (?1,?2,?3,?4)",
        params![
            p,
            fm.get_text("title").unwrap_or_default(),
            fm.get_text("description").unwrap_or_default(),
            parsed.body,
        ],
    )?;

    // Enlaces salientes: SIEMPRE del cuerpo, para cualquier documento (E16-H02 — el core hace lo
    // mismo desde `compute_analysis`, sin ramas por basename).
    //
    // Se materializan las **aristas del grafo**: los destinos internos (documentos y fantasmas),
    // con su path ya normalizado, que es lo que consultan `backlinks`/`isolated`/`dangling`/
    // `blast_radius`. Un enlace externo, un anchor propio o uno a un fichero del proyecto no
    // conectan con ningún documento y no son filas de esta tabla (`§20.7`). El inventario es
    // irrelevante para el path resuelto —`Document` y `Missing` llevan el mismo destino
    // normalizado—, así que basta con el fichero que se está indexando: la extracción sigue siendo
    // por-fichero e incremental.
    let inventario = Inventory::default();
    let mut vistos: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    for raw_link in links::extract_links(&parsed.body) {
        let resuelto = links::resolve(&raw_link, path, &inventario);
        let Some(dst) = resuelto.target.internal_path() else {
            continue;
        };
        // Una arista es única por (src, dst, href): enlazar dos veces con el MISMO href no
        // duplica la fila (la tabla es la adyacencia, no la lista de enlaces del documento).
        if !vistos.insert(format!("{dst}\u{0}{}", resuelto.href)) {
            continue;
        }
        tx.execute(
            "INSERT INTO links (src, dst, href) VALUES (?1,?2,?3)",
            params![p, dst.as_str(), resuelto.href],
        )?;
    }

    // Etiquetas.
    for tag in extract_tags(&fm) {
        tx.execute(
            "INSERT INTO tags (path, tag) VALUES (?1, ?2)",
            params![p, tag],
        )?;
    }

    // Diagnostics locales (el core es la autoridad; los de enlace se sintetizan al leer).
    for check in local_diagnostics(path, raw) {
        let targets: Vec<String> = check
            .targets
            .iter()
            .map(|t| t.as_str().to_string())
            .collect();
        let targets_json = serde_json::to_string(&targets).unwrap_or_else(|_| "[]".to_string());
        tx.execute(
            "INSERT INTO diagnostics (path, code, level, msg, targets_json) VALUES (?1,?2,?3,?4,?5)",
            params![
                p,
                check.code.as_str(),
                level_str(check.level),
                check.msg,
                targets_json,
            ],
        )?;
    }

    Ok(())
}
