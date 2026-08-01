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
//!
//! **E25-H06 — el dueño es demostrable.** Hasta v0.3.1 el guard borraba el fichero **por ruta**, sin
//! comprobar que siguiera siendo el suyo: si otro proceso lo había reclamado por huérfano y lo había
//! recreado, el `Drop` del dueño original liberaba el lock del NUEVO dueño (y a partir de ahí
//! encadenaba). Ahora el cuerpo lleva un **token de propiedad** único por adquisición y el `Drop`
//! solo borra si el token del fichero es el suyo. Con él viaja la **identidad de máquina** (`host`),
//! que es lo que permite que la prueba de vida por pid solo se consulte cuando el pid es de ESTA
//! máquina — ver [`reclamar_si_huerfano`].

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
    /// Token de propiedad de **esta** adquisición (E25-H06). Es el mismo que quedó escrito en el
    /// cuerpo del fichero: el `Drop` solo borra si el que hay en disco sigue siendo éste.
    token: String,
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
        // E25-H06: se borra **por propiedad**, no por ruta. Si el fichero que hay ahí ya no lleva
        // mi token, el lock lo reclamó y lo recreó otro proceso: no es mío y no se toca. Borrarlo
        // liberaría el lock de un escritor vivo —y encadenaría, porque el siguiente `Drop` haría lo
        // mismo con el suyo—, que es exactamente el defecto (a) de E25-H06.
        if !es_mi_lock(&self.path, &self.token) {
            return;
        }
        // Best-effort: la liberación no debe paniquear (podría hacerlo durante el desenrollado de
        // otro panic → doble panic = abort). Si el borrado falla, un lock huérfano es recuperable
        // (E13-H06); un abort no lo es.
        let _ = std::fs::remove_file(&self.path);
    }
}

