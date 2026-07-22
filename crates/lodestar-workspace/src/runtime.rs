//! Scaffold de `.lodestar/runtime/` (E9-H06, `ARCHITECTURE.md §19.4`, `DECISIONES.md §0` D5): el
//! árbol de trabajo desechable del workspace (planes/recibos/staging que usará E13). Se crea al
//! abrir el bundle; `journal`/`audit.jsonl` los crea E13 perezosamente cuando existan — aquí solo
//! se garantizan los subdirectorios base.
//!
//! El walker de conocimiento (`io::load_bundle`) y el watcher siguen excluyendo `.lodestar/`
//! entero (runtime incluido) del índice de conceptos; la config canónica se lee aparte, por
//! `WorkspaceConfig::load` (out-of-band, no como concept).

use std::path::Path;

/// Subdirectorios que garantiza presentes bajo `.lodestar/runtime/`.
const RUNTIME_SUBDIRS: [&str; 3] = ["plans", "receipts", "staging"];

/// Crea `.lodestar/runtime/{plans,receipts,staging}` si no existen.
///
/// Best-effort: un fallo (p. ej. checkout de solo lectura) se reporta por stderr y no aborta la
/// apertura del bundle — mismo criterio que [`crate::gitignore::ensure_gitignore`].
pub(crate) fn ensure_runtime_scaffold(root: &Path) {
    for sub in RUNTIME_SUBDIRS {
        let dir = root.join(".lodestar/runtime").join(sub);
        if let Err(e) = std::fs::create_dir_all(&dir) {
            eprintln!("lodestar: aviso: no se pudo crear {}: {e}", dir.display());
        }
    }
}
