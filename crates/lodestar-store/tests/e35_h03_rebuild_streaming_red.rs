//! Pruebas independientes de E35-H03. La historia exige que SQLite sea una cache derivada:
//! la política viene del workspace, la construcción es insert-only sobre `.next` y el activo
//! solo cambia después de verificar la generación completa.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;
#[cfg(unix)]
use std::path::PathBuf;
#[cfg(unix)]
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use lodestar_core::model;
use lodestar_core::types::{FieldPath, FileMap, LinkTarget, RelPath, Severity};
use lodestar_core::DocumentSet;
use lodestar_store::Store;
use rusqlite::Connection;

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("RelPath válido")
}

fn write(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(target, contents).unwrap();
}

fn markdown(title: &str, body: &str) -> String {
    format!("---\ntitle: {title}\nservice: bench\n---\n\n# {title}\n\n{body}\n")
}

fn failpoint_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[derive(Debug, Default)]
struct SqlTraceSummary {
    build_id: String,
    prepares: usize,
    executes: usize,
    deletes: usize,
    inserts_by_table: BTreeMap<String, usize>,
    lifecycle: Vec<(u64, String, String)>,
    load_footer_seq: Option<u64>,
}

fn read_sql_trace(path: &Path) -> SqlTraceSummary {
    let contents = std::fs::read_to_string(path)
        .unwrap_or_else(|error| panic!("H03: falta la traza SQL NDJSON {:?}: {error}", path));
    let mut lines = contents.lines();
    let header: serde_json::Value = serde_json::from_str(
        lines
            .next()
            .unwrap_or_else(|| panic!("H03: traza vacía: falta header")),
    )
    .expect("H03: header NDJSON válido");
    assert_eq!(
        header["event"].as_str(),
        Some("header"),
        "H03: primer evento header"
    );
    assert_eq!(header["seq"].as_u64(), Some(0), "H03: header seq=0");
    let build_id = header["build_id"]
        .as_str()
        .filter(|value| !value.trim().is_empty())
        .expect("H03: header build_id no vacío")
        .to_owned();
    let mut summary = SqlTraceSummary {
        build_id: build_id.clone(),
        ..SqlTraceSummary::default()
    };
    let mut expected_seq = 1_u64;
    let mut footer = None;
    for (line_number, line) in lines.by_ref().enumerate() {
        let event: serde_json::Value = serde_json::from_str(line).unwrap_or_else(|error| {
            panic!(
                "H03: línea {} de la traza no es JSON: {error}",
                line_number + 2
            )
        });
        assert_eq!(
            event["seq"].as_u64(),
            Some(expected_seq),
            "H03: secuencia NDJSON contigua en línea {}",
            line_number + 2
        );
        assert_eq!(
            event["build_id"].as_str(),
            Some(build_id.as_str()),
            "H03: build_id estable en línea {}",
            line_number + 2
        );
        expected_seq += 1;
        let kind = event["event"]
            .as_str()
            .unwrap_or_else(|| panic!("H03: línea {} sin event", line_number + 2));
        if kind == "footer" {
            if event["complete"].as_bool() != Some(true) {
                summary.load_footer_seq = Some(expected_seq - 1);
                continue;
            }
            assert_eq!(
                event["complete"].as_bool(),
                Some(true),
                "H03: footer complete=true"
            );
            footer = Some(event);
            break;
        }
        if matches!(kind, "integrity_check" | "swap" | "publication") {
            let result = event["result"].as_str().unwrap_or_default().to_owned();
            summary
                .lifecycle
                .push((expected_seq - 1, kind.to_owned(), result));
            continue;
        }
        let sql = event["sql"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| panic!("H03: línea {} sin sql no vacío", line_number + 2));
        let table = event["table"]
            .as_str()
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| panic!("H03: línea {} sin table no vacío", line_number + 2));
        if sql.trim_start().to_ascii_uppercase().starts_with("DELETE") {
            summary.deletes += 1;
        }
        match kind {
            "prepare" => summary.prepares += 1,
            "execute" => {
                summary.executes += 1;
                if sql.to_ascii_uppercase().contains("INSERT") {
                    *summary
                        .inserts_by_table
                        .entry(table.to_owned())
                        .or_default() += 1;
                }
            }
            other => panic!("H03: evento SQL desconocido {other:?}"),
        }
    }
    let footer = footer.expect("H03: falta footer NDJSON");
    assert!(
        lines.next().is_none(),
        "H03: footer debe ser el último evento"
    );
    let counts = footer["counts"].as_object().expect("H03: footer counts");
    assert_eq!(counts["prepare"].as_u64(), Some(summary.prepares as u64));
    assert_eq!(counts["execute"].as_u64(), Some(summary.executes as u64));
    assert_eq!(counts["delete"].as_u64(), Some(summary.deletes as u64));
    assert!(
        !summary.build_id.is_empty(),
        "H03: build_id observable por build"
    );
    assert!(
        summary.prepares > 0,
        "H03: la traza debe incluir prepares reales"
    );
    assert!(
        summary.executes > 0,
        "H03: la traza debe incluir ejecuciones reales"
    );
    summary
}

