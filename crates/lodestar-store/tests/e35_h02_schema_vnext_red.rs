//! Fase roja de E35-H02: contrato observable del esquema SQLite vNext.

use std::collections::BTreeSet;
use std::path::Path;

use lodestar_core::metadata::{catalog, inspect_field};
use lodestar_core::types::{FieldPath, FileMap, RelPath, ValueType};
use lodestar_core::DocumentSet;
use lodestar_store::Store;
use rusqlite::Connection;

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("RelPath válido")
}

fn write_all(root: &Path, files: &FileMap) {
    for (path, contents) in files {
        let target = root.join(path.as_str());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(target, contents).unwrap();
    }
}

fn files() -> FileMap {
    lodestar_fixtures::file_map(&[
        ("a.md", "---\ngraph:\n  backlinks: usuario\ndocument:\n  title: título de usuario\nservice:\n  tier: critical\nunicode: café\n---\n# A\n\n[conocido](b.md) [ausente](missing.md) [código](src/tool.rs)\naguja-única-corpus\n"),
        ("b.md", "---\ntitle: B\nservice:\n  tier: stable\n---\n# B\n\nvuelve a [A](a.md)\n"),
        ("broken.md", "---\ntitle: : :\n  - inválido\n---\n# Diagnóstico\n"),
        ("src/tool.rs", "fn tool() {}\n"),
    ])
}

fn db(root: &Path) -> Connection {
    Connection::open(root.join(".lodestar/index.db")).unwrap()
}

fn columns(conn: &Connection, table: &str) -> Vec<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA table_info({table})"))
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap()
}

fn sql(conn: &Connection, table: &str) -> String {
    conn.query_row(
        "SELECT COALESCE(sql, '') FROM sqlite_master WHERE name=?1",
        [table],
        |row| row.get(0),
    )
    .unwrap()
}

fn index_names(conn: &Connection, table: &str) -> BTreeSet<String> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA index_list({table})"))
        .unwrap();
    stmt.query_map([], |row| row.get::<_, String>(1))
        .unwrap()
        .collect::<Result<BTreeSet<_>, _>>()
        .unwrap()
}

fn foreign_keys(conn: &Connection, table: &str) -> BTreeSet<(String, String, String)> {
    let mut stmt = conn
        .prepare(&format!("PRAGMA foreign_key_list({table})"))
        .unwrap();
    stmt.query_map([], |row| {
        Ok((
            row.get::<_, String>(2)?,
            row.get::<_, String>(6)?,
            row.get::<_, String>(5)?,
        ))
    })
    .unwrap()
    .collect::<Result<BTreeSet<_>, _>>()
    .unwrap()
}

fn core_matches(files: &FileMap, needle: &str) -> Vec<RelPath> {
    let needle = needle.to_lowercase();
    let mut paths = files
        .iter()
        .filter_map(|(path, raw)| {
            if !path.as_str().ends_with(".md") {
                return None;
            }
            let parsed = lodestar_core::model::parse_file(path.as_str(), raw);
            let frontmatter = parsed.frontmatter.unwrap_or_default();
            lodestar_core::text::loose_text_match(path, &frontmatter, &parsed.body, &needle)
                .then_some(path.clone())
        })
        .collect::<Vec<_>>();
    paths.sort();
    paths
}

fn create_v6_nominal_but_unconstrained(conn: &Connection, extra_raw: bool) {
    let (raw_column, raw_insert_column) = if extra_raw {
        (", raw TEXT", ", raw")
    } else {
        ("", "")
    };
    conn.execute_batch(&format!(
        "CREATE TABLE documents (doc_id INTEGER PRIMARY KEY, path TEXT, title TEXT, body TEXT, frontmatter_json TEXT, frontmatter_text TEXT, content_hash BLOB, mtime INTEGER, size INTEGER{raw_column});
         CREATE TABLE fields (field_id INTEGER PRIMARY KEY, field_path TEXT);
         CREATE TABLE metadata (doc_id INTEGER, field_id INTEGER, value_json TEXT, value_type TEXT);
         CREATE TABLE other_files (path TEXT);
         CREATE TABLE links (link_id INTEGER PRIMARY KEY, source_doc_id INTEGER, target_doc_id INTEGER, raw_href TEXT, target_kind TEXT, target_path TEXT, fragment TEXT, resolved INTEGER, is_edge INTEGER);
         CREATE TABLE diagnostics (diagnostic_id INTEGER PRIMARY KEY, doc_id INTEGER, code TEXT, severity TEXT, message TEXT, range_json TEXT);
         CREATE VIRTUAL TABLE documents_fts USING fts5(path UNINDEXED, title, body, frontmatter_text, content=documents, content_rowid=doc_id);
         INSERT INTO documents(doc_id,path,title,body,frontmatter_json,frontmatter_text,content_hash,mtime,size{raw_insert_column})
           VALUES (99, 'sentinel.md', '', 'sentinel', '{{}}', '', X'00', 0, 8{raw_value});
         PRAGMA user_version=6;",
        raw_value = if extra_raw { ", 'legacy raw'" } else { "" },
    ))
    .unwrap();
}

struct FtsSpike {
    bytes: u64,
    objects: BTreeSet<(String, u64)>,
    exclusive: Vec<i64>,
    frontmatter_exclusive: Vec<i64>,
    shared: Vec<i64>,
    absent: Vec<i64>,
    old_after_update: Vec<i64>,
    new_after_update: Vec<i64>,
    old_frontmatter_after_update: Vec<i64>,
    new_frontmatter_after_update: Vec<i64>,
    deleted_after_delete: Vec<i64>,
    deleted_frontmatter_after_delete: Vec<i64>,
}

struct FtsRow {
    path: String,
    title: String,
    body: String,
    frontmatter_text: String,
}

