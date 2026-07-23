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
/// - `4` — E18-H01/H02 (store v2, `§20.12`): `files` → `documents` (fuera las columnas OKF
///   `type`/`status`/`description`/`resource`; `hash` → `content_hash`; `title` es el título
///   derivado). Nueva tabla `metadata` (frontmatter genérico por field path). Fuera `tags`. `links`
///   pasa a `(source_path, raw_href, target_kind, target_path, fragment, resolved)` con TODAS las
///   clases de destino; `diagnostics` gana `range_json` y renombra sus columnas. `other_files`
///   materializa el inventario de ficheros del proyecto (no-`.md`) que clasifica los enlaces
///   `workspaceFile`.
pub const USER_VERSION: i64 = 4;

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
/// Store v2 (`ARCHITECTURE.md §20.12`, E18-H01/H02): la cache deja de espejar el modelo OKF y
/// materializa el modelo genérico.
///
/// - `documents`: título **derivado** (`§20.4`) + `frontmatter_json` (el `value` YAML entero) +
///   `body` + `raw` (sirve el `FileMap` exacto vía `DocumentStore`) + `content_hash` blake3 +
///   `mtime`/`size`. **Sin** las columnas OKF promovidas `type`/`status`/`description`/`resource`.
/// - `metadata`: una fila por propiedad **direccionable** del frontmatter (`ParsedFrontmatter::walk`),
///   mapas intermedios incluidos. Conserva el valor JSON original y su tipo (catálogo cerrado de 6).
/// - `other_files`: los ficheros del proyecto que **no** son documentos (código, imágenes…). Es el
///   inventario que permite clasificar un enlace como `workspaceFile` en vez de `missing` sin
///   inventar un segundo navegador del disco (E18-H02).
/// - `links`: TODAS las clases de destino (`§20.6`) con su clasificación, no solo las aristas del
///   grafo. `target_kind` = discriminante serde de `LinkTarget`; `is_edge` = ¿arista del grafo?
///   (`LinkTarget::internal_path().is_some()`), materializado por el core para no reimplementar
///   `is_markdown` en SQL.
/// - `diagnostics`: solo checks **locales** (los de enlace se sintetizan al leer), con `range_json`.
/// - `files_fts`: FTS5 como acelerador (nunca único pre-filtro). Su rediseño sin campos
///   privilegiados es E18-H03; aquí se conserva para que `fts_candidates` siga acelerando.
pub(crate) fn create_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        r#"
        CREATE TABLE IF NOT EXISTS documents (
            path             TEXT PRIMARY KEY,
            title            TEXT NOT NULL DEFAULT '',
            body             TEXT NOT NULL DEFAULT '',
            raw              TEXT NOT NULL DEFAULT '',
            frontmatter_json TEXT NOT NULL DEFAULT '{}',
            content_hash     BLOB NOT NULL,
            mtime            INTEGER NOT NULL DEFAULT 0,
            size             INTEGER NOT NULL DEFAULT 0
        );

        CREATE TABLE IF NOT EXISTS metadata (
            document_path TEXT NOT NULL,
            field_path    TEXT NOT NULL,
            value_json    TEXT NOT NULL,
            value_type    TEXT NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_metadata_doc ON metadata(document_path);
        CREATE INDEX IF NOT EXISTS idx_metadata_field ON metadata(field_path);

        CREATE TABLE IF NOT EXISTS other_files (
            path TEXT PRIMARY KEY
        );

        CREATE TABLE IF NOT EXISTS links (
            source_path  TEXT NOT NULL,
            raw_href     TEXT NOT NULL,
            target_kind  TEXT NOT NULL,
            target_path  TEXT,
            fragment     TEXT,
            resolved     INTEGER NOT NULL,
            is_edge      INTEGER NOT NULL
        );
        CREATE INDEX IF NOT EXISTS idx_links_target ON links(target_path);
        CREATE INDEX IF NOT EXISTS idx_links_source ON links(source_path);

        CREATE TABLE IF NOT EXISTS diagnostics (
            document_path TEXT NOT NULL,
            code          TEXT NOT NULL,
            severity      TEXT NOT NULL,
            message       TEXT NOT NULL,
            range_json    TEXT
        );
        CREATE INDEX IF NOT EXISTS idx_diag_doc ON diagnostics(document_path);
        CREATE INDEX IF NOT EXISTS idx_diag_severity ON diagnostics(severity);

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
        "SELECT path, title, body, raw, frontmatter_json, content_hash, mtime, size \
         FROM documents LIMIT 0",
        "SELECT document_path, field_path, value_json, value_type FROM metadata LIMIT 0",
        "SELECT path FROM other_files LIMIT 0",
        "SELECT source_path, raw_href, target_kind, target_path, fragment, resolved, is_edge \
         FROM links LIMIT 0",
        "SELECT document_path, code, severity, message, range_json FROM diagnostics LIMIT 0",
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
        DROP TABLE IF EXISTS links;
        DROP TABLE IF EXISTS other_files;
        DROP TABLE IF EXISTS metadata;
        DROP TABLE IF EXISTS documents;
        -- Legado de esquemas anteriores: se dropean para que una cache antigua reconstruida no
        -- arrastre tablas huérfanas. `files`/`tags` son de v0.3 (store v1, E16); `commit_conformance`
        -- de v0.2 (tabla git, retirada en E15-H01).
        DROP TABLE IF EXISTS files;
        DROP TABLE IF EXISTS tags;
        DROP TABLE IF EXISTS commit_conformance;
        "#,
    )?;
    Ok(())
}

/// Borra todas las filas materializadas (para un rebuild limpio). No toca el esquema.
pub(crate) fn truncate_all(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        r#"
        DELETE FROM documents;
        DELETE FROM metadata;
        DELETE FROM other_files;
        DELETE FROM links;
        DELETE FROM diagnostics;
        DELETE FROM files_fts;
        "#,
    )?;
    Ok(())
}
