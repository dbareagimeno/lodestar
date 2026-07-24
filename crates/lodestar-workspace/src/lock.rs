//! Lock de publicación del workspace (E13-H02, `ARCHITECTURE.md §19.5`, `REFACTOR §5.2`):
//! garantiza **un solo publicador a la vez** sobre un mismo workspace. Es control de concurrencia
//! runtime, no estado canónico: el fichero de lock vive bajo `.lodestar/runtime/` (excluido del
//! índice de conocimiento y del `WorkspaceRevision`), así que no viola el invariante #1 («los
//! `.md` en disco son la única fuente de verdad»).
//!
//! Modelo **fail-fast** (no bloqueante): adquirir un lock ya tomado devuelve `Err` de inmediato
//! en vez de esperar. La exclusión mutua se apoya en la creación **atómica y exclusiva** de
//! fichero del sistema de ficheros (`O_CREAT | O_EXCL`): dos `acquire_lock` concurrentes sobre el
//! mismo root nunca obtienen ambos el lock. La liberación es **RAII**: el guard
//! [`WorkspaceLock`] borra el fichero en su `Drop`, de modo que el lock se suelta SIEMPRE —
//! incluido durante el desenrollado de pila de un `panic`.

use std::io::Write;
use std::path::PathBuf;

use crate::error::WorkspaceError;
use crate::Workspace;

/// Nombre del fichero de lock bajo `.lodestar/runtime/`.
const LOCK_FILE: &str = "lock.json";

/// Guard RAII del lock de publicación (E13-H02). Mientras vive, el fichero de lock existe en disco
/// y ningún otro publicador puede adquirirlo. Su [`Drop`] borra el fichero, liberando el lock
/// **siempre** — al salir de alcance normalmente o al desenrollar la pila por un `panic`.
///
/// No es clonable ni copiable a propósito: representa la posesión única del lock. Se obtiene con
/// [`Workspace::acquire_lock`].
#[must_use = "el lock se libera al dropear el guard; descartarlo de inmediato lo suelta al instante"]
pub struct WorkspaceLock {
    /// Ruta del fichero de lock que este guard posee y borrará al dropearse.
    path: PathBuf,
}

impl Drop for WorkspaceLock {
    fn drop(&mut self) {
        // Best-effort: la liberación no debe paniquear (podría hacerlo durante el desenrollado de
        // otro panic → doble panic = abort). Si el borrado falla, un lock huérfano es recuperable
        // (E13-H06); un abort no lo es.
        let _ = std::fs::remove_file(&self.path);
    }
}

impl Workspace {
    /// Ruta del fichero de lock de publicación (bajo `.lodestar/runtime/`), exista o no (E13-H02).
    ///
    /// Determinista: no toca el disco ni depende de si el lock está tomado. La usan las fachadas
    /// (y los tests) para inspeccionar el estado del lock.
    pub fn lock_path(&self) -> PathBuf {
        self.root.join(".lodestar").join("runtime").join(LOCK_FILE)
    }

    /// Adquiere el lock exclusivo de publicación (E13-H02). **Fail-fast**: si el lock ya está
    /// tomado (por este u otro handle sobre el mismo root) devuelve `Err` de inmediato, sin
    /// bloquear — no hay dos escritores.
    ///
    /// La exclusión se apoya en `OpenOptions::create_new` (`O_CREAT | O_EXCL`): la creación del
    /// fichero es atómica a nivel de sistema de ficheros, así que dos intentos concurrentes nunca
    /// tienen ambos éxito. El fichero registra `owner`/`pid`/`timestamp` para diagnóstico; su
    /// contenido no participa en la exclusión (esa la da la existencia del fichero, no su cuerpo).
    ///
    /// Crea `.lodestar/runtime/` si falta. El [`WorkspaceLock`] devuelto libera el lock al
    /// dropearse (RAII), incluso en un `panic`.
    ///
    /// # Errores
    /// - [`WorkspaceError::WriteConflict`] si el lock ya está tomado (el fichero ya existe).
    /// - [`WorkspaceError::Io`] si falla la creación del directorio runtime o la escritura del
    ///   fichero por otro motivo distinto de «ya existe».
    pub fn acquire_lock(&self) -> Result<WorkspaceLock, WorkspaceError> {
        let path = self.lock_path();

        // El scaffold runtime se crea al abrir el workspace, pero garantízalo por si el directorio se
        // borró en caliente (checkout limpio, `rm -rf .lodestar/runtime`, …).
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // `create_new` = O_CREAT | O_EXCL: falla si el fichero ya existe. Es el punto de exclusión
        // mutua atómica; el `AlreadyExists` se traduce a un conflicto de escritura (lock tomado).
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(f) => f,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(WorkspaceError::WriteConflict(format!(
                    "el lock de publicación ya está tomado ({})",
                    path.display()
                )));
            }
            Err(e) => return Err(WorkspaceError::from(e)),
        };

        // Metadatos de diagnóstico (no participan en la exclusión). Best-effort: si la escritura
        // del cuerpo falla, el lock ya está adquirido (el fichero existe) — no se aborta por ello.
        let _ = write!(&mut file, "{}", lock_metadata());

        Ok(WorkspaceLock { path })
    }
}

/// Cuerpo JSON de diagnóstico del fichero de lock: `owner`, `pid` y `timestamp` (epoch en
/// segundos). Es informativo — la exclusión la garantiza la existencia atómica del fichero, no su
/// contenido — así que se compone a mano (sin dependencia extra) y no se parsea de vuelta.
fn lock_metadata() -> String {
    let owner = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "desconocido".to_string());
    let pid = std::process::id();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let owner = owner.replace('\\', "\\\\").replace('"', "\\\"");
    format!("{{\"owner\":\"{owner}\",\"pid\":{pid},\"timestamp\":{ts}}}\n")
}