fn fts_variant_dbstat(ddl: &str, corpus: &[FtsRow], contentless: bool) -> FtsSpike {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(
        "CREATE TABLE documents (doc_id INTEGER PRIMARY KEY, path TEXT UNIQUE NOT NULL, title TEXT NOT NULL, body TEXT NOT NULL, frontmatter_text TEXT NOT NULL);",
    )
    .unwrap();
    conn.execute_batch(ddl).unwrap();
    for (index, row) in corpus.iter().enumerate() {
        conn.execute(
            "INSERT INTO documents(doc_id,path,title,body,frontmatter_text) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![index as i64 + 1, row.path, row.title, row.body, row.frontmatter_text],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO documents_fts(rowid,path,title,body,frontmatter_text) VALUES (?1,?2,?3,?4,?5)",
            rusqlite::params![index as i64 + 1, row.path, row.title, row.body, row.frontmatter_text],
        )
        .unwrap();
    }

    let candidates = |needle: &str| {
        let mut stmt = conn
            .prepare("SELECT rowid FROM documents_fts WHERE documents_fts MATCH ?1 ORDER BY rowid")
            .unwrap();
        stmt.query_map([format!("\"{needle}\"")], |row| row.get::<_, i64>(0))
            .unwrap()
            .collect::<Result<Vec<_>, _>>()
            .unwrap()
    };
    let exclusive = candidates("exclusive_04200");
    let frontmatter_exclusive = candidates("fm_exclusive_09876");
    let shared = candidates("shared_token");
    let absent = candidates("not_present_anywhere");

    // Both variants need the same explicit old-value maintenance protocol. This exercises the
    // FTS5 delete command instead of relying on UPDATE semantics that contentless tables lack.
    let id_update = 7_i64;
    let old = "lifecycle_old_only";
    let new = "lifecycle_new_only";
    let new_frontmatter = "fm_lifecycle_new_only";
    let old_row = &corpus[(id_update - 1) as usize];
    assert_eq!(
        candidates(old),
        vec![id_update],
        "valor antiguo C5 realmente indexado"
    );
    assert_eq!(
        candidates("fm_lifecycle_old_only"),
        vec![id_update],
        "frontmatter antiguo C5 realmente indexado"
    );
    conn.execute(
        "INSERT INTO documents_fts(documents_fts,rowid,path,title,body,frontmatter_text) VALUES ('delete',?1,?2,?3,?4,?5)",
        rusqlite::params![id_update, old_row.path, old_row.title, old_row.body, old_row.frontmatter_text],
    )
    .unwrap();
    conn.execute(
        "UPDATE documents SET body=?1, frontmatter_text=?2 WHERE doc_id=?3",
        rusqlite::params![new, new_frontmatter, id_update],
    )
    .unwrap();
    conn.execute(
        "INSERT INTO documents_fts(rowid,path,title,body,frontmatter_text) VALUES (?1,?2,?3,?4,?5)",
        rusqlite::params![id_update, old_row.path, old_row.title, new, new_frontmatter],
    )
    .unwrap();
    let old_after_update = candidates(old);
    let new_after_update = candidates(new);
    let old_frontmatter_after_update = candidates("fm_lifecycle_old_only");
    let new_frontmatter_after_update = candidates(new_frontmatter);

    let id_delete = 8_i64;
    let delete_row = &corpus[(id_delete - 1) as usize];
    assert_eq!(
        candidates("delete_only_00007"),
        vec![id_delete],
        "valor que se borrará C5 realmente indexado"
    );
    assert_eq!(
        candidates("fm_delete_only_00007"),
        vec![id_delete],
        "frontmatter que se borrará C5 realmente indexado"
    );
    conn.execute(
        "INSERT INTO documents_fts(documents_fts,rowid,path,title,body,frontmatter_text) VALUES ('delete',?1,?2,?3,?4,?5)",
        rusqlite::params![id_delete, delete_row.path, delete_row.title, delete_row.body, delete_row.frontmatter_text],
    )
    .unwrap();
    conn.execute("DELETE FROM documents WHERE doc_id=?1", [id_delete])
        .unwrap();
    let deleted_after_delete = candidates("delete_only_00007");
    let deleted_frontmatter_after_delete = candidates("fm_delete_only_00007");

    conn.execute_batch("CREATE VIRTUAL TABLE temp.dbstat USING dbstat(main)")
        .unwrap();
    let mut objects = BTreeSet::new();
    let mut schema_stmt = conn
        .prepare(
            "SELECT name, COALESCE((SELECT SUM(pgsize) FROM temp.dbstat WHERE temp.dbstat.name=sqlite_schema.name),0)
             FROM sqlite_schema WHERE name LIKE 'documents_fts%' ORDER BY name",
        )
        .unwrap();
    for row in schema_stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })
        .unwrap()
    {
        objects.insert(row.unwrap());
    }
    let mut dbstat_stmt = conn
        .prepare(
            "SELECT name, SUM(pgsize) FROM temp.dbstat
             WHERE name LIKE 'documents_fts%' GROUP BY name ORDER BY name",
        )
        .unwrap();
    for row in dbstat_stmt
        .query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, u64>(1)?))
        })
        .unwrap()
    {
        objects.insert(row.unwrap());
    }
    assert!(objects.iter().any(|(name, _)| name == "documents_fts"));
    assert!(objects
        .iter()
        .any(|(name, _)| name.starts_with("documents_fts_")));
    let bytes: u64 = objects.iter().map(|(_, bytes)| *bytes).sum();
    println!(
        "C5 variant={} corpus={} bytes={} objects={:?} body_exclusive={:?} frontmatter_exclusive={:?} shared_count={} absent={:?} old_after_update={:?} new_after_update={:?} old_fm_after_update={:?} new_fm_after_update={:?} deleted_after_delete={:?} deleted_fm_after_delete={:?}",
        if contentless { "contentless" } else { "external" },
        corpus.len(),
        bytes,
        objects,
        exclusive,
        frontmatter_exclusive,
        shared.len(),
        absent,
        old_after_update,
        new_after_update,
        old_frontmatter_after_update,
        new_frontmatter_after_update,
        deleted_after_delete,
        deleted_frontmatter_after_delete,
    );
    FtsSpike {
        bytes,
        objects,
        exclusive,
        frontmatter_exclusive,
        shared,
        absent,
        old_after_update,
        new_after_update,
        old_frontmatter_after_update,
        new_frontmatter_after_update,
        deleted_after_delete,
        deleted_frontmatter_after_delete,
    }
}

