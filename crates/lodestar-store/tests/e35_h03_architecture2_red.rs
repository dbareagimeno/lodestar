//! Reproducciones de la revisión de arquitectura E35-H03.
//!
//! Estos tests ejercitan el store real y los procesos reales. No añaden seams ni sustitutos de
//! producción: los marcadores son únicamente una barrera determinista del arnés.

use std::path::Path;
use std::process::{Child, Command, ExitStatus};
use std::thread;
use std::time::{Duration, Instant};

use lodestar_store::Store;

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

fn wait_for(path: &Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
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
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            return status;
        }
        if Instant::now() >= deadline {
            let _ = child.kill();
            let _ = child.wait();
            panic!("proceso de rebuild no terminó dentro del límite");
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn child_result(root: &Path, role: &str) -> String {
    std::fs::read_to_string(root.join(format!(".lodestar/h03-{role}-result")))
        .unwrap_or_else(|error| panic!("resultado del proceso {role}: {error}"))
}

/// C6 — El gate de writer debe ser interproceso: dos `Store::open_and_build` sobre el mismo root
/// se serializan, ambos terminan OK y el índice publicado sigue siendo íntegro y consultable.
#[test]
fn c6_store_rebuild_interprocess_serializes_and_publishes_integral_snapshot() {
    if let Some(role) = std::env::var_os("E35_H03_C6_CHILD") {
        let role = role.to_string_lossy();
        let root = std::path::PathBuf::from(std::env::var_os("E35_H03_C6_ROOT").unwrap());
        write(&root, &format!(".lodestar/h03-{role}-ready"), b"ready\n");
        if role == "first" {
            std::env::set_var(
                "LODESTAR_H03_FAILPOINT",
                format!("{}:pause_before_swap", root.display()),
            );
        }
        let result = Store::open_and_build(&root);
        let text = match result {
            Ok(store) => {
                let _ = store.documents();
                "ok".to_string()
            }
            Err(error) => format!("err:{error}"),
        };
        write(&root, &format!(".lodestar/h03-{role}-result"), text);
        return;
    }

    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "docs/old.md",
        markdown("old", &format!("snapshot-old {}", "x".repeat(16 * 1024))),
    );
    for index in 0..24 {
        write(
            root.path(),
            &format!("docs/filler-{index:02}.md"),
            markdown(
                "filler",
                &format!("filler-{index} {}", "y".repeat(8 * 1024)),
            ),
        );
    }

    let executable = std::env::current_exe().unwrap();
    let child_env = |role: &str| {
        let mut command = Command::new(&executable);
        command
            .args([
                "--exact",
                "c6_store_rebuild_interprocess_serializes_and_publishes_integral_snapshot",
                "--nocapture",
            ])
            .env("E35_H03_C6_CHILD", role)
            .env("E35_H03_C6_ROOT", root.path());
        command
    };

    let mut first = child_env("first").spawn().unwrap();
    wait_for(&root.path().join(".lodestar/h03-pause-before-swap"));

    let mut second = child_env("second").spawn().unwrap();
    wait_for(&root.path().join(".lodestar/h03-second-ready"));
    thread::sleep(Duration::from_millis(150));
    let second_completed_while_paused = root.path().join(".lodestar/h03-second-result").exists();

    write(
        root.path(),
        ".lodestar/h03-release-before-swap",
        b"release\n",
    );
    let first_status = wait_child(&mut first);
    let second_status = wait_child(&mut second);
    assert!(
        !second_completed_while_paused,
        "C6 guard: el segundo proceso no puede publicar ni completar I/O mientras el primero sostiene el gate"
    );
    assert!(
        first_status.success(),
        "C6: primer proceso terminó {first_status}"
    );
    assert!(
        second_status.success(),
        "C6: segundo proceso terminó {second_status}"
    );
    assert_eq!(child_result(root.path(), "first"), "ok");
    assert_eq!(child_result(root.path(), "second"), "ok");

    let reopened = Store::open(root.path()).unwrap();
    let old = reopened
        .fts_candidates("snapshot-old")
        .unwrap()
        .into_iter()
        .any(|path| path.as_str() == "docs/old.md");
    assert!(
        old,
        "C6: el índice final debe conservar un snapshot consultable"
    );
    assert!(
        root.path().join(".lodestar/index.db").is_file(),
        "C6: debe existir una generación activa publicada"
    );
}

