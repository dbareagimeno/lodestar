//! DDL y apertura de la cache (`ARCHITECTURE.md §5`, `§20.12`).
//!
//! La cache es derivada y desechable: cualquier versión o forma incompatible se elimina y se
//! vuelve a crear. No existe una migración de filas, y el Markdown del workspace nunca se escribe.

use rusqlite::Connection;
use std::collections::BTreeMap;

use crate::error::StoreError;

/// Versión del esquema vNext de E35-H02. Un bump fuerza reconstrucción total.
pub const USER_VERSION: i64 = 6;

pub(crate) fn apply_pragmas(conn: &Connection) -> Result<(), StoreError> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    Ok(())
}

pub(crate) fn read_user_version(conn: &Connection) -> Result<i64, StoreError> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

pub(crate) fn set_user_version(conn: &Connection) -> Result<(), StoreError> {
    conn.pragma_update(None, "user_version", USER_VERSION)?;
    Ok(())
}

/// Crea el esquema relacional vNext. FTS es contentless: `documents` es la única copia completa
/// SQLite del contenido indexado y su `rowid` es el `doc_id` estable de cada documento.
pub(crate) fn create_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS documents (
            doc_id           INTEGER PRIMARY KEY,
            path             TEXT UNIQUE NOT NULL,
            title            TEXT NOT NULL DEFAULT '',
            body             TEXT NOT NULL DEFAULT '',
            frontmatter_json TEXT NOT NULL DEFAULT '{}',
            frontmatter_text TEXT NOT NULL DEFAULT '',
            content_hash     BLOB NOT NULL,
            mtime            INTEGER NOT NULL DEFAULT 0,
            size             INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS fields (
            field_id   INTEGER PRIMARY KEY,
            field_path TEXT UNIQUE NOT NULL
        );

        CREATE TABLE IF NOT EXISTS metadata (
            doc_id      INTEGER NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
            field_id    INTEGER NOT NULL REFERENCES fields(field_id),
            value_json  TEXT NOT NULL,
            value_type  TEXT NOT NULL,
            PRIMARY KEY (doc_id, field_id)
        );
        CREATE INDEX IF NOT EXISTS idx_metadata_doc ON metadata(doc_id);
        CREATE INDEX IF NOT EXISTS idx_metadata_field ON metadata(field_id);

        CREATE TABLE IF NOT EXISTS other_files (
            path TEXT PRIMARY KEY
        );

        CREATE TABLE IF NOT EXISTS links (
            link_id       INTEGER PRIMARY KEY,
            source_doc_id INTEGER NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
            target_doc_id INTEGER REFERENCES documents(doc_id) ON DELETE SET NULL,
            raw_href      TEXT NOT NULL,
            target_kind   TEXT NOT NULL,
            target_path   TEXT,
            fragment      TEXT,
            resolved      INTEGER NOT NULL,
            is_edge       INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_links_target_doc ON links(target_doc_id);
        CREATE INDEX IF NOT EXISTS idx_links_target_path ON links(target_path);
        CREATE INDEX IF NOT EXISTS idx_links_source_doc ON links(source_doc_id);

        CREATE TABLE IF NOT EXISTS diagnostics (
            diagnostic_id INTEGER PRIMARY KEY,
            doc_id        INTEGER NOT NULL REFERENCES documents(doc_id) ON DELETE CASCADE,
            code          TEXT NOT NULL,
            severity      TEXT NOT NULL,
            message       TEXT NOT NULL,
            range_json    TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_diag_doc ON diagnostics(doc_id);
        CREATE INDEX IF NOT EXISTS idx_diag_severity ON diagnostics(severity);

        CREATE VIRTUAL TABLE IF NOT EXISTS documents_fts USING fts5(
            path UNINDEXED, title, body, frontmatter_text,
            content='', columnsize=0
        );
        "#,
    )?;
    Ok(())
}

fn canonical_sql(sql: &str) -> String {
    sql.split_whitespace()
        .collect::<String>()
        .to_ascii_lowercase()
        .replace('"', "'")
}

fn schema_objects(conn: &Connection) -> Result<BTreeMap<String, (String, String)>, StoreError> {
    let mut stmt = conn.prepare("SELECT name, type, COALESCE(sql, '') FROM sqlite_schema")?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
        ))
    })?;
    let mut objects = BTreeMap::new();
    for row in rows {
        let (name, kind, sql) = row?;
        objects.insert(name, (kind, canonical_sql(&sql)));
    }
    Ok(objects)
}