/// `true` si el fichero de lock de `path` sigue declarando `token` como dueño — E25-H06.
///
/// Conservador por diseño: un fichero que no existe, que no es JSON legible o que no declara token
/// **no** se afirma como propio. La alternativa (borrar ante la duda) es la que rompe el escritor
/// único, y perder un lock hasta que caduque su TTL es recuperable; publicar dos veces sobre el
/// mismo workspace, no.
///
/// La comprobación es inherentemente **best-effort**: entre leer el token y borrar el fichero hay
/// una ventana en la que un tercero podría reclamarlo. Cerrarla exigiría un borrado condicional
/// atómico que el sistema de ficheros no ofrece de forma portable; lo que sí se elimina es el caso
/// **estructural** —el dueño anterior liberando el lock del actual— que es el que encadenaba.
fn es_mi_lock(path: &Path, token: &str) -> bool {
    let Ok(cuerpo) = std::fs::read_to_string(path) else {
        return false;
    };
    let meta: serde_json::Value = serde_json::from_str(&cuerpo).unwrap_or(serde_json::Value::Null);
    meta.get("token").and_then(serde_json::Value::as_str) == Some(token)
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
    /// tienen ambos éxito. El fichero registra `owner`/`pid`/`host`/`timestamp`/`token`; su
    /// contenido no participa en la exclusión (esa la da la existencia del fichero, no su cuerpo),
    /// pero desde E25-H06 **sí** decide quién puede liberarlo y quién puede reclamarlo.
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
    ///   fichero por otro motivo distinto de «ya existe» — incluida la del **cuerpo** (E25-H06: sin
    ///   el token escrito el guard no podría demostrar la propiedad ni liberar su lock, así que la
    ///   adquisición se deshace en vez de publicar un lock que nadie podría soltar).
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

        // El cuerpo ya NO es solo diagnóstico (E25-H06): lleva el token que prueba la propiedad y el
        // `host` que decide si el pid es interpretable aquí. Si no se puede escribir, este guard no
        // podría liberar su propio lock (`es_mi_lock` no encontraría token) y el workspace quedaría
        // cerrado a la escritura hasta que venciera el TTL. Ante ese fallo se deshace la adquisición
        // —el fichero recién creado es indudablemente nuestro— y se propaga el error.
        let token = token_de_propiedad();
        if let Err(e) = escribir_cuerpo(&mut file, &token) {
            drop(file);
            let _ = std::fs::remove_file(&path);
            return Err(WorkspaceError::Io(format!(
                "no se pudo escribir el cuerpo del lock de publicación ({}): {e}",
                path.display()
            )));
        }

        Ok(WorkspaceLock { path, token })
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
/// escribe [`escribir_cuerpo`] **nadie lo leía de vuelta**. Un `lodestar-mcp` muerto por `SIGKILL` o
/// por el OOM killer dejaba el fichero en disco y el workspace quedaba **cerrado a la escritura
/// para siempre**, hasta que un humano lo borrara a mano — y por la frontera MCP el agente solo veía
/// un `WRITE_CONFLICT` pelado, sin pista de qué mirar.
///
/// **E25-H06 — la prueba de vida manda sobre el TTL, y solo vale en la máquina que la puede dar.**
/// Hasta v0.3.1 el criterio era `dueño_muerto || caducado`: el TTL podía reclamar el lock de un
/// dueño **vivo pero suspendido** (portátil dormido, proceso parado en un breakpoint, reloj movido),
/// y `proceso_muerto` preguntaba por el pid en la tabla de procesos **local** aunque el lock lo
/// hubiera escrito otra máquina (workspace en red, namespaces de PID distintos). Ahora:
///
/// 1. Si el lock declara **este** `host` y de su `pid` se puede afirmar algo, ese algo **decide**:
///    dueño muerto → se reclama; dueño vivo → NO se reclama, aunque el TTL haya vencido.
/// 2. En cualquier otro caso —otro host, host ausente, pid ilegible, o una plataforma que no sabe
///    responder (Windows)— el **único** criterio admisible es el [`LOCK_TTL`]: es la red portable
///    para cuando no se puede afirmar nada, tal y como su propio rustdoc declara.
///
/// **Semántica del `host` ausente**: un cuerpo sin `host` es «máquina desconocida», no «esta
/// máquina». Solo lo escriben locks de versiones anteriores a E25-H06 (o cuerpos fabricados), y
/// atribuir su pid a la tabla de procesos local es justo el error que este criterio cierra. El coste
/// es acotado y recuperable —un lock viejo de un proceso muerto tarda el TTL en reclamarse en vez de
/// reclamarse al instante—; el error contrario (reclamar el lock de un escritor vivo de otra
/// máquina) rompe el escritor único, que no lo es.
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
    let host = meta.get("host").and_then(serde_json::Value::as_str);
    let ts = meta.get("timestamp").and_then(serde_json::Value::as_u64);

    let edad = ts.and_then(|t| {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .ok()
            .map(|ahora| ahora.as_secs().saturating_sub(t))
    });
    let caducado = edad.is_some_and(|s| s > LOCK_TTL.as_secs());

    // El pid solo es interpretable si lo escribió ESTA máquina: en otra, ese número o no existe o
    // designa un proceso distinto.
    let vida = if host.is_some_and(es_host_local) {
        pid.map_or(Vida::Desconocida, vida_del_dueño)
    } else {
        Vida::Desconocida
    };

    let reclamable = match vida {
        Vida::Muerta => true,          // certeza: el dueño se fue y dejó el fichero.
        Vida::Viva => false,           // certeza: hay alguien detrás — el TTL NO manda sobre esto.
        Vida::Desconocida => caducado, // red portable: lo único que queda es la antigüedad.
    };

    if reclamable {
        // Best-effort: si el borrado falla (permisos, carrera con otro reclamador), se trata como
        // no reclamado y el llamador se rinde con un conflicto normal.
        if std::fs::remove_file(path).is_ok() {
            return Reclamo::Reclamado;
        }
    }

    let maquina = match host {
        Some(h) if !es_host_local(h) => format!(" en «{h}»"),
        _ => String::new(),
    };
    let detalle = match (pid, edad) {
        (Some(p), Some(s)) => {
            format!("; lo tiene el pid {p}{maquina} de «{owner}» desde hace {s}s")
        }
        (Some(p), None) => format!("; lo tiene el pid {p}{maquina} de «{owner}»"),
        _ => String::new(),
    };
    Reclamo::Vivo(detalle)
}

