//! E35-H03 C6: un crash entre retirar los sidecars y publicar `.next` conserva el activo.
//!
//! El proceso padre mantiene una conexion WAL antigua abierta. El worker construye la nueva
//! generacion y se pausa, mediante un seam observable, despues de retirar los nombres WAL/SHM y
//! antes de `replace_durable`. El padre mata el worker sin liberar la pausa, de modo que ni `Drop`
//! ni un rollback cooperativo pueden reparar el estado. Un tercer proceso abre entonces el Store
//! por nombre y exige que el snapshot anterior siga siendo consultable.

use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use lodestar_core::types::RelPath;
use lodestar_store::Store;

const TEST_NAME: &str = "c6_crash_tras_retirar_sidecars_conserva_snapshot_wal_anterior";
const OLD_FTS: &str = "repair17-old-wal-sentinel";
const PRIOR_FTS: &str = "repair17-prior-wal-sentinel";
const NEW_FTS: &str = "repair17-new-candidate-sentinel";
const ADDED_FTS: &str = "repair17-added-candidate-sentinel";

fn write(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(target, contents).unwrap();
}

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("fixture con RelPath valido")
}

fn child(role: &str, root: &Path) -> Child {
    Command::new(std::env::current_exe().expect("ruta del integration test"))
        .args(["--exact", TEST_NAME, "--nocapture"])
        .env("E35_H03_REPAIR17_CHILD", role)
        .env("E35_H03_REPAIR17_ROOT", root)
        .spawn()
        .expect("arranca proceso hijo")
}

