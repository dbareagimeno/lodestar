//! Fase roja de E35-H02 para el informe reproducible del banco.

use rusqlite::Connection;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::process::{Command, Output};

use lodestar_app::App;
use lodestar_store::Store;

fn bench() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"));
    command.env("RUST_BACKTRACE", "1");
    command
}

fn report(output: Output, context: &str) -> Value {
    assert!(
        output.status.success(),
        "{context}: banco falló; stderr={} stdout={}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(!text.trim().is_empty());
    serde_json::from_str(text.trim())
        .unwrap_or_else(|e| panic!("{context}: JSON inválido: {e}; stdout={text}"))
}

fn tiny_extreme(context: &str) -> Value {
    let output = bench()
        .args([
            "--extreme",
            "--profile",
            "plano",
            "--scale",
            "1",
            "--iterations",
            "1",
        ])
        .output()
        .unwrap();
    report(output, context)
}

fn direct_sqlite_oracle(path: &std::path::Path) -> Value {
    let db = Connection::open(path).expect("C7: abrir conexión SQLite del oráculo");
    db.execute_batch("CREATE VIRTUAL TABLE temp.oracle_dbstat USING dbstat(main)")
        .expect("C7: activar dbstat en la conexión del oráculo");
    let mut schema_stmt = db
        .prepare("SELECT name, type FROM main.sqlite_schema")
        .expect("C7: leer schema SQLite del oráculo");
    let schema: BTreeMap<String, String> = schema_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))
        .expect("C7: filas del schema SQLite del oráculo")
        .collect::<Result<_, _>>()
        .expect("C7: materializar schema SQLite del oráculo");
    let mut pages_stmt = db
        .prepare("SELECT name, SUM(pgsize) FROM temp.oracle_dbstat GROUP BY name")
        .expect("C7: leer páginas dbstat del oráculo");
    let pages: BTreeMap<String, u64> = pages_stmt
        .query_map([], |row| Ok((row.get(0)?, row.get::<_, i64>(1)? as u64)))
        .expect("C7: filas dbstat del oráculo")
        .collect::<Result<_, _>>()
        .expect("C7: materializar páginas dbstat del oráculo");
    let names: BTreeSet<String> = schema.keys().chain(pages.keys()).cloned().collect();
    let objects = names
        .into_iter()
        .map(|name| {
            let schema_type = schema.get(&name).map(String::as_str);
            let size = pages.get(&name).copied().unwrap_or(0);
            let kind = if name == "documents_fts" {
                "fts"
            } else if name.starts_with("documents_fts_") {
                "fts_shadow"
            } else if schema_type == Some("index") || name.starts_with("sqlite_autoindex_") {
                "index"
            } else {
                "table"
            };
            json!({"name": name, "kind": kind, "bytes": size})
        })
        .collect::<Vec<_>>();
    let page_count: u64 = db
        .query_row("PRAGMA page_count", [], |row| row.get::<_, i64>(0))
        .expect("C7: page_count del oráculo") as u64;
    let page_size: u64 = db
        .query_row("PRAGMA page_size", [], |row| row.get::<_, i64>(0))
        .expect("C7: page_size del oráculo") as u64;
    json!({"main_bytes": page_count * page_size, "objects": objects})
}

