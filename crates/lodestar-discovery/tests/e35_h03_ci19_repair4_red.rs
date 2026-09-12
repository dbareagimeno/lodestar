//! Regresión C1: la extensión Markdown es case-insensitive también con `include` personalizado.

use std::path::{Path, PathBuf};

use lodestar_discovery::{discover_inventory, DiscoveryPolicy};

struct TestRoot(PathBuf);

impl TestRoot {
    fn new() -> Self {
        let unique = format!(
            "lodestar-e35-h03-ci19-repair4-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("reloj posterior al epoch")
                .as_nanos()
        );
        let path = std::env::temp_dir().join(unique);
        std::fs::create_dir(&path).expect("crear raíz temporal");
        Self(path)
    }

    fn path(&self) -> &Path {
        &self.0
    }

    fn write(&self, relative: &str, contents: &[u8]) {
        let path = self.path().join(relative);
        std::fs::create_dir_all(path.parent().expect("el fixture tiene directorio padre"))
            .expect("crear directorio del fixture");
        std::fs::write(path, contents).expect("crear fichero del fixture");
    }
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn c1_include_personalizado_tolera_solo_la_extension_markdown_en_mayusculas() {
    let root = TestRoot::new();
    root.write("docs/UPPER.MD", b"# Upper\n");
    root.write("docs/lower.md", b"# Lower\n");
    root.write(
        "fuera/ignorado.MD",
        b"contenido fuera del include: \xff\xfe",
    );
    root.write("docs/fuera.txt", b"asset dentro del directorio");

    let inventory = discover_inventory(
        root.path(),
        &DiscoveryPolicy {
            include: vec!["docs/**/*.md".into()],
            ..DiscoveryPolicy::default()
        },
    )
    .expect("discovery debe completar sin abrir el contenido de candidatos excluidos");
    let documents = inventory
        .documents
        .iter()
        .map(|path| path.as_str())
        .collect::<Vec<_>>();

    assert!(
        documents.contains(&"docs/lower.md"),
        "guarda anti-vacuidad: el include personalizado debe admitir su control minúsculo"
    );
    assert!(
        inventory
            .other_files
            .iter()
            .any(|path| path.as_str() == "fuera/ignorado.MD"),
        "el candidato fuera del include debe quedar excluido sin abrir su contenido no UTF-8"
    );
    assert!(
        inventory
            .other_files
            .iter()
            .any(|path| path.as_str() == "docs/fuera.txt"),
        "un fichero no Markdown dentro de docs no puede entrar por la tolerancia de extensión"
    );

    let guard_root = TestRoot::new();
    guard_root.write("docs/PERMITIDO-fuera.MD", b"# Fuera por nombre\n");
    let case_guard = discover_inventory(
        guard_root.path(),
        &DiscoveryPolicy {
            include: vec!["docs/permitido-*.md".into()],
            ..DiscoveryPolicy::default()
        },
    )
    .expect("el control de capitalización del nombre debe completar");
    assert!(
        !case_guard
            .documents
            .iter()
            .any(|path| path.as_str() == "docs/PERMITIDO-fuera.MD"),
        "la tolerancia de extensión no puede volver case-insensitive el nombre completo"
    );
    assert!(
        case_guard
            .other_files
            .iter()
            .any(|path| path.as_str() == "docs/PERMITIDO-fuera.MD"),
        "el control fuera del include debe conservarse como other_file"
    );
    assert_eq!(
        documents,
        vec!["docs/UPPER.MD", "docs/lower.md"],
        "el include personalizado debe admitir ambas capitalizaciones de la extensión Markdown"
    );
}
