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

// ===========================================================================
// E25-H05 — Un fsync de directorio que falla deja de ser silencio
// (`requirements/epica-25-endurecimiento-escritura.md`, bloque B, defecto (a)). Fase ROJA.
//
// POR QUÉ ESTE TEST VIVE AQUÍ Y NO EN `tests/` (límite estructural, declarado)
//
// La historia lista como ficheros de prueba `tests/transactions.rs` y `tests/escritura.rs`, pero este
// criterio **no es alcanzable desde un test de integración**, por dos razones que se suman:
//
//   1. `mod io` es **privado** (`lib.rs:54`): ni `write_atomic` ni `delete` son visibles fuera del
//      crate, así que un test de `tests/` no puede llamarlos.
//   2. La única forma portable de hacer fallar el fsync de un directorio en Unix es que **no se pueda
//      abrir** (`sync_dir` hace `File::open(dir)`), y eso exige quitarle el bit de lectura — el
//      MISMO bit que necesita el descubrimiento para listar el árbol. Cualquier intento de inyectar
//      el fallo a través de `Workspace` (apply, revert, `write_document`) cambia antes el inventario
//      y la `WorkspaceRevision`, y el error que sale es otro. Hacer fallar `fsync(2)` sobre un
//      directorio que sí se abre no es reproducible sin un sistema de ficheros ad-hoc.
//
// El nivel unitario, en cambio, es exacto: `write_atomic`/`delete` son funciones sobre una raíz
// cualquiera, sin inventario ni cache de por medio, y un directorio `-wx` (0o300) permite
// crear/renombrar/desenlazar dentro pero **no** abrirlo — el fallo de durabilidad aislado y nada más.
//
// QUÉ FIJA (y por qué no es vacuo)
//
// Las dos mitades comprueban primero que la operación **sí hizo su trabajo** (el rename ocurrió / el
// unlink ocurrió) y solo después que devuelve `Err(WorkspaceError::Io)`. Sin esa precondición, un
// fallo temprano —permisos mal puestos, directorio inexistente— haría pasar el test por la razón
// equivocada.
//
// ROJO ESPERADO HOY
// - **(a) escritura**: `write_atomic` llama a `sync_dir` (`io.rs:26`) y `sync_dir` se traga el fallo
//   (`if let Ok(handle)` + `let _ = handle.sync_all()`): devuelve `Ok(())` afirmando una durabilidad
//   que no puede sostener.
// - **(b) borrado**: `delete` (`io.rs:72`) **ni siquiera fsynca** el directorio padre — es la mitad
//   que falta de la durabilidad que el contenido sí tiene, y el defecto (a) de la historia: un corte
//   de energía tras un journal `applied` puede dejar el unlink sin persistir, el documento
//   **reaparece** y el recibo afirma que se borró.
//
// El arreglo (alcance de la historia) es un único chokepoint: `sync_dir` pasa a devolver `Result` y
// sus llamadores propagan en vez de ignorar. Eso incluye el fallo al **abrir** el directorio: si no
// se pudo abrir, no se pudo fsyncar, y afirmar durabilidad ahí es exactamente el silencio que la
// historia cierra (criterio estructural: ni `delete` ni `sync_dir` conservan un `let _ =` sobre una
// operación de durabilidad).
// ===========================================================================

#[cfg(all(test, unix))]
mod durabilidad_del_directorio {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    /// Fija el modo de `dir` (`0o300` = `-wx`: se puede crear, renombrar y desenlazar dentro, pero
    /// **no** abrir el directorio, que es lo que necesita el fsync).
    fn modo(dir: &Path, m: u32) {
        std::fs::set_permissions(dir, std::fs::Permissions::from_mode(m))
            .expect("ajustar los permisos del directorio de la sonda");
    }

    /// `true` si en este entorno los bits de permiso de verdad deniegan abrir un directorio `-wx`.
    /// Corriendo como **root** no deniegan nada y el escenario no se puede inyectar: el test lo dice
    /// y se salta en vez de dar un verde falso.
    fn los_permisos_deniegan() -> bool {
        let sonda = tempfile::tempdir().expect("tempdir de la sonda");
        let dir = sonda.path().join("sub");
        std::fs::create_dir(&dir).expect("crear el directorio de la sonda");
        modo(&dir, 0o300);
        let denegado = std::fs::File::open(&dir).is_err();
        modo(&dir, 0o700);
        denegado
    }

    /// **Criterio 3** (`fallo_de_fsync_de_directorio_es_visible`) — **Dado** un directorio cuyo fsync
    /// falla, **Cuando** se escribe o se borra un `.md`, **Entonces** la operación devuelve
    /// `WorkspaceError::Io` en vez de seguir como si nada.
    #[test]
    fn fallo_de_fsync_de_directorio_es_visible() {
        if !los_permisos_deniegan() {
            eprintln!(
                "AVISO (E25-H05): este entorno no deniega por permisos (¿root?); \
                 `fallo_de_fsync_de_directorio_es_visible` no puede inyectar el fallo del fsync de \
                 directorio y se salta"
            );
            return;
        }
        let tmp = tempfile::tempdir().expect("workspace temporal");
        let root = tmp.path();
        let sub = root.join("sub");
        std::fs::create_dir(&sub).expect("crear sub/");
        std::fs::write(sub.join("a.md"), "viejo").expect("sembrar sub/a.md");
        let rel = RelPath::new("sub/a.md").expect("`sub/a.md` es una ruta relativa válida");

        // (a) ESCRITURA: el rename se hace, el fsync del directorio no.
        modo(&sub, 0o300);
        let escritura = write_atomic(root, &rel, "nuevo");
        let contenido = std::fs::read_to_string(sub.join("a.md")).expect("leer sub/a.md");
        modo(&sub, 0o700);

        assert_eq!(
            contenido, "nuevo",
            "precondición del escenario: con el directorio en `-wx` el temporal se crea y el rename \
             se hace; lo único que falla es el fsync de la entrada de directorio. Si el contenido no \
             cambió, el fallo se inyectó demasiado pronto y el test no mira lo que dice mirar"
        );
        let err = escritura.expect_err(
            "el fsync del directorio falló (no se pudo ni abrir), así que `write_atomic` NO puede \
             devolver `Ok`: estaría afirmando que el rename es durable sin poder saberlo. Es la \
             mitad que le falta a la durabilidad que el contenido sí tiene (`sync_all` antes del \
             rename) — E25-H05, defecto (a)",
        );
        assert!(
            matches!(err, WorkspaceError::Io(_)),
            "y el fallo se reporta como `WorkspaceError::Io`, no como otra cosa: {err:?}"
        );

        // (b) BORRADO: el unlink se hace, y hoy no hay fsync ninguno que pueda fallar.
        modo(&sub, 0o300);
        let borrado = delete(root, &rel);
        let sigue = sub.join("a.md").exists();
        modo(&sub, 0o700);

        assert!(
            !sigue,
            "precondición del escenario: con el directorio en `-wx` el unlink sí se hace; lo que no \
             se puede es persistir la entrada de directorio"
        );
        let err = borrado.expect_err(
            "`io::delete` tiene que fsyncar el directorio padre tras el unlink (E25-H05, alcance: \
             «reusando `io::sync_dir`») y hacer visible su fallo. Hoy hace `remove_file` y nada más: \
             un corte de energía tras un journal `applied` puede dejar el borrado sin persistir, el \
             documento REAPARECE al reabrir y el recibo afirma que se borró",
        );
        assert!(
            matches!(err, WorkspaceError::Io(_)),
            "y el fallo se reporta como `WorkspaceError::Io`: {err:?}"
        );
    }
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