/// C5 — el spike compara ambas variantes sobre el mismo corpus y la elección final es medida.
#[test]
fn c5_spike_fts_mismo_corpus_dbstat_y_eleccion_reconciliada() {
    let corpus = (0..10_000)
        .map(|index| {
            let (body_marker, frontmatter_text) = if index == 6 {
                ("lifecycle_old_only", "fm_lifecycle_old_only")
            } else if index == 7 {
                ("delete_only_00007", "fm_delete_only_00007")
            } else {
                ("", "")
            };
            let body = if index == 4_200 {
                format!(
                    "---\ntitle: row-{index}\n---\n# Row {index}\n\nshared_token exclusive_04200"
                )
            } else if index == 9_876 {
                format!("---\ntitle: row-{index}\n---\n# Row {index}\n\nshared_token row {index}")
            } else {
                format!("---\ntitle: row-{index}\n---\n# Row {index}\n\nshared_token {body_marker}")
            };
            let frontmatter_text = if index == 9_876 {
                "fm_exclusive_09876".to_string()
            } else if frontmatter_text.is_empty() {
                format!("fm_row_{index}")
            } else {
                frontmatter_text.to_string()
            };
            FtsRow {
                path: format!("doc-{index:05}.md"),
                title: format!("Title {index}"),
                body,
                frontmatter_text,
            }
        })
        .collect::<Vec<_>>();
    let contentless = fts_variant_dbstat(
        "CREATE VIRTUAL TABLE documents_fts USING fts5(path UNINDEXED,title,body,frontmatter_text,content='',columnsize=0);",
        &corpus,
        true,
    );
    let external = fts_variant_dbstat(
        "CREATE VIRTUAL TABLE documents_fts USING fts5(path UNINDEXED,title,body,frontmatter_text,content='documents',content_rowid='doc_id');",
        &corpus,
        false,
    );
    assert!(
        contentless.bytes > 0 && external.bytes > 0,
        "dbstat de ambas variantes no vacío"
    );
    assert_eq!(contentless.exclusive, vec![4_201]);
    assert_eq!(contentless.exclusive, external.exclusive, "mismo corpus C5");
    assert_eq!(contentless.frontmatter_exclusive, vec![9_877]);
    assert_eq!(
        contentless.frontmatter_exclusive, external.frontmatter_exclusive,
        "agujas frontmatter iguales"
    );
    assert_eq!(contentless.shared.len(), 10_000);
    assert_eq!(
        contentless.shared, external.shared,
        "candidatos shared iguales"
    );
    assert!(contentless.absent.is_empty() && external.absent.is_empty());
    for spike in [&contentless, &external] {
        assert!(spike
            .objects
            .iter()
            .any(|(name, _)| name == "documents_fts"));
        assert!(spike
            .objects
            .iter()
            .any(|(name, _)| name.starts_with("documents_fts_")));
        assert_eq!(
            spike.objects.iter().map(|(_, bytes)| *bytes).sum::<u64>(),
            spike.bytes
        );
        assert!(
            spike.old_after_update.is_empty(),
            "valor antiguo no sobrevive update"
        );
        assert!(
            spike.old_frontmatter_after_update.is_empty(),
            "frontmatter antiguo no sobrevive update"
        );
        assert_eq!(
            spike.new_after_update,
            vec![7],
            "valor nuevo aparece tras update"
        );
        assert_eq!(
            spike.new_frontmatter_after_update,
            vec![7],
            "frontmatter nuevo aparece tras update"
        );
        assert!(
            spike.deleted_after_delete.is_empty(),
            "fila borrada no aparece tras delete"
        );
        assert!(
            spike.deleted_frontmatter_after_delete.is_empty(),
            "frontmatter de fila borrada no aparece tras delete"
        );
    }

    let root = tempfile::tempdir().unwrap();
    let files = files();
    write_all(root.path(), &files);
    let store = Store::open_and_build(root.path()).unwrap();
    assert!(
        store.documents().unwrap().len() >= 3,
        "cache final no vacía"
    );
    let conn = db(root.path());
    let document_columns = columns(&conn, "documents");
    assert!(
        document_columns.contains(&"doc_id".into()),
        "la elección se liga a documents.doc_id"
    );
    let final_ddl = sql(&conn, "documents_fts").to_ascii_lowercase();
    let final_contentless = final_ddl.contains("content=''") || final_ddl.contains("content=\"\"");
    let final_external =
        final_ddl.contains("content=documents") && final_ddl.contains("content_rowid=doc_id");
    assert_eq!(
        final_contentless as u8 + final_external as u8,
        1,
        "DDL final debe declarar una variante"
    );
    if final_contentless {
        assert!(
            final_ddl.contains("columnsize=0"),
            "contentless vNext debe desactivar columnsize para comparar el coste ratificado"
        );
    }
    let selected = if final_contentless {
        contentless.bytes
    } else {
        external.bytes
    };
    assert!(
        final_contentless,
        "la regla ratificada selecciona contentless cuando es funcional y no cuesta más"
    );
    assert!(
        selected <= external.bytes,
        "la variante elegida no es mayor por dbstat"
    );
}