/// C7 — dbstat desglosa tablas, índices y FTS, y reconcilia main_bytes exactamente.
#[test]
fn c7_dbstat_desglosa_tablas_indices_fts_y_reconcilia_main_bytes() {
    let value = tiny_extreme("C7");
    let dbstat = value["sqlite"]["dbstat"]
        .as_object()
        .expect("C7: sqlite.dbstat");
    let main = dbstat["main_bytes"]
        .as_u64()
        .filter(|b| *b > 0)
        .expect("C7: main_bytes");
    let objects = dbstat["objects"].as_array().expect("C7: objects array");
    assert!(!objects.is_empty(), "C7: objetos no vacíos");
    let mut sum = 0_u64;
    let mut table = false;
    let mut index = false;
    let mut fts = false;
    for value in objects {
        let object = value.as_object().expect("C7: objeto");
        sum = sum
            .checked_add(object["bytes"].as_u64().expect("C7: bytes"))
            .unwrap();
        match object["kind"].as_str() {
            Some("table") => table = true,
            Some("index") => index = true,
            Some("fts") | Some("fts_shadow") => fts = true,
            other => panic!("C7: kind inválido {other:?}"),
        }
    }
    let unattributed = dbstat["unattributed_bytes"]
        .as_u64()
        .expect("C7: unattributed_bytes");
    assert!(table && index && fts, "C7: familias completas");
    assert_eq!(sum + unattributed, main, "C7: reconciliación exacta");

    // Cross-check the bank's JSON against an independent sqlite_schema/dbstat reader over a
    // populated database. The fixture deliberately exercises metadata, links and diagnostics;
    // matching only main_bytes would allow a report that silently omits whole families.
    let root = tempfile::tempdir().unwrap();
    std::fs::write(
        root.path().join("a.md"),
        "---\nservice:\n  tier: critical\n---\n# A\n\n[b](b.md) [missing](missing.md)\n",
    )
    .unwrap();
    std::fs::write(root.path().join("b.md"), "# B\n").unwrap();
    std::fs::write(root.path().join("broken.md"), "---\ntitle: [\n---\n").unwrap();
    let store = Store::open_and_build(root.path()).expect("C7: store poblado");
    let report = store.dbstat_report().expect("C7: informe del banco");
    assert!(
        report["objects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|object| object["name"] == "metadata"),
        "C7: DB poblada con metadata"
    );
    assert!(
        report["objects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|object| object["name"] == "links"),
        "C7: DB poblada con links"
    );
    assert!(
        report["objects"]
            .as_array()
            .unwrap()
            .iter()
            .any(|object| object["name"] == "diagnostics"),
        "C7: DB poblada con diagnostics"
    );
    drop(store);
    let oracle = direct_sqlite_oracle(&root.path().join(".lodestar/index.db"));
    let report_objects = report["objects"].as_array().unwrap();
    let oracle_objects = oracle["objects"].as_array().unwrap();
    let report_names = report_objects
        .iter()
        .map(|object| object["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    let oracle_names = oracle_objects
        .iter()
        .map(|object| object["name"].as_str().unwrap())
        .collect::<std::collections::BTreeSet<_>>();
    assert_eq!(
        report_names, oracle_names,
        "C7: no omitir tablas, índices ni shadows"
    );
    let report_objects_full = report_objects
        .iter()
        .map(|object| {
            (
                object["name"].as_str().unwrap(),
                (
                    object["kind"].as_str().unwrap(),
                    object["bytes"].as_u64().unwrap(),
                ),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    let oracle_objects_full = oracle_objects
        .iter()
        .map(|object| {
            (
                object["name"].as_str().unwrap(),
                (
                    object["kind"].as_str().unwrap(),
                    object["bytes"].as_u64().unwrap(),
                ),
            )
        })
        .collect::<std::collections::BTreeMap<_, _>>();
    assert_eq!(
        report_objects_full, oracle_objects_full,
        "C7: mapa completo name→(kind,bytes) viene de dbstat real"
    );
    assert_eq!(
        report["main_bytes"], oracle["main_bytes"],
        "C7: main_bytes directo"
    );
    for required in [
        "documents",
        "fields",
        "metadata",
        "links",
        "diagnostics",
        "documents_fts",
    ] {
        assert!(
            oracle_names.contains(required),
            "C7: falta objeto real {required}"
        );
    }
    assert!(
        oracle_names
            .iter()
            .any(|name| name.starts_with("documents_fts_")),
        "C7: faltan shadow tables FTS reales"
    );
}

/// C8 — ≤2,5× es objetivo explícito, no gate ni camino de lectura por defecto.
#[test]
fn c8_footprint_25x_es_objetivo_no_gate_y_store_no_es_read_default() {
    let value = tiny_extreme("C8");
    let footprint = value["footprint"].as_object().expect("C8: footprint");
    let objective = footprint["objective"].as_object().expect("C8: objective");
    assert_eq!(
        objective["max_ratio"].as_f64(),
        Some(2.5),
        "C8: target ≤2,5×"
    );
    assert_eq!(objective["gate"].as_bool(), Some(false), "C8: no es gate");
    assert_eq!(
        footprint["read_default"].as_bool(),
        Some(false),
        "C8: SQLite no es lectura por defecto con §14 abierta"
    );

    let root = tempfile::tempdir().unwrap();
    std::fs::write(root.path().join("default.md"), "# lectura por defecto\n").unwrap();
    let app = App::open(root.path()).expect("C8: App::open hermético");
    assert!(
        app.workspace().cache().is_none(),
        "C8: App::open no activa la cache por defecto"
    );
    assert!(
        !root.path().join(".lodestar/index.db").exists(),
        "C8: App::open no crea index.db por defecto"
    );
}
