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
        // Un `.md` no-UTF8 o ilegible NO aborta la carga entera (dejaría toda lectura de la
        // workspace muerta por un solo fichero): se salta con diagnóstico, como hace
        // `vcs::tree_files` con los blobs no-UTF8.
        let entry = match entry {
            Ok(e) => e,
            Err(e) => {
                eprintln!("lodestar: aviso: entrada ilegible en el bundle: {e}");
                continue;
            }
        };
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
            match std::fs::read_to_string(path) {
                Ok(content) => {
                    files.insert(rp, content);
                }
                Err(e) => {
                    eprintln!(
                        "lodestar: aviso: se salta {} (no UTF-8 o ilegible): {e}",
                        path.display()
                    );
                }
            }
        }
    }
    Ok(files)
}

/// Escritura atómica (temp + fsync + rename) — el único camino de escritura de un `.md`.
///
/// - `sync_all` antes del rename: sin él, una caída de energía podía persistir el rename con
///   los datos sin volcar → `.md` truncado (y los `.md` son LA fuente de verdad, sin copia).
/// - Temporal ÚNICO por proceso+secuencia: con nombre fijo, dos procesos escritores (app +
///   agente MCP) sobre el mismo concept se pisaban el temp y publicaban contenido a medias.
pub fn write_atomic(root: &Path, rel: &RelPath, content: &str) -> Result<(), WorkspaceError> {
    let target = root.join(rel.as_str());
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| WorkspaceError::Io(e.to_string()))?;
    }
    let tmp = tmp_sibling(&target);
    let io_err = |e: std::io::Error| WorkspaceError::Io(e.to_string());
    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&tmp).map_err(io_err)?;
        f.write_all(content.as_bytes()).map_err(io_err)?;
        f.sync_all().map_err(io_err)?;
    }
    if let Err(e) = std::fs::rename(&tmp, &target) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io_err(e));
    }
    // Persiste el rename (la entrada del directorio); best-effort en Unix.
    #[cfg(unix)]
    if let Some(parent) = target.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
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
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let mut name = target.file_name().unwrap_or_default().to_os_string();
    name.push(format!(
        ".{}-{}.lodestar-tmp",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    target.with_file_name(name)
}
