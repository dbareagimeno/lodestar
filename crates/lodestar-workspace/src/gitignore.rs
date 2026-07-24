//! Gestión del `.gitignore` versionado del workspace (texto plano, sin `git2` —
//! `ARCHITECTURE.md §19.4`, `DECISIONES.md §0` D5). Ignora la cache derivada (`index.db`) y el
//! runtime desechable (`runtime/`), pero deja versionados los ficheros canónicos de `.lodestar/`
//! (`config.yaml`/`templates/`).
//!
//! Reemplaza a `Vcs::ensure_cache_ignored` (que tocaba `.git/info/exclude`, no versionado): ahora
//! el ajuste vive en el `.gitignore` del propio workspace, para que un repo adoptado lo vea también
//! en `git status`/PR de otros colaboradores.

use std::path::Path;

/// Comentario que marca el bloque gestionado por lodestar dentro del `.gitignore` del usuario.
const MANAGED_COMMENT: &str = "# lodestar: cache y runtime desechables (no versionar)";
/// Entradas que el bloque gestionado garantiza presentes.
const MANAGED_ENTRIES: [&str; 2] = [".lodestar/index.db", ".lodestar/runtime/"];

/// Ajusta `<root>/.gitignore` para que ignore la cache (`.lodestar/index.db`) y el runtime
/// desechable (`.lodestar/runtime/`), preservando cualquier contenido propio del usuario.
///
/// - **Idempotente**: si las entradas ya están presentes, no se reescribe el fichero (ni un
///   byte) — evita duplicar líneas en aperturas sucesivas.
/// - **Adopción**: si el fichero ignoraba `.lodestar/` entero (estilo viejo, el que escribía
///   `Vcs::init`), esa línea se sustituye por las entradas nuevas, de forma que
///   `.lodestar/config.yaml`/`templates/` pasan a quedar versionados.
///
/// Best-effort: un fallo de escritura (p. ej. checkout de solo lectura) se reporta por stderr y
/// no aborta la apertura del workspace — mismo criterio que el `ensure_cache_ignored` al que
/// reemplaza.
pub(crate) fn ensure_gitignore(root: &Path) {
    let path = root.join(".gitignore");
    let current = std::fs::read_to_string(&path).unwrap_or_default();

    if MANAGED_ENTRIES
        .iter()
        .all(|entry| current.lines().any(|l| l.trim() == *entry))
    {
        return; // ya gestionado: nada que hacer (garantiza idempotencia byte-a-byte).
    }

    let is_old_style = |l: &str| {
        matches!(
            l.trim(),
            ".lodestar/" | "/.lodestar/" | ".lodestar" | "/.lodestar"
        )
    };
    let mut lines: Vec<&str> = current.lines().filter(|l| !is_old_style(l)).collect();
    while matches!(lines.last(), Some(l) if l.trim().is_empty()) {
        lines.pop();
    }

    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(MANAGED_COMMENT);
    out.push('\n');
    for entry in MANAGED_ENTRIES {
        out.push_str(entry);
        out.push('\n');
    }

    if let Err(e) = std::fs::write(&path, out) {
        eprintln!("lodestar: aviso: no se pudo ajustar .gitignore: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crea_bloque_en_gitignore_vacio() {
        let dir = tempfile::tempdir().unwrap();
        ensure_gitignore(dir.path());
        let out = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(out.contains(".lodestar/index.db"));
        assert!(out.contains(".lodestar/runtime/"));
    }

    #[test]
    fn preserva_contenido_propio_y_es_idempotente() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "node_modules/\n").unwrap();
        ensure_gitignore(dir.path());
        let primera = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(primera.contains("node_modules/"));
        ensure_gitignore(dir.path());
        let segunda = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert_eq!(primera, segunda);
    }

    #[test]
    fn sustituye_estilo_viejo() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".gitignore"), "/.lodestar/\n").unwrap();
        ensure_gitignore(dir.path());
        let out = std::fs::read_to_string(dir.path().join(".gitignore")).unwrap();
        assert!(!out.lines().any(|l| l.trim() == "/.lodestar/"));
        assert!(out.contains(".lodestar/index.db"));
        assert!(out.contains(".lodestar/runtime/"));
    }
}
