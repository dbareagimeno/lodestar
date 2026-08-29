//! E35-H03 repair14: publicación interproceso sobre un Store abierto antes del swap.
//!
//! Este escenario usa únicamente el Store real y SQLite publicado, con procesos separados porque
//! la garantía que se comprueba es precisamente entre conexiones y locks de procesos distintos.
//! Los marcadores y el failpoint solo hacen observable el orden de la carrera; el contrato se
//! verifica mediante el contenido consultable del Store activo.

use std::path::Path;
use std::process::{Child, Command, ExitStatus};
use std::sync::{Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use lodestar_core::types::RelPath;
use lodestar_store::Store;

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

fn failpoint_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(20);
    while !path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(
        path.exists(),
        "seam determinista no alcanzado: {}",
        path.display()
    );
}

fn wait_child(child: &mut Child) -> ExitStatus {
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("proceso hijo no terminó dentro del límite");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

/// C6 — B abre su conexión al `index.db` antiguo antes de que A publique por rename. Una vez
/// publicado el rebuild de A, el upsert de B debe aplicarse a la generación activa; de lo
/// contrario el éxito de SQLite no se reflejaría en el Store que se puede consultar después.
#[test]
fn c6_store_handle_abierto_antes_del_swap_aplica_upsert_a_la_generacion_activa() {
    if let Some(role) = std::env::var_os("E35_H03_REPAIR14_CHILD") {
        let role = role.to_string_lossy();
        let root = std::path::PathBuf::from(
            std::env::var_os("E35_H03_REPAIR14_ROOT").expect("root del proceso hijo"),
        );
        match role.as_ref() {
            "stale" => {
                // Esta apertura ocurre antes de que A adquiera el gate y publique su generación.
                let store = Store::open(&root).expect("B abre la generación activa anterior");
                let _ = store
                    .documents()
                    .expect("B fuerza una lectura real de la generación WAL anterior");
                assert!(
                    root.join(".lodestar/index.db-shm").exists(),
                    "guard anti-vacuidad: B debe retener también el shared-memory de WAL"
                );
                write(&root, ".lodestar/repair14-b-opened", b"opened\n");
                wait_for(&root.join(".lodestar/repair14-allow-upsert"));
                let changed = store
                    .upsert(
                        &rp("b.md"),
                        "# B publicado tras swap\nrepair14-upsert-sentinel\n",
                        0,
                        0,
                    )
                    .expect("B upsert devuelve éxito");
                write(
                    &root,
                    ".lodestar/repair14-b-result",
                    format!("changed={changed}\n").as_bytes(),
                );
            }
            "rebuild" => {
                std::env::set_var(
                    "LODESTAR_H03_FAILPOINT",
                    format!("{}:pause_before_swap", root.display()),
                );
                match Store::open_and_build(&root) {
                    Ok(store) => {
                        write(&root, ".lodestar/repair14-a-result", b"ok\n");
                        // Mantener A abierto mientras B cierra la generación vieja y publica su
                        // upsert reproduce el peligro específico de winShmPurge: el cierre viejo
                        // no puede borrar por nombre los sidecars que ya pertenecen a A.
                        wait_for(&root.join(".lodestar/repair14-allow-a-check"));
                        assert_eq!(
                            store.documents().expect("A consulta tras el cierre de B"),
                            vec![rp("a.md"), rp("b.md"), rp("old.md")],
                            "A debe seguir compartiendo WAL/SHM con el writer refrescado de B"
                        );
                        assert_eq!(
                            store
                                .fts_candidates("repair14-upsert-sentinel")
                                .expect("A consulta FTS tras el cierre de B"),
                            vec![rp("b.md")],
                            "A debe observar el upsert de B en los sidecars de la generación nueva"
                        );
                        assert!(
                            store
                                .upsert(
                                    &rp("c.md"),
                                    "# A sigue escribiendo\nrepair14-a-live-sentinel\n",
                                    0,
                                    0,
                                )
                                .expect("A escribe después del refresh de B"),
                            "la escritura alterna de A debe cambiar la generación activa"
                        );
                        write(&root, ".lodestar/repair14-a-post-b", b"ok\n");
                    }
                    Err(error) => {
                        write(
                            &root,
                            ".lodestar/repair14-a-result",
                            format!("err:{error}\n").as_bytes(),
                        );
                    }
                }
            }
            other => panic!("rol de proceso desconocido: {other}"),
        }
        return;
    }

    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "old.md",
        "# snapshot viejo\nold-snapshot-sentinel\n",
    );
    {
        let initial = Store::open(root.path()).unwrap();
        initial.rebuild().unwrap();
    }
    let db = root.path().join(".lodestar/index.db");
    let identity_before = lodestar_discovery::filesystem_fingerprint(&db, true)
        .expect("identidad del index.db inicial")
        .identity;
    // A publicará este nuevo candidato en la siguiente generación.
    write(
        root.path(),
        "a.md",
        "# snapshot nuevo\nrebuild-swap-sentinel\n",
    );

    let executable = std::env::current_exe().unwrap();
    let child = |role: &str| {
        Command::new(&executable)
            .args([
                "--exact",
                "c6_store_handle_abierto_antes_del_swap_aplica_upsert_a_la_generacion_activa",
                "--nocapture",
            ])
            .env("E35_H03_REPAIR14_CHILD", role)
            .env("E35_H03_REPAIR14_ROOT", root.path())
            .spawn()
            .unwrap()
    };

    // B queda abierto y reteniendo la conexión al estado anterior antes de iniciar A.
    let mut stale = child("stale");
    wait_for(&root.path().join(".lodestar/repair14-b-opened"));
    assert!(
        root.path().join(".lodestar/index.db-shm").exists(),
        "el padre debe observar el sidecar WAL retenido antes de iniciar A"
    );

    let mut rebuild = child("rebuild");
    wait_for(&root.path().join(".lodestar/h03-pause-before-swap"));
    write(
        root.path(),
        ".lodestar/h03-release-before-swap",
        b"release-rebuild\n",
    );

    wait_for(&root.path().join(".lodestar/repair14-a-result"));
    assert_eq!(
        std::fs::read_to_string(root.path().join(".lodestar/repair14-a-result")).unwrap(),
        "ok\n",
        "A debe publicar una generación completa"
    );
    let identity_after = lodestar_discovery::filesystem_fingerprint(&db, true)
        .expect("identidad del index.db publicado")
        .identity;
    assert_ne!(
        identity_before, identity_after,
        "guard anti-vacuidad: el rebuild debe publicar otra generación mediante reemplazo, no actualizar index.db in-place"
    );

    // B solo adquiere ahora el gate y hace el upsert sobre la conexión que abrió antes del swap.
    write(
        root.path(),
        ".lodestar/repair14-allow-upsert",
        b"release-after-publish\n",
    );
    let stale_status = wait_child(&mut stale);
    assert!(stale_status.success(), "B upsert terminó {stale_status}");
    assert_eq!(
        std::fs::read_to_string(root.path().join(".lodestar/repair14-b-result")).unwrap(),
        "changed=true\n",
        "guard anti-vacuidad: B debe observar un cambio efectivo"
    );

    write(
        root.path(),
        ".lodestar/repair14-allow-a-check",
        b"verify-new-generation-sidecars\n",
    );
    let rebuild_status = wait_child(&mut rebuild);
    assert!(
        rebuild_status.success(),
        "A rebuild/consulta post-B terminó {rebuild_status}"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join(".lodestar/repair14-a-post-b")).unwrap(),
        "ok\n",
        "A debe permanecer coherente mientras B cierra la generación vieja"
    );

    let published = Store::open(root.path()).unwrap();
    assert_eq!(
        published.documents().unwrap(),
        vec![rp("a.md"), rp("b.md"), rp("c.md"), rp("old.md")],
        "C6: el Store activo debe conservar rebuild de A y escrituras alternas de B/A"
    );
    assert_eq!(
        published.fts_candidates("rebuild-swap-sentinel").unwrap(),
        vec![rp("a.md")],
        "guard: el snapshot nuevo de A está publicado"
    );
    assert_eq!(
        published
            .fts_candidates("repair14-upsert-sentinel")
            .unwrap(),
        vec![rp("b.md")],
        "C6: el upsert de B debe quedar visible en el Store activo"
    );
    assert_eq!(
        published
            .fts_candidates("repair14-a-live-sentinel")
            .unwrap(),
        vec![rp("c.md")],
        "C6: la escritura posterior de A debe compartir los mismos sidecars activos"
    );
}

