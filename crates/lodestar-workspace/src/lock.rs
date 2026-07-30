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
use std::path::{Path, PathBuf};

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

impl WorkspaceLock {
    /// Ruta del fichero de lock que este guard posee.
    ///
    /// Interno: existe para que quien **exige** un testigo del lock (`Workspace::gc_receipts_con_el_lock_tomado`,
    /// E25-H03) pueda comprobar que el testigo es el de *ese* workspace y no el de otro. No se expone
    /// en la API pública — el guard es una prueba de posesión, no un accessor de rutas.
    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
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
    /// Es uno de los cuatro chokepoints de escritura de E23-H12: tomar el lock es la puerta de
    /// `change_apply`/`change_revert`, así que aquí se ajusta el `.gitignore` gestionado
    /// ([`Workspace::ensure_managed_gitignore`]) — abrir el workspace ya no lo hace.
    ///
    /// # Errores
    /// - [`WorkspaceError::WriteConflict`] si el lock ya está tomado (el fichero ya existe).
    /// - [`WorkspaceError::Io`] si falla la creación del directorio runtime o la escritura del
    ///   fichero por otro motivo distinto de «ya existe».
    pub fn acquire_lock(&self) -> Result<WorkspaceLock, WorkspaceError> {
        let path = self.lock_path();

        // Se va a escribir de verdad: la cache y el runtime que nacerán de aquí no deben acabar
        // versionados (E23-H12; idempotente byte a byte si el bloque ya está).
        self.ensure_managed_gitignore();

        // Nadie garantiza ya el directorio de runtime al abrir (E23-H12 retiró el scaffold): cada
        // consumidor lo crea justo antes de escribir, y esto cubre además el caso de que lo borren
        // en caliente (checkout limpio, `rm -rf .lodestar/runtime`, …).
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
                // El lock existe. Antes de rendirse: ¿lo dejó un proceso que ya no está? (E23-H23)
                match reclamar_si_huerfano(&path) {
                    Reclamo::Reclamado => std::fs::OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(&path)
                        // Si en la ventana entre el borrado y esta reapertura otro proceso ganó la
                        // carrera, `AlreadyExists` otra vez: se rinde, no se reintenta en bucle.
                        .map_err(|_| {
                            WorkspaceError::WriteConflict(format!(
                                "el lock de publicación lo tomó otro proceso mientras se \
                                 reclamaba uno huérfano ({})",
                                path.display()
                            ))
                        })?,
                    Reclamo::Vivo(detalle) => {
                        return Err(WorkspaceError::WriteConflict(format!(
                            "el lock de publicación ya está tomado ({}){detalle}",
                            path.display()
                        )));
                    }
                }
            }
            Err(e) => return Err(WorkspaceError::from(e)),
        };

        // Metadatos de diagnóstico (no participan en la exclusión). Best-effort: si la escritura
        // del cuerpo falla, el lock ya está adquirido (el fichero existe) — no se aborta por ello.
        let _ = write!(&mut file, "{}", lock_metadata());

        Ok(WorkspaceLock { path })
    }
}

/// Cuánto puede vivir un lock antes de considerarse huérfano — E23-H23.
///
/// Es la **red portable** para cuando no se puede preguntar por el proceso (Windows, o un lock sin
/// `pid` legible). Deliberadamente generoso: la transacción más larga medida en el arnés de escala
/// (~10.000 documentos, E14-H05) ronda los 8 segundos, así que 15 minutos son tres órdenes de
/// magnitud de margen. Reclamar el lock de un escritor **vivo pero lento** rompería el invariante de
/// escritor único, que es mucho peor que esperar.
const LOCK_TTL: std::time::Duration = std::time::Duration::from_secs(15 * 60);

/// Resultado de examinar un lock ya existente.
enum Reclamo {
    /// Era huérfano y se ha borrado: se puede reintentar la creación.
    Reclamado,
    /// Su dueño sigue vivo (o no se pudo determinar que no lo esté). Lleva el detalle legible.
    Vivo(String),
}