/// RelPath Unix — un backslash de un nombre POSIX literal no es un separador ni un `RelPath`
/// válido. El descubrimiento lo diagnostica y omite sin enmascarar al documento real `a/b.md`.
#[cfg(unix)]
#[test]
fn relpath_unix_literal_backslash_is_diagnosed_without_masking_real_document() {
    let root = tempfile::tempdir().unwrap();
    let real_path = "a/b.md";
    let literal_path = "a\\b.md";
    let real = markdown("real", "realbackslashsentinel");
    let impostor = markdown("impostor", "impostorbackslashsentinel");
    write(root.path(), real_path, &real);
    write(root.path(), literal_path, &impostor);
    assert!(
        root.path().join(real_path).is_file() && root.path().join(literal_path).is_file(),
        "guard: las fixtures real e impostora deben coexistir como dos entradas POSIX distintas"
    );

    let policy = lodestar_discovery::DiscoveryPolicy::default();
    let inventory = lodestar_discovery::discover_inventory(root.path(), &policy).unwrap();
    let canonical_documents: Vec<&str> = inventory
        .documents
        .iter()
        .map(|path| path.as_str())
        .collect();
    assert_eq!(
        canonical_documents,
        vec![real_path],
        "el inventario canónico debe omitir el literal inválido sin convertirlo ni colisionarlo con a/b.md"
    );
    let path_diagnostics: Vec<_> = inventory
        .diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.code.as_str() == "PATH-NOT-UTF8")
        .collect();
    assert_eq!(
        path_diagnostics.len(),
        1,
        "el nombre POSIX literal no representable como RelPath debe producir un único PATH-NOT-UTF8: {:?}",
        inventory.diagnostics
    );
    assert!(
        path_diagnostics[0].targets.is_empty(),
        "PATH-NOT-UTF8 no puede inventar un RelPath para el nombre literal"
    );
    assert!(
        path_diagnostics[0].msg.contains(literal_path),
        "el diagnóstico debe localizar la entrada omitida mediante su representación de disco: {:?}",
        path_diagnostics[0].msg
    );

    let store = Store::open_and_build(root.path()).unwrap();
    for phase in ["open_and_build", "rebuild repetido"] {
        let documents = store.documents().unwrap();
        let document_paths: Vec<&str> = documents.iter().map(|path| path.as_str()).collect();
        assert_eq!(
            document_paths,
            vec![real_path],
            "{phase}: la cache debe contener solo el documento real, sin conversión ni colisión"
        );
        let cached = store.document_set();
        assert_eq!(
            cached.files().len(),
            1,
            "{phase}: el impostor no puede crear una segunda fila documental"
        );
        assert_eq!(
            cached.files().values().next().map(String::as_str),
            Some(real.as_str()),
            "{phase}: a/b.md debe conservar exactamente el contenido real, nunca el del impostor"
        );
        assert_eq!(
            store
                .fts_candidates("realbackslashsentinel")
                .unwrap()
                .iter()
                .map(|path| path.as_str())
                .collect::<Vec<_>>(),
            vec![real_path],
            "{phase}: FTS debe indexar el centinela real bajo a/b.md"
        );
        assert!(
            store
                .fts_candidates("impostorbackslashsentinel")
                .unwrap()
                .is_empty(),
            "{phase}: FTS jamás debe contener el centinela impostor"
        );
        assert!(
            store
                .search("impostorbackslashsentinel")
                .unwrap()
                .is_empty(),
            "{phase}: la búsqueda semántica tampoco puede revelar contenido del path omitido"
        );

        if phase == "open_and_build" {
            store.rebuild().unwrap();
        }
    }
}

/// §20.12.2 — cualquier cambio del árbol entre snapshot e indexación aborta. Este caso muta un
/// `other_file` (no un documento) y añade un documento admitido: solo validar cuerpos Markdown no
/// puede hacer pasar la prueba por accidente.
#[test]
fn rebuild_aborts_on_other_file_change_between_passes_and_keeps_active_snapshot() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "docs/old.md",
        markdown("old", "toctou-active-old-sentinel"),
    );
    write(root.path(), "assets/other.bin", b"other-before");
    let _store = Store::open_and_build(root.path()).unwrap();

    let failpoint = format!("{}:after_snapshot_before_read", root.path().display());
    std::env::set_var("LODESTAR_H03_FAILPOINT", &failpoint);
    let root_for_rebuild = root.path().to_path_buf();
    let rebuilding = std::thread::spawn(move || Store::open(&root_for_rebuild).unwrap().rebuild());
    wait_for(
        &root
            .path()
            .join(".lodestar/h03-pause-after-snapshot-before-read"),
    );
    write(root.path(), "assets/other.bin", b"other-after");
    write(
        root.path(),
        "docs/added.md",
        markdown("added", "toctou-added-document"),
    );
    write(
        root.path(),
        ".lodestar/h03-release-after-snapshot-before-read",
        b"release\n",
    );
    let result = rebuilding.join().unwrap();
    std::env::remove_var("LODESTAR_H03_FAILPOINT");
    let active = Store::open(root.path()).unwrap();
    assert!(
        active
            .fts_candidates("toctou-active-old-sentinel")
            .unwrap()
            .iter()
            .any(|path| path.as_str() == "docs/old.md"),
        "TOCTOU: el índice activo anterior debe seguir consultable"
    );
    assert!(
        active
            .fts_candidates("toctou-added-document")
            .unwrap()
            .is_empty(),
        "TOCTOU: la generación parcial no puede publicarse como activa"
    );
    assert!(
        result.is_err(),
        "TOCTOU: mutar un other_file y añadir un documento entre pasadas debe abortar"
    );
}
