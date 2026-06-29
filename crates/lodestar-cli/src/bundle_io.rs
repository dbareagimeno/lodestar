//! Lectura del bundle desde disco (equivalente al `open_ephemeral` hasta que exista la workspace).
//!
//! Usa `ignore::WalkBuilder` (respeta `.gitignore`, excluye `.lodestar/` y `.git/`). El único
//! chokepoint de path-traversal sigue siendo `RelPath` del core.

use std::path::{Path, PathBuf};

use anyhow::Context;
use lodestar_core::types::{FileMap, RelPath};

/// Carga todos los `.md` del bundle a un `FileMap` con claves `RelPath` relativas al root.
pub fn load_bundle(root: &Path) -> anyhow::Result<FileMap> {
    let mut files = FileMap::new();
    let walker = ignore::WalkBuilder::new(root)
        .hidden(false)
        .git_ignore(true)
        .filter_entry(|e| {
            let name = e.file_name().to_string_lossy();
            name != ".lodestar" && name != ".git"
        })
        .build();
    for entry in walker {
        let entry = entry.context("recorriendo el bundle")?;
        let path = entry.path();
        if !path.is_file() || path.extension().map(|e| e != "md").unwrap_or(true) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        let rp = match RelPath::new(&rel) {
            Ok(rp) => rp,
            Err(_) => continue, // rutas no representables se ignoran
        };
        let content =
            std::fs::read_to_string(path).with_context(|| format!("leyendo {}", path.display()))?;
        files.insert(rp, content);
    }
    Ok(files)
}

/// Escritura atómica (temp + rename) de un `.md`, creando los directorios necesarios.
pub fn write_atomic(root: &Path, rel: &RelPath, content: &str) -> anyhow::Result<()> {
    let target = root.join(rel.as_str());
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).with_context(|| format!("creando {}", parent.display()))?;
    }
    let tmp = tmp_sibling(&target);
    std::fs::write(&tmp, content).with_context(|| format!("escribiendo {}", tmp.display()))?;
    std::fs::rename(&tmp, &target)
        .with_context(|| format!("renombrando a {}", target.display()))?;
    Ok(())
}

/// Borra un fichero del bundle (para la purga de tags obsoletos).
pub fn delete(root: &Path, rel: &RelPath) -> anyhow::Result<()> {
    let target = root.join(rel.as_str());
    if target.exists() {
        std::fs::remove_file(&target).with_context(|| format!("borrando {}", target.display()))?;
    }
    Ok(())
}

fn tmp_sibling(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".lodestar-tmp");
    target.with_file_name(name)
}
