//! Tests de los workspaces de ejemplo universales (E15-H05).
//!
//! Verifican que los fixtures que consumen las historias de descubrimiento (E15-H07) y de enlaces
//! (E17) son deterministas y se materializan tal cual en disco.

use std::collections::BTreeSet;

use lodestar_core::types::{FileMap, RelPath};
use lodestar_fixtures as fx;

/// Lee todos los `.md` bajo `root` a un `FileMap`, recorriendo el árbol a mano (sin depender de
/// `lodestar-workspace`: este crate no lo conoce y el descubrimiento real es E15-H07).
fn leer_md(root: &std::path::Path) -> FileMap {
    fn recorrer(dir: &std::path::Path, root: &std::path::Path, out: &mut FileMap) {
        for entrada in std::fs::read_dir(dir).expect("directorio legible") {
            let ruta = entrada.expect("entrada legible").path();
            if ruta.is_dir() {
                recorrer(&ruta, root, out);
            } else if ruta.extension().is_some_and(|e| e == "md") {
                let rel = ruta
                    .strip_prefix(root)
                    .expect("bajo el root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let contenido = std::fs::read_to_string(&ruta).expect("UTF-8");
                out.insert(RelPath::new(&rel).expect("path válido"), contenido);
            }
        }
    }
    let mut out = FileMap::new();
    recorrer(root, root, &mut out);
    out
}

#[test]
fn fixture_arbitrary_roundtrip() {
    let files = fx::arbitrary();
    let dir = tempfile::tempdir().unwrap();
    fx::materialize(&files, dir.path()).unwrap();

    assert_eq!(
        leer_md(dir.path()),
        files,
        "materializar y releer debe devolver el mismo FileMap"
    );

    // Los 4 documentos, incluido el de tres niveles de profundidad.
    let paths: BTreeSet<&str> = files.keys().map(|p| p.as_str()).collect();
    assert_eq!(
        paths,
        BTreeSet::from([
            "README.md",
            "one/first.md",
            "two/levels/second.md",
            "three/levels/deep/third.md",
        ])
    );

    // Ningún documento tiene frontmatter ni hay index.md: es el caso "proyecto que nunca ha visto
    // Lodestar" del §Resultado esperado.
    assert!(!files.values().any(|c| c.starts_with("---")));
    assert!(!paths.iter().any(|p| p.ends_with("index.md")));

    // Los enlaces cruzados raíz ↔ profundo existen en ambos sentidos.
    assert!(files[&RelPath::new("README.md").unwrap()].contains("three/levels/deep/third.md"));
    assert!(
        files[&RelPath::new("three/levels/deep/third.md").unwrap()].contains("../../../README.md")
    );
}

#[test]
fn fixture_edge_cases_materializa() {
    let files = fx::with_edge_cases();
    let dir = tempfile::tempdir().unwrap();
    fx::materialize(&files, dir.path()).unwrap();

    // Path con espacios y directorio oculto llegan a disco tal cual.
    assert!(dir.path().join("notas/con espacios.md").is_file());
    assert!(dir.path().join(".oculto/secreto.md").is_file());

    // Mismo basename en dos árboles distintos, ambos presentes y distinguibles por path.
    assert!(dir.path().join("docs/auth.md").is_file());
    assert!(dir.path().join("packages/api/docs/auth.md").is_file());

    // El documento con los casos de enlace lleva el href con %20 y el de capitalización errónea.
    let raiz = &files[&RelPath::new("raiz.md").unwrap()];
    assert!(raiz.contains("notas/con%20espacios.md"));
    assert!(raiz.contains("Docs/Auth.md"));
}

#[test]
fn fixture_disk_only_materializa() {
    let dir = tempfile::tempdir().unwrap();
    let limite = 1024;
    fx::materialize_disk_only(dir.path(), limite).unwrap();

    // No UTF-8: existe como fichero pero no se puede leer como texto.
    assert!(std::fs::read_to_string(dir.path().join("binario.md")).is_err());
    // Un byte por encima del límite.
    assert_eq!(
        std::fs::metadata(dir.path().join("enorme.md"))
            .unwrap()
            .len(),
        limite as u64 + 1
    );
    // Ficheros de control del descubrimiento y sus víctimas.
    assert!(dir.path().join(".gitignore").is_file());
    assert!(dir.path().join("vendor/dep.md").is_file());
    assert!(dir.path().join(".lodestarignore").is_file());
    assert!(dir.path().join("borradores/wip.md").is_file());
    // Fichero del proyecto que no es Markdown (destino de un enlace WorkspaceFile).
    assert!(dir.path().join("src/auth/token_service.rs").is_file());

    #[cfg(unix)]
    assert!(
        std::fs::symlink_metadata(dir.path().join("enlace.md"))
            .unwrap()
            .file_type()
            .is_symlink(),
        "enlace.md debe ser un symlink, no un fichero regular"
    );
}

#[test]
fn fixtures_son_deterministas() {
    assert_eq!(fx::arbitrary(), fx::arbitrary());
    assert_eq!(fx::with_edge_cases(), fx::with_edge_cases());
}
