//! Síntesis on-demand (`ARCHITECTURE.md §5`, `§10` fila 10): lo que invalidaría en cascada
//! NO se materializa — se deriva al leer vía SQL/CTE. `LINK-STUB` y el aislamiento se sintetizan
//! aquí (su definición canónica vive en el core; la paridad lo verifica).
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

/// Todos los documentos del workspace, en orden estable (E16-H02: ningún basename queda fuera).
pub(crate) fn documents(conn: &Connection) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(conn, "SELECT path FROM files ORDER BY path", &[])
}

/// Backlinks de un documento (`inn[path]`): quien lo enlaza, venga de donde venga.
pub(crate) fn backlinks(conn: &Connection, path: &RelPath) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        r#"SELECT DISTINCT l.src
           FROM links l JOIN files f ON f.path = l.dst
           WHERE l.dst = ?1
           ORDER BY l.src"#,
        &[&path.as_str()],
    )
}

/// Aislados (`Analysis::isolated`, `§20.7`): documentos sin enlaces internos entrantes **ni**
/// salientes. Es la misma definición del core — la paridad lo verifica.
pub(crate) fn isolated(conn: &Connection) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        r#"SELECT f.path FROM files f
           WHERE NOT EXISTS (SELECT 1 FROM links l WHERE l.dst = f.path)
             AND NOT EXISTS (SELECT 1 FROM links o WHERE o.src = f.path)
           ORDER BY f.path"#,
        &[],
    )
}

/// Destinos colgantes (ghosts): enlaces a un `.md` que no existe en el workspace.
pub(crate) fn dangling(conn: &Connection) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        r#"SELECT DISTINCT l.dst
           FROM links l LEFT JOIN files f ON f.path = l.dst
           WHERE f.path IS NULL
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
               WHERE br.d < ?2
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

/// Búsqueda de subcadena con la MISMA función del core (`query::loose_text_match`): basename +
/// cualquier valor de frontmatter + cuerpo, con lowercase Unicode. Antes era un LIKE de SQL que
/// divergía en tres frentes (sin escapar `\`, `lower()` solo-ASCII —«PROGRAMACIÓN» no casaba—,
/// y sin basename/fm). SQL solo sirve las filas; la verdad es del core. FTS solo acelera.
pub(crate) fn search_substring(
    conn: &Connection,
    needle: &str,
) -> Result<Vec<RelPath>, StoreError> {
    let needle_lower = needle.to_lowercase();
    let mut stmt = conn.prepare("SELECT path, raw FROM files ORDER BY path")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut out = Vec::new();
    for row in rows {
        let (path, raw) = row?;
        let Ok(rp) = RelPath::new(&path) else {
            continue;
        };
        let parsed = lodestar_core::model::parse_file(rp.as_str(), &raw);
        let fm = parsed.frontmatter.unwrap_or_default();
        if lodestar_core::query::loose_text_match(&rp, &fm, &parsed.body, &needle_lower) {
            out.push(rp);
        }
    }
    Ok(out)
}
