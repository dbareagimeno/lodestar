//! Gestión del `.gitignore` versionado del workspace (texto plano, sin `git2` —
//! `ARCHITECTURE.md §19.4`, `DECISIONES.md §0` D5). Ignora la cache derivada (`index.db`) y el
//! runtime desechable (`runtime/`), pero deja versionados los ficheros canónicos de `.lodestar/`
//! (`config.yaml`/`templates/`).
//!
//! Reemplaza a `Vcs::ensure_cache_ignored` (que tocaba `.git/info/exclude`, no versionado): ahora
//! el ajuste vive en el `.gitignore` del propio workspace, para que un repo adoptado lo vea también
//! en `git status`/PR de otros colaboradores.
//!
//! **Cuándo se dispara (E23-H12)**: ya NO al abrir el workspace — leer un proyecto ajeno no puede
//! modificarlo. El único punto de entrada es [`crate::Workspace::ensure_managed_gitignore`], que
//! llaman los cuatro chokepoints que cubren todo camino de escritura: `enable_cache`,
//! `acquire_lock` y, en `lodestar-app`, `persist_plan` y `try_append_audit`. Quien crea lo
//! derivado es quien lo ignora.

use std::path::Path;

/// Comentario que marca el bloque gestionado por lodestar dentro del `.gitignore` del usuario.
const MANAGED_COMMENT: &str = "# lodestar: cache y runtime desechables (no versionar)";
/// Entradas que el bloque gestionado garantiza presentes.
const MANAGED_ENTRIES: [&str; 2] = [".lodestar/index.db", ".lodestar/runtime/"];

/// Ajusta `<root>/.gitignore` para que ignore la cache (`.lodestar/index.db`) y el runtime
/// desechable (`.lodestar/runtime/`), preservando cualquier contenido propio del usuario.
///
/// - **Idempotente**: si las entradas ya están presentes, no se reescribe el fichero (ni un
///   byte) — evita duplicar líneas, y es lo que permite llamarlo en CADA escritura sin churnear
///   el fichero del usuario (E23-H12).
/// - **Adopción**: si el fichero ignoraba `.lodestar/` entero (estilo viejo, el que escribía
///   `Vcs::init`), esa línea se sustituye por las entradas nuevas, de forma que
///   `.lodestar/config.yaml`/`templates/` pasan a quedar versionados.
/// - **Respetuoso con el fin de línea** (E25-H06): se reemite con el estilo **dominante** del
///   fichero — un `.gitignore` en CRLF sigue en CRLF, incluidas las líneas nuevas del bloque
///   gestionado. Hasta v0.3.1 se reconstruía con `str::lines`, que se traga el `\r`, y el usuario se
///   encontraba un diff espurio de fichero entero en un fichero **versionado**.
/// - **Atómico** (E25-H06): se publica por el mismo protocolo temp+fsync+rename que los `.md`
///   ([`crate::io::write_bytes_atomic`]) en vez de truncar el fichero vivo con `std::fs::write`. Es
///   el único fichero versionado del usuario que el motor toca, y se toca en CADA escritura: un
///   lector concurrente (`git status`) o un crash no pueden verlo a medias.
///
/// Best-effort: un fallo de escritura (p. ej. checkout de solo lectura) se reporta por stderr y
/// no aborta la operación que lo invocó — mismo criterio que el `ensure_cache_ignored` al que
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

    let eol = fin_de_linea_dominante(&current);

    let is_old_style = |l: &str| {
        matches!(
            l.trim(),
            ".lodestar/" | "/.lodestar/" | ".lodestar" | "/.lodestar"
        )
    };
    // `str::lines` ya descarta el `\r` final de cada línea, así que reconstruir con `eol` reemite
    // el estilo del fichero en TODAS las líneas (las del usuario y las nuevas).
    let mut lines: Vec<&str> = current.lines().filter(|l| !is_old_style(l)).collect();
    while matches!(lines.last(), Some(l) if l.trim().is_empty()) {
        lines.pop();
    }

    let mut out = lines.join(eol);
    if !out.is_empty() {
        out.push_str(eol);
        out.push_str(eol);
    }
    out.push_str(MANAGED_COMMENT);
    out.push_str(eol);
    for entry in MANAGED_ENTRIES {
        out.push_str(entry);
        out.push_str(eol);
    }

    if let Err(e) = crate::io::write_bytes_atomic(&path, out.as_bytes()) {
        eprintln!("lodestar: aviso: no se pudo ajustar .gitignore: {e}");
        return;
    }
    // Persiste el rename. Best-effort como el resto de la función: el `.gitignore` es una comodidad
    // para el repo del usuario, no conocimiento canónico, y su fallo no puede tumbar la escritura
    // que lo invocó.
    if let Err(e) = crate::io::sync_dir(root) {
        eprintln!("lodestar: aviso: no se pudo persistir el ajuste de .gitignore: {e}");
    }
}

/// Estilo de fin de línea **dominante** del fichero: `"\r\n"` si la mayoría de sus saltos son CRLF,
/// `"\n"` en cualquier otro caso (incluido el fichero vacío o sin saltos) — E25-H06.
///
/// Se decide por mayoría y no por «hay algún CRLF» para que un fichero mixto no cambie de estilo por
/// una línea suelta, y para que preservar CRLF no se convierta en imponerlo: quien trabaja en Unix
/// no puede llevarse el diff espurio que este arreglo le quita a quien trabaja en Windows.
fn fin_de_linea_dominante(contenido: &str) -> &'static str {
    let saltos = contenido.matches('\n').count();
    let crlf = contenido.matches("\r\n").count();
    if crlf * 2 > saltos {
        "\r\n"
    } else {
        "\n"
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

    /// E25-H06: el estilo de fin de línea se decide por **mayoría**, no por «hay algún CRLF». Un
    /// fichero mixto con un CRLF suelto sigue siendo un fichero LF, y viceversa.
    #[test]
    fn el_fin_de_linea_lo_decide_la_mayoria() {
        assert_eq!(fin_de_linea_dominante(""), "\n", "fichero vacío → LF");
        assert_eq!(fin_de_linea_dominante("a\nb\n"), "\n");
        assert_eq!(fin_de_linea_dominante("a\r\nb\r\n"), "\r\n");
        assert_eq!(
            fin_de_linea_dominante("a\r\nb\nc\nd\n"),
            "\n",
            "un CRLF suelto en un fichero LF no cambia el estilo"
        );
        assert_eq!(
            fin_de_linea_dominante("a\nb\r\nc\r\nd\r\n"),
            "\r\n",
            "un LF suelto en un fichero CRLF tampoco"
        );
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