/// Qué se puede **afirmar** sobre el proceso que dejó un lock — E25-H06.
///
/// Tres estados y no dos: «no consta que esté muerto» y «consta que está vivo» llevan a decisiones
/// distintas. Con `Viva` el TTL deja de mandar; con `Desconocida` el TTL es lo único que hay.
// Fuera de Unix solo se produce `Desconocida` (no hay prueba de vida portable), así que `Muerta` y
// `Viva` son «nunca construidas» ahí; existen para el criterio completo y `reclamar_si_huerfano` las
// sigue consumiendo en TODAS las plataformas. El `allow` es condicional a propósito: en Unix el
// dead_code real tiene que seguir vigilado.
#[cfg_attr(not(unix), allow(dead_code))]
#[derive(Clone, Copy)]
enum Vida {
    /// Consta que el proceso ya no existe.
    Muerta,
    /// Consta que el proceso existe (aunque sea de otro usuario).
    Viva,
    /// No se puede afirmar nada: plataforma sin prueba de vida, o pid no interpretable.
    Desconocida,
}

/// Qué se puede afirmar del proceso `pid` **en esta máquina**.
///
/// En Unix, `kill(pid, 0)`: `ESRCH` es [`Vida::Muerta`]; éxito o `EPERM` (existe pero es de otro
/// usuario) es [`Vida::Viva`]. Fuera de Unix siempre [`Vida::Desconocida`]: no hay prueba de vida
/// portable, y el criterio de reclamo queda entero en manos del [`LOCK_TTL`].
///
/// El llamante debe haber comprobado antes que el pid es de esta máquina ([`es_host_local`]).
#[cfg(unix)]
fn vida_del_dueño(pid: u64) -> Vida {
    // Un pid que no cabe en `pid_t` no es un proceso de este sistema; no se afirma nada.
    let Ok(pid) = i32::try_from(pid) else {
        return Vida::Desconocida;
    };
    if pid <= 0 {
        return Vida::Desconocida;
    }
    // SAFETY: `kill` con señal 0 no envía nada — solo comprueba permisos y existencia del proceso.
    // No toca memoria del proceso llamante ni tiene efectos observables.
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return Vida::Viva;
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(e) if e == libc::ESRCH => Vida::Muerta,
        Some(e) if e == libc::EPERM => Vida::Viva,
        _ => Vida::Desconocida,
    }
}

#[cfg(not(unix))]
fn vida_del_dueño(_pid: u64) -> Vida {
    Vida::Desconocida
}

/// `true` si `host` es el nombre de **esta** máquina — E25-H06.
///
/// Si la identidad local no se puede determinar ([`host_local`] devuelve `None`), no se afirma la
/// coincidencia: «desconocido» nunca es «el mío». Es la respuesta conservadora, y la que deja el
/// reclamo en manos del TTL.
fn es_host_local(host: &str) -> bool {
    host_local().is_some_and(|local| local == host)
}

/// Identidad de la máquina que escribe (o examina) un lock — E25-H06.
///
/// Es el **nombre de host** del sistema, cacheado por proceso: `gethostname(2)` en Unix (`libc` ya
/// era dependencia para `kill`, así que no entra código externo nuevo) y las variables de entorno
/// `COMPUTERNAME`/`HOSTNAME` fuera de Unix, donde la prueba de vida no existe y el `host` es solo
/// diagnóstico.
///
/// **No se le añade un identificador de arranque**, aunque la historia lo contemplaba «donde sea
/// barato»: en el único sitio donde lo es (Linux, `/proc/sys/kernel/random/boot_id`) **empeoraría**
/// el resultado. Un lock que sobrevive a un reinicio es el caso de huérfano más común, y hoy se
/// reclama al instante porque su pid ya no existe; con boot-id pasaría a ser «otra máquina» y habría
/// que esperar el TTL. Lo único que el boot-id evitaría —que un pid reutilizado tras el reinicio
/// pase por vivo— ya se resuelve de forma segura: se respeta el lock y lo reclama el TTL.
///
/// `None` si no se puede determinar (nombre vacío o llamada fallida); ver [`es_host_local`].
fn host_local() -> Option<String> {
    static HOST: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
    HOST.get_or_init(leer_host).clone()
}

