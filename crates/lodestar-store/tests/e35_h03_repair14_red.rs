//! E35-H03 repair14: publicación interproceso sobre un Store abierto antes del swap.
//!
//! Este escenario usa únicamente el Store real y SQLite publicado, con procesos separados porque
//! la garantía que se comprueba es precisamente entre conexiones y locks de procesos distintos.
//! Los marcadores y el failpoint solo hacen observable el orden de la carrera; el contrato se
//! verifica mediante el contenido consultable del Store activo.

use std::path::Path;
use std::process::{Child, Command, ExitStatus};
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
                let result = Store::open_and_build(&root);
                let text = match result {
                    Ok(_) => "ok\n".to_string(),
                    Err(error) => format!("err:{error}\n"),
                };
                write(&root, ".lodestar/repair14-a-result", text.as_bytes());
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

    let mut rebuild = child("rebuild");
    wait_for(&root.path().join(".lodestar/h03-pause-before-swap"));
    write(
        root.path(),
        ".lodestar/h03-release-before-swap",
        b"release-rebuild\n",
    );

    let rebuild_status = wait_child(&mut rebuild);
    assert!(
        rebuild_status.success(),
        "A rebuild terminó {rebuild_status}"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join(".lodestar/repair14-a-result")).unwrap(),
        "ok\n",
        "A debe publicar una generación completa"
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

    let published = Store::open(root.path()).unwrap();
    assert_eq!(
        published.documents().unwrap(),
        vec![rp("a.md"), rp("b.md"), rp("old.md")],
        "C6: el Store activo debe conservar rebuild de A y upsert de B"
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
}
