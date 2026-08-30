//! Regresiones C1 para las fronteras de la policy canónica de discovery.

use std::path::{Path, PathBuf};

use lodestar_discovery::{discover_inventory, DiscoveryPolicy};

struct TestRoot(PathBuf);

impl TestRoot {
    fn new(case: &str) -> Self {
        let unique = format!(
            "lodestar-e35-h03-repair12-{case}-{}-{}",
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
}

impl Drop for TestRoot {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn c1_max_document_bytes_es_inclusivo_y_max_mas_uno_se_rechaza() {
    let root = TestRoot::new("inclusive-limit");
    let limit = 32;
    std::fs::write(root.path().join("exacto.md"), vec![b'a'; limit])
        .expect("crear candidato justo en el límite");
    std::fs::write(root.path().join("excedido.md"), vec![b'b'; limit + 1])
        .expect("crear control negativo sobre el límite");

    let inventory = discover_inventory(
        root.path(),
        &DiscoveryPolicy {
            max_document_bytes: limit,
            ..DiscoveryPolicy::default()
        },
    )
    .expect("discovery debe completar con ambos ficheros presentes");
    let documents = inventory
        .documents
        .iter()
        .map(|path| path.as_str())
        .collect::<Vec<_>>();
    let too_large = inventory
        .diagnostics
        .iter()
        .filter(|check| check.code.as_str() == "DOC-TOO-LARGE")
        .collect::<Vec<_>>();

    assert!(
        documents.contains(&"exacto.md"),
        "un candidato de exactamente {limit} bytes debe quedar admitido: {documents:?}"
    );
    assert!(
        !inventory
            .other_files
            .iter()
            .any(|path| path.as_str() == "exacto.md"),
        "el documento en el máximo inclusivo no puede reclasificarse como asset"
    );
    assert!(
        !documents.contains(&"excedido.md"),
        "la guarda max+1 debe impedir una admisión vacua: {documents:?}"
    );
    assert!(
        inventory
            .other_files
            .iter()
            .any(|path| path.as_str() == "excedido.md"),
        "el candidato max+1 debe conservarse como other_file"
    );
    assert_eq!(
        too_large.len(),
        1,
        "solo max+1 debe emitir DOC-TOO-LARGE: {:?}",
        inventory.diagnostics
    );
    assert!(
        too_large[0].msg.contains("excedido.md"),
        "el diagnóstico debe señalar el control negativo max+1: {:?}",
        too_large[0]
    );
}

#[cfg(unix)]
#[test]
fn c1_symlink_a_fichero_emite_diagnostico_generico_no_de_directorio() {
    use std::os::unix::fs::symlink;

    let root = TestRoot::new("file-symlink");
    std::fs::write(root.path().join("destino.txt"), b"asset").expect("crear destino regular");
    symlink("destino.txt", root.path().join("enlace.md")).expect("crear symlink a fichero");

    let inventory = discover_inventory(root.path(), &DiscoveryPolicy::default())
        .expect("discovery debe diagnosticar sin seguir el enlace");
    let diagnostic = inventory
        .diagnostics
        .iter()
        .find(|check| {
            check.code.as_str() == "SYMLINK-UNSUPPORTED" && check.msg.contains("enlace.md")
        })
        .expect("el symlink a fichero debe producir SYMLINK-UNSUPPORTED");

    assert!(
        !diagnostic.msg.contains("a un directorio"),
        "un destino regular no puede describirse como directorio: {diagnostic:?}"
    );
    assert!(
        inventory
            .other_files
            .iter()
            .any(|path| path.as_str() == "enlace.md"),
        "el enlace rechazado debe permanecer como asset enlazable"
    );
    assert!(
        !inventory
            .documents
            .iter()
            .any(|path| path.as_str() == "enlace.md"),
        "el enlace rechazado no debe admitirse como documento"
    );
}

#[cfg(unix)]
#[test]
fn c1_symlink_a_directorio_emite_diagnostico_especifico() {
    use std::os::unix::fs::symlink;

    let root = TestRoot::new("directory-symlink");
    std::fs::create_dir(root.path().join("destino")).expect("crear directorio destino");
    symlink("destino", root.path().join("atajo")).expect("crear symlink a directorio");

    let inventory = discover_inventory(root.path(), &DiscoveryPolicy::default())
        .expect("discovery debe diagnosticar sin seguir el directorio");
    let diagnostic = inventory
        .diagnostics
        .iter()
        .find(|check| check.code.as_str() == "SYMLINK-UNSUPPORTED" && check.msg.contains("atajo"))
        .expect("el symlink a directorio debe producir SYMLINK-UNSUPPORTED");

    assert!(
        diagnostic.msg.contains("a un directorio"),
        "el diagnóstico debe distinguir el destino directorio: {diagnostic:?}"
    );
    assert!(
        inventory
            .other_files
            .iter()
            .any(|path| path.as_str() == "atajo"),
        "el enlace de directorio rechazado debe quedar representado en other_files"
    );
    assert!(
        !inventory
            .directories
            .iter()
            .any(|path| path.as_str() == "atajo"),
        "el symlink rechazado no debe confundirse con un directorio recorrido"
    );
}