#[cfg(unix)]
fn leer_host() -> Option<String> {
    // `HOST_NAME_MAX` ronda los 64 bytes en Linux y los 255 en macOS; 256 cubre ambos con margen.
    let mut buf = vec![0u8; 256];
    // SAFETY: `gethostname` escribe como mucho `buf.len()` bytes en el puntero que se le pasa, que
    // apunta a un buffer nuestro de exactamente ese tamaño y vivo durante toda la llamada.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast::<libc::c_char>(), buf.len()) };
    if rc != 0 {
        return None;
    }
    // POSIX no garantiza el NUL final si el nombre se truncó: se corta por el primer NUL si lo hay.
    let fin = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    buf.truncate(fin);
    let nombre = String::from_utf8_lossy(&buf).trim().to_string();
    (!nombre.is_empty()).then_some(nombre)
}

#[cfg(not(unix))]
fn leer_host() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .ok()
        .map(|h| h.trim().to_string())
        .filter(|h| !h.is_empty())
}

/// Token único de una adquisición del lock — E25-H06. Es la prueba de propiedad que el `Drop`
/// compara contra el cuerpo del fichero antes de borrarlo.
///
/// Solo tiene que ser **irrepetible**, no criptográfico: combina un valor aleatorio del proceso
/// (`RandomState` se siembra del sistema operativo, sin dependencias nuevas), el reloj en
/// nanosegundos y un contador propio, de modo que dos adquisiciones no coinciden ni dentro del mismo
/// proceso ni entre procesos que reutilicen un pid. **No contiene el pid**: los arneses que fabrican
/// escenarios sustituyendo el pid en el cuerpo no deben poder alterar el token de rebote.
fn token_de_propiedad() -> String {
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);

    let aleatorio = std::collections::hash_map::RandomState::new()
        .build_hasher()
        .finish();
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos());
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    format!("{aleatorio:016x}-{nanos:x}-{seq:x}")
}

/// Escribe el cuerpo JSON del lock: `owner`, `pid`, `host`, `timestamp` (epoch en segundos) y
/// `token` de propiedad.
///
/// La exclusión mutua la garantiza la **existencia atómica** del fichero, no su contenido. Pero este
/// cuerpo **sí se lee de vuelta**: desde E23-H23 para distinguir un lock vivo de uno que dejó un
/// proceso muerto ([`reclamar_si_huerfano`]) y desde E25-H06 para saber **quién** puede liberarlo
/// ([`es_mi_lock`]) y en qué máquina es interpretable su pid.
///
/// Se serializa con `serde_json` —y no a mano, como hasta v0.3.1— porque ahora hay dos campos de
/// texto de procedencia externa (`owner` del entorno, `host` del sistema) y escaparlos a mano es un
/// footgun sin ninguna contrapartida: la dependencia ya estaba ahí para leerlo.
fn escribir_cuerpo(file: &mut std::fs::File, token: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let owner = std::env::var("USER")
        .or_else(|_| std::env::var("USERNAME"))
        .unwrap_or_else(|_| "desconocido".to_string());
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let cuerpo = serde_json::json!({
        "owner": owner,
        "pid": std::process::id(),
        "host": host_local().unwrap_or_else(|| "desconocido".to_string()),
        "timestamp": ts,
        "token": token,
    });
    writeln!(file, "{cuerpo}")
}
