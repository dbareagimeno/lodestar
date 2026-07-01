//! Motor de indexación: extracción de filas desde el core y upsert/delete transaccional
//! (`ARCHITECTURE.md §5`). El store **no reimplementa checks**: los diagnostics locales salen
//! de `core::analyze`; ORPHAN/LINK-STUB (no locales) se **sintetizan** al leer (ver `synth`).

use rusqlite::{params, Transaction};

use lodestar_core::model;
use lodestar_core::types::{CheckCode, FileKind, FileMap, RelPath, Severity};
use lodestar_core::Bundle;

use crate::error::StoreError;

/// Diagnostics **locales** de un fichero (todos menos ORPHAN/LINK-STUB, que se sintetizan).
///
/// Se computan con el core (autoridad) sobre un bundle de un solo fichero: como los checks
/// locales dependen solo del contenido propio, el resultado es idéntico al del bundle completo.
fn local_diagnostics(path: &RelPath, raw: &str) -> Vec<lodestar_core::types::Check> {
    let mut fm = FileMap::new();
    fm.insert(path.clone(), raw.to_string());
    let bundle = Bundle::from_files(fm);
    bundle
        .analyze()
        .per_file
        .get(path)
        .cloned()
        .unwrap_or_default()
        .into_iter()
        .filter(|c| !matches!(c.code, CheckCode::Orphan | CheckCode::LinkStub))
        .collect()
}

fn kind_str(kind: FileKind) -> &'static str {
    match kind {
        FileKind::Concept => "concept",
        FileKind::Index => "index",
        FileKind::Log => "log",
    }
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
fn extract_tags(fm: &lodestar_core::types::Frontmatter) -> Vec<String> {
    match &fm.tags {
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
    let kind = parsed.kind;
    let fm = parsed.fm.clone().unwrap_or_default();
    let hash = blake3::hash(raw.as_bytes());
    let fm_json = serde_json::to_string(&fm).unwrap_or_else(|_| "{}".to_string());
    let p = path.as_str();

    tx.execute(
        r#"INSERT INTO files
           (path, kind, type, title, description, status, resource,
            frontmatter_json, body, raw, hash, mtime, size)
           VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13)"#,
        params![
            p,
            kind_str(kind),
            fm.r#type,
            fm.title,
            fm.description,
            fm.status,
            fm.resource,
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
            fm.title.clone().unwrap_or_default(),
            fm.description.clone().unwrap_or_default(),
            parsed.body,
        ],
    )?;

    // Enlaces salientes. Index usa el raw entero; concept usa el cuerpo; log no enlaza (como el core).
    let (link_source, is_index) = match kind {
        FileKind::Index => (raw, 1i64),
        FileKind::Concept => (parsed.body.as_str(), 0i64),
        FileKind::Log => ("", 0i64),
    };
    if !link_source.is_empty() {
        for (href, dst) in model::out_links_with_href(p, link_source) {
            tx.execute(
                "INSERT INTO links (src, dst, href, src_is_index) VALUES (?1,?2,?3,?4)",
                params![p, dst, href, is_index],
            )?;
        }
    }

    // Etiquetas.
    for tag in extract_tags(&fm) {
        tx.execute(
            "INSERT INTO tags (path, tag) VALUES (?1, ?2)",
            params![p, tag],
        )?;
    }

    // Diagnostics locales (el core es la autoridad; ORPHAN/LINK-STUB se sintetizan).
    for check in local_diagnostics(path, raw) {
        let targets: Vec<String> = check.targets.iter().map(|t| t.as_str().to_string()).collect();
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
