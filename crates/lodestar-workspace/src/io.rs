//! El **único escritor**: lectura del bundle y escritura atómica temp+rename (`§6`).

use std::path::{Path, PathBuf};

use lodestar_core::types::{FileMap, RelPath};

use crate::error::WorkspaceError;

/// Carga todos los `.md` del bundle a un `FileMap` (excluye `.lodestar/` y `.git/`).
pub fn load_bundle(root: &Path) -> Result<FileMap, WorkspaceError> {
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
        let entry = entry.map_err(|e| WorkspaceError::Io(e.to_string()))?;
        let path = entry.path();
        if !path.is_file() || path.extension().map(|e| e != "md").unwrap_or(true) {
            continue;
        }
        let rel = path
            .strip_prefix(root)
            .unwrap_or(path)
            .to_string_lossy()
            .replace('\\', "/");
        if let Ok(rp) = RelPath::new(&rel) {
            let content =
                std::fs::read_to_string(path).map_err(|e| WorkspaceError::Io(e.to_string()))?;
            files.insert(rp, content);
        }
    }
    Ok(files)
}

/// Escritura atómica (temp + rename) — el único camino de escritura de un `.md`.
pub fn write_atomic(root: &Path, rel: &RelPath, content: &str) -> Result<(), WorkspaceError> {
    let target = root.join(rel.as_str());
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| WorkspaceError::Io(e.to_string()))?;
    }
    let tmp = tmp_sibling(&target);
    std::fs::write(&tmp, content).map_err(|e| WorkspaceError::Io(e.to_string()))?;
    std::fs::rename(&tmp, &target).map_err(|e| WorkspaceError::Io(e.to_string()))?;
    Ok(())
}

/// Borra un fichero (purga de tags obsoletos).
pub fn delete(root: &Path, rel: &RelPath) -> Result<(), WorkspaceError> {
    let target = root.join(rel.as_str());
    if target.exists() {
        std::fs::remove_file(&target).map_err(|e| WorkspaceError::Io(e.to_string()))?;
    }
    Ok(())
}

fn tmp_sibling(target: &Path) -> PathBuf {
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(".lodestar-tmp");
    target.with_file_name(name)
}
