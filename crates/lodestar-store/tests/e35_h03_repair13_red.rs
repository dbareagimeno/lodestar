//! E35-H03 repair13 — paridad de metadata en colisiones, reconciliación de contadores y
//! candidatos FTS para valores YAML cuya representación semántica no aparece en los bytes raw.

use std::fs;
use std::path::Path;

use lodestar_core::metadata::inspect_field;
use lodestar_core::types::{FieldPath, RelPath};
use lodestar_core::DocumentSet;
use lodestar_store::Store;
use rusqlite::Connection;

fn write(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(target, contents).unwrap();
}

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("fixture con RelPath valido")
}

fn database(root: &Path) -> Connection {
    Connection::open(root.join(".lodestar/index.db")).expect("indice publicado abrible")
}

fn row_count(db: &Connection, table: &str) -> u64 {
    assert!(
        [
            "documents",
            "other_files",
            "metadata",
            "links",
            "diagnostics"
        ]
        .contains(&table),
        "el helper solo admite familias relacionales cerradas"
    );
    db.query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
        row.get(0)
    })
    .unwrap()
}

/// C7 — dos identidades de metadata bajo un namespace reservado pueden rendir el mismo
/// `FieldPath` anclado. En ese caso debe ganar la forma que el core resuelve, sin segunda fila ni
/// sustitucion dependiente del orden del `walk`.
#[test]
fn c7_rebuild_prioriza_identidad_anclada_del_core_en_colision_reservada() {
    let root = tempfile::tempdir().unwrap();
    let raw = "---\ndocument:\n  title:\n    slug: nested-wins-repair13\n  \"title.slug\": flat-loses-repair13\n---\n# Collision\n";
    write(root.path(), "collision.md", raw);

    let files = lodestar_fixtures::file_map(&[("collision.md", raw)]);
    let documents = DocumentSet::from_files(files);
    let anchored = FieldPath::from_segments(["frontmatter", "document", "title", "slug"])
        .expect("path anclado valido");
    let inspected = inspect_field(&documents, &anchored);
    assert_eq!(
        inspected.present_in, 1,
        "guard anti-vacuidad: el core resuelve la forma anclada"
    );
    assert_eq!(
        inspected.values.len(),
        1,
        "guard anti-vacuidad: la colision tiene un valor core inequivoco"
    );
    assert_eq!(
        serde_json::to_value(&inspected.values[0].value).unwrap(),
        serde_json::json!("nested-wins-repair13"),
        "el oraculo es el acceso anclado del core, no el orden de insercion SQL"
    );

    let store = Store::open(root.path()).unwrap();
    store.rebuild().expect("cold rebuild canonico");
    assert_eq!(
        store.documents().unwrap(),
        vec![rp("collision.md")],
        "guard anti-vacuidad: se proyecto el documento de la colision"
    );

    let db = database(root.path());
    let actual: Vec<(String, String, String)> = db
        .prepare(
            "SELECT f.field_path,m.value_json,m.value_type \
             FROM metadata m JOIN fields f ON f.field_id=m.field_id \
             WHERE f.field_path='frontmatter.document.title.slug'",
        )
        .unwrap()
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        actual,
        vec![(
            "frontmatter.document.title.slug".into(),
            "\"nested-wins-repair13\"".into(),
            "string".into(),
        )],
        "C7: SQLite conserva exactamente la identidad/valor que gana en el core"
    );
}

/// C4 — el informe de un rebuild no vacio reconcilia todas las inserciones relacionales lógicas.
/// La fórmula observable es `documents + other_files + metadata + links + diagnostics`; FTS se
/// informa por separado y `rows_written` es la suma de ambos contadores.
#[test]
fn c4_rebuild_report_reconcilia_exactamente_todas_las_familias_relacionales() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "a.md",
        "---\nteam: platform-repair13\n---\n# A\n\n<<<<<<< ours\n[Target](b.md)\n=======\n>>>>>>> theirs\n",
    );
    write(root.path(), "b.md", "# B\n\nbody-repair13\n");
    write(root.path(), "asset.bin", b"asset-repair13\n");

    let store = Store::open(root.path()).unwrap();
    let report = store.rebuild().expect("rebuild instrumentado real");
    let db = database(root.path());

    let documents = row_count(&db, "documents");
    let other_files = row_count(&db, "other_files");
    let metadata = row_count(&db, "metadata");
    let links = row_count(&db, "links");
    let diagnostics = row_count(&db, "diagnostics");
    for (family, count) in [
        ("documents", documents),
        ("other_files", other_files),
        ("metadata", metadata),
        ("links", links),
        ("diagnostics", diagnostics),
    ] {
        assert!(
            count > 0,
            "guard anti-vacuidad: la fixture debe insertar filas de {family}"
        );
    }

    let expected_relational = documents + other_files + metadata + links + diagnostics;
    let reported_relational = report["relational_inserts"]
        .as_u64()
        .expect("C4: contador relational_inserts");
    assert_eq!(
        reported_relational, expected_relational,
        "C4: relational_inserts = documents({documents}) + other_files({other_files}) + metadata({metadata}) + links({links}) + diagnostics({diagnostics})"
    );
    assert_eq!(
        report["fts_inserts"].as_u64(),
        Some(documents),
        "guard: una proyeccion FTS por documento valido"
    );
    assert_eq!(
        report["rows_written"].as_u64(),
        Some(expected_relational + documents),
        "C4: rows_written reconcilia relacional + FTS"
    );

    let index_phase = report["phases"]
        .as_array()
        .expect("C4: fases observables")
        .iter()
        .find(|phase| phase["name"] == "index")
        .expect("C4: fase index presente");
    assert_eq!(
        index_phase["counters"]["relational_inserts"].as_u64(),
        Some(expected_relational),
        "C4: el contador por fase y el resumen reconcilian la misma carga"
    );
}

/// C7 — FTS debe ser un superset del matcher core también cuando el valor textual solo aparece
/// tras decodificar escapes YAML. `documents.body` contiene el raw (`\\u0066...`), por lo que
/// estas agujas prueban de forma no redundante la columna `frontmatter_text`, tanto para string
/// como para array.
#[test]
fn c7_fts_frontmatter_indexa_valores_yaml_decodificados_string_y_array() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "string.md",
        "---\nsummary: \"\\u0066tsstringneedle-repair13\"\n---\n# String\n",
    );
    write(
        root.path(),
        "array.md",
        "---\nowners: [\"\\u0066tsarrayneedle-repair13\"]\n---\n# Array\n",
    );

    let store = Store::open(root.path()).unwrap();
    store.rebuild().expect("cold rebuild con FTS real");
    assert_eq!(
        store.documents().unwrap(),
        vec![rp("array.md"), rp("string.md")],
        "guard anti-vacuidad: ambos documentos se proyectaron"
    );
    assert!(
        !fs::read_to_string(root.path().join("string.md"))
            .unwrap()
            .contains("ftsstringneedle-repair13"),
        "guard: la aguja string decodificada no aparece en el raw"
    );
    assert!(
        !fs::read_to_string(root.path().join("array.md"))
            .unwrap()
            .contains("ftsarrayneedle-repair13"),
        "guard: la aguja array decodificada no aparece en el raw"
    );

    assert_eq!(
        store.fts_candidates("ftsstringneedle-repair13").unwrap(),
        vec![rp("string.md")],
        "C7: frontmatter_text indexa el string semantico decodificado"
    );
    assert_eq!(
        store.fts_candidates("ftsarrayneedle-repair13").unwrap(),
        vec![rp("array.md")],
        "C7: frontmatter_text indexa las strings de un array decodificado"
    );
}
