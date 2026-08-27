//! E35-H03 C1/C6 — regresión de una alta que aparece durante el rebuild.
//!
//! Esta prueba usa únicamente el seam ya expuesto por H03 (`after_snapshot_before_read`).
//! La raíz es un enlace simbólico deliberado: la implementación actual compara solamente el
//! fingerprint del enlace, no el directorio al que apunta, por lo que una alta en el destino no
//! invalida el snapshot y puede publicarse de forma silenciosa.

use std::path::Path;
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use lodestar_core::types::RelPath;
use lodestar_discovery::{discover_inventory, DiscoveryPolicy};
use lodestar_store::Store;

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

fn markdown(sentinel: &str) -> String {
    format!("---\ntitle: stable\n---\n\n# Stable\n\n{sentinel}\n")
}

fn failpoint_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(2);
    while !path.exists() && Instant::now() < deadline {
        thread::yield_now();
    }
    assert!(
        path.exists(),
        "seam H03 no observable en {}",
        path.display()
    );
}

#[cfg(unix)]
#[test]
fn c1_c6_alta_externa_durante_rebuild_exige_abort_y_preserva_activo() {
    let _env = failpoint_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    std::env::remove_var("LODESTAR_H03_FAILPOINT");

    let fixture = tempfile::tempdir().unwrap();
    let destination = fixture.path().join("workspace");
    let root = fixture.path().join("root");
    std::fs::create_dir(&destination).unwrap();
    std::os::unix::fs::symlink("workspace", &root).unwrap();
    write(&root, "stable.md", markdown("repair8-old-active"));

    let initial = Store::open_and_build(&root).unwrap();
    assert_eq!(
        initial.documents().unwrap(),
        vec![rp("stable.md")],
        "guard: existe una generación activa inequívoca"
    );
    assert_eq!(
        initial.fts_candidates("repair8-old-active").unwrap(),
        vec![rp("stable.md")],
        "guard: el sentinel anterior es consultable"
    );

    let discovered = discover_inventory(&root, &DiscoveryPolicy::default()).unwrap();
    assert_eq!(
        discovered.documents,
        vec![rp("stable.md")],
        "guard anti-vacuidad: la alta todavía no pertenece al inventario canónico"
    );

    let failpoint = format!("{}:after_snapshot_before_read", root.display());
    std::env::set_var("LODESTAR_H03_FAILPOINT", failpoint);
    let root_for_rebuild = root.clone();
    let rebuilding = thread::spawn(move || {
        let store = Store::open(&root_for_rebuild).unwrap();
        store.rebuild_from_discovered_inventory(&discovered)
    });

    let pause = destination.join(".lodestar/h03-pause-after-snapshot-before-read");
    wait_for(&pause);
    let external_root = destination.clone();
    let external = thread::spawn(move || {
        write(
            &external_root,
            "added.md",
            markdown("repair8-added-after-fingerprints"),
        );
    });
    external.join().unwrap();
    assert!(
        destination.join("added.md").is_file(),
        "guard: el proceso externo creó el Markdown admitido"
    );
    std::fs::write(
        destination.join(".lodestar/h03-release-after-snapshot-before-read"),
        b"release\n",
    )
    .unwrap();

    let result = rebuilding.join().unwrap();
    std::env::remove_var("LODESTAR_H03_FAILPOINT");
    assert!(
        result.is_err(),
        "C1/C6: una alta durante rebuild debe abortar antes del swap; el activo no puede adoptar un inventario obsoleto"
    );

    let active = Store::open(&root).unwrap();
    assert_eq!(
        active.documents().unwrap(),
        vec![rp("stable.md")],
        "C6: el activo anterior conserva exactamente su conjunto documental"
    );
    assert_eq!(
        active.fts_candidates("repair8-old-active").unwrap(),
        vec![rp("stable.md")],
        "C6: el sentinel anterior sigue consultable"
    );
    assert!(
        active
            .fts_candidates("repair8-added-after-fingerprints")
            .unwrap()
            .is_empty(),
        "C1/C6: el documento añadido no puede publicarse en una generación parcial"
    );
}
