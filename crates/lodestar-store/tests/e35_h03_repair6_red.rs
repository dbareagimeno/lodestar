//! Rojo de reparación E35-H03: el snapshot canónico empieza en discovery y la garantía
//! insert-only debe depender de SQLite/auditoría exhaustiva, no de contadores voluntarios.

use std::path::Path;
use std::sync::{Mutex, OnceLock};

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

fn markdown(title: &str, sentinel: &str) -> String {
    format!("---\ntitle: {title}\n---\n\n# {title}\n\n{sentinel}\n")
}

fn failpoint_env_lock() -> &'static Mutex<()> {
    static LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    LOCK.get_or_init(|| Mutex::new(()))
}

fn assert_previous_active_survives(root: &Path, context: &str) {
    let active = Store::open(root).expect("la generación activa anterior sigue abriéndose");
    assert_eq!(
        active.documents().unwrap(),
        vec![rp("stable.md")],
        "{context}: nunca debe publicarse una cache parcial ni una segunda fila"
    );
    assert_eq!(
        active.fts_candidates("repairsixtoldactive").unwrap(),
        vec![rp("stable.md")],
        "{context}: el centinela de la generación anterior debe seguir consultable"
    );
    assert!(
        active
            .fts_candidates("repairsixnewsource")
            .unwrap()
            .is_empty(),
        "{context}: el snapshot nuevo no puede publicarse después de abortar"
    );
    assert!(
        active
            .fts_candidates("repairsixaddedafterinventory")
            .unwrap()
            .is_empty(),
        "{context}: una generación incompleta jamás puede adoptar el alta omitida"
    );
}

/// C1+C6 — el instante autoritativo del snapshot es la primera pasada canónica, no la entrada
/// posterior en `Store::rebuild_from_discovered_inventory`.
#[test]
fn c1_c6_added_document_after_canonical_discovery_aborts_without_partial_publication() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "stable.md",
        markdown("stable old", "repairsixtoldactive"),
    );
    let initial = Store::open_and_build(root.path()).unwrap();
    assert_eq!(
        initial.fts_candidates("repairsixtoldactive").unwrap(),
        vec![rp("stable.md")],
        "guard: existe una generación activa previa distinguible"
    );

    let discovered = discover_inventory(root.path(), &DiscoveryPolicy::default()).unwrap();
    assert_eq!(
        discovered.documents,
        vec![rp("stable.md")],
        "guard: la primera pasada canónica terminó antes del alta"
    );

    write(
        root.path(),
        "stable.md",
        markdown("stable new", "repairsixnewsource"),
    );
    write(
        root.path(),
        "added.md",
        markdown("added", "repairsixaddedafterinventory"),
    );
    assert!(
        root.path().join("added.md").is_file()
            && !discovered
                .documents
                .iter()
                .any(|path| path.as_str() == "added.md"),
        "guard anti-vacuidad: added.md existe en la segunda pasada pero no en el snapshot"
    );

    let result = initial.rebuild_from_discovered_inventory(&discovered);
    let error = result.expect_err(
        "un alta admitida después del discovery debe invalidar el snapshot y abortar el rebuild",
    );
    let message = error.to_string().to_ascii_lowercase();
    assert!(
        message.contains("inventory")
            || message.contains("inventario")
            || message.contains("changed"),
        "el error debe atribuir el aborto al cambio del inventario: {message}"
    );
    assert_previous_active_survives(root.path(), "alta posterior al inventario");
}

/// C3+C6 — un `DELETE` directo que el contador voluntario no ve debe ser bloqueado por el
/// authorizer de SQLite y no puede llegar al swap.
#[test]
fn c3_c6_untraced_delete_is_denied_by_sqlite_and_keeps_previous_active() {
    let _env_lock = failpoint_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "stable.md",
        markdown("stable old", "repairsixtoldactive"),
    );
    let store = Store::open_and_build(root.path()).unwrap();
    write(
        root.path(),
        "stable.md",
        markdown("stable new", "repairsixnewsource"),
    );

    let failpoint = format!("{}:untraced_delete_during_build", root.path().display());
    std::env::set_var("LODESTAR_H03_FAILPOINT", failpoint);
    let result = store.rebuild();
    std::env::remove_var("LODESTAR_H03_FAILPOINT");

    let error = result.expect_err(
        "el seam debe intentar un DELETE directo y SQLite debe denegarlo antes del swap",
    );
    let message = error.to_string().to_ascii_lowercase();
    assert!(
        message.contains("not authorized")
            || message.contains("authoriz")
            || (message.contains("delete")
                && (message.contains("deneg") || message.contains("prohib"))),
        "C3: el fallo debe demostrar que SQLite denegó el DELETE no trazado: {message}"
    );
    assert_previous_active_survives(root.path(), "DELETE directo denegado");
}

/// C3+C6 — preparar SQL fuera del auditor debe detectarse como divergencia y abortar antes de
/// publicar, aunque ese statement no cambie el resultado lógico final.
#[test]
fn c3_c6_untraced_prepare_is_audit_failure_and_keeps_previous_active() {
    let _env_lock = failpoint_env_lock()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "stable.md",
        markdown("stable old", "repairsixtoldactive"),
    );
    let store = Store::open_and_build(root.path()).unwrap();
    write(
        root.path(),
        "stable.md",
        markdown("stable new", "repairsixnewsource"),
    );

    let failpoint = format!("{}:untraced_prepare_during_build", root.path().display());
    std::env::set_var("LODESTAR_H03_FAILPOINT", failpoint);
    let result = store.rebuild();
    std::env::remove_var("LODESTAR_H03_FAILPOINT");

    let error = result.expect_err(
        "el seam debe preparar SQL fuera del auditor y la divergencia debe abortar antes del swap",
    );
    let message = error.to_string().to_ascii_lowercase();
    assert!(
        message.contains("prepare")
            && (message.contains("audit")
                || message.contains("untraced")
                || message.contains("no traz")),
        "C3: el fallo debe identificar la preparación no auditada: {message}"
    );
    assert_previous_active_survives(root.path(), "prepare directo no auditado");
}