fn rebuild_report(store: &Store) -> serde_json::Value {
    let report = store.rebuild().expect("rebuild rojo debe poder ejecutarse");
    serde_json::to_value(report).expect("RebuildReport debe ser serializable")
}

fn metadata_type(value: &serde_yaml::Value) -> &'static str {
    match value {
        serde_yaml::Value::Null => "null",
        serde_yaml::Value::Bool(_) => "boolean",
        serde_yaml::Value::Number(_) => "number",
        serde_yaml::Value::String(_) => "string",
        serde_yaml::Value::Sequence(_) => "array",
        serde_yaml::Value::Mapping(_) => "object",
        serde_yaml::Value::Tagged(tagged) => metadata_type(&tagged.value),
    }
}

fn canonical_metadata(files: &FileMap) -> BTreeSet<(String, String, String, String)> {
    let mut result = BTreeSet::new();
    for (path, raw) in files {
        let parsed = model::parse_file(path.as_str(), raw);
        let Some(frontmatter) = parsed.frontmatter else {
            continue;
        };
        for (field_path, value) in frontmatter.walk() {
            let field_path = if field_path.es_namespace_reservado() {
                field_path.anclado()
            } else {
                field_path
            };
            let _ = FieldPath::parse(field_path.to_string().as_str())
                .expect("C7: la ruta canónica debe ser un FieldPath válido");
            result.insert((
                path.as_str().to_owned(),
                field_path.to_string(),
                serde_json::to_string(
                    &serde_json::to_value(value).expect("metadata representable como JSON"),
                )
                .expect("metadata JSON serializable"),
                metadata_type(value).to_owned(),
            ));
        }
    }
    result
}

fn logical_table_counts(db: &Connection) -> BTreeMap<String, usize> {
    [
        "documents",
        "metadata",
        "links",
        "diagnostics",
        "other_files",
    ]
    .into_iter()
    .map(|table| {
        let count: i64 = db
            .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                row.get(0)
            })
            .unwrap();
        (table.to_owned(), count as usize)
    })
    .collect()
}

#[cfg(unix)]
#[repr(C)]
struct KernelTimeval {
    seconds: i64,
    microseconds: i64,
}

#[cfg(unix)]
#[repr(C)]
struct KernelRusage {
    user_time: KernelTimeval,
    system_time: KernelTimeval,
    max_rss: i64,
    rest: [i64; 14],
}

#[cfg(unix)]
unsafe extern "C" {
    fn getrusage(who: i32, usage: *mut KernelRusage) -> i32;
}

#[cfg(unix)]
fn child_max_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<KernelRusage>::uninit();
    // SAFETY: `getrusage` initializes the caller-provided rusage structure for the requested
    // resource class. This test calls it immediately after wait(), so RUSAGE_CHILDREN includes
    // the isolated rebuild child and no sampling seam in production.
    let result = unsafe { getrusage(-1, usage.as_mut_ptr()) };
    assert_eq!(result, 0, "C2: getrusage(RUSAGE_CHILDREN) debe funcionar");
    // macOS reports ru_maxrss in bytes; Linux and the other Unix targets report KiB.
    let max_rss = unsafe { usage.assume_init() }.max_rss as u64;
    if cfg!(target_os = "macos") {
        max_rss
    } else {
        max_rss.saturating_mul(1024)
    }
}