/// C1 — una cache incompatible se descarta, sube versión y conserva Markdown byte a byte.
#[test]
fn c1_cache_incompatible_se_reconstruye_sin_migrar_markdown() {
    let root = tempfile::tempdir().unwrap();
    let markdown = "# byte a byte\n\ncontenido fuera de SQLite\n";
    std::fs::write(root.path().join("control.md"), markdown).unwrap();
    let cache_dir = root.path().join(".lodestar");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let conn = Connection::open(cache_dir.join("index.db")).unwrap();
    conn.execute_batch("CREATE TABLE documents (path TEXT PRIMARY KEY, sentinel TEXT); INSERT INTO documents(path, sentinel) VALUES ('sentinel.md', 'no migrar'); PRAGMA user_version=5;").unwrap();
    drop(conn);
    let before = std::fs::read(root.path().join("control.md")).unwrap();
    let store = Store::open_and_build(root.path()).unwrap();
    assert!(!store.documents().unwrap().is_empty(), "corpus C1 no vacío");
    assert_eq!(
        before,
        std::fs::read(root.path().join("control.md")).unwrap()
    );
    let conn = db(root.path());
    let version: i64 = conn
        .query_row("PRAGMA user_version", [], |row| row.get(0))
        .unwrap();
    assert!(
        version > 5,
        "C1 exige user_version vNext > v5, no {version}"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM documents WHERE path='sentinel.md'",
            [],
            |r| r.get::<_, i64>(0),
        )
        .unwrap(),
        0,
        "fila sentinela incompatible no sobrevive"
    );
    assert!(
        !columns(&conn, "documents").contains(&"sentinel".into()),
        "DDL viejo no sobrevive"
    );
    assert!(
        columns(&conn, "documents").contains(&"doc_id".into()),
        "DDL vNext completo"
    );
}

/// C1 — user_version=6 no basta: una cache con las columnas nominales pero sin integridad se tira.
#[test]
fn c1_v6_columnas_nominales_sin_constraints_fks_ni_indices_se_reconstruye() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("control.md"),
        "# control\n\nbytes exactos\n",
    )
    .unwrap();
    let cache_dir = root.path().join(".lodestar");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let conn = Connection::open(cache_dir.join("index.db")).unwrap();
    create_v6_nominal_but_unconstrained(&conn, false);
    drop(conn);

    let store = Store::open_and_build(root.path()).unwrap();
    assert!(
        !store.documents().unwrap().is_empty(),
        "C1 v6 corpus no vacío"
    );
    let conn = db(root.path());
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        6,
        "C1 v6 conserva la versión vNext"
    );
    assert_eq!(
        index_names(&conn, "documents"),
        BTreeSet::from(["sqlite_autoindex_documents_1".to_string()]),
        "C1 v6 debe reconstruir UNIQUE(path), no solo las columnas"
    );
    assert_eq!(
        index_names(&conn, "metadata"),
        BTreeSet::from([
            "sqlite_autoindex_metadata_1".to_string(),
            "idx_metadata_doc".to_string(),
            "idx_metadata_field".to_string(),
        ])
    );
    assert!(
        !foreign_keys(&conn, "metadata").is_empty(),
        "C1 v6 FKs metadata"
    );
    assert!(!foreign_keys(&conn, "links").is_empty(), "C1 v6 FKs links");
    assert!(
        !foreign_keys(&conn, "diagnostics").is_empty(),
        "C1 v6 FKs diagnostics"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM documents WHERE path='sentinel.md'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0,
        "la fila de la cache no se migra in-place"
    );
}

/// C1 — una forma v6 parecida con la columna heredada `raw` también es incompatible.
#[test]
fn c1_v6_con_columna_extra_raw_se_reconstruye_y_raw_desaparece() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("control.md"),
        "# control\n\nno migrar raw\n",
    )
    .unwrap();
    let cache_dir = root.path().join(".lodestar");
    std::fs::create_dir_all(&cache_dir).unwrap();
    let conn = Connection::open(cache_dir.join("index.db")).unwrap();
    create_v6_nominal_but_unconstrained(&conn, true);
    drop(conn);

    let store = Store::open_and_build(root.path()).unwrap();
    assert!(
        !store.documents().unwrap().is_empty(),
        "C1 raw corpus no vacío"
    );
    let conn = db(root.path());
    assert_eq!(
        conn.query_row("PRAGMA user_version", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        6
    );
    assert!(
        !columns(&conn, "documents").contains(&"raw".to_string()),
        "la columna raw heredada no sobrevive al rebuild"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM documents WHERE path='sentinel.md'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0
    );
}

