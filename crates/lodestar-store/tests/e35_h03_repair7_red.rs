//! Regresión E35-H03 §20.12.2: el fingerprint de cada documento pertenece a la primera pasada
//! canónica. Volver a capturarlo al entrar en el store deja invisible una modificación in-place.

use std::path::Path;

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

fn root_entries(root: &Path) -> Vec<String> {
    let mut entries: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .map(|entry| {
            entry
                .unwrap()
                .file_name()
                .into_string()
                .expect("fixture con nombres UTF-8")
        })
        .filter(|name| name != ".lodestar")
        .collect();
    entries.sort();
    entries
}

/// C1+C6 / §20.12.2 — cambiar únicamente los bytes de un path admitido después de
/// `discover_inventory` invalida el snapshot. El store debe abortar antes del swap y conservar el
/// activo anterior, aunque la frontera de directorio sea exactamente la misma.
#[test]
fn c1_c6_in_place_document_change_after_discovery_aborts_and_keeps_active() {
    let root = tempfile::tempdir().unwrap();
    let old = markdown("repairsevenoldmarker");
    let new = markdown("repairsevennewmarker");
    assert_eq!(
        old.len(),
        new.len(),
        "guard: el cambio no debe poder detectarse solo por tamaño"
    );
    write(root.path(), "stable.md", &old);

    let store = Store::open_and_build(root.path()).unwrap();
    assert_eq!(
        store.fts_candidates("repairsevenoldmarker").unwrap(),
        vec![rp("stable.md")],
        "guard: existe una generación activa anterior inequívoca"
    );

    let discovered = discover_inventory(root.path(), &DiscoveryPolicy::default()).unwrap();
    assert_eq!(
        discovered.documents,
        vec![rp("stable.md")],
        "guard: la primera pasada canónica incluye el documento que se mutará"
    );
    let entries_before = root_entries(root.path());
    let size_before = std::fs::metadata(root.path().join("stable.md"))
        .unwrap()
        .len();

    write(root.path(), "stable.md", &new);

    let entries_after = root_entries(root.path());
    let size_after = std::fs::metadata(root.path().join("stable.md"))
        .unwrap()
        .len();
    assert_eq!(
        entries_after, entries_before,
        "guard anti-vacuidad: no hubo alta, baja ni rename; la frontera contiene las mismas entradas"
    );
    assert_eq!(
        size_after, size_before,
        "guard anti-vacuidad: tampoco cambió el tamaño del documento"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("stable.md")).unwrap(),
        new,
        "guard: los bytes de la segunda pasada sí son distintos"
    );

    let result = store.rebuild_from_discovered_inventory(&discovered);
    let error = result.expect_err(
        "el fingerprint capturado durante discovery debe detectar la mutación in-place y abortar",
    );
    let message = error.to_string().to_ascii_lowercase();
    assert!(
        message.contains("fingerprint")
            || message.contains("snapshot")
            || message.contains("changed")
            || message.contains("cambi"),
        "el error debe atribuir el aborto a la divergencia del snapshot: {message}"
    );

    let active = Store::open(root.path()).expect("el activo anterior sigue íntegro y abrible");
    assert_eq!(
        active.documents().unwrap(),
        vec![rp("stable.md")],
        "el activo anterior conserva su conjunto documental"
    );
    assert_eq!(
        active.fts_candidates("repairsevenoldmarker").unwrap(),
        vec![rp("stable.md")],
        "el contenido anterior debe seguir consultable"
    );
    assert!(
        active
            .fts_candidates("repairsevennewmarker")
            .unwrap()
            .is_empty(),
        "el contenido mutado no puede publicarse tras el aborto"
    );
}
