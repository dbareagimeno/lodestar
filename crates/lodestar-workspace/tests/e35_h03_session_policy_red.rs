//! E35-H03 C1 — una sesión conserva una única `DiscoveryPolicy` para core y SQLite.
//!
//! La configuración se lee una vez en `Workspace::open`. Cambiar el YAML en disco durante esa
//! misma sesión no puede hacer que `Store::reconcile_all` vuelva a cargarlo y adopte otro universo
//! de documentos mientras el core continúa usando la policy capturada al abrir.

use std::collections::BTreeSet;
use std::path::Path;

use lodestar_core::types::RelPath;
use lodestar_workspace::Workspace;

fn write(root: &Path, relative: &str, contents: &str) {
    let path = root.join(relative);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn paths(items: &[&str]) -> BTreeSet<RelPath> {
    items
        .iter()
        .map(|path| RelPath::new(path).unwrap())
        .collect()
}

#[test]
fn c1_reconcile_all_no_recarga_discovery_policy_a_mitad_de_sesion() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        ".lodestar/config.yaml",
        "discovery:\n  include: [\"**/*.md\"]\n  exclude: [\"oculto/**\"]\n",
    );
    write(root.path(), "visible/a.md", "# visible\n\naguja-visible\n");
    write(root.path(), "oculto/b.md", "# oculto\n\naguja-oculta\n");

    let mut workspace = Workspace::open(root.path()).unwrap();
    let session_documents = paths(&["visible/a.md"]);
    assert_eq!(
        workspace
            .document_set()
            .unwrap()
            .analyze()
            .documents
            .iter()
            .cloned()
            .collect::<BTreeSet<_>>(),
        session_documents,
        "guarda anti-vacuidad: la policy inicial debe distinguir los dos documentos"
    );
    workspace.enable_cache().unwrap();
    assert_eq!(
        workspace
            .cache()
            .unwrap()
            .documents()
            .unwrap()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        session_documents,
        "guarda anti-vacuidad: el rebuild inicial usa la policy efectiva de la sesión"
    );

    // El YAML nuevo describe deliberadamente el universo opuesto. La sesión abierta conserva la
    // policy vieja; tanto reconcile_all directo como un evento del watcher deben usar esa misma
    // autoridad en memoria, nunca volver a cargar config.yaml.
    write(
        root.path(),
        ".lodestar/config.yaml",
        "discovery:\n  include: [\"**/*.md\"]\n  exclude: [\"visible/**\"]\n",
    );
    workspace.cache().unwrap().reconcile_all().unwrap();

    let core_after_config_change = workspace
        .document_set()
        .unwrap()
        .analyze()
        .documents
        .iter()
        .cloned()
        .collect::<BTreeSet<_>>();
    let sqlite_after_config_change = workspace
        .cache()
        .unwrap()
        .documents()
        .unwrap()
        .into_iter()
        .collect::<BTreeSet<_>>();
    assert_eq!(
        core_after_config_change, session_documents,
        "guarda anti-vacuidad: el core conserva la config capturada por Workspace::open"
    );
    assert_eq!(
        sqlite_after_config_change, core_after_config_change,
        "C1 (requirements/e35-h03:20,46): reconcile_all no puede recargar config.yaml y separar la DiscoveryPolicy de SQLite de la efectiva del Workspace"
    );
}