/// C1 — una cache v6 exacta con objetos SQLite heredados también debe reconstruirse completa;
/// ningún trigger externo puede sabotear el rebuild ni sobrevivir junto al Markdown canónico.
#[test]
fn c1_v6_objetos_heredados_y_trigger_extra_se_descartan() {
    let root = tempfile::tempdir().unwrap();
    let corpus = files();
    write_all(root.path(), &corpus);
    let store = Store::open_and_build(root.path()).unwrap();
    drop(store);

    let before = corpus
        .iter()
        .filter(|(path, _)| path.as_str().ends_with(".md"))
        .map(|(path, raw)| (path.clone(), raw.as_bytes().to_vec()))
        .collect::<BTreeSet<_>>();
    let conn = db(root.path());
    conn.execute_batch(
        "CREATE TABLE files (path TEXT PRIMARY KEY, raw TEXT);
         INSERT INTO files(path, raw) VALUES ('legacy.md', 'sentinel');
         CREATE TABLE rogue_extra (sentinel TEXT);
         INSERT INTO rogue_extra(sentinel) VALUES ('rogue sentinel');
         CREATE TRIGGER sabotage_metadata BEFORE INSERT ON metadata
         BEGIN SELECT RAISE(ABORT, 'legacy metadata trigger'); END;",
    )
    .unwrap();
    drop(conn);

    let rebuilt = Store::open_and_build(root.path());
    assert!(
        rebuilt.is_ok(),
        "C1 debe descartar objetos heredados antes de indexar"
    );
    drop(rebuilt);
    let conn = db(root.path());
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='table' AND name='files'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0,
        "tabla files heredada ausente"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE type='trigger' AND name='sabotage_metadata'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0,
        "trigger heredado ausente"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM sqlite_schema WHERE name='rogue_extra'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        0,
        "objeto de nombre arbitrario ausente"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM rogue_extra WHERE sentinel='rogue sentinel'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0),
        0,
        "sentinela rogue ausente"
    );
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM files WHERE path='legacy.md'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap_or(0),
        0,
        "sentinela heredada ausente"
    );
    for (path, expected) in before {
        let actual: String = conn
            .query_row(
                "SELECT body FROM documents WHERE path=?1",
                [path.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(actual.as_bytes(), expected, "Markdown exacto de {path}");
    }
}

/// C2 — IDs enteros, FKs y destino dirigido conocido/desconocido.
#[test]
fn c2_documentos_y_relaciones_usan_ids_con_destino_dangling() {
    let root = tempfile::tempdir().unwrap();
    let corpus = files();
    write_all(root.path(), &corpus);
    let store = Store::open_and_build(root.path()).unwrap();
    assert!(store.documents().unwrap().len() >= 3, "corpus C2 no vacío");
    let conn = db(root.path());
    assert_eq!(
        conn.query_row("PRAGMA foreign_keys", [], |r| r.get::<_, i64>(0))
            .unwrap(),
        1
    );
    let dc = columns(&conn, "documents");
    assert!(dc.contains(&"doc_id".into()) && dc.contains(&"path".into()));
    let (typ, pk): (String, i64) = conn
        .query_row(
            "SELECT type,pk FROM pragma_table_info('documents') WHERE name='doc_id'",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert_eq!(typ.to_ascii_uppercase(), "INTEGER");
    assert_eq!(pk, 1, "doc_id INTEGER PRIMARY KEY");
    let n: i64 = conn
        .query_row("SELECT COUNT(*) FROM documents", [], |r| r.get(0))
        .unwrap();
    let distinct: i64 = conn
        .query_row("SELECT COUNT(DISTINCT doc_id) FROM documents", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(n >= 3);
    assert_eq!(n, distinct, "IDs distintos");
    for (table, col) in [
        ("metadata", "doc_id"),
        ("diagnostics", "doc_id"),
        ("links", "source_doc_id"),
        ("links", "target_doc_id"),
    ] {
        assert!(
            columns(&conn, table).contains(&col.into()),
            "{table}.{col} referenciado por ID"
        );
    }
    assert!(!columns(&conn, "metadata").contains(&"document_path".into()));
    assert!(!columns(&conn, "diagnostics").contains(&"document_path".into()));
    assert!(!columns(&conn, "links").contains(&"source_path".into()));

    assert_eq!(
        index_names(&conn, "documents"),
        BTreeSet::from(["sqlite_autoindex_documents_1".to_string()]),
        "C2: solo UNIQUE(path) en documents"
    );
    assert_eq!(
        index_names(&conn, "fields"),
        BTreeSet::from(["sqlite_autoindex_fields_1".to_string()]),
        "C2: solo UNIQUE(field_path) en fields"
    );
    assert_eq!(
        index_names(&conn, "metadata"),
        BTreeSet::from([
            "sqlite_autoindex_metadata_1".to_string(),
            "idx_metadata_doc".to_string(),
            "idx_metadata_field".to_string(),
        ])
    );
    assert_eq!(
        index_names(&conn, "links"),
        BTreeSet::from([
            "idx_links_source_doc".to_string(),
            "idx_links_target_doc".to_string(),
            "idx_links_target_path".to_string(),
        ])
    );
    assert_eq!(
        index_names(&conn, "diagnostics"),
        BTreeSet::from(["idx_diag_doc".to_string(), "idx_diag_severity".to_string()])
    );
    assert_eq!(
        index_names(&conn, "other_files"),
        BTreeSet::from(["sqlite_autoindex_other_files_1".to_string()])
    );
    assert_eq!(
        foreign_keys(&conn, "metadata"),
        BTreeSet::from([
            (
                "documents".to_string(),
                "CASCADE".to_string(),
                "NO ACTION".to_string()
            ),
            (
                "fields".to_string(),
                "NO ACTION".to_string(),
                "NO ACTION".to_string()
            ),
        ])
    );
    assert_eq!(
        foreign_keys(&conn, "links"),
        BTreeSet::from([
            (
                "documents".to_string(),
                "CASCADE".to_string(),
                "NO ACTION".to_string()
            ),
            (
                "documents".to_string(),
                "SET NULL".to_string(),
                "NO ACTION".to_string()
            ),
        ])
    );
    assert_eq!(
        foreign_keys(&conn, "diagnostics"),
        BTreeSet::from([(
            "documents".to_string(),
            "CASCADE".to_string(),
            "NO ACTION".to_string(),
        )])
    );
    let known: (i64, i64) = conn
        .query_row(
            "SELECT source_doc_id,target_doc_id FROM links WHERE raw_href LIKE '%b.md' LIMIT 1",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .unwrap();
    assert!(known.0 > 0 && known.1 > 0, "enlace conocido usa IDs");
    let missing: (i64, Option<i64>, Option<String>) = conn.query_row("SELECT source_doc_id,target_doc_id,target_path FROM links WHERE target_path LIKE '%missing.md' LIMIT 1", [], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).unwrap();
    assert!(
        missing.0 > 0 && missing.1.is_none() && missing.2.is_some(),
        "missing conserva target_path sin ID"
    );
    let diagnostics: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM diagnostics WHERE doc_id IS NOT NULL",
            [],
            |r| r.get(0),
        )
        .unwrap();
    assert!(diagnostics > 0, "diagnostics no vacíos y referenciados");
    drop(conn);
    assert!(store.remove(&rp("b.md")).unwrap(), "borrado C2 efectivo");
    let conn = db(root.path());
    for (label, sql) in [
        ("metadata huérfana", "SELECT COUNT(*) FROM metadata m LEFT JOIN documents d ON d.doc_id=m.doc_id WHERE d.doc_id IS NULL"),
        ("diagnostic huérfano", "SELECT COUNT(*) FROM diagnostics x LEFT JOIN documents d ON d.doc_id=x.doc_id WHERE d.doc_id IS NULL"),
        ("origen link huérfano", "SELECT COUNT(*) FROM links l LEFT JOIN documents d ON d.doc_id=l.source_doc_id WHERE d.doc_id IS NULL"),
        ("destino link huérfano", "SELECT COUNT(*) FROM links l LEFT JOIN documents d ON d.doc_id=l.target_doc_id WHERE l.target_doc_id IS NOT NULL AND d.doc_id IS NULL"),
    ] {
        let orphan: i64 = conn.query_row(sql, [], |r| r.get(0)).unwrap();
        assert_eq!(orphan, 0, "{label} tras borrar b.md");
    }
}

/// C2 — reconcile_all debe materializar el destino conocido cuando origen y destino aparecen en
/// la misma tanda; nunca deja un dangling transitorio con `target_path` persistido.
#[test]
fn c2_reconcile_all_reatta_destino_conocido_en_la_misma_tanda() {
    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("base.md"), "# base\n").unwrap();
    let store = Store::open_and_build(root.path()).unwrap();

    std::fs::write(root.path().join("a.md"), "# A\n\n[zeta](z.md)\n").unwrap();
    std::fs::write(root.path().join("z.md"), "# Z\n\n[alpha](a.md)\n").unwrap();
    let event = store.reconcile_all().unwrap();
    assert!(
        event.changed.iter().any(|path| path == &rp("a.md"))
            && event.changed.iter().any(|path| path == &rp("z.md")),
        "C2 reconcile debe observar ambos documentos nuevos"
    );

    let conn = db(root.path());
    for (source_path, target_path, label) in [("a.md", "z.md", "a→z"), ("z.md", "a.md", "z→a")]
    {
        let row: (i64, Option<i64>, Option<String>) = conn
            .query_row(
                "SELECT source.doc_id, link.target_doc_id, link.target_path
                 FROM links link
                 JOIN documents source ON source.doc_id=link.source_doc_id
                 WHERE source.path=?1 AND link.raw_href=?2",
                [source_path, target_path],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        let target_doc_id: i64 = conn
            .query_row(
                "SELECT doc_id FROM documents WHERE path=?1",
                [target_path],
                |row| row.get(0),
            )
            .unwrap();
        assert!(row.0 > 0, "{label}: origen tiene doc_id");
        assert_eq!(
            row.1,
            Some(target_doc_id),
            "{label}: enlace conocido usa target_doc_id del destino"
        );
        assert_eq!(
            row.2, None,
            "{label}: destino conocido no conserva target_path"
        );
    }
}

/// C2/C6 — la clasificación de navegación del core se conserva en la tabla `links`, incluso
/// cuando el destino es un directorio y por tanto no puede recibir `target_doc_id`.
#[test]
fn c2_c6_workspace_directory_preserva_core_y_path_sql() {
    let root = tempfile::tempdir().unwrap();
    let corpus = lodestar_fixtures::file_map(&[("docs/sub/a.md", "# A\n\n[up](../)\n")]);
    write_all(root.path(), &corpus);

    let core = DocumentSet::from_files(corpus.clone());
    let core_link = core
        .analyze()
        .outgoing
        .get(&rp("docs/sub/a.md"))
        .and_then(|links| links.iter().find(|link| link.href == "../"))
        .expect("core debe observar el enlace al directorio docs");
    assert!(matches!(
        &core_link.target,
        lodestar_core::types::LinkTarget::WorkspaceDirectory(Some(path))
            if path == &rp("docs")
    ));

    let store = Store::open_and_build(root.path()).unwrap();
    assert_eq!(
        store.outgoing_links(&rp("docs/sub/a.md")).unwrap(),
        vec![(
            "../".to_string(),
            "workspaceDirectory".to_string(),
            Some("docs".to_string()),
            None,
        )],
        "Store debe conservar la clasificación/path del core"
    );
    let conn = db(root.path());
    let row: (String, Option<String>, Option<i64>) = conn
        .query_row(
            "SELECT l.target_kind, l.target_path, l.target_doc_id
             FROM links l JOIN documents d ON d.doc_id=l.source_doc_id
             WHERE d.path='docs/sub/a.md' AND l.raw_href='../'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(row.0, "workspaceDirectory");
    assert_eq!(row.1.as_deref(), Some("docs"));
    assert_eq!(row.2, None, "un directorio no tiene target_doc_id");
}

/// C6 — el inventario incremental reclasifica un destino no documental al aparecer y vuelve a
/// `missing` al desaparecer, sin modificar el Markdown que contiene el enlace.
#[test]
fn c6_reconcile_reclasifica_asset_bin_missing_workspace_file_missing() {
    let root = tempfile::tempdir().unwrap();
    let raw = "# A\n\n[asset](asset.bin)\n";
    let files = lodestar_fixtures::file_map(&[("a.md", raw)]);
    write_all(root.path(), &files);
    let store = Store::open_and_build(root.path()).unwrap();

    let core_missing = DocumentSet::from_files(files.clone());
    let missing = core_missing
        .analyze()
        .outgoing
        .get(&rp("a.md"))
        .unwrap()
        .iter()
        .find(|link| link.href == "asset.bin")
        .unwrap();
    assert!(matches!(
        &missing.target,
        lodestar_core::types::LinkTarget::Missing(path) if path == &rp("asset.bin")
    ));
    assert_eq!(
        store.outgoing_links(&rp("a.md")).unwrap(),
        vec![(
            "asset.bin".to_string(),
            "missing".to_string(),
            Some("asset.bin".to_string()),
            None,
        )]
    );

    std::fs::write(root.path().join("asset.bin"), "binary\n").unwrap();
    store.reconcile_all().unwrap();
    let core_workspace_file = DocumentSet::with_other_files(files.clone(), [rp("asset.bin")]);
    let workspace_file = core_workspace_file
        .analyze()
        .outgoing
        .get(&rp("a.md"))
        .unwrap()
        .iter()
        .find(|link| link.href == "asset.bin")
        .unwrap();
    assert!(matches!(
        &workspace_file.target,
        lodestar_core::types::LinkTarget::WorkspaceFile(path) if path == &rp("asset.bin")
    ));
    assert_eq!(
        store.outgoing_links(&rp("a.md")).unwrap(),
        vec![(
            "asset.bin".to_string(),
            "workspaceFile".to_string(),
            Some("asset.bin".to_string()),
            None,
        )]
    );

    std::fs::remove_file(root.path().join("asset.bin")).unwrap();
    store.reconcile_all().unwrap();
    let core_missing_again = DocumentSet::from_files(files);
    let missing_again = core_missing_again
        .analyze()
        .outgoing
        .get(&rp("a.md"))
        .unwrap()
        .iter()
        .find(|link| link.href == "asset.bin")
        .unwrap();
    assert!(matches!(
        &missing_again.target,
        lodestar_core::types::LinkTarget::Missing(path) if path == &rp("asset.bin")
    ));
    assert_eq!(
        store.outgoing_links(&rp("a.md")).unwrap(),
        vec![(
            "asset.bin".to_string(),
            "missing".to_string(),
            Some("asset.bin".to_string()),
            None,
        )]
    );
}

/// C3 — `fields` es el diccionario único con paths reservados anclados al core.
#[test]
fn c3_diccionario_fields_normaliza_paths_anclados_sin_duplicados() {
    let root = tempfile::tempdir().unwrap();
    let corpus = files();
    write_all(root.path(), &corpus);
    let store = Store::open_and_build(root.path()).unwrap();
    assert!(store.documents().unwrap().len() >= 3);
    let conn = db(root.path());
    assert_eq!(columns(&conn, "fields"), vec!["field_id", "field_path"]);
    let count: i64 = conn
        .query_row("SELECT COUNT(*) FROM fields", [], |r| r.get(0))
        .unwrap();
    let distinct: i64 = conn
        .query_row("SELECT COUNT(DISTINCT field_path) FROM fields", [], |r| {
            r.get(0)
        })
        .unwrap();
    assert!(count > 0, "fields no vacío");
    assert_eq!(count, distinct, "paths únicos");
    for raw in ["graph.backlinks", "document.title", "service.tier"] {
        let path = FieldPath::parse(raw).unwrap();
        let expected = if path.es_namespace_reservado() {
            path.anclado().to_string()
        } else {
            path.to_string()
        };
        let one: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM fields WHERE field_path=?1",
                [&expected],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(one, 1, "path anclado único {expected}");
        let joined: i64 = conn.query_row("SELECT COUNT(*) FROM metadata m JOIN fields f ON f.field_id=m.field_id WHERE f.field_path=?1", [&expected], |r| r.get(0)).unwrap();
        assert!(joined > 0, "metadata apunta a field_id {expected}");
    }
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM fields WHERE field_path IN ('graph.backlinks','document.title')",
            [],
            |r| r.get::<_, i64>(0)
        )
        .unwrap(),
        0,
        "no conservar paths crudos"
    );
}

/// C3 — una clave reservada anclada y una clave YAML literal que imprime el mismo path no deben
/// provocar dos filas para el mismo nombre público. La forma anclada es la que el core resuelve.
#[test]
fn c3_colision_path_anclado_prioriza_core_y_no_duplica_metadata() {
    let root = tempfile::tempdir().unwrap();
    let files = lodestar_fixtures::file_map(&[(
        "collision.md",
        "---\ngraph:\n  backlinks: 7\n\"frontmatter.graph.backlinks\": literal\n---\n# Colisión\n",
    )]);
    write_all(root.path(), &files);

    let document_set = DocumentSet::from_files(files.clone());
    assert_eq!(document_set.files().len(), 1, "fixture C3 no vacío");
    let anchored = FieldPath::from_segments(["frontmatter", "graph", "backlinks"])
        .expect("path anclado válido");
    let literal =
        FieldPath::from_segments(["frontmatter.graph.backlinks"]).expect("clave literal válida");
    let core_catalog = catalog(&document_set);
    let same_display: Vec<_> = core_catalog
        .fields
        .iter()
        .filter(|entry| entry.field.to_string() == "frontmatter.graph.backlinks")
        .collect();
    assert!(
        same_display.len() >= 2,
        "el core debe exponer ambas identidades que colisionan por Display: {core_catalog:?}"
    );
    let anchored_stats = core_catalog
        .fields
        .iter()
        .find(|entry| entry.field == anchored)
        .expect("el catálogo core debe conservar la forma anclada");
    assert!(
        core_catalog
            .fields
            .iter()
            .any(|entry| entry.field == literal),
        "la fixture debe ejercitar también la clave literal, no solo la forma anidada"
    );
    assert_eq!(
        anchored_stats.inferred_types.get(&ValueType::Number),
        Some(&1),
        "la forma anclada del core gana frente al string literal: {anchored_stats:?}"
    );
    let inspected = inspect_field(&document_set, &anchored);
    assert_eq!(inspected.present_in, 1, "la forma anclada está presente");
    assert_eq!(inspected.missing_in, 0, "el workspace tiene un documento");
    assert_eq!(
        inspected.inferred_types.get(&ValueType::Number),
        Some(&1),
        "inspect_field debe resolver graph.backlinks anclado al número 7: {inspected:?}"
    );
    assert_eq!(
        inspected.values,
        vec![lodestar_core::types::ValueCount {
            value: serde_yaml::from_str("7").unwrap(),
            count: 1,
        }],
        "inspect_field debe devolver el valor de la forma anclada, no el literal"
    );

    let store = Store::open_and_build(root.path()).expect(
        "la colisión de Display no debe fallar por UNIQUE; fields debe ser un diccionario de rutas",
    );
    assert_eq!(store.documents().unwrap().len(), 1, "store C3 no vacío");
    let conn = db(root.path());
    let field_count: i64 = conn
        .query_row(
            "SELECT COUNT(*) FROM fields WHERE field_path='frontmatter.graph.backlinks'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(field_count, 1, "una sola ruta pública en fields");
    let metadata: (i64, String, String) = conn
        .query_row(
            "SELECT m.doc_id,m.value_json,m.value_type \
             FROM metadata m JOIN fields f ON f.field_id=m.field_id \
             WHERE f.field_path='frontmatter.graph.backlinks'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert!(
        metadata.0 > 0,
        "metadata debe referenciar el documento por ID"
    );
    assert_eq!(
        metadata.1, "7",
        "metadata conserva el valor de la forma anclada"
    );
    assert_eq!(metadata.2, "number", "metadata conserva el tipo del core");
    assert_eq!(
        conn.query_row(
            "SELECT COUNT(*) FROM metadata m JOIN fields f ON f.field_id=m.field_id \
             WHERE f.field_path='frontmatter.graph.backlinks'",
            [],
            |row| row.get::<_, i64>(0),
        )
        .unwrap(),
        1,
        "un documento y un field producen una sola fila de metadata"
    );
}

/// C4 — FTS contentless/external-content ligado a `rowid=doc_id`, sin raw+body+contenido.
#[test]
fn c4_fts_vnext_no_duplica_corpus_y_devuelve_candidatos_reales() {
    let root = tempfile::tempdir().unwrap();
    let corpus = files();
    write_all(root.path(), &corpus);
    let store = Store::open_and_build(root.path()).unwrap();
    assert!(store.documents().unwrap().len() >= 3);
    let needle = "aguja-única-corpus";
    let candidates = store.fts_candidates(needle).unwrap();
    assert_eq!(
        candidates,
        vec![rp("a.md")],
        "FTS debe devolver exactamente los candidatos, no todo el corpus"
    );
    let core = core_matches(&corpus, needle);
    assert_eq!(
        core,
        vec![rp("a.md")],
        "oráculo core C4 no vacío y determinista"
    );
    assert_eq!(
        store.search(needle).unwrap(),
        core,
        "el candidato FTS debe confirmarse con la semántica textual del core"
    );
    let conn = db(root.path());
    let escaped = format!("\"{}\"", needle.replace('"', "\"\""));
    let mut stmt = conn
        .prepare(
            "SELECT f.rowid, d.doc_id, d.path
             FROM documents_fts f
             JOIN documents d ON d.doc_id=f.rowid
             WHERE documents_fts MATCH ?1
             ORDER BY d.path",
        )
        .unwrap();
    let direct_candidates = stmt
        .query_map([escaped], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, i64>(1)?,
                row.get::<_, String>(2)?,
            ))
        })
        .unwrap()
        .collect::<Result<Vec<_>, _>>()
        .unwrap();
    let a_doc_id: i64 = conn
        .query_row(
            "SELECT doc_id FROM documents WHERE path='a.md'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(
        direct_candidates,
        vec![(a_doc_id, a_doc_id, "a.md".to_string())],
        "C4 FTS MATCH debe devolver (rowid, doc_id, path) exactos y no todo el corpus"
    );
    let dc = columns(&conn, "documents");
    assert!(dc.contains(&"doc_id".into()) && dc.contains(&"body".into()));
    assert!(!dc.contains(&"raw".into()), "no duplicación raw + body");
    for (path, raw) in corpus
        .iter()
        .filter(|(path, _)| path.as_str().ends_with(".md"))
    {
        let stored: String = conn
            .query_row(
                "SELECT body FROM documents WHERE path=?1",
                [path.as_str()],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            stored.as_bytes(),
            raw.as_bytes(),
            "documents.body debe conservar el Markdown exacto de {path}"
        );
    }
    let ddl = sql(&conn, "documents_fts").to_ascii_lowercase();
    assert!(!ddl.is_empty());
    let contentless = ddl.contains("content=''") || ddl.contains("content=\"\"");
    let external = ddl.contains("content=documents") && ddl.contains("content_rowid=doc_id");
    assert_eq!(
        contentless as u8 + external as u8,
        1,
        "exactamente una variante FTS5 vNext"
    );
    assert!(!direct_candidates.is_empty(), "FTS ligado no vacío");
}

