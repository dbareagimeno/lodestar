//! E35-H03 C1/C7 — rutas y cuerpos no UTF-8 en el rebuild streaming.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

use lodestar_core::types::{LinkTarget, RelPath};
use lodestar_core::DocumentSet;
use lodestar_discovery::{discover_inventory, DiscoveryPolicy};
use lodestar_store::Store;
use rusqlite::Connection;

fn write(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(target, contents).unwrap();
}

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("fixture RelPath valida")
}

fn trace_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// C1/C7 + §20.9 — En Unix, una entrada regular admitida cuyo nombre contiene `0xff` no se
/// convierte con perdida en un `RelPath`: queda fuera del inventario tipado, produce exactamente
/// un `PATH-NOT-UTF8` sin targets y no impide reconstruir/indexar el documento valido vecino.
#[cfg(target_os = "linux")]
#[test]
fn c1_c7_non_utf8_filename_is_diagnosed_without_aborting_rebuild() {
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt as _;

    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "good.md",
        "# Good\n\nutf8-valid-neighbour-sentinel\n",
    );
    let invalid_name = OsString::from_vec(b"bad-\xff.md".to_vec());
    let invalid_path = root.path().join(&invalid_name);
    std::fs::write(&invalid_path, b"# Must stay outside typed inventory\n").unwrap();
    assert!(
        std::fs::symlink_metadata(&invalid_path)
            .unwrap()
            .file_type()
            .is_file(),
        "guard anti-vacuidad: bad-\\xff.md existe como fichero regular real"
    );

    let inventory = discover_inventory(root.path(), &DiscoveryPolicy::default())
        .expect("C1: un nombre no UTF-8 no debe abortar discovery");
    assert_eq!(
        inventory.documents,
        vec![rp("good.md")],
        "C1/C7: no se inventa un documento bad-\u{fffd}.md"
    );
    assert!(
        inventory.other_files.is_empty(),
        "una ruta no representable tampoco puede colarse en other_files mediante un RelPath inventado: {:?}",
        inventory.other_files
    );
    let path_not_utf8: Vec<_> = inventory
        .diagnostics
        .iter()
        .filter(|check| check.code.as_str() == "PATH-NOT-UTF8")
        .collect();
    assert_eq!(
        path_not_utf8.len(),
        1,
        "§20.9: exactamente un PATH-NOT-UTF8 para la unica entrada no representable: {:?}",
        inventory.diagnostics
    );
    assert!(
        path_not_utf8[0].targets.is_empty(),
        "PATH-NOT-UTF8 no puede inventar un RelPath target"
    );

    let store = Store::open_and_build(root.path())
        .expect("C1: la misma entrada no UTF-8 tampoco debe abortar el rebuild");
    assert_eq!(
        store.documents().unwrap(),
        vec![rp("good.md")],
        "C7: el resto del workspace se conserva exactamente"
    );
    assert_eq!(
        store
            .fts_candidates("utf8-valid-neighbour-sentinel")
            .unwrap(),
        vec![rp("good.md")],
        "guard anti-vacuidad: el documento vecino se proyecto en FTS"
    );
}

