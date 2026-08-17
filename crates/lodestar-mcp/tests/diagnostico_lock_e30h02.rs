//! **E30-H02 · fase ROJA (diagnóstico)** — sonda de la flakiness de
//! `crash_por_senal_no_deja_parciales` (`crates/lodestar-mcp/tests/crash_senal.rs`).
//!
//! Este fichero **no** es un criterio de aceptación de la historia: es la instrumentación que la
//! spec exige *antes* de tocar nada («Diagnóstico primero, arreglo después»). Replica el escenario
//! del test flaky —`SIGKILL` a mitad de un `change_apply`, reapertura sobre el mismo directorio,
//! `change_apply` inmediato— pero, en vez de asertar el invariante nuclear, **captura la evidencia**
//! de qué ve el segundo proceso cuando el primer `change_apply` responde `WRITE_CONFLICT`:
//!
//! - el `pid` que el fichero de lock declara y el `pid` del proceso que el arnés mató,
//! - el `host` del lock frente al host local (rama `es_host_local` de `reclamar_si_huerfano`),
//! - qué responde `libc::kill(pid, 0)` en ese instante y en varios reintentos posteriores
//!   (la rama exacta de `vida_del_dueño`: `Viva` / `Muerta` / `Desconocida`),
//! - la edad del lock frente al `LOCK_TTL` (15 min: nunca vencido en un test).
//!
//! Está `#[ignore]` a propósito: es una sonda de varios minutos con presión artificial, no parte de
//! la suite normal. Se ejecuta a mano:
//!
//! ```text
//! cargo test -p lodestar-mcp --test diagnostico_lock_e30h02 -- --ignored --nocapture
//! ```
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Igual que el `NOTAS` del test flaky: cuantas más notas, más ancha la ventana de publicación.
const NOTAS: usize = 60;

/// Repeticiones de la sonda. El fallo reportado ronda el 50 %, así que con esto sobra para verlo si
/// reproduce; si no reproduce ni una vez, eso también es evidencia.
const REPETICIONES: usize = 40;

struct Sesion {
    proc: Child,
    stdin: Option<ChildStdin>,
    stdout: BufReader<ChildStdout>,
    id: u64,
}

impl Sesion {
    fn abrir(root: &Path) -> Self {
        let mut proc = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
            .arg("--root")
            .arg(root)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("arrancar lodestar-mcp");
        let stdin = proc.stdin.take().expect("stdin");
        let stdout = BufReader::new(proc.stdout.take().expect("stdout"));
        let mut sesion = Sesion {
            proc,
            stdin: Some(stdin),
            stdout,
            id: 0,
        };
        sesion.envia(
            "initialize",
            serde_json::json!({
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": {"name": "lodestar-diagnostico-lock", "version": "1"}
            }),
        );
        let respuesta = sesion.lee();
        assert_eq!(respuesta["result"]["serverInfo"]["name"], "lodestar-mcp");
        sesion.notificacion("notifications/initialized");
        sesion
    }

    fn pid(&self) -> u32 {
        self.proc.id()
    }

    fn envia(&mut self, metodo: &str, params: serde_json::Value) {
        self.id += 1;
        let msg = serde_json::json!({
            "jsonrpc": "2.0", "id": self.id, "method": metodo, "params": params
        });
        let stdin = self.stdin.as_mut().expect("stdin vivo");
        writeln!(stdin, "{msg}").expect("escribir la petición");
        stdin.flush().expect("flush");
    }

    fn lee(&mut self) -> serde_json::Value {
        let mut linea = String::new();
        self.stdout.read_line(&mut linea).expect("leer respuesta");
        serde_json::from_str(&linea).expect("stdout = JSON-RPC puro")
    }

    fn notificacion(&mut self, metodo: &str) {
        let stdin = self.stdin.as_mut().expect("stdin vivo");
        writeln!(
            stdin,
            "{}",
            serde_json::json!({"jsonrpc":"2.0","method":metodo})
        )
        .expect("escribir notificación");
        stdin.flush().expect("flush notificación");
    }

    fn tool_cruda(&mut self, nombre: &str, args: serde_json::Value) -> serde_json::Value {
        self.envia(
            "tools/call",
            serde_json::json!({"name": nombre, "arguments": args}),
        );
        self.lee()["result"].clone()
    }

