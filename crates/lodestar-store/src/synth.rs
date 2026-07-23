//! Síntesis on-demand (`ARCHITECTURE.md §5`, `§10` fila 10): lo que invalidaría en cascada
//! NO se materializa — se deriva al leer vía SQL/CTE. El aislamiento, los colgantes y los
//! **diagnósticos de enlace** se sintetizan aquí (su definición canónica vive en el core; la
//! paridad lo verifica).
//!
//! Invariante: estas consultas devuelven lo mismo que el `core` equivalente. Si difieren,
//! es **bug de la cache** (gana el core) — lo captura el test de paridad.

use rusqlite::Connection;

use lodestar_core::types::{Check, FileMap, RelPath};
use lodestar_core::DocumentSet;

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

/// Diagnósticos de **enlace** del workspace, por documento (E17-H03).
///
/// No se materializan porque dependen del inventario entero —crear un fichero repara el enlace
/// roto de otro documento, y renombrarlo puede introducir un `LINK-CASE-MISMATCH` en un tercero—,
/// así que guardarlos obligaría a invalidar en cascada (`§10` fila 10, la razón por la que
/// `LINK-STUB` tampoco se materializaba).
///
/// El veredicto lo da **el core**, no SQL: la cache sirve las filas (`files.path`/`files.raw`) y
/// el core las clasifica con [`DocumentSet::analyze`]. Es el mismo patrón que
/// [`search_substring`] — reproducir en SQL la resolución de `§20.6` (contención, percent-decoding,
/// plegado Unicode de mayúsculas) sería un segundo algoritmo divergente, justo lo que prohíbe el
/// invariante #3.
fn link_diagnostics(conn: &Connection) -> Result<Vec<(RelPath, Vec<Check>)>, StoreError> {
    let mut stmt = conn.prepare("SELECT path, raw FROM documents ORDER BY path")?;
    let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
    let mut files = FileMap::new();
    for row in rows {
        let (path, raw) = row?;
        if let Ok(rp) = RelPath::new(&path) {
            files.insert(rp, raw);
        }
    }
    // El inventario debe declarar los `other_files` (código, imágenes…) igual que el core (E18-H04):
    // sin ellos, un enlace a un fichero del proyecto (`[c](src/x.rs)`) se resolvería `Missing` y
    // emitiría un `LINK-TARGET-MISSING` (`warn`) que el core, con el inventario completo, clasifica
    // `WorkspaceFile` silencioso. Es la asimetría que inflaba `warn_count` en la cache; el core es la
    // autoridad, así que la cache resuelve con el MISMO inventario (invariante #3).
    let doc_set = DocumentSet::with_other_files(files, read_other_files(conn)?);
    Ok(doc_set
        .analyze()
        .diagnostics
        .iter()
        .map(|(p, cs)| {
            (
                p.clone(),
                cs.iter()
                    .filter(|c| crate::index::es_de_enlace(c.code))
                    .cloned()
                    .collect(),
            )
        })
        .collect())
}

/// `hard_fail` = nº de ficheros con algún check `Err` (conteo, no `.max()`; `§10` fila 4).
///
/// Une los diagnósticos materializados (locales) con los de enlace sintetizados: un documento
/// cuyo único error es un enlace roto cuenta igual que uno con el frontmatter sin cerrar.
pub(crate) fn hard_fail(conn: &Connection) -> Result<usize, StoreError> {
    let mut con_error: std::collections::BTreeSet<RelPath> = rows_to_relpaths(
        conn,
        "SELECT DISTINCT document_path FROM diagnostics WHERE severity = 'err' ORDER BY document_path",
        &[],
    )?
    .into_iter()
    .collect();
    for (path, checks) in link_diagnostics(conn)? {
        if checks
            .iter()
            .any(|c| c.level == lodestar_core::types::Severity::Err)
        {
            con_error.insert(path);
        }
    }
    Ok(con_error.len())
}

/// `warn_count` = nº total de checks `Warn` (suma sobre ficheros), materializados + sintetizados.
pub(crate) fn warn_count(conn: &Connection) -> Result<usize, StoreError> {
    let n: i64 = conn.query_row(
        "SELECT COUNT(*) FROM diagnostics WHERE severity = 'warn'",
        [],
        |r| r.get(0),
    )?;
    let sintetizados: usize = link_diagnostics(conn)?
        .iter()
        .flat_map(|(_, cs)| cs.iter())
        .filter(|c| c.level == lodestar_core::types::Severity::Warn)
        .count();
    Ok(n as usize + sintetizados)
}

