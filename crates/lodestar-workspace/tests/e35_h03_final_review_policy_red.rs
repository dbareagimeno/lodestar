//! Reproducción independiente de la paridad C1 para la policy efectiva de E35-H03.
//!
//! `note.txt` es deliberadamente regular, UTF-8 y admitido por `include: ["**/*"]`.  El
//! inventario canónico y la reconstrucción de cache deben decidir lo mismo sobre él; comparar
//! únicamente el número de documentos escondería la diferencia `Document`/`workspaceFile`.

use std::path::Path;

use lodestar_core::types::{LinkTarget, RelPath};
use lodestar_workspace::Workspace;

fn write(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(target, contents).unwrap();
}

fn target_kind(target: &LinkTarget) -> &'static str {
    match target {
        LinkTarget::Document(_) => "document",
        LinkTarget::WorkspaceFile(_) => "workspaceFile",
        LinkTarget::Missing(_) => "missing",
        LinkTarget::ExternalUri(_) => "externalUri",
        LinkTarget::SelfAnchor(_) => "selfAnchor",
        LinkTarget::EscapesWorkspace => "escapesWorkspace",
        LinkTarget::WorkspaceDirectory(_) => "workspaceDirectory",
    }
}

#[test]
fn c1_include_all_regular_utf8_file_keeps_core_and_cache_inventory_identical() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        ".lodestar/config.yaml",
        "discovery:\n  include: [\"**/*\"]\n  exclude: []\n  respectGitignore: true\n  respectLodestarIgnore: true\n  maxDocumentBytes: 4096\n",
    );
    write(
        root.path(),
        "index.md",
        "# index\n\n[regular note](note.txt)\n",
    );
    write(
        root.path(),
        "note.txt",
        "regular UTF-8 note used to distinguish document admission from workspaceFile\n",
    );

    let mut workspace = Workspace::open(root.path()).unwrap();
    // `enable_cache` is a write chokepoint: it creates the managed `.gitignore`.  The oracle is
    // therefore the canonical discovery of the *stable post-enable state*, not a pre-write
    // inventory captured before that file existed.
    workspace.enable_cache().unwrap();
    let (canonical, diagnostics) = workspace.document_set_with_discovery().unwrap();
    assert!(
        diagnostics.is_empty(),
        "fixture must not hide discovery errors: {diagnostics:?}"
    );
    let note = RelPath::new("note.txt").unwrap();
    let gitignore = RelPath::new(".gitignore").unwrap();
    let source = RelPath::new("index.md").unwrap();
    assert!(
        canonical.files().contains_key(&note),
        "guard: policy include [\"**/*\"] must admit note.txt"
    );
    assert!(
        canonical.files().contains_key(&gitignore),
        "guard: policy include [\"**/*\"] must admit the managed .gitignore"
    );
    let canonical_link = canonical
        .analyze()
        .outgoing
        .get(&source)
        .and_then(|links| links.first())
        .expect("guard: index.md must contain a real link to note.txt");
    assert!(
        matches!(
            canonical_link.target,
            LinkTarget::Document(_) | LinkTarget::WorkspaceFile(_)
        ),
        "guard: note.txt must be classified as an observable internal target"
    );

    let cache = workspace.cache().unwrap();
    let expected_documents = canonical.analyze().documents.clone();
    let cached_documents = cache.documents().unwrap();
    let cached_link = cache
        .outgoing_links(&source)
        .unwrap()
        .into_iter()
        .next()
        .expect("guard: rebuilt cache must preserve the real link row");

    assert_eq!(
        (expected_documents, target_kind(&canonical_link.target)),
        (cached_documents, cached_link.1.as_str()),
        "C1: document admission and note.txt link classification must be identical between normal discovery and enable_cache rebuild"
    );
    assert_eq!(cached_link.2.as_deref(), Some("note.txt"));
    assert!(
        matches!(canonical_link.target, LinkTarget::Document(ref path) if path == &note),
        "guard: note.txt must remain a Document target in the canonical analysis"
    );
    assert_eq!(cached_link.1, "document");
}
