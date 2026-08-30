//! Regresiones E35-H03 repair15.
//!
//! Estos tests observan exclusivamente la interfaz pública del Store y los artefactos publicados
//! en el workspace. La durabilidad física de `fsync`/write-through no es observable de forma
//! fiable desde una prueba de integración; sí lo son la generación activa y la ausencia de una
//! `.next` adoptable después de una reconstrucción satisfactoria.

use std::path::Path;

use lodestar_core::types::RelPath;
use lodestar_store::Store;

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("RelPath válido")
}

fn write(root: &Path, path: &str, content: &str) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(target, content).unwrap();
}

fn frontmatter_only(encoded_description: &str) -> String {
    format!(
        "---\ntitle: Stable repair15 title\ndescription: \"{encoded_description}\"\n---\n\n# Stable repair15 heading\n\nbody without the marker\n"
    )
}

/// C7 + §20.12.1 — una reconstrucción en frío debe dejar en `documents` la tupla FTS antigua
/// exacta que el siguiente upsert/remove necesita para borrar una fila contentless.
#[test]
fn c7_rebuild_then_upsert_and_remove_keeps_frontmatter_only_fts_consistent() {
    let root = tempfile::tempdir().unwrap();
    let old_marker = "repair15oldfrontmattertoken";
    let new_marker = "repair15newfrontmattertoken";
    let path = rp("frontmatter-only.md");
    // YAML decodifica la secuencia Unicode antes de poblar `frontmatter_text`; así el marcador
    // semántico no aparece literalmente en `documents.body` (que conserva el Markdown original).
    let old = frontmatter_only(r"repair15\u006fldfrontmattertoken");
    let new = frontmatter_only(r"repair15\u006eewfrontmattertoken");

    // Anti-vacuidad: los marcadores solo pueden proceder de frontmatter y el cambio no puede
    // convertirse accidentalmente en una sustitución de cuerpo/título/path.
    assert_eq!(old.matches(old_marker).count(), 0);
    assert_eq!(new.matches(new_marker).count(), 0);
    assert!(!old.contains(new_marker));
    assert!(!new.contains(old_marker));
    assert!(old.contains("body without the marker"));
    assert_eq!(old.lines().count(), new.lines().count());

    write(root.path(), path.as_str(), &old);
    let store = Store::open(root.path()).unwrap();
    store.rebuild().unwrap();

    // Guarda: la aguja es un candidato FTS real antes de cualquier mutación; no basta con que
    // documents exista o que la búsqueda confirmada oculte un índice desfasado.
    assert_eq!(store.documents().unwrap(), vec![path.clone()]);
    assert_eq!(
        store.fts_candidates(old_marker).unwrap(),
        vec![path.clone()]
    );
    assert!(store.fts_candidates(new_marker).unwrap().is_empty());

    write(root.path(), path.as_str(), &new);
    assert!(store.upsert(&path, &new, 0, new.len() as i64).unwrap());
    assert!(
        store.fts_candidates(old_marker).unwrap().is_empty(),
        "el candidato del frontmatter antiguo no sobrevive al upsert"
    );
    assert_eq!(
        store.fts_candidates(new_marker).unwrap(),
        vec![path.clone()],
        "el candidato nuevo aparece exactamente una vez tras el upsert"
    );

    assert!(store.remove(&path).unwrap());
    assert!(
        store.fts_candidates(old_marker).unwrap().is_empty(),
        "remove no debe descubrir un candidato antiguo dejado por el rebuild"
    );
    assert!(
        store.fts_candidates(new_marker).unwrap().is_empty(),
        "remove debe borrar también el candidato nuevo"
    );
    assert!(store.documents().unwrap().is_empty());
}

/// C5 + §20.12.2 — un rebuild satisfactorio publica exactamente el snapshot nuevo y deja una
/// generación activa que otro `Store` puede reabrir sin depender de `index.db.next`.
#[test]
fn c5_rebuild_publica_snapshot_nuevo_reabrible_sin_next() {
    let root = tempfile::tempdir().unwrap();
    let old_path = rp("old-generation.md");
    let new_path = rp("new-generation.md");
    let old_marker = "repair15-old-generation-marker";
    let new_marker = "repair15-new-generation-marker";

    write(
        root.path(),
        old_path.as_str(),
        &format!("# Old generation\n\n{old_marker}\n"),
    );
    let store = Store::open(root.path()).unwrap();
    store.rebuild().unwrap();
    assert_eq!(
        store.fts_candidates(old_marker).unwrap(),
        vec![old_path.clone()],
        "anti-vacuidad: existe un snapshot activo anterior distinto"
    );

    std::fs::remove_file(root.path().join(old_path.as_str())).unwrap();
    write(
        root.path(),
        new_path.as_str(),
        &format!("# New generation\n\n{new_marker}\n"),
    );
    store.rebuild().unwrap();

    let active = root.path().join(".lodestar/index.db");
    let next = root.path().join(".lodestar/index.db.next");
    assert!(active.is_file(), "C5: debe existir la generación publicada");
    assert!(
        !next.exists(),
        "C5: una reconstrucción satisfactoria no deja `.next` pendiente de adopción"
    );
    drop(store);

    let reopened = Store::open(root.path()).unwrap();
    assert_eq!(
        reopened.documents().unwrap(),
        vec![new_path.clone()],
        "C5: la base reabierta contiene exactamente el snapshot nuevo"
    );
    assert!(
        reopened.fts_candidates(old_marker).unwrap().is_empty(),
        "C5: el sentinela de la generación anterior no sobrevive a la publicación"
    );
    assert_eq!(
        reopened.fts_candidates(new_marker).unwrap(),
        vec![new_path],
        "C5: el sentinela nuevo procede de la generación activa reabierta"
    );
    assert!(
        !next.exists(),
        "C5: reabrir el activo no debe crear ni adoptar `index.db.next`"
    );
}
