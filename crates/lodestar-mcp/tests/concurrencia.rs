//! E23-H09 · Bordes — **concurrencia ENTRE PROCESOS** (`requirements/epica-23-cierre-migracion.md`).
//!
//! Por qué un fichero propio y por qué por la frontera MCP: la única prueba de concurrencia del
//! repo hasta esta historia (`crates/lodestar-app/tests/escala.rs::bench_concurrencia_segura`) usa
//! **dos hilos del mismo proceso**. Pero la primitiva de exclusión es
//! `OpenOptions::create_new` (`O_CREAT | O_EXCL`) sobre `.lodestar/runtime/lock.json`
//! (`crates/lodestar-workspace/src/lock.rs`), que es **inter-proceso**: un `Mutex` de Rust
//! protegería igual de bien dos hilos y nada del despliegue real. Y el despliegue real son N
//! servidores `lodestar-mcp` sobre el mismo checkout, o un MCP conviviendo con un `lodestar check`
//! en CI.
//!
//! Los dos tests de aquí son, por tanto, **cobertura que faltaba**, no fase roja: se espera que
//! pasen con el motor tal cual está.
//!
//! ## Cómo se hace real la carrera
//! Un plan y su aplicación son dos `tools/call` distintas. Si cada proceso hiciera
//! «plan → apply» de un tirón, el segundo planificaría sobre la base que el primero ya publicó y
//! **no habría carrera**: ganaría por orden de llegada sin tocar el lock. Aquí los dos servidores
//! se mantienen **vivos** y la secuencia se entrelaza a mano:
//!
//! ```text
//! A: plan  ──►  (respuesta)          B: plan  ──►  (respuesta)      ← ambos sobre la MISMA base r0
//!                         A: apply ──┐   B: apply ──┐               ← escritos sin leer en medio
//!                                    └── respuestas ─┘
//! ```
//!
//! ## Familia de códigos del perdedor (igual que `escala.rs`)
//! El perdedor pierde en uno de dos puntos, ambos limpios y **antes** de publicar:
//! `WRITE_CONFLICT` (el lock ya está tomado, o la base cambió bajo el lock) o `PLAN_STALE` (el
//! `planHash` recomputado ya no casa la base que selló el ganador). El test acepta la familia
//! `{WRITE_CONFLICT, PLAN_STALE}` y **registra por stderr** cuál ocurrió: lo esencial —exactamente
//! uno publica, el otro se rechaza sin corromper— es determinista con cualquier entrelazado.

use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::time::{Duration, Instant};

/// Escribe un fichero dentro del workspace temporal, creando los directorios intermedios.
fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().expect("ruta con padre")).expect("crear directorios");
    std::fs::write(p, content).expect("escribir fichero");
}

/// Ruta del lock de publicación (`crates/lodestar-workspace/src/lock.rs`: `Workspace::lock_path`).
/// Se replica aquí a propósito, como constante del **contrato observable** desde fuera del
/// proceso: si el nombre o la ubicación cambian, este test tiene que enterarse.
fn lock_path(root: &Path) -> std::path::PathBuf {
    root.join(".lodestar").join("runtime").join("lock.json")
}

