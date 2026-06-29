//! Red confinada al binario `git` (`ARCHITECTURE.md §13.2`, `§12` seguridad).
//!
//! Subproceso con **argumentos fijos validados**, **jamás** interpola input no confiable, **nunca**
//! corre en open/index (solo por acción explícita del usuario). Hereda el auth del usuario.

use std::path::Path;
use std::process::Command;

use lodestar_core::types::{SyncKind, SyncOutcome};

use crate::error::VcsError;

/// Ejecuta `git <args>` en el root del bundle. Los `args` son **literales fijos** del llamante.
pub fn run_git(root: &Path, args: &[&str], kind: SyncKind) -> Result<SyncOutcome, VcsError> {
    // Defensa: ningún argumento puede venir de input de usuario; todos son literales del código.
    debug_assert!(args.iter().all(|a| !a.is_empty()));
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|e| VcsError::Io(format!("no se pudo lanzar `git`: {e}")))?;
    let summary = if output.status.success() {
        String::from_utf8_lossy(&output.stdout).trim().to_string()
    } else {
        String::from_utf8_lossy(&output.stderr).trim().to_string()
    };
    Ok(SyncOutcome {
        kind,
        ok: output.status.success(),
        summary,
    })
}
