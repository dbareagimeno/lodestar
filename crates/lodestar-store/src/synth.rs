//! Síntesis on-demand (`ARCHITECTURE.md §5`, `§10` fila 10): lo que invalidaría en cascada
//! NO se materializa — se deriva al leer vía SQL/CTE. `LINK-STUB`/`ORPHAN` se sintetizan aquí
//! (su definición canónica vive en el core; la paridad lo verifica).
//!
//! Invariante: estas consultas devuelven lo mismo que el `core` equivalente. Si difieren,
//! es **bug de la cache** (gana el core) — lo captura el test de paridad.

use rusqlite::Connection;

use lodestar_core::types::RelPath;

use crate::error::StoreError;

fn rows_to_relpaths(
    conn: &Connection,
    sql: &str,
    p: &[&dyn rusqlite::ToSql],
) -> Result<Vec<RelPath>, StoreError> {
    let mut stmt = conn.prepare(sql)?;
    let iter = stmt.query_map(p, |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for s in iter {
        out.push(RelPath::new(&s?)?);
    }
    Ok(out)
}

/// `hard_fail` = nº de ficheros con algún check `Err` (conteo, no `.max()`; `§10` fila 4).
pub(crate) fn hard_fail(conn: &Connection) -> Result<usize, StoreError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(DISTINCT path) FROM diagnostics WHERE level = 'err'",
        [],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}

/// `warn_count` = nº total de checks `Warn` (suma sobre ficheros).
pub(crate) fn warn_count(conn: &Connection) -> Result<usize, StoreError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM diagnostics WHERE level = 'warn'",
        [],
        |r| r.get(0),
    )?;
    Ok(n as usize)
}

/// Concepts (ficheros no reservados), en orden estable.
pub(crate) fn concepts(conn: &Connection) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        "SELECT path FROM files WHERE kind = 'concept' ORDER BY path",
        &[],
    )
}

/// Paths listados por algún `index.md` (de ahí se deriva `in_index`).
pub(crate) fn in_index(conn: &Connection) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        "SELECT DISTINCT dst FROM links WHERE src_is_index = 1 ORDER BY dst",
        &[],
    )
}

/// Backlinks de un concept (`inn[path]`): concepts que lo enlazan y él existe como concept.
pub(crate) fn backlinks(conn: &Connection, path: &RelPath) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        r#"SELECT DISTINCT l.src
           FROM links l JOIN files f ON f.path = l.dst
           WHERE l.src_is_index = 0 AND l.dst = ?1 AND f.kind = 'concept'
           ORDER BY l.src"#,
        &[&path.as_str()],
    )
}

/// Huérfanos: concepts sin backlinks entrantes y que no están en ningún `index.md`.
pub(crate) fn orphans(conn: &Connection) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        r#"SELECT f.path FROM files f
           WHERE f.kind = 'concept'
             AND NOT EXISTS (
                 SELECT 1 FROM links l WHERE l.src_is_index = 0 AND l.dst = f.path
             )
             AND NOT EXISTS (
                 SELECT 1 FROM links l2 WHERE l2.src_is_index = 1 AND l2.dst = f.path
             )
           ORDER BY f.path"#,
        &[],
    )
}

/// Destinos colgantes (ghosts): enlaces de concepts a algo que no existe como concept/index.
/// Coincide con `core`: `!existe` o (existe y es `log.md`).
pub(crate) fn dangling(conn: &Connection) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        r#"SELECT DISTINCT l.dst
           FROM links l LEFT JOIN files f ON f.path = l.dst
           WHERE l.src_is_index = 0 AND (f.path IS NULL OR f.kind = 'log')
           ORDER BY l.dst"#,
        &[],
    )
}

/// Blast-radius direccional (`Direction::In`): CTE recursivo sobre aristas **inversas**.
/// Devuelve el conjunto de nodos alcanzables siguiendo backlinks hasta `depth` (incluye la raíz).
pub(crate) fn blast_radius(
    conn: &Connection,
    root: &RelPath,
    depth: u32,
) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        r#"WITH RECURSIVE br(path, d) AS (
               SELECT ?1, 0
               UNION
               SELECT l.src, br.d + 1
               FROM links l
               JOIN br ON l.dst = br.path
               JOIN files f ON f.path = l.dst
               WHERE l.src_is_index = 0 AND f.kind = 'concept' AND br.d < ?2
           )
           SELECT DISTINCT path FROM br ORDER BY path"#,
        &[&root.as_str(), &depth],
    )
}

/// Candidatos FTS5 para una expresión de usuario. La expresión se **escapa** como frase
/// (comillas dobladas + envoltura) para neutralizar operadores/inyección (`§12` seguridad).
/// Es un **acelerador**: el llamante confirma con la semántica de subcadena del core.
pub(crate) fn fts_candidates(conn: &Connection, needle: &str) -> Result<Vec<RelPath>, StoreError> {
    let escaped = format!("\"{}\"", needle.replace('"', "\"\""));
    rows_to_relpaths(
        conn,
        "SELECT path FROM files_fts WHERE files_fts MATCH ?1 ORDER BY path",
        &[&escaped],
    )
}

/// Búsqueda de subcadena (semántica del core): title/description/body que contienen `needle`
/// (case-insensitive). Es LA verdad; FTS solo acelera. Incluye matches parciales dentro de un token.
pub(crate) fn search_substring(conn: &Connection, needle: &str) -> Result<Vec<RelPath>, StoreError> {
    let like = format!("%{}%", needle.to_lowercase().replace('%', "\\%").replace('_', "\\_"));
    rows_to_relpaths(
        conn,
        r#"SELECT path FROM files
           WHERE lower(title)       LIKE ?1 ESCAPE '\'
              OR lower(description) LIKE ?1 ESCAPE '\'
              OR lower(body)        LIKE ?1 ESCAPE '\'
           ORDER BY path"#,
        &[&like],
    )
}