#[cfg(unix)]
fn wait_with_timeout(child: &mut Child, timeout: Duration) -> Option<std::process::ExitStatus> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait().expect("esperar hijo") {
            return Some(status);
        }
        if started.elapsed() >= timeout {
            return None;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn c2_rebuild_from_inventory_lee_cada_cuerpo_exactamente_una_vez() {
    let root = tempfile::tempdir().unwrap();
    let payload = markdown("regular", "sentinel-c2-regular-reader");
    write(root.path(), "docs/once.md", &payload);
    let audit = root.path().join(".lodestar/.c2-reader-audit.ndjson");
    std::env::set_var("LODESTAR_H03_TEST_READ_AUDIT", &audit);
    let store = Store::open(root.path()).expect("C2: Store::open");
    let report = store
        .rebuild_from_inventory(&[rp("docs/once.md")], &BTreeSet::new())
        .expect("C2: rebuild con fichero regular");
    std::env::remove_var("LODESTAR_H03_TEST_READ_AUDIT");

    let events: Vec<serde_json::Value> = std::fs::read_to_string(&audit)
        .expect("C2: seam de auditoría de lectura real, no documents_read autodeclarado")
        .lines()
        .map(|line| serde_json::from_str(line).expect("C2: evento NDJSON válido"))
        .collect();
    assert_eq!(
        events.len(),
        1,
        "C2: exactamente una apertura/lectura del payload"
    );
    let event = &events[0];
    assert_eq!(event["event"].as_str(), Some("payload_read"));
    assert_eq!(event["path"].as_str(), Some("docs/once.md"));
    assert_eq!(event["open_count"].as_u64(), Some(1));
    assert_eq!(event["read_count"].as_u64(), Some(1));
    assert_eq!(event["bytes"].as_u64(), Some(payload.len() as u64));
    assert_eq!(
        report["documents_read"].as_u64(),
        Some(1),
        "C2: el contador del informe debe reconciliar con la auditoría, no sustituirla"
    );
}

#[cfg(unix)]
#[test]
fn c2_rebuild_streaming_mide_rss_durante_proceso_y_no_inventa_inventario_con_cuerpos() {
    if std::env::var_os("LODESTAR_H03_C2_CHILD").is_some() {
        std::env::remove_var("LODESTAR_H03_SQL_TRACE");
        std::env::remove_var("LODESTAR_H03_FAILPOINT");
        let root = PathBuf::from(std::env::var_os("LODESTAR_H03_C2_ROOT").unwrap());
        write(&root, ".c2-started", b"started");
        let store = Store::open(&root).unwrap();
        let report = rebuild_report(&store);
        write(
            &root,
            ".c2-report",
            serde_json::to_vec(&report).expect("C2: informe serializable"),
        );
        write(&root, ".c2-done", b"done");
        return;
    }
    let _env_lock = failpoint_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::remove_var("LODESTAR_H03_SQL_TRACE");
    std::env::remove_var("LODESTAR_H03_FAILPOINT");
    fn measure(root: &Path, document_count: usize, per_document: usize) -> serde_json::Value {
        for index in 0..document_count {
            write(
                root,
                &format!("docs/{index:03}.md"),
                markdown(
                    &format!("doc-{index}"),
                    &format!("c2-{index} {}", "x".repeat(per_document)),
                ),
            );
        }
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "c2_rebuild_streaming_mide_rss_durante_proceso_y_no_inventa_inventario_con_cuerpos",
                "--nocapture",
            ])
            .env("LODESTAR_H03_C2_CHILD", "1")
            .env("LODESTAR_H03_C2_ROOT", root)
            .spawn()
            .unwrap();
        let started = Instant::now();
        while !root.join(".c2-started").exists() && started.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(2));
        }
        assert!(
            root.join(".c2-started").exists(),
            "C2: el hijo debe marcar el inicio medido"
        );
        let _status = wait_with_timeout(&mut child, Duration::from_secs(30))
            .unwrap_or_else(|| panic!("C2: rebuild aislado no termina"));
        let status = child
            .wait()
            .expect("C2: esperar el hijo inmediatamente antes de getrusage");
        let mut report: serde_json::Value = serde_json::from_slice(
            &std::fs::read(root.join(".c2-report")).expect("C2: informe durante rebuild"),
        )
        .expect("C2: informe JSON válido");
        assert!(
            status.success(),
            "C2: el rebuild aislado debe terminar correctamente"
        );
        // La medición que gobierna C2 es externa al rebuild: el padre espera el hijo y consulta
        // inmediatamente la RSS máxima de sus hijos al kernel. El campo autoafirmado del informe
        // no puede sustituirla.
        report["external_peak_rss_bytes"] = serde_json::json!(child_max_rss_bytes());
        report
    }

    // Dos corpus homogéneos hacen observable la pendiente: el pico no puede seguir los bytes
    // totales si cada payload se libera al proyectarse.
    let small = tempfile::tempdir().unwrap();
    let large = tempfile::tempdir().unwrap();
    let per_document = 2 * 1024 * 1024;
    let small_report = measure(small.path(), 4, per_document);
    let large_report = measure(large.path(), 36, per_document);
    let corpus_delta = (32 * per_document) as u64;
    let small_peak = small_report["external_peak_rss_bytes"]
        .as_u64()
        .expect("C2: RSS externa del hijo debe ser observable");
    let large_peak = large_report["external_peak_rss_bytes"]
        .as_u64()
        .expect("C2: RSS externa del hijo debe ser observable");
    assert!(
        small_peak > 0 && large_peak > 0,
        "C2: RSS medido debe ser positivo"
    );
    assert!(
        large_peak.saturating_sub(small_peak) < corpus_delta / 2,
        "C2: el pico debe quedar acotado al escalar corpus; pequeño={small_peak}, grande={large_peak}, delta_corpus={corpus_delta}"
    );
    let small_live = small_report["max_live_body_bytes"]
        .as_u64()
        .expect("C2: memoria viva de payload observable");
    let large_live = large_report["max_live_body_bytes"]
        .as_u64()
        .expect("C2: memoria viva de payload observable");
    assert!(
        large_live.saturating_sub(small_live) < corpus_delta / 2,
        "C2: payload vivo no puede seguir los bytes totales; pequeño={small_live}, grande={large_live}"
    );
    let db = Connection::open(large.path().join(".lodestar/index.db")).unwrap();
    let columns: Vec<String> = db
        .prepare("PRAGMA table_info(other_files)")
        .unwrap()
        .query_map([], |row| row.get(1))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        columns,
        vec!["path"],
        "C2: inventario de otros ficheros solo conserva paths"
    );
}

