//! C1 de E35-H03: el rebuild que arranca una sesión debe recibir la política efectiva del
//! workspace.  El store no puede inventar una política distinta leyendo el árbol por su cuenta.

use std::collections::BTreeSet;

use lodestar_core::types::RelPath;
use lodestar_workspace::Workspace;

fn write(root: &std::path::Path, path: &str, bytes: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(target, bytes).unwrap();
}

fn markdown(title: &str, body: &str) -> String {
    format!("---\ntitle: {title}\n---\n\n# {title}\n\n{body}\n")
}

#[test]
fn c1_enable_cache_aplica_la_politica_canonica_sin_leer_excluidos() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        ".lodestar/config.yaml",
        "discovery:\n  include: [\"docs/**/*.md\"]\n  exclude: [\"docs/privado.md\", \"docs/grande.md\"]\n  respectGitignore: true\n  respectLodestarIgnore: true\n  maxDocumentBytes: 256\n",
    );
    write(
        root.path(),
        "docs/admitido.md",
        markdown("admitido", "aguja-c1-admitida [asset](../assets/logo.svg)"),
    );
    write(
        root.path(),
        "docs/privado.md",
        markdown("privado", "aguja-c1-privada"),
    );
    write(
        root.path(),
        "docs/grande.md",
        markdown(
            "grande",
            &("aguja-c1-grande ".to_string() + &"x".repeat(4096)),
        ),
    );
    write(root.path(), ".gitignore", "docs/gitignored.md\n");
    write(root.path(), ".lodestarignore", "docs/lodestarignored.md\n");
    write(
        root.path(),
        "docs/gitignored.md",
        markdown("gitignored", "aguja-c1-gitignore"),
    );
    write(
        root.path(),
        "docs/lodestarignored.md",
        markdown("lodestarignored", "aguja-c1-lodestarignore"),
    );
    write(root.path(), "docs/no-utf8.md", [0xff, 0xfe, 0xfd, 0xfc]);
    write(
        root.path(),
        "notes/fora.md",
        markdown("fora", "aguja-c1-fora"),
    );
    write(root.path(), "assets/logo.svg", "<svg>c1</svg>\n");

    let mut workspace = Workspace::open(root.path()).unwrap();
    let expected_set = workspace.document_set().unwrap();
    let expected = expected_set.analyze();
    assert_eq!(
        expected.documents,
        vec![RelPath::new("docs/admitido.md").unwrap()],
        "C1: la aguja de la policy debe distinguir el inventario canónico"
    );
    workspace.enable_cache().unwrap();
    let cache = workspace.cache().unwrap();

    assert_eq!(
        cache
            .documents()
            .unwrap()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        expected
            .documents
            .clone()
            .into_iter()
            .collect::<BTreeSet<_>>(),
        "C1: enable_cache debe reconstruir exactamente el inventario de la sesión"
    );
    assert!(cache
        .fts_candidates("aguja-c1-admitida")
        .unwrap()
        .contains(&RelPath::new("docs/admitido.md").unwrap()));
    for needle in [
        "aguja-c1-privada",
        "aguja-c1-grande",
        "aguja-c1-fora",
        "aguja-c1-gitignore",
        "aguja-c1-lodestarignore",
    ] {
        assert!(
            cache.fts_candidates(needle).unwrap().is_empty(),
            "C1: contenido no admitido no puede ser leído/indexado: {needle}"
        );
    }
    let (_, diagnostics) = workspace.document_set_with_discovery().unwrap();
    assert!(
        diagnostics
            .iter()
            .any(|check| check.code.as_str() == "DOC-NOT-UTF8"),
        "C1: un Markdown no-UTF8 real debe producir diagnóstico de descubrimiento"
    );
    assert!(cache
        .outgoing_links(&RelPath::new("docs/admitido.md").unwrap())
        .unwrap()
        .iter()
        .any(|(_, kind, path, _)| kind == "workspaceFile"
            && path.as_deref() == Some("assets/logo.svg")));
}
