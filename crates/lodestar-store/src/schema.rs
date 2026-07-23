//! DDL y apertura de la cache (`ARCHITECTURE.md §5`, `§10` fila 10). El store es el
//! **dueño único del DDL**. Migraciones por `user_version`: si no coincide, rebuild limpio.

use rusqlite::Connection;

use crate::error::StoreError;

/// Versión del esquema de cache. Un bump fuerza reconstrucción total (la cache es desechable).
///
/// - `1` — esquema de v0.2 (incluía la tabla git `commit_conformance`).
/// - `2` — E15-H01: git desaparece del repo; el DDL pierde `commit_conformance`. Una cache de v0.2
///   se detecta antigua por este número y se reconstruye limpia.
/// - `3` — E16-H02: ningún nombre de fichero activa reglas especiales. El DDL pierde `files.kind`
///   (todos los `.md` son documentos) y `links.src_is_index` (la pertenencia por índices no
///   existe).
pub const USER_VERSION: i64 = 3;

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
///   (permite servir el `FileMap` exacto vía `ConceptStore`) + `hash` blake3 + `mtime`/`size`.
///   **Sin `kind`** desde E16-H02: todos los `.md` son documentos.
/// - `links`: una sola tabla `(src, dst, href)`. **Sin `src_is_index`** desde E16-H02: un enlace
///   desde un `index.md` es una arista como cualquier otra.
/// - `tags`: `(path, tag)`.
/// - `diagnostics`: solo checks **locales** (una columna por campo del `Check`).
/// - `files_fts`: FTS5 sobre `(title, description, body)` como acelerador (nunca único pre-filtro).
pub(crate) fn create_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS files (
            path             TEXT PRIMARY KEY,
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
            href         TEXT NOT NULL
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
        "#,
    )?;
    Ok(())
}

/// Valida que el esquema **real** en disco coincide con el esperado por esta versión del código.
///
/// El check por `user_version` no basta: un build antiguo pudo escribir `user_version=1` con un
/// esquema distinto (p. ej. una tabla `files` sin la columna `hash`). Como `create_schema` es
/// `IF NOT EXISTS`, esa tabla vieja sobrevive y el upsert revienta con «no column named hash».
/// Aquí preparamos un `SELECT` con **todas** las columnas esperadas de cada tabla (con `LIMIT 0`,
/// sin leer filas): si algún `prepare` falla, el esquema ha derivado y hay que reconstruirlo.
/// Es barato — solo compila las sentencias, no ejecuta consultas.
pub(crate) fn schema_is_current(conn: &Connection) -> Result<bool, StoreError> {
    // Cada entrada lista las columnas que el resto del store da por hechas.
    let probes = [
        "SELECT path, type, title, description, status, resource, \
         frontmatter_json, body, raw, hash, mtime, size FROM files LIMIT 0",
        "SELECT src, dst, href FROM links LIMIT 0",
        "SELECT path, tag FROM tags LIMIT 0",
        "SELECT path, code, level, msg, targets_json FROM diagnostics LIMIT 0",
        "SELECT path, title, description, body FROM files_fts LIMIT 0",
    ];
    for sql in probes {
        if conn.prepare(sql).is_err() {
            return Ok(false);
        }
    }
    Ok(true)
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
        -- Legado de v0.2 (tabla git, retirada en E15-H01): se dropea para que una cache antigua
        -- reconstruida no arrastre la tabla huérfana.
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
