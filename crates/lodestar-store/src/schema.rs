//! DDL y apertura de la cache (`ARCHITECTURE.md §5`, `§10` fila 10). El store es el
//! **dueño único del DDL**. Migraciones por `user_version`: si no coincide, rebuild limpio.

use rusqlite::Connection;

use crate::error::StoreError;

/// Versión del esquema de cache. Un bump fuerza reconstrucción total (la cache es desechable).
pub const USER_VERSION: i64 = 1;

/// Aplica los `PRAGMA` de sesión (WAL + claves foráneas + busy_timeout).
pub(crate) fn apply_pragmas(conn: &Connection) -> Result<(), StoreError> {
    conn.pragma_update(None, "journal_mode", "WAL")?;
    conn.pragma_update(None, "foreign_keys", "ON")?;
    // WAL admite UN escritor: sin busy_timeout, la segunda conexión concurrente (app abierta +
    // `lodestar reindex`, o dos ventanas) falla al instante con SQLITE_BUSY en vez de esperar.
    conn.busy_timeout(std::time::Duration::from_millis(5000))?;
    Ok(())
}

/// Lee `PRAGMA user_version`.
pub(crate) fn read_user_version(conn: &Connection) -> Result<i64, StoreError> {
    Ok(conn.query_row("PRAGMA user_version", [], |r| r.get(0))?)
}

/// Escribe `PRAGMA user_version`.
pub(crate) fn set_user_version(conn: &Connection) -> Result<(), StoreError> {
    conn.pragma_update(None, "user_version", USER_VERSION)?;
    Ok(())
}

/// Crea el esquema completo si no existe. Idempotente (`IF NOT EXISTS`).
///
/// - `files`: frontmatter promovido a columnas + `frontmatter_json` para el resto + `body` + `raw`
///   (permite servir el `FileMap` exacto vía `ConceptStore`) + `hash` blake3 + `mtime`/`size` + `kind`.
/// - `links`: una sola tabla con flag `src_is_index` (de ahí se deriva `in_index`).
/// - `tags`: `(path, tag)`.
/// - `diagnostics`: solo checks **locales** (una columna por campo del `Check`).
/// - `files_fts`: FTS5 sobre `(title, description, body)` como acelerador (nunca único pre-filtro).
pub(crate) fn create_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS files (
            path             TEXT PRIMARY KEY,
            kind             TEXT NOT NULL,
            type             TEXT,
            title            TEXT,
            description      TEXT,
            status           TEXT,
            resource         TEXT,
            frontmatter_json TEXT NOT NULL DEFAULT '{}',
            body             TEXT NOT NULL DEFAULT '',
            raw              TEXT NOT NULL DEFAULT '',
            hash             BLOB NOT NULL,
            mtime            INTEGER NOT NULL DEFAULT 0,
            size             INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS links (
            src          TEXT NOT NULL,
            dst          TEXT NOT NULL,
            href         TEXT NOT NULL,
            src_is_index INTEGER NOT NULL DEFAULT 0
        );
        CREATE INDEX IF NOT EXISTS idx_links_dst ON links(dst);
        CREATE INDEX IF NOT EXISTS idx_links_src ON links(src);

        CREATE TABLE IF NOT EXISTS tags (
            path TEXT NOT NULL,
            tag  TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_tags_tag ON tags(tag);
        CREATE INDEX IF NOT EXISTS idx_tags_path ON tags(path);

        CREATE TABLE IF NOT EXISTS diagnostics (
            path         TEXT NOT NULL,
            code         TEXT NOT NULL,
            level        TEXT NOT NULL,
            msg          TEXT NOT NULL,
            targets_json TEXT NOT NULL DEFAULT '[]'
        );
        CREATE INDEX IF NOT EXISTS idx_diag_path ON diagnostics(path);
        CREATE INDEX IF NOT EXISTS idx_diag_level ON diagnostics(level);

        CREATE VIRTUAL TABLE IF NOT EXISTS files_fts USING fts5(
            path UNINDEXED, title, description, body
        );

        CREATE TABLE IF NOT EXISTS commit_conformance (
            tree_oid   TEXT PRIMARY KEY,
            hard_fail  INTEGER NOT NULL,
            warn_count INTEGER NOT NULL,
            conform    INTEGER NOT NULL
        );
        "#,
    )?;
    Ok(())
}

/// Elimina el esquema completo (para migrar cuando `user_version` no coincide: rebuild limpio).
pub(crate) fn drop_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        r#"
        DROP TABLE IF EXISTS files_fts;
        DROP TABLE IF EXISTS diagnostics;
        DROP TABLE IF EXISTS tags;
        DROP TABLE IF EXISTS links;
        DROP TABLE IF EXISTS files;
        DROP TABLE IF EXISTS commit_conformance;
        "#,
    )?;
    Ok(())
}

/// Borra todas las filas materializadas (para un rebuild limpio). No toca el esquema.
pub(crate) fn truncate_all(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        r#"
        DELETE FROM files;
        DELETE FROM links;
        DELETE FROM tags;
        DELETE FROM diagnostics;
        DELETE FROM files_fts;
        "#,
    )?;
    Ok(())
}