/// Un servidor `lodestar-mcp` **vivo**, con su stdin/stdout tomados: permite entrelazar mensajes
/// entre dos procesos (enviar a los dos antes de leer de ninguno), que es lo que hace real la
/// carrera. `roundtrip` (en `mcp.rs`) no sirve aquí porque cierra el stdin y drena la salida.
struct Servidor {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

impl Servidor {
    /// Arranca un servidor sobre `dir`. El `App` se abre en el arranque del proceso, así que al
    /// volver de aquí el proceso ya está escuchando (el primer `envia` no compite con la apertura).
    fn arranca(dir: &Path) -> Self {
        let mut child = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
            .arg("--root")
            .arg(dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("arrancar lodestar-mcp");
        let stdin = child.stdin.take().expect("stdin del servidor");
        let stdout = BufReader::new(child.stdout.take().expect("stdout del servidor"));
        Servidor {
            child,
            stdin,
            stdout,
        }
    }

    /// Envía una línea JSON-RPC **sin** esperar respuesta (el punto de entrelazado).
    fn envia(&mut self, linea: &str) {
        writeln!(self.stdin, "{linea}").expect("escribir en el stdin del servidor");
        self.stdin.flush().expect("flush del stdin del servidor");
    }

    /// Lee una línea de respuesta y la parsea (stdout es JSON-RPC puro).
    fn lee(&mut self) -> serde_json::Value {
        let mut linea = String::new();
        let n = self
            .stdout
            .read_line(&mut linea)
            .expect("leer del stdout del servidor");
        assert!(
            n > 0,
            "el servidor cerró stdout sin responder (¿murió?): buffer «{linea}»"
        );
        serde_json::from_str(&linea).expect("stdout = JSON-RPC puro")
    }

    /// Cierra el stdin y espera al proceso (libera el lock por RAII si aún lo tuviera).
    fn cierra(mut self) {
        drop(self.stdin);
        let _ = self.child.wait();
    }
}

/// Línea `tools/call change_plan` con `operations` + `policy` permisiva.
fn linea_plan(id: u32, operations: serde_json::Value) -> String {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": { "name": "change_plan", "arguments": {
            "operations": operations,
            // Permisiva a propósito: lo que se mide es la concurrencia, no el veredicto de validez.
            "policy": { "requireValidResult": false, "allowWarnings": true }
        } }
    })
    .to_string()
}

/// Línea `tools/call change_apply` por `changeSetId` (sin `expectedWorkspaceRevision`: el
/// discriminante debe ser el lock / el `planHash`, no el control optimista del llamante).
fn linea_apply(id: u32, change_set_id: &str) -> String {
    serde_json::json!({
        "jsonrpc": "2.0", "id": id, "method": "tools/call",
        "params": { "name": "change_apply", "arguments": { "changeSetId": change_set_id } }
    })
    .to_string()
}

/// El `changeSetId` de una respuesta `change_plan`.
fn change_set_id(resp: &serde_json::Value) -> String {
    resp["result"]["structuredContent"]["changeSetId"]
        .as_str()
        .unwrap_or_else(|| panic!("change_plan debe devolver un `changeSetId` (string): {resp:?}"))
        .to_string()
}

/// ¿Esta respuesta de `change_apply` publicó? (`structuredContent.applied == true`).
fn aplicado(resp: &serde_json::Value) -> bool {
    resp["result"]["structuredContent"]["applied"] == serde_json::Value::Bool(true)
}

/// El código estable de error que viaja en el `content` de una respuesta con `isError`
/// (`crates/lodestar-mcp/src/tools.rs`: el texto es `ErrorCode::as_str()`, nunca el `Debug`).
fn codigo_de_error(resp: &serde_json::Value) -> String {
    assert_eq!(
        resp["result"]["isError"], true,
        "se esperaba una respuesta de error de EJECUCIÓN (isError): {resp:?}"
    );
    assert!(
        resp["error"].is_null(),
        "un rechazo del motor NO debe ser un error de protocolo JSON-RPC: {resp:?}"
    );
    resp["result"]["content"][0]["text"]
        .as_str()
        .unwrap_or_else(|| panic!("el error debe traer su código estable como texto: {resp:?}"))
        .to_string()
}

/// Workspace mínimo: un solo documento con una clave de frontmatter que los dos planes se disputan.
fn workspace_un_documento() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        dir.path(),
        "nota.md",
        "---\nestado: inicial\n---\n\n# Nota\n\ncuerpo que no cambia.\n",
    );
    dir
}

