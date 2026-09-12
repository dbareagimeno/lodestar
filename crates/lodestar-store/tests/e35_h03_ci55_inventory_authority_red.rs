//! E35-H03 C1 — ningún rebuild público puede convertir un inventario arbitrario en otra verdad.
//!
//! `DiscoveryPolicy` gobierna tanto el inventario como las filas publicadas. Este test conserva
//! una generación canónica activa y entrega a la API pública únicamente paths que discovery
//! rechaza por reglas distintas: exclude, ambos ignores, límite y plano de control.

use std::collections::BTreeSet;
use std::path::Path;

use lodestar_core::types::RelPath;
use lodestar_discovery::{discover_inventory, load_policy};
use lodestar_store::Store;

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("CI55 RelPath válido")
}

fn write(root: &Path, path: &str, bytes: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).expect("CI55 crear directorio de fixture");
    }
    std::fs::write(target, bytes).expect("CI55 escribir fixture");
}

fn markdown(title: &str, needle: &str) -> String {
    format!("# {title}\n\n{needle}\n")
}

#[test]
fn c1_rebuild_publico_rechaza_inventario_fuera_de_policy_y_preserva_activo() {
    let root = tempfile::tempdir().expect("CI55 workspace temporal");
    write(
        root.path(),
        ".lodestar/config.yaml",
        "discovery:\n  include: [\"**/*.md\"]\n  exclude: [\"docs/excluded.md\"]\n  respectGitignore: true\n  respectLodestarIgnore: true\n  maxDocumentBytes: 256\n",
    );
    write(root.path(), ".gitignore", "docs/git-ignored.md\n");
    write(root.path(), ".lodestarignore", "docs/lodestar-ignored.md\n");

    let admitted = rp("docs/admitted.md");
    let excluded = rp("docs/excluded.md");
    let git_ignored = rp("docs/git-ignored.md");
    let lodestar_ignored = rp("docs/lodestar-ignored.md");
    let oversized = rp("docs/oversized.md");
    let control_plane = rp(".lodestar/control-plane.md");

    write(
        root.path(),
        admitted.as_str(),
        markdown("Admitted CI55", "ci55-active-canonical-needle"),
    );
    write(
        root.path(),
        excluded.as_str(),
        markdown("Excluded CI55", "ci55-excluded-needle"),
    );
    write(
        root.path(),
        git_ignored.as_str(),
        markdown("Git ignored CI55", "ci55-git-ignore-needle"),
    );
    write(
        root.path(),
        lodestar_ignored.as_str(),
        markdown("Lodestar ignored CI55", "ci55-lodestar-ignore-needle"),
    );
    write(
        root.path(),
        oversized.as_str(),
        markdown(
            "Oversized CI55",
            &format!("ci55-oversized-needle {}", "x".repeat(512)),
        ),
    );
    write(
        root.path(),
        control_plane.as_str(),
        markdown("Control plane CI55", "ci55-control-plane-needle"),
    );

    let policy = load_policy(root.path()).expect("CI55 cargar policy efectiva");
    assert_eq!(
        policy.max_document_bytes, 256,
        "CI55 anti-vacuidad: la prueba debe ejercer el límite configurado"
    );
    assert!(
        std::fs::metadata(root.path().join(oversized.as_str()))
            .expect("CI55 metadata oversized")
            .len()
            > policy.max_document_bytes as u64,
        "CI55 anti-vacuidad: oversized.md debe superar realmente maxDocumentBytes"
    );
    let discovered = discover_inventory(root.path(), &policy).expect("CI55 discovery canónico");
    assert_eq!(
        discovered.documents,
        vec![admitted.clone()],
        "CI55 anti-vacuidad: la policy efectiva debe admitir solo el control positivo"
    );

    let store = Store::open_and_build(root.path()).expect("CI55 publicar generación canónica");
    assert_eq!(
        store
            .documents()
            .expect("CI55 documentos activos iniciales"),
        vec![admitted.clone()],
        "CI55 anti-vacuidad: existe una generación activa anterior distinta"
    );
    assert_eq!(
        store
            .fts_candidates("ci55-active-canonical-needle")
            .expect("CI55 consultar sentinela activo"),
        vec![admitted.clone()],
        "CI55 anti-vacuidad: el activo anterior es consultable por FTS"
    );

    let arbitrary = [
        (excluded.clone(), "ci55-excluded-needle"),
        (git_ignored.clone(), "ci55-git-ignore-needle"),
        (lodestar_ignored.clone(), "ci55-lodestar-ignore-needle"),
        (oversized.clone(), "ci55-oversized-needle"),
        (control_plane.clone(), "ci55-control-plane-needle"),
    ];
    assert!(
        arbitrary
            .iter()
            .all(|(path, _)| root.path().join(path.as_str()).is_file()),
        "CI55 anti-vacuidad: todos los paths no canónicos existen y se entregan a la API pública"
    );
    assert!(
        arbitrary
            .iter()
            .all(|(path, _)| !discovered.documents.contains(path)),
        "CI55 anti-vacuidad: ningún path arbitrario pertenece al inventario canónico"
    );

    // Cada categoría se intenta por separado: `.lodestar` puede hacer visible una divergencia de
    // directorio al crear `.next`, pero eso no debe ocultar que exclude/ignores/límite también son
    // autoridad. Tras cada rechazo debe seguir activa la misma generación canónica.
    for (path, needle) in arbitrary {
        let rebuild = store.rebuild_from_inventory(std::slice::from_ref(&path), &BTreeSet::new());
        let active = Store::open(root.path()).expect("CI55 reabrir generación activa");
        assert_eq!(
            active
                .documents()
                .expect("CI55 consultar documentos tras el intento"),
            vec![admitted.clone()],
            "C1 CI55: {} no puede reemplazar el inventario canónico; rebuild={rebuild:?}",
            path.as_str()
        );
        assert_eq!(
            active
                .fts_candidates("ci55-active-canonical-needle")
                .expect("CI55 consultar FTS tras el intento"),
            vec![admitted.clone()],
            "C1 CI55: el snapshot anterior debe seguir consultable tras intentar {}; rebuild={rebuild:?}",
            path.as_str()
        );
        assert!(
            rebuild.is_err(),
            "C1 CI55: la API pública debe rechazar {}, no filtrarlo ni aceptarlo",
            path.as_str()
        );
        assert!(
            active
                .fts_candidates(needle)
                .expect("CI55 consultar aguja prohibida")
                .is_empty(),
            "C1 CI55: {} nunca debe alcanzar FTS",
            path.as_str()
        );
    }
}
