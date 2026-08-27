//! Reparaciones rojas independientes para E35-H03.
//!
//! Cada prueba fija un contrato observable que la implementación actual todavía viola.  Este
//! fichero no aporta seams de producción: FIFO, failpoints y trazas son únicamente mecanismos de
//! observación del comportamiento ya expuesto por el store.

use std::collections::BTreeSet;
use std::path::Path;
#[cfg(unix)]
use std::sync::{mpsc, Arc};
use std::sync::{Mutex, OnceLock};
#[cfg(unix)]
use std::thread;
#[cfg(unix)]
use std::time::{Duration, Instant};

#[cfg(unix)]
use std::os::unix::fs::MetadataExt;

use lodestar_core::types::RelPath;
use lodestar_store::Store;

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("RelPath válido")
}

fn write(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(target, bytes).unwrap();
}

fn markdown(title: &str, body: &str) -> String {
    format!("---\ntitle: {title}\n---\n\n# {title}\n\n{body}\n")
}

fn env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

#[cfg(unix)]
fn wait_for_marker(path: &Path) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() && Instant::now() < deadline {
        thread::yield_now();
    }
    path.exists()
}

/// El gate de publicación forma parte del root, no de cada instancia de `Store`: dos handles
/// abiertos sobre el mismo workspace no deben poder escribir `.next` o el activo a la vez.
#[cfg(unix)]
#[test]
fn c6_dos_store_open_del_mismo_root_comparten_gate_de_escritura() {
    let _env = env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::remove_var("LODESTAR_H03_FAILPOINT");

    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "docs/old.md",
        markdown("old", "sentinel-old-repair4"),
    );
    let first = Arc::new(Store::open_and_build(root.path()).unwrap());
    let second = Arc::new(Store::open(root.path()).unwrap());
    for index in 0..16 {
        write(
            root.path(),
            &format!("docs/new-{index:02}.md"),
            markdown(
                &format!("new-{index}"),
                &format!("sentinel-new-repair4-{index} {}", "x".repeat(8 * 1024)),
            ),
        );
    }

    let failpoint = format!("{}:pause_before_swap", root.path().display());
    std::env::set_var("LODESTAR_H03_FAILPOINT", failpoint);
    let rebuilding = {
        let first = Arc::clone(&first);
        thread::spawn(move || first.rebuild())
    };
    let pause = root.path().join(".lodestar/h03-pause-before-swap");
    assert!(
        wait_for_marker(&pause),
        "C6 repair4: el primer handle debe quedar observable antes del swap"
    );

    let next = root.path().join(".lodestar/index.db.next");
    let before = std::fs::metadata(&next).expect("C6 repair4: `.next` del primer build");
    let before_shape = (before.dev(), before.ino(), before.len());
    let (started_tx, started_rx) = mpsc::channel();
    let rebuilding_second = {
        let second = Arc::clone(&second);
        thread::spawn(move || {
            started_tx.send(()).unwrap();
            second.rebuild()
        })
    };
    started_rx.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(
        !wait_for_next_change(&next, before_shape),
        "C6 repair4: un segundo Store::open no puede reemplazar `.next` mientras el primero está pausado"
    );

    std::fs::write(
        root.path().join(".lodestar/h03-release-before-swap"),
        b"release\n",
    )
    .unwrap();
    assert!(rebuilding.join().unwrap().is_ok());
    std::env::remove_var("LODESTAR_H03_FAILPOINT");
    assert!(rebuilding_second.join().unwrap().is_ok());
}

#[cfg(unix)]
fn wait_for_next_change(path: &Path, before: (u64, u64, u64)) -> bool {
    let deadline = Instant::now() + Duration::from_secs(2);
    while Instant::now() < deadline {
        if let Ok(metadata) = std::fs::metadata(path) {
            if (metadata.dev(), metadata.ino(), metadata.len()) != before {
                return true;
            }
        }
        thread::yield_now();
    }
    false
}