fn table_shape(conn: &Connection, table: &str) -> Result<String, StoreError> {
    let quoted = table.replace('"', "\"\"");
    let mut shape = String::new();
    let mut stmt = conn.prepare(&format!("PRAGMA table_info(\"{quoted}\")"))?;
    let rows = stmt.query_map([], |row| {
        Ok(format!(
            "c:{}:{}:{}:{}:{}:{};",
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, i64>(3)?,
            row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            row.get::<_, i64>(5)?
        ))
    })?;
    for row in rows {
        shape.push_str(&row?);
    }

    let mut stmt = conn.prepare(&format!("PRAGMA foreign_key_list(\"{quoted}\")"))?;
    let rows = stmt.query_map([], |row| {
        Ok(format!(
            "f:{}:{}:{}:{}:{}:{}:{}:{};",
            row.get::<_, i64>(0)?,
            row.get::<_, i64>(1)?,
            row.get::<_, String>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, Option<String>>(4)?.unwrap_or_default(),
            row.get::<_, String>(5)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(7)?
        ))
    })?;
    for row in rows {
        shape.push_str(&row?);
    }

    let mut stmt = conn.prepare(&format!("PRAGMA index_list(\"{quoted}\")"))?;
    let rows = stmt.query_map([], |row| {
        Ok((
            row.get::<_, i64>(0)?,
            row.get::<_, String>(1)?,
            row.get::<_, i64>(2)?,
            row.get::<_, String>(3)?,
            row.get::<_, i64>(4)?,
        ))
    })?;
    let indexes = rows.collect::<Result<Vec<_>, _>>()?;
    for (seq, name, unique, origin, partial) in indexes {
        shape.push_str(&format!("i:{seq}:{name}:{unique}:{origin}:{partial}:"));
        let index_quoted = name.replace('"', "\"\"");
        let mut info = conn.prepare(&format!("PRAGMA index_info(\"{index_quoted}\")"))?;
        let columns = info.query_map([], |row| {
            Ok(format!(
                "{}:{}:{};",
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, Option<String>>(2)?.unwrap_or_default()
            ))
        })?;
        for column in columns {
            shape.push_str(&column?);
        }
    }
    Ok(shape)
}

/// Comprueba el DDL real, no solo `user_version`: una cache manipulada con la misma versión también
/// debe reconstruirse de forma limpia. La referencia se crea con el mismo constructor, y solo se
/// se exige igualdad del conjunto de objetos: una tabla, vista, trigger o índice ajeno puede
/// alterar la semántica del escritor y por tanto invalida la cache junto con cualquier diferencia
/// de forma de los objetos esperados.
pub(crate) fn schema_is_current(conn: &Connection) -> Result<bool, StoreError> {
    let reference = Connection::open_in_memory()?;
    apply_pragmas(&reference)?;
    create_schema(&reference)?;
    let expected = schema_objects(&reference)?;
    let actual = schema_objects(conn)?;
    if actual.keys().ne(expected.keys()) {
        return Ok(false);
    }
    for (name, expected_shape) in &expected {
        let Some(actual_shape) = actual.get(name) else {
            return Ok(false);
        };
        if actual_shape != expected_shape {
            return Ok(false);
        }
        if expected_shape.0 == "table"
            && !name.starts_with("documents_fts")
            && !expected_shape.1.contains("createvirtualtable")
            && table_shape(&reference, name)? != table_shape(conn, name)?
        {
            return Ok(false);
        }
    }
    Ok(true)
}

pub(crate) fn truncate_all(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        r#"
        DROP TABLE documents_fts;
        CREATE VIRTUAL TABLE documents_fts USING fts5(
            path UNINDEXED, title, body, frontmatter_text,
            content='', columnsize=0
        );
        DELETE FROM diagnostics;
        DELETE FROM links;
        DELETE FROM metadata;
        DELETE FROM other_files;
        DELETE FROM documents;
        DELETE FROM fields;
        "#,
    )?;
    Ok(())
}