// ===========================================================================
// Criterio `dos_procesos_un_ganador`
//
// Dado dos PROCESOS `lodestar-mcp` aplicando planes que tocan el MISMO fichero, Cuando ambos
// planes están calculados sobre la misma base y se aplican entrelazados, Entonces exactamente uno
// responde `applied: true` y el otro se rechaza con un código de la familia
// {WRITE_CONFLICT, PLAN_STALE}, sin corromper el documento en disputa.
// ===========================================================================
#[test]
fn dos_procesos_un_ganador() {
    let dir = workspace_un_documento();
    let raiz = dir.path();

    let mut a = Servidor::arranca(raiz);
    let mut b = Servidor::arranca(raiz);

    // (1) Los DOS planes se calculan antes de que ninguno aplique: misma base r0 para ambos.
    //     Tocan la misma clave del mismo documento con valores distintos, así que el ganador es
    //     identificable por el contenido final del `.md` y los dos resultados son incompatibles.
    a.envia(&linea_plan(
        1,
        serde_json::json!([
            { "op": "patch_frontmatter", "ref": { "path": "nota.md" },
              "patch": { "estado": "ganador-A" } }
        ]),
    ));
    let plan_a = a.lee();
    b.envia(&linea_plan(
        1,
        serde_json::json!([
            { "op": "patch_frontmatter", "ref": { "path": "nota.md" },
              "patch": { "estado": "ganador-B" } }
        ]),
    ));
    let plan_b = b.lee();

    let id_a = change_set_id(&plan_a);
    let id_b = change_set_id(&plan_b);
    assert_ne!(
        id_a, id_b,
        "dos planes con patches distintos deben tener changeSetId distintos (si no, la carrera \
         sería entre el mismo plan consigo mismo y el test no probaría nada): {plan_a:?} {plan_b:?}"
    );

    // Control de que la carrera es genuina: el disco sigue en su estado inicial DESPUÉS de que los
    // dos planes existan (planificar no escribe), o sea que los dos parten de la misma base.
    let antes = std::fs::read_to_string(raiz.join("nota.md")).expect("leer nota.md");
    assert!(
        antes.contains("estado: inicial"),
        "planificar no debe escribir: `nota.md` debe seguir en su estado inicial: {antes:?}"
    );

    // (2) Los dos apply salen sin leer nada en medio: ambos procesos están bloqueados en `read`,
    //     así que despiertan prácticamente a la vez y se disputan el lock de verdad.
    a.envia(&linea_apply(2, &id_a));
    b.envia(&linea_apply(2, &id_b));
    let resp_a = a.lee();
    let resp_b = b.lee();

    // --- Propiedad 1: EXACTAMENTE uno publica. ---
    let ganan = aplicado(&resp_a) as u8 + aplicado(&resp_b) as u8;
    assert_eq!(
        ganan, 1,
        "de dos procesos que aplican planes sobre el mismo fichero debe publicar exactamente uno; \
         A={resp_a:?} B={resp_b:?}"
    );

    // --- Propiedad 2: el perdedor se rechaza LIMPIAMENTE con un código de la familia conflicto. ---
    let (etiqueta_perdedor, resp_perdedor) = if aplicado(&resp_a) {
        ("B", &resp_b)
    } else {
        ("A", &resp_a)
    };
    let codigo = codigo_de_error(resp_perdedor);
    eprintln!("[dos_procesos] perdedor={etiqueta_perdedor} código={codigo}");
    assert!(
        codigo.contains("WRITE_CONFLICT") || codigo.contains("PLAN_STALE"),
        "el perdedor debe rechazarse con un código de la familia conflicto \
         {{WRITE_CONFLICT, PLAN_STALE}}, no con «{codigo}»: {resp_perdedor:?}"
    );

    // --- Propiedad 3: el documento en disputa queda ÍNTEGRO y con el valor del ganador. ---
    let despues = std::fs::read_to_string(raiz.join("nota.md")).expect("leer nota.md");
    let esperado = if aplicado(&resp_a) {
        "ganador-A"
    } else {
        "ganador-B"
    };
    let descartado = if aplicado(&resp_a) {
        "ganador-B"
    } else {
        "ganador-A"
    };
    assert!(
        despues.contains(esperado),
        "`nota.md` debe llevar el valor del proceso que reportó éxito («{esperado}»): {despues:?}"
    );
    assert!(
        !despues.contains(descartado),
        "el patch del perdedor («{descartado}») NO puede haberse colado en el documento: {despues:?}"
    );
    assert!(
        !despues.contains("estado: inicial"),
        "el ganador SÍ escribió: el valor inicial no puede sobrevivir (control anti-vacuo): \
         {despues:?}"
    );
    assert!(
        despues.contains("cuerpo que no cambia."),
        "el cuerpo, que ningún plan tocaba, debe sobrevivir intacto: {despues:?}"
    );

    // --- Propiedad 4: el lock se soltó (RAII) — no queda huérfano tras la carrera. ---
    a.cierra();
    b.cierra();
    assert!(
        !lock_path(raiz).exists(),
        "tras la carrera el lock de publicación debe estar liberado, no huérfano en {:?}",
        lock_path(raiz)
    );
}