/// `Store::open_and_build` no puede ser una ruta pública con descubrimiento propio: el inventario
/// debe venir de `Workspace::document_set_with_discovery`, que aplica `.lodestarignore`, include,
/// exclude y `maxDocumentBytes`. Esta prueba fija la divergencia observable actual y el contrato
/// esperado sin intentar un compile-fail (Rust no permite probar la ausencia de un método público
/// desde una integración de forma portable).
#[test]
fn c1_open_and_build_debe_recibir_inventario_de_policy_canonica() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        ".lodestar/config.yaml",
        "discovery:\n  include: [\"docs/**/*.md\"]\n  exclude: [\"docs/excluded.md\"]\n  respectGitignore: true\n  respectLodestarIgnore: true\n  maxDocumentBytes: 256\n",
    );
    write(
        root.path(),
        "docs/admitted.md",
        markdown("admitted", "needle-admitted-repair4"),
    );
    write(
        root.path(),
        "docs/excluded.md",
        markdown("excluded", "needle-excluded-repair4"),
    );
    write(
        root.path(),
        "docs/too-large.md",
        markdown(
            "too-large",
            &format!("needle-large-repair4 {}", "x".repeat(4096)),
        ),
    );
    write(root.path(), ".lodestarignore", "docs/lodestar-ignored.md\n");
    write(
        root.path(),
        "docs/lodestar-ignored.md",
        markdown("ignored", "needle-lodestar-ignore-repair4"),
    );
    write(root.path(), ".gitignore", "docs/git-ignored.md\n");
    write(
        root.path(),
        "docs/git-ignored.md",
        markdown("git-ignored", "needle-git-ignore-repair4"),
    );

    let store = Store::open_and_build(root.path()).unwrap();
    assert_eq!(
        store.documents().unwrap(),
        vec![rp("docs/admitted.md")],
        "C1 repair4: open_and_build debe indexar exactamente el inventario canónico, no descubrir por su cuenta"
    );
    for needle in [
        "needle-excluded-repair4",
        "needle-large-repair4",
        "needle-lodestar-ignore-repair4",
        "needle-git-ignore-repair4",
    ] {
        assert!(
            store.fts_candidates(needle).unwrap().is_empty(),
            "C1 repair4: contenido fuera de policy no debe llegar a FTS: {needle}"
        );
    }
    assert!(
        store
            .fts_candidates("needle-admitted-repair4")
            .unwrap()
            .contains(&rp("docs/admitted.md")),
        "C1 repair4: el documento admitido debe tener una fila FTS real"
    );
}

/// El seam root-qualified pausa inmediatamente después del snapshot y antes de abrir el fichero.
/// El test reescribe un fichero regular con payload distinto pero del mismo tamaño, restaura el
/// mtime capturado y libera la barrera. Solo una verificación de contenido/lectura protegida puede
/// abortar la generación; el índice activo anterior debe quedar consultable.
#[cfg(unix)]
#[test]
fn c6_toctou_mismo_tamano_mtime_restaurado_aborta_y_preserva_activo() {
    let _env = env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::remove_var("LODESTAR_H03_FAILPOINT");
    let root = tempfile::tempdir().unwrap();
    let old = markdown("old", "sentinel-old-toctou-repair4");
    let new = markdown("new", "sentinel-new-toctou-repair4");
    assert_eq!(
        old.len(),
        new.len(),
        "C6 repair4: los payloads TOCTOU deben conservar exactamente el tamaño"
    );
    write(root.path(), "doc.md", &old);
    let initial = Store::open_and_build(root.path()).unwrap();
    assert!(initial
        .fts_candidates("sentinel-old-toctou-repair4")
        .unwrap()
        .contains(&rp("doc.md")));
    drop(initial);

    let source = root.path().join("doc.md");
    let stamp = root.path().join("mtime-stamp");
    std::process::Command::new("touch")
        .args(["-r", source.to_str().unwrap(), stamp.to_str().unwrap()])
        .status()
        .expect("C6 repair4: touch -r para conservar mtime")
        .success()
        .then_some(())
        .expect("C6 repair4: conservar mtime inicial");

    let failpoint = format!("{}:after_snapshot_before_read", root.path().display());
    std::env::set_var("LODESTAR_H03_FAILPOINT", failpoint);
    let rebuilding = {
        let root = root.path().to_path_buf();
        thread::spawn(move || {
            let store = Store::open(&root).unwrap();
            store.rebuild_from_inventory(&[rp("doc.md")], &BTreeSet::new())
        })
    };
    let pause = root
        .path()
        .join(".lodestar/h03-pause-after-snapshot-before-read");
    if !wait_for_marker(&pause) {
        std::env::remove_var("LODESTAR_H03_FAILPOINT");
        let _ = rebuilding.join();
        panic!("C6 repair4: falta seam root-qualified after_snapshot_before_read antes de leer");
    }

    write(root.path(), "doc.md", &new);
    std::process::Command::new("touch")
        .args(["-r", stamp.to_str().unwrap(), source.to_str().unwrap()])
        .status()
        .expect("C6 repair4: restaurar mtime del fichero regular")
        .success()
        .then_some(())
        .expect("C6 repair4: mtime TOCTOU restaurado");
    assert_eq!(
        std::fs::metadata(&source).unwrap().len(),
        old.len() as u64,
        "C6 repair4: el tamaño sigue siendo idéntico al snapshot"
    );
    std::fs::write(
        root.path()
            .join(".lodestar/h03-release-after-snapshot-before-read"),
        b"release\n",
    )
    .unwrap();

    let result = rebuilding.join().unwrap();
    std::env::remove_var("LODESTAR_H03_FAILPOINT");
    assert!(
        result.is_err(),
        "C6 repair4: cambiar el cuerpo con igual tamaño/mtime debe abortar el rebuild"
    );
    let store = Store::open(root.path()).unwrap();
    assert!(
        store
            .fts_candidates("sentinel-old-toctou-repair4")
            .unwrap()
            .contains(&rp("doc.md")),
        "C6 repair4: el índice activo anterior permanece consultable"
    );
    assert!(
        store
            .fts_candidates("sentinel-new-toctou-repair4")
            .unwrap()
            .is_empty(),
        "C6 repair4: una generación TOCTOU no puede publicarse"
    );
}