    /// Mata con `SIGKILL` y **cosecha** (`wait`), igual que `Sesion::matar` del test flaky.
    fn matar(mut self) {
        // SAFETY: `libc::kill` sobre el pid de un hijo propio.
        unsafe {
            libc::kill(self.proc.id() as i32, libc::SIGKILL);
        }
        let _ = self.proc.wait();
        self.stdin.take();
    }

    /// Mata con `SIGKILL` **sin** cosechar: el hijo queda ZOMBI. Es la variante que la sonda usa
    /// para comprobar si el zombi es lo que hace pasar al lock por vivo.
    fn matar_sin_cosechar(&mut self) {
        // SAFETY: `libc::kill` sobre el pid de un hijo propio.
        unsafe {
            libc::kill(self.proc.id() as i32, libc::SIGKILL);
        }
        self.stdin.take();
    }

    fn cerrar(mut self) {
        self.stdin.take();
        let _ = self.proc.wait();
    }
}

fn escribe(root: &Path, rel: &str, contenido: &str) {
    let p = root.join(rel);
    std::fs::create_dir_all(p.parent().unwrap()).unwrap();
    std::fs::write(p, contenido).unwrap();
}

fn construye(root: &Path) {
    escribe(
        root,
        "hub.md",
        "---\nstatus: draft\n---\n\n# Hub\n\nel centro\n",
    );
    for i in 0..NOTAS {
        escribe(
            root,
            &format!("notas/n{i:03}.md"),
            &format!(
                "---\nidx: {i}\n---\n\n# Nota {i}\n\nVer el [hub](../hub.md) y relleno {}\n",
                "x".repeat(400)
            ),
        );
    }
}

/// Lo que el fichero de lock declara, si existe.
#[derive(Debug, Default)]
struct Lock {
    existe: bool,
    pid: Option<u64>,
    host: Option<String>,
    owner: Option<String>,
    edad_s: Option<u64>,
    crudo: String,
}

fn lee_lock(root: &Path) -> Lock {
    let p = root.join(".lodestar/runtime/lock.json");
    let Ok(crudo) = std::fs::read_to_string(&p) else {
        return Lock::default();
    };
    let v: serde_json::Value = serde_json::from_str(&crudo).unwrap_or(serde_json::Value::Null);
    let ts = v.get("timestamp").and_then(serde_json::Value::as_u64);
    let ahora = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    Lock {
        existe: true,
        pid: v.get("pid").and_then(serde_json::Value::as_u64),
        host: v
            .get("host")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        owner: v
            .get("owner")
            .and_then(serde_json::Value::as_str)
            .map(str::to_string),
        edad_s: ts.map(|t| ahora.saturating_sub(t)),
        crudo: crudo.trim().to_string(),
    }
}

/// La misma prueba de vida que `lodestar_workspace::lock::vida_del_dueño`, replicada aquí para
/// observarla desde fuera sin tocar producción.
fn vida(pid: u64) -> &'static str {
    let Ok(pid) = i32::try_from(pid) else {
        return "Desconocida(pid no cabe)";
    };
    if pid <= 0 {
        return "Desconocida(pid<=0)";
    }
    // SAFETY: señal 0 = solo comprobación de existencia/permisos.
    let rc = unsafe { libc::kill(pid, 0) };
    if rc == 0 {
        return "Viva(rc=0)";
    }
    match std::io::Error::last_os_error().raw_os_error() {
        Some(e) if e == libc::ESRCH => "Muerta(ESRCH)",
        Some(e) if e == libc::EPERM => "Viva(EPERM)",
        _ => "Desconocida(errno raro)",
    }
}

fn host_local() -> String {
    let mut buf = vec![0u8; 256];
    // SAFETY: `gethostname` escribe como mucho `buf.len()` bytes en nuestro buffer.
    let rc = unsafe { libc::gethostname(buf.as_mut_ptr().cast::<libc::c_char>(), buf.len()) };
    if rc != 0 {
        return String::new();
    }
    let fin = buf.iter().position(|b| *b == 0).unwrap_or(buf.len());
    buf.truncate(fin);
    String::from_utf8_lossy(&buf).trim().to_string()
}

fn plan_y_apply_sin_leer(s: &mut Sesion) {
    s.envia(
        "tools/call",
        serde_json::json!({"name": "change_plan", "arguments": {
            "operations": [{"op": "move", "from": "hub.md", "to": "notas/hub.md",
                            "rewriteInboundLinks": true}],
            "policy": {"requireValidResult": true, "allowWarnings": true}
        }}),
    );
    let r = s.lee();
    let cs = r["result"]["structuredContent"]["changeSetId"]
        .as_str()
        .expect("changeSetId")
        .to_string();
    s.envia(
        "tools/call",
        serde_json::json!({"name": "change_apply", "arguments": {"changeSetId": cs}}),
    );
}