/// Un fallo retirando sidecars ocurre después de cerrar la conexión vieja pero antes del rename.
/// El RootState compartido debe volver a una conexión real sobre el activo anterior: dejar el
/// placeholder in-memory haría que otra apertura reutilizase una base sin esquema.
#[test]
fn c6_fallo_de_sidecars_antes_del_swap_restaura_la_conexion_activa() {
    let _env_lock = failpoint_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "old.md",
        "# anterior\nrepair14-before-sidecar-failure\n",
    );
    let store = Store::open_and_build(root.path()).unwrap();
    write(
        root.path(),
        "new.md",
        "# candidato\nrepair14-unpublished-sidecar-failure\n",
    );

    std::env::set_var(
        "LODESTAR_H03_FAILPOINT",
        format!("{}:sidecar_cleanup", root.path().display()),
    );
    let result = store.rebuild();
    std::env::remove_var("LODESTAR_H03_FAILPOINT");
    assert!(
        result.is_err(),
        "el failpoint debe abortar antes del rename"
    );

    assert_eq!(
        store.documents().unwrap(),
        vec![rp("old.md")],
        "el mismo handle debe recuperar la generación anterior disk-backed"
    );
    let reopened = Store::open(root.path()).unwrap();
    assert_eq!(
        reopened.documents().unwrap(),
        vec![rp("old.md")],
        "otra apertura no puede reutilizar el placeholder in-memory"
    );
    assert!(reopened
        .fts_candidates("repair14-before-sidecar-failure")
        .unwrap()
        .contains(&rp("old.md")));
    assert!(reopened
        .fts_candidates("repair14-unpublished-sidecar-failure")
        .unwrap()
        .is_empty());
}

