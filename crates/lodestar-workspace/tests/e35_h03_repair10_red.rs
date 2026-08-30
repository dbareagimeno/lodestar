//! E35-H03 C1/C7 — el consumidor workspace finaliza el diagnostico UTF-8 canonico.

use std::fs;
use std::path::Path;

use lodestar_core::types::RelPath;
use lodestar_workspace::Workspace;

fn write(root: &Path, path: &str, contents: impl AsRef<[u8]>) {
    let target = root.join(path);
    if let Some(parent) = target.parent() {
        fs::create_dir_all(parent).unwrap();
    }
    fs::write(target, contents).unwrap();
}

fn rp(path: &str) -> RelPath {
    RelPath::new(path).expect("RelPath valido")
}

/// C1/C7 — la primera pasada solo inventaria candidatos; la lectura canonica del workspace debe
/// reclasificar el candidato no UTF-8 como fichero y emitir exactamente su diagnostico de
/// discovery. `enable_cache` ejercita ademas el rebuild real con la misma policy efectiva.
#[test]
fn c1_c7_workspace_y_cache_comparten_clasificacion_utf8_diferida() {
    let root = tempfile::tempdir().unwrap();
    write(
        root.path(),
        "00-source.md",
        "# Source\n\n[valid](zz-valid.md) [invalid](yy-invalid.md)\n",
    );
    write(root.path(), "zz-valid.md", "# Valid\n");
    write(root.path(), "yy-invalid.md", [0xff, 0xfe, b'x', b'\n']);

    let mut workspace = Workspace::open(root.path()).unwrap();
    let (core, diagnostics) = workspace.document_set_with_discovery().unwrap();
    assert_eq!(
        core.analyze().documents,
        vec![rp("00-source.md"), rp("zz-valid.md")],
        "guard anti-vacuidad: el core excluye el cuerpo no UTF-8"
    );
    let not_utf8: Vec<_> = diagnostics
        .iter()
        .filter(|check| check.code.as_str() == "DOC-NOT-UTF8")
        .collect();
    assert_eq!(not_utf8.len(), 1, "un solo candidato no UTF-8");
    assert_eq!(not_utf8[0].targets, vec![rp("yy-invalid.md")]);

    workspace.enable_cache().expect("rebuild real de la cache");
    let cache = workspace.cache().expect("cache activa");
    assert_eq!(
        cache.documents().unwrap(),
        core.analyze().documents,
        "C7: cache y core publican el mismo conjunto final"
    );
    let links = cache.outgoing_links(&rp("00-source.md")).unwrap();
    assert!(
        links.iter().any(|(_, kind, path, _)| {
            kind == "document" && path.as_deref() == Some("zz-valid.md")
        }),
        "el candidato UTF-8 posterior termina como document"
    );
    assert!(
        links.iter().any(|(_, kind, path, _)| {
            kind == "workspaceFile" && path.as_deref() == Some("yy-invalid.md")
        }),
        "el candidato no UTF-8 posterior permanece workspaceFile"
    );
}