/// Sonda A — el escenario del test flaky, **cosechando** el proceso muerto (igual que
/// `Sesion::matar`). Registra, en cada repetición, si el primer `change_apply` del segundo proceso
/// falla y con qué evidencia de lock.
#[test]
#[ignore = "sonda de diagnóstico de E30-H02: minutos de ejecución, se lanza a mano"]
fn sonda_e30h02_reclamo_tras_sigkill_cosechado() {
    let retrasos_ms = [40, 70, 100, 130, 170];
    let mut fallos = 0usize;
    for i in 0..REPETICIONES {
        let ms = retrasos_ms[i % retrasos_ms.len()];
        let dir = tempfile::tempdir().unwrap();
        construye(dir.path());

        let mut s = Sesion::abrir(dir.path());
        let pid_muerto = s.pid();
        plan_y_apply_sin_leer(&mut s);
        std::thread::sleep(std::time::Duration::from_millis(ms));
        s.matar();

        let lock = lee_lock(dir.path());

        let mut s2 = Sesion::abrir(dir.path());
        let plan2 = s2.tool_cruda(
            "change_plan",
            serde_json::json!({
                "operations": [{"op": "create", "path": "testigo.md", "body": "# Testigo\n"}],
                "policy": {"requireValidResult": false, "allowWarnings": true}
            }),
        );
        let cs2 = plan2["structuredContent"]["changeSetId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let r = s2.tool_cruda(
            "change_apply",
            serde_json::json!({"changeSetId": cs2.clone()}),
        );
        let error = r["isError"].as_bool().unwrap_or(false) || cs2.is_empty();
        if error {
            fallos += 1;
            eprintln!(
                "E30-H02 · FALLO en repetición {i} (retraso {ms} ms)\n  \
                 pid matado (y cosechado) = {pid_muerto}\n  \
                 lock: existe={} pid={:?} host={:?} owner={:?} edad={:?}s\n  \
                 host local = «{}»  ·  ¿pid del lock == pid matado? {}\n  \
                 vida(pid del lock) AHORA = {}\n  \
                 crudo = {}\n  \
                 respuesta = {r}",
                lock.existe,
                lock.pid,
                lock.host,
                lock.owner,
                lock.edad_s,
                host_local(),
                lock.pid == Some(u64::from(pid_muerto)),
                lock.pid.map_or("(sin pid)", vida),
                lock.crudo,
            );
        }
        s2.cerrar();
    }
    eprintln!("E30-H02 · sonda A (cosechado): {fallos}/{REPETICIONES} fallos");
}