/// C1/C7 + §20.12.2 — Un candidato que supera path y tamano en la primera pasada pero cuyo cuerpo
/// falla UTF-8 en la segunda se reclasifica como `other_file`, nunca se proyecta como documento o
/// FTS, y el rebuild conserva en su resultado observable exactamente un `DOC-NOT-UTF8` dirigido al
/// path representable. Un enlace desde un documento valido demuestra la clasificacion semantica.
#[test]
fn c1_c7_second_pass_non_utf8_body_is_other_file_and_reported() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "source.md",
        "# Source\n\n[invalid body](bad-body.md)\n\nutf8-valid-source-sentinel\n",
    );
    write(
        root.path(),
        "bad-body.md",
        [
            0xff, b'i', b'n', b'v', b'a', b'l', b'i', b'd', b'-', b'b', b'o', b'd', b'y', b'-',
            b's', b'e', b'n', b't', b'i', b'n', b'e', b'l', b'\n',
        ],
    );

    let first_pass = discover_inventory(root.path(), &DiscoveryPolicy::default()).unwrap();
    assert_eq!(
        first_pass.documents,
        vec![rp("bad-body.md"), rp("source.md")],
        "guard anti-vacuidad: path y tamano admiten ambos candidatos antes de leer cuerpos"
    );
    assert!(
        first_pass.diagnostics.is_empty(),
        "guard de dos pasadas: discovery no debe abrir ni clasificar aun el cuerpo: {:?}",
        first_pass.diagnostics
    );

    let store = Store::open(root.path()).unwrap();
    let report = store
        .rebuild()
        .expect("el cuerpo no UTF-8 no aborta rebuild");
    assert_eq!(
        store.documents().unwrap(),
        vec![rp("source.md")],
        "solo el candidato UTF-8 termina como documento"
    );
    assert_eq!(
        store.fts_candidates("utf8-valid-source-sentinel").unwrap(),
        vec![rp("source.md")],
        "guard anti-vacuidad: el documento valido si se proyecto"
    );
    assert!(
        store
            .fts_candidates("invalid-body-sentinel")
            .unwrap()
            .is_empty(),
        "C1/C7: el payload no UTF-8 no puede entrar en FTS"
    );
    let links = store.outgoing_links(&rp("source.md")).unwrap();
    assert!(
        links.iter().any(|(_, kind, path, _)| {
            kind == "workspaceFile" && path.as_deref() == Some("bad-body.md")
        }),
        "C7: el candidato reclasificado debe conservarse como workspaceFile: {links:?}"
    );

    let db = Connection::open(root.path().join(".lodestar/index.db")).unwrap();
    let other_files: Vec<String> = db
        .prepare("SELECT path FROM other_files ORDER BY path")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        other_files,
        vec!["bad-body.md"],
        "§20.12.2: la reclasificacion se materializa en other_files"
    );

    let diagnostics = report
        .get("diagnostics")
        .and_then(serde_json::Value::as_array)
        .unwrap_or_else(|| {
            panic!(
                "§20.12.2: el rebuild debe conservar los diagnosticos producidos en la segunda pasada; report={report}"
            )
        });
    let not_utf8: Vec<_> = diagnostics
        .iter()
        .filter(|check| check["code"] == "DOC-NOT-UTF8")
        .collect();
    assert_eq!(
        not_utf8.len(),
        1,
        "exactamente un DOC-NOT-UTF8 para el unico cuerpo invalido: {diagnostics:?}"
    );
    assert_eq!(
        not_utf8[0]["targets"],
        serde_json::json!(["bad-body.md"]),
        "DOC-NOT-UTF8 si tiene el RelPath representable del candidato"
    );

    let relational_inserts = report["relational_inserts"]
        .as_u64()
        .expect("C4: el informe expone relational_inserts");
    let fts_inserts = report["fts_inserts"]
        .as_u64()
        .expect("C4: el informe expone fts_inserts");
    let rows_written = report["rows_written"]
        .as_u64()
        .expect("C4: el informe expone rows_written");
    assert_eq!(
        relational_inserts, 3,
        "C4: relational_inserts reconcilia 1 documents + 1 other_files reclasificado + 1 links"
    );
    assert_eq!(
        fts_inserts, 1,
        "C4: el unico documento valido produce exactamente una fila FTS"
    );
    assert_eq!(
        rows_written, 4,
        "C4: rows_written reconcilia las 3 inserciones relacionales + 1 FTS"
    );
    assert_eq!(
        rows_written,
        relational_inserts + fts_inserts,
        "C4: el total informado debe ser la suma exacta de ambos contadores"
    );
}