#[cfg(windows)]
#[test]
fn c6_fallo_tras_apartar_el_primer_sidecar_revierte_nombres_y_snapshot() {
    let _env_lock = failpoint_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "old.md",
        "# anterior\nrepair14-before-partial-retirement\n",
    );
    let store = Store::open_and_build(root.path()).unwrap();
    assert_eq!(store.documents().unwrap(), vec![rp("old.md")]);
    let cache = root.path().join(".lodestar");
    assert!(cache.join("index.db-shm").exists());
    assert!(cache.join("index.db-wal").exists());
    write(
        root.path(),
        "new.md",
        "# candidato\nrepair14-unpublished-partial-retirement\n",
    );

    std::env::set_var(
        "LODESTAR_H03_FAILPOINT",
        format!("{}:sidecar_cleanup_after_first", root.path().display()),
    );
    let worker = thread::spawn(move || {
        let result = store.rebuild();
        (store, result)
    });
    wait_for(&cache.join("h03-sidecar-first-staged"));
    assert!(
        !cache.join("index.db-shm").exists(),
        "la pausa debe ocurrir después de apartar físicamente el primer sidecar"
    );
    assert!(
        std::fs::read_dir(&cache).unwrap().any(|entry| entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .starts_with("index.db-shm.lodestar-stale-")),
        "la pausa debe exponer el tombstone reversible de SHM"
    );
    write(
        root.path(),
        ".lodestar/h03-release-sidecar-first-staged",
        b"rollback\n",
    );
    let (store, result) = worker.join().expect("worker de rollback termina");
    std::env::remove_var("LODESTAR_H03_FAILPOINT");
    assert!(result.is_err(), "la inyección intermedia debe abortar");

    assert!(cache.join("index.db-shm").exists(), "SHM debe volver");
    assert!(cache.join("index.db-wal").exists(), "WAL debe conservarse");
    assert_eq!(store.documents().unwrap(), vec![rp("old.md")]);
    assert!(store
        .fts_candidates("repair14-before-partial-retirement")
        .unwrap()
        .contains(&rp("old.md")));
    assert!(store
        .fts_candidates("repair14-unpublished-partial-retirement")
        .unwrap()
        .is_empty());
    assert!(
        std::fs::read_dir(cache).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .to_string_lossy()
            .contains(".lodestar-stale-")),
        "el rollback no puede dejar tombstones de una publicación abortada"
    );
}

/// El soporte de publicación de Lodestar no puede relajar el share mode del VFS SQLite por
/// defecto. Una base ajena que siga usando ese VFS debe conservar el comportamiento win32 normal:
/// sin `FILE_SHARE_DELETE`, un reemplazo mientras la conexión está abierta falla con access denied
/// o sharing violation. Este guard detecta hooks globales que arreglan Lodestar alterando todas las
/// bases del proceso.
#[cfg(windows)]
#[test]
fn c6_vfs_de_lodestar_no_modifica_sqlite_ajeno() {
    use std::os::windows::ffi::OsStrExt;

    use windows_sys::Win32::Storage::FileSystem::{MoveFileExW, MOVEFILE_REPLACE_EXISTING};

    let root = tempfile::tempdir().unwrap();
    let _store = Store::open(root.path()).expect("inicializa el VFS acotado de Lodestar");

    let foreign = root.path().join("foreign.db");
    let replacement = root.path().join("replacement.db");
    let foreign_conn = rusqlite::Connection::open(&foreign).expect("abre SQLite ajeno");
    foreign_conn
        .execute_batch("CREATE TABLE sentinel(value INTEGER); INSERT INTO sentinel VALUES (73);")
        .unwrap();
    std::fs::write(&replacement, b"replacement").unwrap();

    let foreign_wide: Vec<u16> = foreign
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let replacement_wide: Vec<u16> = replacement
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect();
    let moved = unsafe {
        MoveFileExW(
            replacement_wide.as_ptr(),
            foreign_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING,
        )
    };
    let error = std::io::Error::last_os_error();

    assert_eq!(
        moved, 0,
        "un Store no debe alterar el VFS SQLite por defecto"
    );
    assert!(
        matches!(error.raw_os_error(), Some(5) | Some(32)),
        "Windows debe conservar ERROR_ACCESS_DENIED (5) o ERROR_SHARING_VIOLATION (32) para la base ajena: {error}"
    );
    assert_eq!(
        foreign_conn
            .query_row("SELECT value FROM sentinel", [], |row| row.get::<_, i64>(0))
            .unwrap(),
        73,
        "guard anti-vacuidad: la conexión ajena sigue viva y conserva su centinela"
    );
}
