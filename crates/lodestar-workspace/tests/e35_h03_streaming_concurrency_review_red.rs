//! Revisión independiente de E35-H03: los tests de esta batería fijan los seams que faltaban
//! después del primer verde. No aceptan que `enable_cache` convierta el snapshot canónico en una
//! colección de cuerpos retenidos, ni que un swap publique por encima de un escritor concurrente.

use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use lodestar_core::types::RelPath;
use lodestar_store::Store;
use lodestar_workspace::Workspace;

fn write(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(target, bytes).unwrap();
}

fn markdown(index: usize, body_bytes: usize) -> String {
    let mut body = format!("# doc-{index}\n\nneedle-{index} ");
    body.push_str(&"x".repeat(body_bytes.saturating_sub(body.len())));
    format!("---\ntitle: doc-{index}\n---\n\n{body}\n")
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
fn process_rss_bytes() -> u64 {
    let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
    // SAFETY: getrusage initializes the output structure on success.
    let status = unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) };
    assert_eq!(status, 0, "H03 review: getrusage debe estar disponible");
    // SAFETY: status=0 means the structure was initialized.
    let usage = unsafe { usage.assume_init() };
    let raw = u64::try_from(usage.ru_maxrss).unwrap();
    if cfg!(target_os = "macos") {
        raw
    } else {
        raw.saturating_mul(1024)
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn process_rss_bytes() -> u64 {
    0
}

/// C2 review: el hijo mide el camino completo `Workspace::enable_cache`, antes de hacer una
/// consulta que pueda atribuir memoria a las lecturas. La comparación de dos escalas evita que un
/// RSS final bajo o un corpus vacío hagan pasar una implementación que materializa el FileMap.
#[test]
fn c2_enable_cache_no_retiene_los_cuerpos_del_snapshot_canonico() {
    const CHILD: &str = "E35_H03_ENABLE_CACHE_RSS_CHILD";
    if std::env::var_os(CHILD).is_some() {
        let root = PathBuf::from(std::env::var_os("E35_H03_ENABLE_CACHE_ROOT").unwrap());
        let expected = std::env::var("E35_H03_ENABLE_CACHE_DOCUMENTS")
            .unwrap()
            .parse::<usize>()
            .unwrap();
        let before = process_rss_bytes();
        let mut workspace = Workspace::open(&root).unwrap();
        workspace.enable_cache().unwrap();
        let after = process_rss_bytes();
        let documents = workspace.cache().unwrap().documents().unwrap();
        assert_eq!(documents.len(), expected, "C2 review: corpus no vacuo");
        println!(
            "{{\"rss_before\":{before},\"rss_after\":{after},\"rss_delta\":{},\"documents\":{}}}",
            after.saturating_sub(before),
            documents.len()
        );
        return;
    }

    if cfg!(not(any(target_os = "macos", target_os = "linux"))) {
        eprintln!("C2 review skipped: RSS process metric unavailable on this platform");
        return;
    }

    let executable = std::env::current_exe().unwrap();
    let body_bytes = 1024 * 1024;
    let mut deltas = Vec::new();
    for documents in [16_usize, 96] {
        let root = tempfile::tempdir().unwrap();
        write(
            root.path(),
            ".lodestar/config.yaml",
            "discovery:\n  include: [\"docs/**/*.md\"]\n  respectGitignore: true\n  respectLodestarIgnore: true\n",
        );
        for index in 0..documents {
            write(
                root.path(),
                &format!("docs/doc-{index}.md"),
                markdown(index, body_bytes),
            );
        }
        let output = std::process::Command::new(&executable)
            .args([
                "--exact",
                "c2_enable_cache_no_retiene_los_cuerpos_del_snapshot_canonico",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .env("E35_H03_ENABLE_CACHE_ROOT", root.path())
            .env("E35_H03_ENABLE_CACHE_DOCUMENTS", documents.to_string())
            .env("RUST_TEST_THREADS", "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "C2 review: hijo falló: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8_lossy(&output.stdout);
        let line = stdout
            .lines()
            .find_map(|line| line.find('{').map(|offset| &line[offset..]))
            .unwrap_or_else(|| {
                panic!(
                    "C2 review: hijo debe emitir la medición JSON; stdout={} stderr={}",
                    String::from_utf8_lossy(&output.stdout),
                    String::from_utf8_lossy(&output.stderr)
                )
            });
        let report: serde_json::Value = serde_json::from_str(line).unwrap();
        assert_eq!(report["documents"].as_u64(), Some(documents as u64));
        let delta = report["rss_delta"].as_u64().expect("C2 review: RSS delta");
        assert!(
            delta > body_bytes as u64,
            "C2 review: cuerpos reales medidos"
        );
        deltas.push(delta);
    }

    let additional_bytes = 80_u64 * body_bytes as u64;
    assert!(
        deltas[1].saturating_sub(deltas[0]) <= additional_bytes / 4,
        "C2 review: el crecimiento RSS entre escalas no debe materializar el corpus completo; small={} large={} additional={additional_bytes}",
        deltas[0],
        deltas[1]
    );
}

fn failpoint_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

/// C6 review: el contrato de pausa permite observar el índice viejo desde un `Store` ya abierto,
/// mientras un escritor queda detrás del mismo gate. El marcador/release es root-qualified para
/// que dos workspaces en el mismo proceso no se interfieran.
#[test]
fn c6_pause_before_swap_preserva_lecturas_y_serializa_el_escritor() {
    let _env_lock = failpoint_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::remove_var("LODESTAR_H03_FAILPOINT");

    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "docs/old.md",
        "# old\n\nold-generation-sentinel\n",
    );
    let store = Arc::new(Store::open_and_build(root.path()).unwrap());
    write(
        root.path(),
        "docs/new.md",
        "# new\n\nnew-generation-sentinel\n",
    );

    let pause_marker = root.path().join(".lodestar/h03-pause-before-swap");
    let release_marker = root.path().join(".lodestar/h03-release-before-swap");
    let writer_waiting_marker = root.path().join(".lodestar/h03-writer-waiting");
    std::env::set_var(
        "LODESTAR_H03_FAILPOINT",
        format!("{}:pause_before_swap", root.path().display()),
    );
    let building = {
        let store = Arc::clone(&store);
        thread::spawn(move || store.rebuild())
    };

    let deadline = Instant::now() + Duration::from_secs(2);
    while !pause_marker.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        pause_marker.exists(),
        "C6 review: falta el seam determinista pause_before_swap; el rebuild no puede observarse durante el swap"
    );
    assert!(
        store
            .fts_candidates("old-generation-sentinel")
            .unwrap()
            .contains(&RelPath::new("docs/old.md").unwrap()),
        "C6 review: un lector ya abierto debe seguir viendo la generación activa"
    );

    let (writer_done, writer_result) = mpsc::channel();
    let writer = {
        let store = Arc::clone(&store);
        thread::spawn(move || {
            let result = store.upsert(
                &RelPath::new("docs/writer.md").unwrap(),
                "# writer\n\nwriter-generation-sentinel\n",
                0,
                38,
            );
            writer_done.send(result).unwrap();
        })
    };
    let writer_deadline = Instant::now() + Duration::from_secs(2);
    while !writer_waiting_marker.exists() && Instant::now() < writer_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    assert!(
        writer_waiting_marker.exists(),
        "C6 review: falta marcador root-qualified writer_waiting antes de comprobar el bloqueo"
    );
    assert!(
        writer_result
            .recv_timeout(Duration::from_millis(150))
            .is_err(),
        "C6 review: el escritor no puede publicar sobre el índice activo mientras el rebuild está pausado"
    );

    std::fs::write(&release_marker, "release\n").unwrap();
    let rebuild_result = building.join().unwrap();
    std::env::remove_var("LODESTAR_H03_FAILPOINT");
    assert!(
        rebuild_result.is_ok(),
        "C6 review: liberar la pausa debe completar el swap"
    );
    writer.join().unwrap();
    assert!(
        writer_result
            .recv_timeout(Duration::from_secs(2))
            .expect("C6 review: escritor finalmente desbloqueado")
            .is_ok(),
        "C6 review: escritor desbloqueado debe aplicar su cambio"
    );
    assert!(
        store
            .fts_candidates("new-generation-sentinel")
            .unwrap()
            .contains(&RelPath::new("docs/new.md").unwrap()),
        "C6 review: generación nueva publicada"
    );
    assert!(
        store
            .fts_candidates("writer-generation-sentinel")
            .unwrap()
            .contains(&RelPath::new("docs/writer.md").unwrap()),
        "C6 review: el cambio concurrente no puede perderse después del swap"
    );
}