// ===========================================================================
// Criterio `lock_huerfano`
//
// Dado un `.lodestar/runtime/lock.json` de un PID que ya no existe, Cuando se aplica un plan,
// Entonces el comportamiento es el DOCUMENTADO: rechazo inmediato (fail-fast) con WRITE_CONFLICT,
// nunca un bloqueo eterno.
//
// ⚠ HALLAZGO documentado por este test (no es un fallo del test: es el comportamiento real, y se
// asevera tal cual a propósito). El lock **no se reclama nunca**:
//   - `acquire_lock` solo mira si el fichero EXISTE; el `pid` que escribe `lock_metadata()` es
//     puramente informativo y **nadie lo lee de vuelta** (el propio módulo lo dice: «su contenido
//     no participa en la exclusión»).
//   - No hay TTL, ni comprobación de proceso vivo, ni reclamo por antigüedad.
//   - `Workspace::recovery_pending()` cubre el journal de la transacción, no el lock.
// Consecuencia práctica: si un `lodestar-mcp` muere por `SIGKILL` (OOM, `kill -9`, corte de
// energía) el fichero de lock sobrevive y el workspace queda **permanentemente cerrado a la
// escritura** hasta que un humano borre el fichero a mano — y el mensaje de error no dice cuál es
// ese fichero ni que se pueda borrar. No es un cuelgue (eso sería peor), pero sí una denegación de
// servicio persistente sin ruta de salida autodescriptiva.
//
// El test fija las dos mitades: (a) no cuelga y rechaza con WRITE_CONFLICT; (b) el lock huérfano
// sigue ahí byte a byte después del intento, y basta borrarlo para que el MISMO plan se aplique
// —lo que demuestra que el lock huérfano era el único discriminante (control anti-vacuo) y que no
// existe reclamo automático.
// ===========================================================================

/// Un PID que con certeza **no** corresponde a ningún proceso vivo: se arranca el propio binario
/// con `--help` (sale 0 de inmediato) y se espera a que muera. Es portable —no depende de rangos
/// de PID del sistema— y realista: es exactamente el hueco que deja un servidor que se fue.
fn pid_muerto() -> u32 {
    let mut hijo = Command::new(env!("CARGO_BIN_EXE_lodestar-mcp"))
        .arg("--help")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("arrancar el proceso sonda");
    let pid = hijo.id();
    hijo.wait().expect("esperar al proceso sonda");
    pid
}

/// Límite de espera de un roundtrip. Solo tiene que distinguir «colgado» (infinito) de «lento»,
/// así que se elige **muy** holgado: en local un roundtrip completo tarda ~10 ms, pero un runner de
/// CI cargado —o una máquina compilando otra cosa a la vez— puede tardar segundos solo en hacer
/// `spawn` del binario. Un valor ajustado convertiría una aserción de robustez en un test frágil.
const LIMITE_ROUNDTRIP: Duration = Duration::from_secs(180);

/// Ejecuta un roundtrip completo (proceso nuevo, N líneas, N respuestas) con **límite de tiempo**:
/// si el motor se quedara colgado esperando el lock, el test falla con un mensaje que lo nombra en
/// vez de dejar el CI colgado hasta el timeout global. Es la aserción de «no es un bloqueo eterno».
fn roundtrip_con_limite(
    dir: &Path,
    lineas: Vec<String>,
    esperadas: usize,
    limite: Duration,
) -> Vec<serde_json::Value> {
    let raiz = dir.to_path_buf();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut s = Servidor::arranca(&raiz);
        for l in &lineas {
            s.envia(l);
        }
        let mut out = Vec::new();
        for _ in 0..esperadas {
            out.push(s.lee());
        }
        s.cierra();
        let _ = tx.send(out);
    });
    rx.recv_timeout(limite).unwrap_or_else(|_| {
        panic!(
            "el servidor no respondió en {:?}: BLOQUEO — el lock nunca debe hacer esperar \
             (el modelo es fail-fast, `crates/lodestar-workspace/src/lock.rs`)",
            limite
        )
    })
}