/// C1/C7 + §20.12.2 — La clasificacion diferida de un candidato no UTF-8 forma parte del
/// inventario canonico tambien en el camino incremental. Un `reconcile_all` sin cambios en disco
/// no puede olvidar el `other_file` ni degradar a `missing` el enlace que el core clasifica como
/// `WorkspaceFile`.
#[test]
fn c1_c7_reconcile_preserva_candidato_no_utf8_como_workspace_file() {
    let root = tempfile::tempdir().unwrap();
    let source = "# Source\n\n[invalid body](bad.md)\n\nutf8-reconcile-source-sentinel\n";
    write(root.path(), "source.md", source);
    write(root.path(), "bad.md", [0xff, 0xfe, b'b', b'a', b'd', b'\n']);

    let files = lodestar_fixtures::file_map(&[("source.md", source)]);
    let core = DocumentSet::with_other_files(files, [rp("bad.md")]);
    let core_analysis = core.analyze();
    let core_link = core_analysis
        .outgoing
        .get(&rp("source.md"))
        .and_then(|links| links.iter().find(|link| link.href == "bad.md"))
        .expect("guard anti-vacuidad: el core analiza el enlace a bad.md");
    assert!(
        matches!(&core_link.target, LinkTarget::WorkspaceFile(path) if path == &rp("bad.md")),
        "oraculo C7: el inventario canonico clasifica bad.md como WorkspaceFile"
    );

    let store = Store::open_and_build(root.path()).expect("cold rebuild inicial");
    assert_eq!(
        store.outgoing_links(&rp("source.md")).unwrap(),
        vec![(
            "bad.md".to_string(),
            "workspaceFile".to_string(),
            Some("bad.md".to_string()),
            None,
        )],
        "guard anti-vacuidad: el cold rebuild parte de la clasificacion correcta"
    );
    assert_eq!(
        store.document_set().analyze(),
        core_analysis,
        "guard de paridad: antes de reconciliar SQLite coincide exactamente con el core"
    );

    let event = store
        .reconcile_all()
        .expect("reconcile_all sin cambios debe ser estable");
    assert!(
        event.removed.is_empty(),
        "guard causal: reconcile no puede declarar borrado ningun documento: {event:?}"
    );
    assert_eq!(
        std::fs::read(root.path().join("source.md")).unwrap(),
        source.as_bytes(),
        "guard causal: source.md no cambio entre rebuild y reconcile"
    );
    assert_eq!(
        std::fs::read(root.path().join("bad.md")).unwrap(),
        [0xff, 0xfe, b'b', b'a', b'd', b'\n'],
        "guard causal: el candidato no UTF-8 tampoco cambio"
    );
    let db = Connection::open(root.path().join(".lodestar/index.db")).unwrap();
    let other_files: Vec<String> = db
        .prepare("SELECT path FROM other_files ORDER BY path")
        .unwrap()
        .query_map([], |row| row.get(0))
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    assert_eq!(
        other_files,
        vec!["bad.md"],
        "C1: el candidato no UTF-8 sigue materializado en other_files tras reconcile"
    );
    assert_eq!(
        store.outgoing_links(&rp("source.md")).unwrap(),
        vec![(
            "bad.md".to_string(),
            "workspaceFile".to_string(),
            Some("bad.md".to_string()),
            None,
        )],
        "C7: reconcile no puede degradar WorkspaceFile a missing"
    );
    assert_eq!(
        store.document_set().analyze(),
        core_analysis,
        "C7: la cache reconciliada conserva paridad exacta con el core"
    );
}