/// La medición de RSS debe estar asociada a cada fase y provenir del proceso, no ser el mismo
/// valor tomado después del swap ni `max_live_body_bytes` reutilizado como centinela/payload.
#[test]
fn c4_rebuild_reporta_rss_real_por_fase_en_orden_y_no_reutiliza_payload() {
    let root = tempfile::tempdir().unwrap();
    let body = "rss-sentinel-repair4 ".to_owned() + &"x".repeat(256 * 1024);
    write(root.path(), "docs/rss.md", markdown("rss", &body));
    let store = Store::open(root.path()).unwrap();
    let report = store.rebuild().unwrap();
    let phases = report["phases"]
        .as_array()
        .expect("C4 repair4: phases debe ser un array");
    assert_eq!(phases.len(), 4, "C4 repair4: cuatro fases observables");
    let names: Vec<_> = phases
        .iter()
        .map(|phase| phase["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["inventory", "index", "validate", "swap"]);
    let mut samples = Vec::new();
    let mut windows = Vec::new();
    for phase in phases {
        let started = phase["phase_started_monotonic_ns"]
            .as_u64()
            .expect("C4 repair4: inicio monotónico por fase");
        let sampled = phase["rss_sample_monotonic_ns"]
            .as_u64()
            .expect("C4 repair4: instante monotónico de muestra RSS");
        let finished = phase["phase_finished_monotonic_ns"]
            .as_u64()
            .expect("C4 repair4: fin monotónico por fase");
        assert!(
            started <= sampled && sampled <= finished,
            "C4 repair4: muestra RSS dentro de su ventana: start={started}, sample={sampled}, finish={finished}"
        );
        let rss = phase["peak_rss_bytes"]
            .as_u64()
            .expect("C4 repair4: muestra RSS positiva por fase");
        assert!(rss > 0, "C4 repair4: RSS positiva por fase");
        samples.push(rss);
        windows.push((started, finished));
    }
    for pair in windows.windows(2) {
        assert!(
            pair[0].1 <= pair[1].0,
            "C4 repair4: ventanas de fases ordenadas y no solapadas: {windows:?}"
        );
    }
    let live_body = report["max_live_body_bytes"]
        .as_u64()
        .expect("C4 repair4: payload vivo medido");
    assert!(
        samples.iter().all(|sample| *sample != live_body),
        "C4 repair4: RSS debe estar separado del payload/centinela, muestras={samples:?} payload={live_body}"
    );
}

/// La traza debe reconciliar exactamente diez prepares con el mismo `build_id`; el total permanece
/// constante respecto a N sin acoplar el test a los literales SQL de la implementación.
#[test]
fn c3_prepared_statement_count_incluye_streaming_projection_y_es_constante() {
    let mut counts = Vec::new();
    for document_count in [1_usize, 5] {
        let root = tempfile::tempdir().unwrap();
        for index in 0..document_count {
            write(
                root.path(),
                &format!("docs/{index}.md"),
                markdown(
                    &format!("doc-{index}"),
                    &format!(
                        "body-repair4-{index} [next]({}.md)",
                        (index + 1) % document_count
                    ),
                ),
            );
        }
        let trace = root.path().join(".lodestar/repair4-sql.ndjson");
        let _env = env_lock()
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        std::env::set_var("LODESTAR_H03_SQL_TRACE", &trace);
        let store = Store::open(root.path()).unwrap();
        let report = store.rebuild().unwrap();
        std::env::remove_var("LODESTAR_H03_SQL_TRACE");

        let lines = std::fs::read_to_string(&trace).unwrap();
        let prepares: Vec<serde_json::Value> = lines
            .lines()
            .filter_map(|line| serde_json::from_str(line).ok())
            .filter(|event: &serde_json::Value| event["event"] == "prepare")
            .collect();
        let sql: Vec<&str> = prepares
            .iter()
            .map(|event| event["sql"].as_str().unwrap())
            .collect();
        let build_ids: BTreeSet<&str> = prepares
            .iter()
            .map(|event| event["build_id"].as_str().unwrap())
            .collect();
        assert_eq!(
            build_ids.len(),
            1,
            "C3 repair4: todos los prepares deben proceder del mismo build_id"
        );
        assert_eq!(
            prepares.len(),
            10,
            "C3 repair4: exactamente diez prepares por rebuild: {sql:?}"
        );
        assert_eq!(
            report["prepared_statement_count"].as_u64(),
            Some(prepares.len() as u64),
            "C3 repair4: contador y traza deben reconciliar"
        );
        counts.push(prepares.len());
    }
    assert_eq!(
        counts[0], counts[1],
        "C3 repair4: preparar una vez por build, no una vez por documento: {counts:?}"
    );
}