/// C4/C5/C6 — el FTS del Store real no puede conservar candidatos de valores reemplazados o
/// eliminados: se comprueba el ciclo completo con `fts_candidates`, no solo la búsqueda confirmada.
#[test]
fn c4_fts_candidates_reflejan_upsert_y_remove_sin_valores_antiguos() {
    let root = tempfile::tempdir().unwrap();
    let corpus = files();
    write_all(root.path(), &corpus);
    let store = Store::open_and_build(root.path()).unwrap();
    let old = "aguja-única-corpus";
    let new = "aguja-nueva-corpus";
    assert_eq!(store.fts_candidates(old).unwrap(), vec![rp("a.md")]);
    assert!(store.fts_candidates(new).unwrap().is_empty());

    let replacement = "# A reemplazado\n\naguja-nueva-corpus\n";
    assert!(store
        .upsert(&rp("a.md"), replacement, 0, replacement.len() as i64)
        .unwrap());
    assert!(
        store.fts_candidates(old).unwrap().is_empty(),
        "el valor antiguo no sobrevive al upsert"
    );
    assert_eq!(
        store.fts_candidates(new).unwrap(),
        vec![rp("a.md")],
        "el valor nuevo aparece exactamente una vez tras el upsert"
    );

    assert!(store.remove(&rp("a.md")).unwrap());
    assert!(
        store.fts_candidates(new).unwrap().is_empty(),
        "el valor nuevo desaparece tras remove"
    );
}