#[test]
fn c3_rebuild_insert_only_reutiliza_proyecciones_y_no_borra_por_documento() {
    let mut prepared_counts = BTreeSet::new();
    for document_count in [4_usize, 8] {
        let root = tempfile::tempdir().unwrap();
        for index in 0..document_count {
            let next = (index + 1) % document_count;
            write(
                root.path(),
                &format!("docs/{index:03}.md"),
                markdown(
                    &format!("doc-{index}"),
                    &format!(
                        "[next](docs/{next:03}.md) [asset](../assets/blob.bin) aguja-c3-{index}\npriority: {index}"
                    ),
                ),
            );
        }
        write(root.path(), "assets/blob.bin", b"asset-c3\0bytes");
        write(
            root.path(),
            "docs/diagnostico.md",
            "---\ntitle: [\n---\n# diagnóstico c3\n",
        );
        let trace_path = root
            .path()
            .join(".lodestar/index.db.next.h03-sql-trace.ndjson");
        let _env_lock = failpoint_env_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::set_var("LODESTAR_H03_SQL_TRACE", &trace_path);
        let store = Store::open(root.path()).unwrap();
        let report = rebuild_report(&store);
        std::env::remove_var("LODESTAR_H03_SQL_TRACE");
        let trace = read_sql_trace(&trace_path);
        assert_eq!(
            trace.deletes, 0,
            "C3: la traza real no puede contener DELETE"
        );
        prepared_counts.insert(trace.prepares);
        let report_object = report
            .as_object()
            .expect("C3: rebuild debe devolver un informe JSON, no null");
        assert_eq!(
            report_object["delete_statements"].as_u64(),
            Some(0),
            "C3: la generación nueva es insert-only"
        );
        let prepared = report_object["prepared_statement_count"]
            .as_u64()
            .expect("C3: informe debe contar statements preparados");
        assert!(
            prepared > 0,
            "C3: debe reutilizar al menos un statement preparado"
        );
        assert_eq!(
            prepared, trace.prepares as u64,
            "C3: el informe debe reconciliar con la traza SQL real"
        );
        assert_eq!(
            report_object["documents_read"].as_u64(),
            Some((document_count + 1) as u64),
            "C3: el informe debe contar cada documento admitido"
        );
        assert!(
            report_object["rows_written"]
                .as_u64()
                .is_some_and(|rows| rows > document_count as u64),
            "C3: el informe debe contar filas reales de las proyecciones"
        );
        assert_eq!(store.documents().unwrap().len(), document_count + 1);
        for index in 0..document_count {
            assert_eq!(
                store.fts_candidates(&format!("aguja-c3-{index}")).unwrap(),
                vec![rp(&format!("docs/{index:03}.md"))],
                "C3: cada aguja debe devolver exactamente su documento esperado"
            );
        }
        let db = Connection::open(root.path().join(".lodestar/index.db")).unwrap();
        let final_counts = logical_table_counts(&db);
        assert!(
            final_counts["other_files"] > 0,
            "C3: el corpus debe contener al menos un asset en other_files"
        );
        for (table, expected) in &final_counts {
            assert_eq!(
                trace.inserts_by_table.get(table).copied().unwrap_or(0),
                *expected,
                "C3: inserts ejecutados de {table} deben coincidir con filas finales"
            );
        }
        assert!(
            trace
                .inserts_by_table
                .keys()
                .all(|table| table == "documents_fts" || final_counts.contains_key(table)),
            "C3: la traza no puede declarar inserts en tablas no proyectadas: {:?}",
            trace.inserts_by_table.keys().collect::<Vec<_>>()
        );
        for table in ["metadata", "links", "diagnostics"] {
            let count: i64 = db
                .query_row(&format!("SELECT COUNT(*) FROM {table}"), [], |row| {
                    row.get(0)
                })
                .unwrap();
            assert!(count > 0, "C3: la familia {table} debe tener filas reales");
        }
        assert_eq!(
            trace.inserts_by_table.get("documents_fts").copied(),
            Some(document_count + 1),
            "C3: la traza FTS debe insertar exactamente los documentos admitidos"
        );
    }
    assert_eq!(
        prepared_counts.len(),
        1,
        "C3: el número de preparaciones no puede crecer con N"
    );
}