/// Decide si un lock existente es **huérfano** y, si lo es, lo borra — E23-H23.
///
/// Hasta E23-H23 esto no existía: `acquire_lock` solo miraba si el fichero existía, y el `pid` que
/// escribe [`lock_metadata`] **nadie lo leía de vuelta**. Un `lodestar-mcp` muerto por `SIGKILL` o
/// por el OOM killer dejaba el fichero en disco y el workspace quedaba **cerrado a la escritura
/// para siempre**, hasta que un humano lo borrara a mano — y por la frontera MCP el agente solo veía
/// un `WRITE_CONFLICT` pelado, sin pista de qué mirar.
///
/// Dos criterios, en orden:
/// 1. **El dueño ya no existe** (solo Unix): `kill(pid, 0)` responde `ESRCH`. Es inmediato y exacto.
/// 2. **El lock es más viejo que [`LOCK_TTL`]**: red portable para Windows y para un lock cuyo
///    cuerpo no se pueda leer.
///
/// Ante la duda **no se reclama**: un fichero ilegible, un `pid` ausente o un reloj que va hacia
/// atrás dejan el lock intacto. Perder disponibilidad es recuperable; romper el escritor único, no.
fn reclamar_si_huerfano(path: &Path) -> Reclamo {
    let cuerpo = match std::fs::read_to_string(path) {
        Ok(c) => c,
        // Ilegible: no hay información con la que decidir, así que se respeta.
        Err(e) => return Reclamo::Vivo(format!("; no se pudo leer el lock ({e})")),
    };
    let meta: serde_json::Value = serde_json::from_str(&cuerpo).unwrap_or(serde_json::Value::Null);
    let pid = meta.get("pid").and_then(serde_json::Value::as_u64);
    let owner = meta
        .get("owner")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("desconocido");
    let ts = meta.get("timestamp").and_then(serde_json::Value::as_u64);

    let edad = ts.and_then(|t| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|ahora| ahora.as_secs().saturating_sub(t))
    });

    let dueño_muerto = pid.is_some_and(proceso_muerto);
    let caducado = edad.is_some_and(|s| s > LOCK_TTL.as_secs());

    if dueño_muerto || caducado {
        // Best-effort: si el borrado falla (permisos, carrera con otro reclamador), se trata como
        // no reclamado y el llamador se rinde con un conflicto normal.
        if std::fs::remove_file(path).is_ok() {
            return Reclamo::Reclamado;
        }
    }

    let detalle = match (pid, edad) {
        (Some(p), Some(s)) => format!("; lo tiene el pid {p} de «{owner}» desde hace {s}s"),
        (Some(p), None) => format!("; lo tiene el pid {p} de «{owner}»"),
        _ => String::new(),
    };
    Reclamo::Vivo(detalle)
}

/// `true` si se puede afirmar que el proceso `pid` **ya no existe**.
///
/// En Unix, `kill(pid, 0)` con `ESRCH`. Un `EPERM` (existe pero es de otro usuario) cuenta como
/// vivo, que es la respuesta conservadora. Fuera de Unix devuelve siempre `false`: no se afirma
/// nada, y el criterio de reclamo queda en manos del [`LOCK_TTL`].
#[cfg(unix)]
fn proceso_muerto(pid: u64) -> bool {
    // Un pid que no cabe en `pid_t` no es un proceso de este sistema; no se afirma nada.
    let Ok(pid) = i32::try_from(pid) else {
        return false;
    };
    if pid <= 0 {
        return false;
    }
    // SAFETY: `kill` con señal 0 no envía nada — solo comprueba permisos y existencia del proceso.
    // No toca memoria del proceso llamante ni tiene efectos observables.
    let rc = unsafe { libc::kill(pid, 0) };
    rc == -1 && std::io::Error::last_os_error().raw_os_error() == Some(libc::ESRCH)
}

#[cfg(not(unix))]
fn proceso_muerto(_pid: u64) -> bool {
    false
}

/// Cuerpo JSON del fichero de lock: `owner`, `pid` y `timestamp` (epoch en segundos).
///
/// La exclusión mutua la garantiza la **existencia atómica** del fichero, no su contenido. Pero
/// desde E23-H23 este cuerpo **sí se lee de vuelta** ([`reclamar_si_huerfano`]): es lo que permite
/// distinguir un lock vivo de uno que dejó un proceso muerto. Se compone a mano —sin serializador—
/// porque escribirlo no puede fallar de formas interesantes; leerlo sí usa `serde_json`, que tolera
/// un cuerpo corrupto sin paniquear.
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
