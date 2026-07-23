//! Motor de indexación: extracción de filas desde el core y upsert/delete transaccional
//! (`ARCHITECTURE.md §5`). El store **no reimplementa checks**: los diagnostics locales salen
//! de `core::analyze`; `LINK-STUB` (no local) se **sintetiza** al leer (ver `synth`).

use rusqlite::{params, Transaction};

use lodestar_core::model;
use lodestar_core::types::{CheckCode, FileMap, RelPath, Severity};
use lodestar_core::DocumentSet;

use crate::error::StoreError;

/// Diagnostics **locales** de un fichero (todos menos `LINK-STUB`, que se sintetiza al leer).
///
/// `ORPHAN` desapareció del filtro con E16-H02: el core ya no lo emite (el aislamiento es una
/// propiedad del grafo, no un diagnóstico), así que no hay nada que filtrar.
///
/// Se computan con el core (autoridad) sobre un workspace de un solo fichero: como los checks
/// locales dependen solo del contenido propio, el resultado es idéntico al del workspace completo.
fn local_diagnostics(path: &RelPath, raw: &str) -> Vec<lodestar_core::types::Check> {
    let mut fm = FileMap::new();
    fm.insert(path.clone(), raw.to_string());
    let doc_set = DocumentSet::from_files(fm);
    doc_set
        .analyze()
        .per_file
        .get(path)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| !matches!(c.code, CheckCode::LinkStub))
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
    for (href, dst) in model::out_links_with_href(p, &parsed.body) {
        tx.execute(
            "INSERT INTO links (src, dst, href) VALUES (?1,?2,?3)",
            params![p, dst, href],
        )?;
    }

    // Etiquetas.
    for tag in extract_tags(&fm) {
        tx.execute(
            "INSERT INTO tags (path, tag) VALUES (?1, ?2)",
            params![p, tag],
        )?;
    }

    // Diagnostics locales (el core es la autoridad; `LINK-STUB` se sintetiza al leer).
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