/// C6 — la forma nueva conserva igualdad observable con el core.
#[test]
fn c6_paridad_core_store_con_unicode_enlaces_dangling_y_diagnostics() {
    let root = tempfile::tempdir().unwrap();
    let corpus = files();
    write_all(root.path(), &corpus);
    let store = Store::open_and_build(root.path()).unwrap();
    let document_set = DocumentSet::from_files(corpus);
    let analysis = document_set.analyze();
    assert!(analysis.documents.len() >= 3, "core no vacío");
    let mut expected = analysis.documents.clone();
    expected.retain(|path| path.as_str().ends_with(".md"));
    expected.sort();
    let mut actual = store.documents().unwrap();
    actual.sort();
    assert_eq!(actual, expected);
    assert_eq!(
        store.search("aguja-única-corpus").unwrap(),
        vec![rp("a.md")]
    );
    let mut dangling: Vec<_> = analysis
        .dangling
        .iter()
        .filter(|link| link.target == rp("missing.md"))
        .map(|link| link.target.clone())
        .collect();
    dangling.sort();
    dangling.dedup();
    let mut observed = store.dangling().unwrap();
    observed.sort();
    assert_eq!(observed, dangling);
    assert!(
        store.outgoing_links(&rp("a.md")).unwrap().len() >= 3,
        "enlaces conocidos/workspaceFile/missing"
    );
    assert!(
        columns(&db(root.path()), "documents").contains(&"doc_id".into()),
        "paridad sobre vNext"
    );
}