/// C4 — El footer NDJSON final es evidencia externa del trabajo de carga, no un resumen
/// independiente. En un corpus con un documento, un enlace y un asset debe reconciliar exactamente
/// sus inserciones relacionales y FTS con las filas publicadas y con `RebuildReport`.
#[test]
fn c4_footer_sql_trace_reconcilia_rows_written_con_relacional_y_fts() {
    let _env_lock = trace_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "source.md",
        "# Source\n\n[asset](asset.bin)\n\ntrace-footer-source-sentinel\n",
    );
    write(root.path(), "asset.bin", b"trace-footer-asset-sentinel\n");
    let trace_path = root
        .path()
        .join(".lodestar/index.db.next.h03-sql-trace.ndjson");

    std::env::set_var("LODESTAR_H03_SQL_TRACE", &trace_path);
    let store = Store::open(root.path()).unwrap();
    let report = store.rebuild().expect("rebuild instrumentado real");
    std::env::remove_var("LODESTAR_H03_SQL_TRACE");

    let events: Vec<serde_json::Value> = std::fs::read_to_string(&trace_path)
        .expect("guard anti-vacuidad: la traza NDJSON existe")
        .lines()
        .map(|line| serde_json::from_str(line).expect("evento NDJSON valido"))
        .collect();
    assert!(
        events.len() > 3,
        "guard anti-vacuidad: header, SQL real y footer final son observables"
    );
    let footer = events
        .iter()
        .rev()
        .find(|event| event["event"] == "footer" && event["complete"] == true)
        .expect("C4: footer final complete=true");
    assert_eq!(
        events.last(),
        Some(footer),
        "guard: el footer completo es el ultimo evento de la traza"
    );
    let counts = footer["counts"].as_object().expect("C4: footer counts");
    let footer_relational = counts["relational_inserts"]
        .as_u64()
        .expect("C4: footer relational_inserts");
    let footer_fts = counts["fts_inserts"]
        .as_u64()
        .expect("C4: footer fts_inserts");
    let footer_rows = counts["rows_written"]
        .as_u64()
        .expect("C4: footer rows_written");

    let db = Connection::open(root.path().join(".lodestar/index.db")).unwrap();
    let actual_relational: u64 = db
        .query_row(
            "SELECT (SELECT COUNT(*) FROM documents) +\
                    (SELECT COUNT(*) FROM other_files) +\
                    (SELECT COUNT(*) FROM metadata) +\
                    (SELECT COUNT(*) FROM links) +\
                    (SELECT COUNT(*) FROM diagnostics)",
            [],
            |row| row.get(0),
        )
        .unwrap();
    // `documents_fts` es contentless y no admite un scan sin MATCH. El contrato inserta una
    // proyeccion FTS por documento valido; se reconcilia esa cardinalidad con la tabla dueña y se
    // demuestra la fila real mediante una consulta MATCH.
    let actual_fts: u64 = db
        .query_row("SELECT COUNT(*) FROM documents", [], |row| row.get(0))
        .unwrap();
    assert_eq!(
        store
            .fts_candidates("trace-footer-source-sentinel")
            .unwrap(),
        vec![rp("source.md")],
        "guard anti-vacuidad: la proyeccion FTS contada es consultable"
    );
    assert_eq!(
        (actual_relational, actual_fts),
        (3, 1),
        "guard anti-vacuidad: 1 documento + 1 other_file + 1 link y 1 proyeccion FTS"
    );
    assert_eq!(
        footer_relational, actual_relational,
        "C4: el footer reconcilia todas las familias relacionales publicadas"
    );
    assert_eq!(
        footer_fts, actual_fts,
        "C4: el footer reconcilia las inserciones FTS publicadas"
    );
    assert_eq!(
        report["relational_inserts"].as_u64(),
        Some(actual_relational),
        "guard: el informe reconcilia el mismo trabajo relacional"
    );
    assert_eq!(
        report["fts_inserts"].as_u64(),
        Some(actual_fts),
        "guard: el informe reconcilia el mismo trabajo FTS"
    );
    assert_eq!(
        footer_rows,
        footer_relational + footer_fts,
        "C4: footer counts.rows_written = counts.relational_inserts + counts.fts_inserts"
    );
    assert_eq!(
        footer_rows,
        report["rows_written"].as_u64().unwrap(),
        "C4: footer, informe y filas publicadas expresan el mismo total"
    );
}