fn wait_child(child: &mut Child, limit: Duration) -> ExitStatus {
    let deadline = Instant::now() + limit;
    loop {
        if let Some(status) = child.try_wait().expect("consulta estado del hijo") {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("proceso hijo no termino dentro de {limit:?}");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn wait_for_crash_marker(worker: &mut Child, marker: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if marker.exists() {
            return;
        }
        if let Some(status) = worker.try_wait().expect("consulta estado del worker") {
            panic!(
                "C6 rojo: el rebuild termino ({status}) antes de crear el marcador post-sidecars requerido: {}",
                marker.display()
            );
        }
        if Instant::now() >= deadline {
            let _ = worker.kill();
            let _ = worker.wait();
            panic!(
                "C6 rojo: no aparecio a tiempo el marcador post-sidecars requerido: {}",
                marker.display()
            );
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn verify_old_snapshot(root: &Path) {
    let store = Store::open(root).expect("otro proceso reabre el index.db activo");
    assert_eq!(
        store.documents().expect("consulta documentos del activo"),
        vec![rp("old.md"), rp("prior.md")],
        "C6: ni el cuerpo nuevo ni el documento candidato pueden adoptarse tras el crash"
    );
    assert_eq!(
        store
            .fts_candidates(OLD_FTS)
            .expect("consulta FTS del sentinela WAL anterior"),
        vec![rp("old.md")],
        "C6: el sentinela anterior debe seguir siendo consultable, no solo existir index.db"
    );
    assert_eq!(
        store
            .fts_candidates(PRIOR_FTS)
            .expect("consulta FTS del segundo documento anterior"),
        vec![rp("prior.md")],
        "guard anti-vacuidad: el snapshot anterior debe contener datos efectivos en FTS"
    );
    assert!(
        store
            .fts_candidates(NEW_FTS)
            .expect("consulta FTS del cuerpo candidato")
            .is_empty(),
        "el cuerpo nuevo de old.md no puede sustituir al snapshot activo"
    );
    assert!(
        store
            .fts_candidates(ADDED_FTS)
            .expect("consulta FTS del documento candidato")
            .is_empty(),
        "index.db.next no puede adoptar added.md"
    );
}

fn verify_in_separate_process(root: &Path) {
    let mut verifier = child("verify", root);
    let status = wait_child(&mut verifier, Duration::from_secs(20));
    assert!(
        status.success(),
        "el verificador externo del snapshot anterior termino {status}"
    );
}

/// C6 — Si el proceso muere despues de retirar WAL/SHM y antes del unico reemplazo atomico, el
/// `index.db` anterior conserva documentos y FTS consultables desde otro proceso. La generacion
/// `.next` no se adopta y ningun byte Markdown cambia durante el intento.
#[test]
fn c6_crash_tras_retirar_sidecars_conserva_snapshot_wal_anterior() {
    let role = std::env::var("E35_H03_REPAIR17_CHILD").ok();
    if let Some(role) = role.as_deref() {
        let root = PathBuf::from(
            std::env::var_os("E35_H03_REPAIR17_ROOT").expect("root del proceso hijo"),
        );
        match role {
            "worker" => {
                std::env::set_var(
                    "LODESTAR_H03_FAILPOINT",
                    format!("{}:pause_after_sidecar_cleanup_before_swap", root.display()),
                );
                let store = Store::open(&root).expect("worker abre la generacion anterior");
                store
                    .rebuild()
                    .expect("el worker solo debe detenerse en el seam previo a replace_durable");
            }
            "verify" => verify_old_snapshot(&root),
            other => panic!("rol de proceso desconocido: {other}"),
        }
        return;
    }

    let root = tempfile::tempdir().unwrap();
    let bootstrap_markdown = b"# Bootstrap anterior\nrepair17-bootstrap-sentinel\n";
    let old_markdown = b"# Snapshot anterior en WAL\nrepair17-old-wal-sentinel\n";
    let prior_markdown = b"# Documento previo\nrepair17-prior-wal-sentinel\n";
    write(root.path(), "old.md", bootstrap_markdown);
    write(root.path(), "prior.md", prior_markdown);

    // Mantener esta conexion abierta hace real el caso WAL: otro proceso debe poder retirar los
    // nombres de sidecar aunque todavia haya un lector de la generacion anterior.
    let active_store = Store::open_and_build(root.path()).expect("crea snapshot WAL anterior");
    write(root.path(), "old.md", old_markdown);
    assert!(
        active_store
            .upsert(
                &rp("old.md"),
                std::str::from_utf8(old_markdown).unwrap(),
                0,
                0
            )
            .expect("materializa sentinela anterior en WAL"),
        "guard anti-vacuidad: el upsert anterior debe escribir frames WAL efectivos"
    );
    assert_eq!(
        active_store.documents().unwrap(),
        vec![rp("old.md"), rp("prior.md")],
        "guard anti-vacuidad: el snapshot previo contiene ambos documentos"
    );
    assert_eq!(
        active_store.fts_candidates(OLD_FTS).unwrap(),
        vec![rp("old.md")],
        "guard anti-vacuidad: el sentinela previo esta materializado en FTS"
    );

    let cache = root.path().join(".lodestar");
    let active = cache.join("index.db");
    let wal = cache.join("index.db-wal");
    let shm = cache.join("index.db-shm");
    assert!(
        wal.exists(),
        "guard anti-vacuidad: el activo debe estar en WAL"
    );
    assert!(
        shm.exists(),
        "guard anti-vacuidad: el activo debe tener SHM"
    );
    assert!(
        std::fs::metadata(&wal).unwrap().len() > 32,
        "guard anti-vacuidad: WAL debe contener frames, no ser un sidecar vacio"
    );
    let active_identity = lodestar_discovery::filesystem_fingerprint(&active, true)
        .expect("identidad del index.db anterior")
        .identity;

    // El disco Markdown ya representa el candidato que el rebuild intentara publicar. Estos son
    // los bytes que deben sobrevivir exactamente iguales al crash.
    let new_old_markdown = b"# Snapshot candidato\nrepair17-new-candidate-sentinel\n";
    let added_markdown = b"# Documento candidato\nrepair17-added-candidate-sentinel\n";
    write(root.path(), "old.md", new_old_markdown);
    write(root.path(), "added.md", added_markdown);
    let markdown_before = [
        (root.path().join("old.md"), new_old_markdown.as_slice()),
        (root.path().join("prior.md"), prior_markdown.as_slice()),
        (root.path().join("added.md"), added_markdown.as_slice()),
    ];

    // Antes de provocar el crash, una apertura verdaderamente externa confirma que la generacion
    // activa todavia es la anterior aunque Markdown ya describa el candidato.
    verify_in_separate_process(root.path());

    let marker = cache.join("h03-sidecars-retired-before-swap");
    let release = cache.join("h03-release-sidecars-retired-before-swap");
    let mut worker = child("worker", root.path());
    wait_for_crash_marker(&mut worker, &marker);

    assert!(
        !wal.exists() && !shm.exists(),
        "guard de orden: el seam debe ocurrir despues de retirar ambos nombres WAL/SHM"
    );
    let next = cache.join("index.db.next");
    assert!(
        next.exists() && std::fs::metadata(&next).unwrap().len() > 0,
        "guard de orden: la generacion .next debe existir antes del seam de crash"
    );
    assert!(
        !release.exists(),
        "guard anti-vacuidad: el padre no debe liberar cooperativamente el worker"
    );

    worker
        .kill()
        .expect("mata el worker sin ejecutar Drop/rollback");
    let killed = worker.wait().expect("recoge el worker abortado");
    assert!(
        !killed.success(),
        "guard anti-vacuidad: el worker debe morir, no completar el rebuild"
    );

    assert_eq!(
        lodestar_discovery::filesystem_fingerprint(&active, true)
            .expect("identidad del index.db tras el crash")
            .identity,
        active_identity,
        "C6: index.db.next no puede haberse adoptado antes de replace_durable"
    );
    assert!(
        next.exists(),
        "guard anti-vacuidad: el crash deja .next sin adoptar, no simula un fallo previo al build"
    );
    for (path, expected) in markdown_before {
        assert_eq!(
            std::fs::read(&path).expect("relee Markdown tras el crash"),
            expected,
            "C6: el rebuild derivado no puede modificar {}",
            path.display()
        );
    }

    verify_in_separate_process(root.path());
    drop(active_store);
}