/// Todos los documentos del workspace, en orden estable (E16-H02: ningún basename queda fuera).
pub(crate) fn documents(conn: &Connection) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(conn, "SELECT path FROM documents ORDER BY path", &[])
}

/// Backlinks de un documento (`incoming[path]`): quien lo enlaza, venga de donde venga.
///
/// Solo las **aristas del grafo** (`is_edge = 1`): un enlace externo o a un fichero del proyecto no
/// es un entrante. La forma es la del store v1 —`source_path`/`target_path` sustituyen a `src`/`dst`—
/// y el JOIN con `documents` restringe a destinos que existen, como antes.
pub(crate) fn backlinks(conn: &Connection, path: &RelPath) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        r#"SELECT DISTINCT l.source_path
           FROM links l JOIN documents f ON f.path = l.target_path
           WHERE l.target_path = ?1 AND l.is_edge = 1
           ORDER BY l.source_path"#,
        &[&path.as_str()],
    )
}

/// Aislados (`Analysis::isolated`, `§20.7`): documentos sin enlaces internos entrantes **ni**
/// salientes. Es la misma definición del core — la paridad lo verifica. El filtro `is_edge = 1` es
/// esencial ahora que `links` guarda TODAS las clases: un documento con solo enlaces externos sigue
/// estando aislado.
pub(crate) fn isolated(conn: &Connection) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        r#"SELECT f.path FROM documents f
           WHERE NOT EXISTS (SELECT 1 FROM links l WHERE l.target_path = f.path AND l.is_edge = 1)
             AND NOT EXISTS (SELECT 1 FROM links o WHERE o.source_path = f.path AND o.is_edge = 1)
           ORDER BY f.path"#,
        &[],
    )
}

/// Destinos colgantes (ghosts): aristas del grafo (`is_edge = 1`) hacia un documento que no existe.
/// El `is_edge = 1` descarta los `workspaceFile` (existen, pero no son documentos) y los destinos
/// sin path.
pub(crate) fn dangling(conn: &Connection) -> Result<Vec<RelPath>, StoreError> {
    rows_to_relpaths(
        conn,
        r#"SELECT DISTINCT l.target_path
           FROM links l LEFT JOIN documents f ON f.path = l.target_path
           WHERE l.is_edge = 1 AND f.path IS NULL
           ORDER BY l.target_path"#,
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
               SELECT l.source_path, br.d + 1
               FROM links l
               JOIN br ON l.target_path = br.path
               JOIN documents f ON f.path = l.target_path
               WHERE l.is_edge = 1 AND br.d < ?2
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
        "SELECT path FROM documents_fts WHERE documents_fts MATCH ?1 ORDER BY path",
        &[&escaped],
    )
}

/// Los `other_files` materializados (los ficheros del proyecto que **no** son documentos): el
/// inventario con el que la síntesis de diagnósticos y `DocumentSet::from_store` clasifican un
/// enlace como `WorkspaceFile` en vez de `Missing` (E18-H04). La tabla `other_files` la vuelca el
/// único escritor en cada walk completo (`lib::write_other_files`).
pub(crate) fn read_other_files(conn: &Connection) -> Result<Vec<RelPath>, StoreError> {
    let mut stmt = conn.prepare("SELECT path FROM other_files")?;
    let iter = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = Vec::new();
    for s in iter {
        if let Ok(rp) = RelPath::new(&s?) {
            out.push(rp);
        }
    }
    Ok(out)
}

/// Enlaces salientes de un documento **con su clasificación** (`Analysis::outgoing`, `§20.12`): una
/// tupla `(raw_href, target_kind, target_path, fragment)` por enlace del cuerpo, leída tal cual de
/// la tabla `links` materializada. Devuelve TODAS las clases de destino (no solo las aristas del
/// grafo): externos, anchors propios y ficheros del proyecto incluidos.
///
/// La cache no reclasifica nada: `target_kind`/`target_path`/`fragment` los escribió
/// `index::upsert_file` proyectando el `LinkTarget` que resolvió el core. Es la superficie por la
/// que el test de paridad compara la clasificación de enlaces (invariante #3: gana el core).
pub(crate) fn outgoing_links(
    conn: &Connection,
    source: &RelPath,
) -> Result<Vec<crate::OutgoingLink>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT raw_href, target_kind, target_path, fragment FROM links WHERE source_path = ?1",
    )?;
    let rows = stmt.query_map([source.as_str()], |r| {
        Ok((
            r.get::<_, String>(0)?,
            r.get::<_, String>(1)?,
            r.get::<_, Option<String>>(2)?,
            r.get::<_, Option<String>>(3)?,
        ))
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
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
    let mut stmt = conn.prepare("SELECT path, raw FROM documents ORDER BY path")?;
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