#[test]
fn c5_integrity_check_precede_swap_y_publica_exactamente_la_generacion_nueva() {
    let root = tempfile::tempdir().unwrap();
    write(root.path(), "doc.md", markdown("old", "sentinel-activo-c5"));
    let store = Store::open_and_build(root.path()).unwrap();
    assert!(store
        .fts_candidates("sentinel-activo-c5")
        .unwrap()
        .contains(&rp("doc.md")));
    drop(store);
    let active_path = root.path().join(".lodestar/index.db");
    #[cfg(unix)]
    let active_before = {
        let metadata = std::fs::metadata(&active_path).unwrap();
        (metadata.dev(), metadata.ino())
    };
    write(root.path(), "doc.md", markdown("new", "sentinel-nuevo-c5"));
    let next = root.path().join(".lodestar/index.db.next");
    let trace_path = root
        .path()
        .join(".lodestar/index.db.next.h03-sql-trace.ndjson");
    let _env_lock = failpoint_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::remove_var("LODESTAR_H03_SQL_TRACE");
    std::env::set_var("LODESTAR_H03_SQL_TRACE", &trace_path);
    let rebuilt = Store::open(root.path()).unwrap();
    let report = rebuild_report(&rebuilt);
    std::env::remove_var("LODESTAR_H03_SQL_TRACE");
    let trace = read_sql_trace(&trace_path);
    let integrity_event = trace
        .lifecycle
        .iter()
        .find(|(_, kind, _)| kind == "integrity_check")
        .expect("C5: falta evento lifecycle integrity_check");
    assert_eq!(
        integrity_event.2, "ok",
        "C5: integrity_check lifecycle debe resultar ok"
    );
    let publication_event = trace
        .lifecycle
        .iter()
        .find(|(_, kind, _)| kind == "swap" || kind == "publication")
        .expect("C5: falta evento lifecycle swap/publication");
    assert!(
        integrity_event.0 < publication_event.0,
        "C5: integrity_check debe preceder a swap/publication por seq"
    );
    if let Some(load_footer_seq) = trace.load_footer_seq {
        assert!(
            load_footer_seq < integrity_event.0 && load_footer_seq < publication_event.0,
            "C5: lifecycle debe ocurrir después del footer de carga"
        );
    }
    assert_eq!(
        report["integrity_checked_before_swap"].as_bool(),
        Some(true),
        "C5: un único hecho debe demostrar integrity_check antes de publicar"
    );
    assert!(rebuilt
        .fts_candidates("sentinel-nuevo-c5")
        .unwrap()
        .contains(&rp("doc.md")));
    assert!(rebuilt
        .fts_candidates("sentinel-activo-c5")
        .unwrap()
        .is_empty());
    let published_db = Connection::open(&active_path).unwrap();
    let integrity_result: String = published_db
        .query_row("PRAGMA integrity_check", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        integrity_result, "ok",
        "C5: el index.db publicado debe superar exactamente integrity_check=ok"
    );
    #[cfg(unix)]
    {
        let metadata = std::fs::metadata(&active_path).unwrap();
        assert_ne!(
            active_before,
            (metadata.dev(), metadata.ino()),
            "C5: publicar debe reemplazar el inode activo en Unix"
        );
    }
    assert!(
        !next.exists(),
        "C5: generación previa no puede quedar adoptable"
    );
}