#[test]
fn lock_huerfano() {
    let dir = workspace_un_documento();
    let raiz = dir.path();

    // (1) Plan válido, con el workspace todavía sin lock (planificar no toca el lock).
    let plan = roundtrip_con_limite(
        raiz,
        vec![linea_plan(
            1,
            serde_json::json!([
                { "op": "patch_frontmatter", "ref": { "path": "nota.md" },
                  "patch": { "estado": "tras-el-lock-huerfano" } }
            ]),
        )],
        1,
        LIMITE_ROUNDTRIP,
    );
    let id = change_set_id(&plan[0]);

    // (2) Un lock de un proceso que ya no existe, con la MISMA forma que escribe `lock_metadata()`.
    let pid = pid_muerto();
    let cuerpo_huerfano = format!("{{\"owner\":\"fantasma\",\"pid\":{pid},\"timestamp\":0}}\n");
    std::fs::create_dir_all(lock_path(raiz).parent().unwrap()).expect("crear runtime/");
    std::fs::write(lock_path(raiz), &cuerpo_huerfano).expect("plantar el lock huérfano");

    // (3) Aplicar. Fail-fast: debe responder ya, no esperar. El límite es generoso a propósito
    //     (un cuelgue real no responde NUNCA, así que basta con distinguirlo de «lento»); el
    //     tiempo real se registra abajo, y en local es de milisegundos.
    let t0 = Instant::now();
    let resp = roundtrip_con_limite(raiz, vec![linea_apply(2, &id)], 1, LIMITE_ROUNDTRIP);
    let transcurrido = t0.elapsed();
    eprintln!("[lock_huerfano] respuesta en {transcurrido:?} (pid huérfano plantado: {pid})");

    let codigo = codigo_de_error(&resp[0]);
    assert!(
        codigo.contains("WRITE_CONFLICT"),
        "con el lock tomado (aunque sea por un muerto) el apply debe rechazarse con \
         WRITE_CONFLICT, no con «{codigo}»: {resp:?}"
    );

    // El documento no se tocó: un rechazo por lock ocurre ANTES de publicar.
    let en_disco = std::fs::read_to_string(raiz.join("nota.md")).expect("leer nota.md");
    assert!(
        en_disco.contains("estado: inicial"),
        "un apply rechazado por el lock no debe haber escrito nada: {en_disco:?}"
    );

    // (4) HALLAZGO: el lock huérfano sigue exactamente igual. Nadie lo reclama, ni siquiera
    //     mirando el `pid` que él mismo guarda; y el error no dice qué fichero borrar.
    let tras_el_intento =
        std::fs::read_to_string(lock_path(raiz)).expect("el lock huérfano debe seguir en disco");
    assert_eq!(
        tras_el_intento, cuerpo_huerfano,
        "el motor NO reclama el lock de un PID muerto: el fichero sobrevive byte a byte al intento \
         fallido. Documentado como defecto conocido en la cabecera de este bloque; si algún día se \
         implementa el reclamo (TTL o comprobación de proceso vivo), este test debe REESCRIBIRSE, \
         no relajarse"
    );

    // (5) Control anti-vacuo + prueba de que no hay reclamo automático: borrado el lock a mano, el
    //     MISMO plan (mismo changeSetId, mismo disco) se aplica sin tocar nada más.
    std::fs::remove_file(lock_path(raiz)).expect("borrar el lock huérfano a mano");
    let resp2 = roundtrip_con_limite(raiz, vec![linea_apply(3, &id)], 1, LIMITE_ROUNDTRIP);
    assert!(
        aplicado(&resp2[0]),
        "borrado el lock huérfano, el MISMO plan debe aplicarse: el lock era el único \
         discriminante (si esto fallara, el WRITE_CONFLICT de arriba podría deberse a otra cosa y \
         el test sería vacuo): {resp2:?}"
    );
    let final_ = std::fs::read_to_string(raiz.join("nota.md")).expect("leer nota.md");
    assert!(
        final_.contains("tras-el-lock-huerfano"),
        "el apply que sí pasó debe haber publicado el patch: {final_:?}"
    );
    assert!(
        !lock_path(raiz).exists(),
        "el apply exitoso debe soltar su propio lock (RAII) al terminar"
    );
}