/// Sonda B — el mismo escenario pero **sin cosechar** el proceso muerto: el hijo queda ZOMBI y su
/// pid sigue respondiendo `rc=0` a `kill(pid, 0)`. Es la hipótesis de causa raíz aislada: si aquí
/// el `WRITE_CONFLICT` es sistemático y en la sonda A no aparece, la ventana es exactamente la que
/// va del `SIGKILL` a la cosecha del proceso muerto (que en el uso real —donde nadie cosecha al
/// servidor MCP— la hace el `init`/el padre, no el arnés).
#[test]
#[ignore = "sonda de diagnóstico de E30-H02: se lanza a mano"]
fn sonda_e30h02_reclamo_con_dueño_zombi() {
    let mut fallos = 0usize;
    let n = 6usize;
    for i in 0..n {
        let dir = tempfile::tempdir().unwrap();
        construye(dir.path());

        let mut s = Sesion::abrir(dir.path());
        let pid_muerto = s.pid();
        plan_y_apply_sin_leer(&mut s);
        std::thread::sleep(std::time::Duration::from_millis(100));
        s.matar_sin_cosechar(); // ← el hijo queda ZOMBI: `kill(pid,0)` devuelve 0.

        let lock = lee_lock(dir.path());
        let vida_zombi = lock.pid.map_or("(sin pid)", vida);

        let mut s2 = Sesion::abrir(dir.path());
        let plan2 = s2.tool_cruda(
            "change_plan",
            serde_json::json!({
                "operations": [{"op": "create", "path": "testigo.md", "body": "# Testigo\n"}],
                "policy": {"requireValidResult": false, "allowWarnings": true}
            }),
        );
        let cs2 = plan2["structuredContent"]["changeSetId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let r = s2.tool_cruda("change_apply", serde_json::json!({"changeSetId": cs2}));
        let error = r["isError"].as_bool().unwrap_or(false);
        if error {
            fallos += 1;
        }
        eprintln!(
            "E30-H02 · sonda B repetición {i}: pid={pid_muerto} lock.pid={:?} \
             vida(zombi)={vida_zombi} → ¿apply falló? {error}\n    respuesta={}",
            lock.pid,
            serde_json::to_string(&r).unwrap_or_default()
        );
        s2.cerrar();
        // Ahora sí se cosecha, para no dejar zombis colgando del proceso de test.
        let _ = s.proc.wait();
    }
    eprintln!("E30-H02 · sonda B (zombi): {fallos}/{n} fallos");
}

/// Sonda C — **la segunda rama de `Vivo`**: un lock cuyo cuerpo aún no está escrito.
///
/// `acquire_lock` crea el fichero con `create_new` (vacío) y **después** le escribe el cuerpo con
/// `pid`/`host`/`token`. Entre las dos operaciones hay una ventana; si el proceso muere ahí, el
/// fichero existe y está **vacío**. `reclamar_si_huerfano` lo lee, `serde_json` falla, `meta` queda
/// en `Null`, y entonces `pid = None` → `Vida::Desconocida`, `ts = None` → `caducado = false`:
/// `reclamable = false` y el lock se declara **`Vivo` con detalle vacío**, o sea el mensaje
/// «el lock de publicación ya está tomado (…)» **sin** el sufijo del pid. Es un huérfano
/// irreclamable hasta el TTL de 15 minutos… que ni siquiera se aplica, porque sin `timestamp` no
/// hay edad que comparar: **el lock queda tomado para siempre**.
///
/// La sonda fabrica ese estado a mano (fichero de lock vacío) porque la ventana real es de
/// microsegundos y no se puede apuntar con un `sleep`; lo que demuestra es que el estado, si se
/// alcanza, es terminal.
#[test]
#[ignore = "sonda de diagnóstico de E30-H02: se lanza a mano"]
fn sonda_e30h02_lock_con_cuerpo_no_escrito() {
    for (etiqueta, cuerpo) in [
        ("vacío (create_new sin escribir el cuerpo)", ""),
        ("JSON truncado a medio escribir", "{\"owner\":\"dbar"),
        ("JSON válido sin pid ni timestamp", "{\"owner\":\"x\"}"),
    ] {
        let dir = tempfile::tempdir().unwrap();
        construye(dir.path());
        let runtime = dir.path().join(".lodestar/runtime");
        std::fs::create_dir_all(&runtime).unwrap();
        std::fs::write(runtime.join("lock.json"), cuerpo).unwrap();

        let mut s = Sesion::abrir(dir.path());
        let plan = s.tool_cruda(
            "change_plan",
            serde_json::json!({
                "operations": [{"op": "create", "path": "testigo.md", "body": "# Testigo\n"}],
                "policy": {"requireValidResult": false, "allowWarnings": true}
            }),
        );
        let cs = plan["structuredContent"]["changeSetId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let r = s.tool_cruda("change_apply", serde_json::json!({"changeSetId": cs}));
        eprintln!(
            "E30-H02 · sonda C [{etiqueta}] → ¿apply falló? {}\n    respuesta={}",
            r["isError"].as_bool().unwrap_or(false),
            serde_json::to_string(&r).unwrap_or_default()
        );
        s.cerrar();
    }
}

/// Sonda E — **la ventana `[create_new, escribir_cuerpo)`, apuntada con precisión**.
///
/// La sonda D barre retrasos a ciegas y en una máquina descargada nunca cae dentro de la ventana
/// (microsegundos). Ésta la apunta: en vez de dormir un tiempo fijo, **sondea el fichero de lock
/// desde fuera** y manda el `SIGKILL` en cuanto el fichero EXISTE, que es —por construcción de
/// `acquire_lock`— el instante inmediatamente posterior al `create_new` y anterior (o simultáneo)
/// a la escritura del cuerpo. Es el mismo truco de disparo por estado durable que ya usan
/// `crash_tras_publicar_deja_transaccion_reversible` y `crash_durante_revert_deja_inversa_reversible`
/// en `crash_senal.rs`: observación de disco desde otro proceso, sin seams en el binario.
///
/// Registra, por repetición, el tamaño del cuerpo del lock al morir (0 = ventana acertada) y si el
/// segundo proceso consigue publicar. Es la demostración de que el estado que la sonda C fabrica a
/// mano es ALCANZABLE por un crash real, no una hipótesis de laboratorio.
#[test]
#[ignore = "sonda de diagnóstico de E30-H02: se lanza a mano"]
fn sonda_e30h02_sigkill_apuntado_a_la_ventana_del_cuerpo() {
    let n = 30usize;
    let mut vacios = 0usize;
    let mut fallos = 0usize;
    for i in 0..n {
        let dir = tempfile::tempdir().unwrap();
        construye(dir.path());
        let lock_path = dir.path().join(".lodestar/runtime/lock.json");

        let mut s = Sesion::abrir(dir.path());
        plan_y_apply_sin_leer(&mut s);

        // Disparo por estado de disco: se mata en cuanto el fichero de lock EXISTE.
        let t0 = std::time::Instant::now();
        while t0.elapsed() < std::time::Duration::from_secs(30) && !lock_path.exists() {
            std::hint::spin_loop();
        }
        s.matar();

        let lock = lee_lock(dir.path());
        let vacio = lock.existe && lock.crudo.is_empty();
        if vacio {
            vacios += 1;
        }

        let mut s2 = Sesion::abrir(dir.path());
        let plan2 = s2.tool_cruda(
            "change_plan",
            serde_json::json!({
                "operations": [{"op": "create", "path": "testigo.md", "body": "# Testigo\n"}],
                "policy": {"requireValidResult": false, "allowWarnings": true}
            }),
        );
        let cs2 = plan2["structuredContent"]["changeSetId"]
            .as_str()
            .unwrap_or_default()
            .to_string();
        let r = s2.tool_cruda("change_apply", serde_json::json!({"changeSetId": cs2}));
        let error = r["isError"].as_bool().unwrap_or(false);
        if error {
            fallos += 1;
            eprintln!(
                "E30-H02 · sonda E repetición {i}: cuerpo_len={} vacío={vacio} → APPLY FALLÓ\n    \
                 respuesta={}",
                lock.crudo.len(),
                serde_json::to_string(&r).unwrap_or_default()
            );
        } else {
            eprintln!(
                "E30-H02 · sonda E repetición {i}: cuerpo_len={} vacío={vacio} → apply ok",
                lock.crudo.len()
            );
        }
        s2.cerrar();
    }
    eprintln!(
        "E30-H02 · sonda E: {vacios}/{n} muertes con el lock creado y VACÍO; {fallos}/{n} fallos \
         del primer apply del segundo proceso"
    );
}

/// Sonda D — **¿queda algún lock en disco tras el `SIGKILL`, y con qué cuerpo?**
///
/// Barre los mismos retrasos que el test flaky (más algunos muy cortos, para cubrir la ventana en
/// la que el lock acaba de crearse) y, en cada uno, vuelca el estado exacto del fichero de lock
/// inmediatamente después de matar: si existe, si tiene cuerpo, y qué dice la prueba de vida sobre
/// su pid. Es el mapa de qué estados de lock produce de verdad este escenario.
#[test]
#[ignore = "sonda de diagnóstico de E30-H02: se lanza a mano"]
fn sonda_e30h02_mapa_de_estados_del_lock() {
    let retrasos_ms = [1, 3, 5, 8, 12, 20, 40, 70, 100, 130, 170];
    for ms in retrasos_ms {
        for _ in 0..4 {
            let dir = tempfile::tempdir().unwrap();
            construye(dir.path());
            let mut s = Sesion::abrir(dir.path());
            let pid_muerto = s.pid();
            plan_y_apply_sin_leer(&mut s);
            std::thread::sleep(std::time::Duration::from_millis(ms));
            s.matar();
            let lock = lee_lock(dir.path());
            eprintln!(
                "E30-H02 · sonda D retraso={ms}ms pid={pid_muerto}: lock.existe={} \
                 cuerpo_len={} pid={:?} vida={} edad={:?}",
                lock.existe,
                lock.crudo.len(),
                lock.pid,
                lock.pid.map_or("(sin pid)", vida),
                lock.edad_s,
            );
        }
    }
}