#[cfg(unix)]
#[test]
fn c6_fallo_antes_del_swap_conserva_activo_y_fuente_byte_a_byte() {
    let root = tempfile::tempdir().unwrap();
    let old = markdown("old", "sentinel-anterior-c6");
    write(root.path(), "doc.md", &old);
    let initial = Store::open_and_build(root.path()).unwrap();
    drop(initial);
    let before = std::fs::read(root.path().join("doc.md")).unwrap();
    write(root.path(), "doc.md", markdown("new", "sentinel-nuevo-c6"));
    let changed = std::fs::read(root.path().join("doc.md")).unwrap();
    assert_ne!(before, changed, "C6: inyección debe distinguir snapshots");
    let _env_lock = failpoint_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    for phase in ["corrupt_next_before_integrity", "before_swap"] {
        let failpoint = format!("{}:{phase}", root.path().display());
        std::env::set_var("LODESTAR_H03_FAILPOINT", &failpoint);
        let result = Store::open(root.path()).and_then(|store| store.rebuild());
        std::env::remove_var("LODESTAR_H03_FAILPOINT");
        assert!(result.is_err(), "C6: failpoint {phase} debe ser observable");
        let error = result.expect_err("C6: el failpoint debe devolver error");
        let next_path = root.path().join(".lodestar/index.db.next");
        assert!(
            next_path.exists(),
            "C6: una generación fallida debe conservar físicamente `.next`"
        );
        if phase == "corrupt_next_before_integrity" {
            let message = error.to_string().to_ascii_lowercase();
            assert!(
                message.contains("integrity") || message.contains("integridad"),
                "C6: corromper `.next` debe hacer fallar el integrity_check real: {message}"
            );
            let next_check = Connection::open(&next_path).and_then(|connection| {
                connection.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            });
            assert!(
                next_check
                    .as_ref()
                    .map(|result| !result.eq_ignore_ascii_case("ok"))
                    .unwrap_or(true),
                "C6: `.next` corrupta debe fallar o devolver integrity_check distinto de ok"
            );
        }
        let reopened = Store::open(root.path()).unwrap();
        assert!(reopened
            .fts_candidates("sentinel-anterior-c6")
            .unwrap()
            .contains(&rp("doc.md")));
        assert!(reopened
            .fts_candidates("sentinel-nuevo-c6")
            .unwrap()
            .is_empty());
        assert_eq!(
            changed,
            std::fs::read(root.path().join("doc.md")).unwrap(),
            "C6: Markdown canónico no se toca tras {phase}"
        );
        let _ = std::fs::remove_file(next_path);
    }
}

