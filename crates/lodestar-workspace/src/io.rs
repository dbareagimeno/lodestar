//! El **único escritor**: escritura atómica temp+rename (`§6`).
//!
//! La **lectura** del inventario vive desde E15-H07 en [`crate::discovery`]: el `load_bundle` que
//! ocupaba este módulo se retiró al quedar sin llamadores (`ARCHITECTURE.md §20.5`).

use std::path::{Path, PathBuf};

use lodestar_core::types::RelPath;

use crate::error::WorkspaceError;

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
