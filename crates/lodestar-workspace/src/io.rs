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
///   agente MCP) sobre el mismo documento se pisaban el temp y publicaban contenido a medias.
pub fn write_atomic(root: &Path, rel: &RelPath, content: &str) -> Result<(), WorkspaceError> {
    let target = root.join(rel.as_str());
    if let Some(parent) = target.parent() {
        std::fs::create_dir_all(parent).map_err(|e| WorkspaceError::Io(e.to_string()))?;
    }
    write_bytes_atomic(&target, content.as_bytes())?;
    // Persiste el rename (la entrada del directorio); best-effort en Unix.
    if let Some(parent) = target.parent() {
        sync_dir(parent);
    }
    Ok(())
}

/// Escribe `bytes` en `path` de forma **atómica y durable** (temporal hermano + `sync_all` +
/// `rename`), SIN fsync de la entrada de directorio.
///
/// Es el protocolo del único escritor extraído para que lo compartan todas las escrituras que deben
/// sobrevivir a un corte de energía: el `.md` canónico ([`write_atomic`]) y, desde E25-H02, las
/// copias de recuperación y sus manifiestos (`recovery.rs`). El `sync_all` es lo que hace la
/// diferencia: sin él, una caída puede persistir la entrada de directorio con el contenido aún en
/// la caché de página, dejando un fichero **truncado** — que es exactamente lo que una copia de
/// recuperación no puede ser.
///
/// El fsync del **directorio** se deja al llamante ([`sync_dir`]) para poder hacerlo **una vez** al
/// final de un lote de escrituras (el caso de `backup_originals`) en lugar de una por fichero.
pub(crate) fn write_bytes_atomic(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
    let io_err = |e: std::io::Error| WorkspaceError::Io(e.to_string());
    let tmp = tmp_sibling(path);
    {
        use std::io::Write as _;
        let mut f = std::fs::File::create(&tmp).map_err(io_err)?;
        f.write_all(bytes).map_err(io_err)?;
        f.sync_all().map_err(io_err)?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io_err(e));
    }
    Ok(())
}

/// Persiste las entradas de directorio de `dir` (los renames ya hechos dentro de él); best-effort en
/// Unix y no-op en el resto de plataformas. Sin esto, una caída puede perder el propio **nombre** de
/// un fichero cuyo contenido sí está volcado.
pub(crate) fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(handle) = std::fs::File::open(dir) {
        let _ = handle.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
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