#[test]
fn c7_paridad_core_store_compara_valores_conjuntos_links_diagnostics_y_fts() {
    let root = tempfile::tempdir().unwrap();
    let all_files = lodestar_fixtures::file_map(&[("a.md", "---\nservice:\n  tier: crítico\nowners: [uno, dos]\n---\n# A\n\n[b](b.md#seccion) [asset](asset.bin) [roto](missing.md)\naguja-unicode-c7\n"), ("b.md", "---\nservice:\n  tier: estable\n---\n# B\n\n[vuelve](a.md)\n"), ("bad.md", "---\nservice: [\n---\n# Bad\n\naguja-bad-c7\n"), ("asset.bin", "bytes\n")]);
    for (path, contents) in &all_files {
        write(root.path(), path.as_str(), contents);
    }
    let files: FileMap = all_files
        .iter()
        .filter(|(path, _)| path.is_markdown())
        .map(|(path, contents)| (path.clone(), contents.clone()))
        .collect();
    let core = DocumentSet::with_other_files(files.clone(), [rp("asset.bin")]);
    let analysis = core.analyze();
    let store = Store::open_and_build(root.path()).unwrap();
    assert_eq!(
        store.documents().unwrap(),
        analysis.documents,
        "C7: conjunto de documentos"
    );
    let db = Connection::open(root.path().join(".lodestar/index.db")).unwrap();
    let actual: Vec<(String, String, serde_json::Value)> = db
        .prepare("SELECT path, body, frontmatter_json FROM documents ORDER BY path")
        .unwrap()
        .query_map([], |row| {
            let raw: String = row.get(2)?;
            Ok((
                row.get(0)?,
                row.get(1)?,
                serde_json::from_str(&raw).unwrap(),
            ))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    let expected: Vec<(String, String, serde_json::Value)> = files
        .iter()
        .map(|(path, raw)| {
            let parsed = model::parse_file(path.as_str(), raw);
            let fm = parsed
                .frontmatter
                .as_ref()
                .map(|f| serde_json::to_value(&f.value).unwrap())
                .unwrap_or_else(|| serde_json::json!({}));
            (path.as_str().into(), raw.clone(), fm)
        })
        .collect();
    assert_eq!(actual, expected, "C7: cuerpos y metadata exactos");
    let sql_metadata: BTreeSet<(String, String, String, String)> = db
        .prepare(
            "SELECT d.path, f.field_path, m.value_json, m.value_type \
             FROM metadata m \
             JOIN documents d ON d.doc_id = m.doc_id \
             JOIN fields f ON f.field_id = m.field_id \
             ORDER BY d.path, f.field_path",
        )
        .unwrap()
        .query_map([], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))
        })
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        sql_metadata,
        canonical_metadata(&files),
        "C7: metadata SQL unido a fields debe coincidir con ParsedFrontmatter::walk"
    );
    for path in &analysis.documents {
        let expected_links: Vec<_> = analysis.outgoing[path]
            .iter()
            .map(|link| {
                let kind = serde_json::to_value(&link.target).unwrap()["kind"]
                    .as_str()
                    .unwrap()
                    .to_string();
                let target = match &link.target {
                    LinkTarget::Document(p)
                    | LinkTarget::WorkspaceFile(p)
                    | LinkTarget::Missing(p) => Some(p.as_str().to_string()),
                    LinkTarget::WorkspaceDirectory(p) => p.as_ref().map(|p| p.as_str().to_string()),
                    _ => None,
                };
                (link.href.clone(), kind, target, link.fragment.clone())
            })
            .collect();
        assert_eq!(
            store.outgoing_links(path).unwrap(),
            expected_links,
            "C7: enlaces completos para {}",
            path.as_str()
        );
    }
    let sql_diagnostics: BTreeSet<_> = db.prepare("SELECT d.path, x.code, x.severity, x.message, x.range_json FROM diagnostics x JOIN documents d ON d.doc_id=x.doc_id").unwrap().query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?, row.get::<_, String>(3)?, row.get::<_, Option<String>>(4)?))).unwrap().collect::<Result<_, _>>().unwrap();
    let core_diagnostics: BTreeSet<_> = analysis
        .diagnostics
        .iter()
        .flat_map(|(path, checks)| {
            checks
                .iter()
                .filter(|check| {
                    !matches!(
                        check.code,
                        lodestar_core::types::CheckCode::LinkTargetMissing
                            | lodestar_core::types::CheckCode::LinkEscapesWorkspace
                            | lodestar_core::types::CheckCode::LinkCaseMismatch
                    )
                })
                .map(move |check| {
                    (
                        path.as_str().to_string(),
                        check.code.as_str().to_string(),
                        match check.level {
                            Severity::Pass => "pass",
                            Severity::Info => "info",
                            Severity::Warn => "warn",
                            Severity::Err => "err",
                        }
                        .into(),
                        check.msg.clone(),
                        check
                            .range
                            .map(|range| serde_json::to_string(&range).unwrap()),
                    )
                })
        })
        .collect();
    assert_eq!(
        sql_diagnostics, core_diagnostics,
        "C7: diagnósticos locales completos"
    );
    assert_eq!(
        store.fts_candidates("aguja-unicode-c7").unwrap(),
        vec![rp("a.md")]
    );
    assert_eq!(
        store.validation_counts().unwrap(),
        (analysis.hard_fail(), analysis.warn_count()),
        "C7: agregados de validación"
    );
    assert_eq!(
        store.document_set().analyze(),
        analysis,
        "C7: DocumentSet servido por SQLite debe ser idéntico al core"
    );
}
