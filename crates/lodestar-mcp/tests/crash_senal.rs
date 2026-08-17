//! **E24-H14** — Crash REAL del servidor por señal, durante `change_apply`.
//!
//! El escenario 12 de `§17` («crash durante la publicación») estaba **degradado**: solo probaba
//! durabilidad tras cerrar y reabrir el servidor limpiamente. El crash de verdad lo cubrían los
//! failpoints, que hasta E24-H13 componían el estado post-crash a mano y en un orden distinto al
//! del orquestador.
//!
//! Este fichero cierra el hueco por el otro extremo: mata el **binario** con `SIGKILL` a mitad de
//! la publicación. Es la única prueba que no depende de ningún `Drop`, de ningún `unwind` y de
//! ninguna reconstrucción del estado — el SO se lleva el proceso por delante, que es exactamente
//! lo que pasa en un corte de luz o un `kill -9`.
//!
//! Nació como sonda externa durante la revisión de la v0.3.0: 30 `SIGKILL` escalonados que
//! confirmaron que el invariante nuclear aguanta, y de paso destaparon tres defectos que la suite
//! no veía (el `WRITE_CONFLICT` sistemático de E24-H03, el workspace que se presentaba como roto de
//! E24-H04 y las fugas de E24-H05/H06). Aquí queda como test permanente.
//!
//! Solo Unix: `SIGKILL` no tiene equivalente en Windows con la misma garantía de «ni un `Drop` se
//! ejecuta». El invariante que prueba es del motor, no del sistema operativo.
#![cfg(unix)]

use std::collections::BTreeMap;
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

/// Nº de notas que enlazan al hub. Cuantas más, más ancha la ventana de publicación y más
/// probable que el `SIGKILL` caiga DENTRO de los renames.
const NOTAS: usize = 60;

/// Sesión MCP viva contra el binario real.
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
                "clientInfo": {"name": "lodestar-crash-senal", "version": "1"}
            }),
        );
        let respuesta = sesion.lee();
        assert_eq!(respuesta["result"]["serverInfo"]["name"], "lodestar-mcp");
        sesion.notificacion("notifications/initialized");
        sesion
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

    /// `tools/call` que exige éxito y devuelve el `structuredContent`.
    fn tool(&mut self, nombre: &str, args: serde_json::Value) -> serde_json::Value {
        self.envia(
            "tools/call",
            serde_json::json!({"name": nombre, "arguments": args}),
        );
        let r = self.lee();
        let res = &r["result"];
        assert!(
            !res["isError"].as_bool().unwrap_or(false),
            "la tool «{nombre}» falló: {r}"
        );
        res["structuredContent"].clone()
    }

    /// `tools/call` que **no** exige éxito: devuelve el `result` entero, `isError` incluido. Lo
    /// necesita E25-H04, que tiene que distinguir «revirtió» de «respondió `PLAN_EXPIRED`».
    fn tool_cruda(&mut self, nombre: &str, args: serde_json::Value) -> serde_json::Value {
        self.envia(
            "tools/call",
            serde_json::json!({"name": nombre, "arguments": args}),
        );
        self.lee()["result"].clone()
    }

    /// **Mata el proceso con `SIGKILL`**: ni un destructor se ejecuta.
    fn matar(mut self) {
        // SAFETY: `libc::kill` sobre el pid de un hijo propio.
        unsafe {
            libc::kill(self.proc.id() as i32, libc::SIGKILL);
        }
        let _ = self.proc.wait();
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

/// Instantánea byte a byte de los `.md` del árbol (excluye `.lodestar/`).
fn instantanea(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(base: &Path, dir: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let p = e.path();
            if p.is_dir() {
                if p.file_name().and_then(|n| n.to_str()) == Some(".lodestar") {
                    continue;
                }
                walk(base, &p, out);
            } else if p.extension().is_some_and(|x| x == "md") {
                let rel = p.strip_prefix(base).unwrap().to_string_lossy().to_string();
                out.insert(rel, std::fs::read(&p).unwrap());
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// Un `.md` «a medias» se detecta por no decodificar como UTF-8 o por dejar el bloque de
/// frontmatter sin cerrar. El único escritor (temp+fsync+rename) no debería producir ninguno.
fn md_a_medias(data: &[u8]) -> Option<String> {
    let Ok(txt) = std::str::from_utf8(data) else {
        return Some("no es UTF-8".to_string());
    };
    if txt.starts_with("---") && !txt[3..].contains("\n---") {
        return Some("frontmatter sin cerrar".to_string());
    }
    None
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

/// **E24-H14** — `SIGKILL` a mitad de la publicación: ni un `.md` a medias, y el canónico converge
/// a uno de los dos bordes de la transacción.
///
/// Se escalonan varios retrasos porque el punto exacto en que cae el `SIGKILL` no es
/// determinista: barrer la ventana es lo que hace que el test ejerza de verdad los renames y no
/// siempre la fase de staging.
#[test]
fn crash_por_senal_no_deja_parciales() {
    let retrasos_ms = [40, 70, 100, 130, 170];
    let mut hubo_pendiente = false;

    for (i, ms) in retrasos_ms.iter().enumerate() {
        let dir = tempfile::tempdir().unwrap();
        construye(dir.path());
        let original = instantanea(dir.path());

        let mut s = Sesion::abrir(dir.path());
        let plan = s.tool(
            "change_plan",
            serde_json::json!({
                "operations": [{"op": "move", "from": "hub.md", "to": "notas/hub.md",
                                "rewriteInboundLinks": true}],
                "policy": {"requireValidResult": true, "allowWarnings": true}
            }),
        );
        let cs = plan["changeSetId"]
            .as_str()
            .expect("changeSetId")
            .to_string();

        // Se lanza el apply y se mata el proceso a mitad, SIN leer la respuesta.
        s.envia(
            "tools/call",
            serde_json::json!({"name": "change_apply", "arguments": {"changeSetId": cs}}),
        );
        std::thread::sleep(std::time::Duration::from_millis(*ms));
        s.matar();

        // (1) INVARIANTE NUCLEAR: ni un `.md` a medias, justo después del crash.
        for (rel, data) in &instantanea(dir.path()) {
            assert!(
                md_a_medias(data).is_none(),
                "retraso {ms} ms: «{rel}» quedó a medias tras el SIGKILL ({}). El único escritor \
                 escribe temp+fsync+rename: un `.md` parcial no debería ser observable jamás",
                md_a_medias(data).unwrap()
            );
        }

        // (2) Se reabre —como haría el proceso siguiente— y se deja que el motor recupere.
        let mut s2 = Sesion::abrir(dir.path());
        let estado = s2.tool("workspace_status", serde_json::json!({}));
        if estado["recovery"]["pendingTransaction"]
            .as_bool()
            .unwrap_or(false)
        {
            hubo_pendiente = true;
        }

        // Una transacción cualquiera dispara la recuperación (E24-H03) y debe funcionar AL PRIMER
        // INTENTO: hasta entonces daba WRITE_CONFLICT siempre.
        let plan2 = s2.tool(
            "change_plan",
            serde_json::json!({
                "operations": [{"op": "create", "path": "testigo.md", "body": "# Testigo\n"}],
                "policy": {"requireValidResult": false, "allowWarnings": true}
            }),
        );
        let cs2 = plan2["changeSetId"].as_str().expect("changeSetId");
        s2.envia(
            "tools/call",
            serde_json::json!({"name": "change_apply", "arguments": {"changeSetId": cs2}}),
        );
        let r = s2.lee();
        assert!(
            !r["result"]["isError"].as_bool().unwrap_or(false),
            "retraso {ms} ms: tras un crash, el PRIMER `change_apply` debe funcionar (E24-H03): {r}"
        );

        // (3) Convergencia: el conjunto de rutas es el de uno de los dos bordes.
        let mut final_ = instantanea(dir.path());
        final_.remove("testigo.md");
        let rutas: Vec<&String> = final_.keys().collect();
        let rutas_orig: Vec<&String> = original.keys().collect();
        let movido = original.contains_key("hub.md") && !final_.contains_key("hub.md");
        assert!(
            rutas == rutas_orig || (movido && final_.contains_key("notas/hub.md")),
            "retraso {ms} ms (caso {i}): el canónico debe converger a uno de los dos bordes, \
             jamás a un estado parcial. Rutas: {rutas:?}"
        );

        // (4) El plano de control no acumula basura (E24-H05/H06).
        let staging = dir.path().join(".lodestar/runtime/staging");
        let huerfanos = std::fs::read_dir(&staging)
            .map(|rd| rd.flatten().count())
            .unwrap_or(0);
        assert_eq!(
            huerfanos, 0,
            "retraso {ms} ms: tras recuperar y publicar no puede quedar staging huérfano"
        );

        s2.cerrar();
    }

    // Control anti-vacuo del BARRIDO: si ningún retraso llegó a dejar transacción pendiente, el
    // test no ha ejercido la recuperación y solo prueba que el apply no se rompe. No falla el
    // build por eso —el punto de corte depende de la máquina— pero se hace visible.
    if !hubo_pendiente {
        eprintln!(
            "AVISO (E24-H14): ningún retraso dejó una transacción pendiente en esta máquina; el \
             test no ejerció la recuperación. Si esto es sistemático, sube los retrasos o NOTAS."
        );
    }
}

// ---------------------------------------------------------------------------
// E25-H04 — Publicar implica recibo, también cuando el proceso muere
// (`requirements/epica-25-endurecimiento-escritura.md`, bloque B). Fase ROJA.
//
// El criterio de arriba (E24-H14) mira el CONOCIMIENTO: ni un `.md` a medias y convergencia a uno de
// los dos bordes. Eso ya aguanta. Lo que no aguanta es el PLANO DE CONTROL: cuando el `SIGKILL` cae
// entre el último rename y el recibo, el canónico queda publicado y la transacción queda
// **irreversible para siempre** — `change_revert` carga el recibo primero y, al no encontrarlo,
// responde `PLAN_EXPIRED` (`crates/lodestar-app/src/lib.rs:1787-1790`).
//
// Y hay un segundo filo, que es el que hace de este test algo más que un duplicado del de arriba: con
// el recibo persistido junto al journal (el alcance de la historia), un crash en esa ventana deja un
// journal `applied` en disco, así que al reabrir la recuperación toma la vía **COMPLETAR**
// (`recovery.rs`, `finish_recovery`) — que hoy **borra las copias de recuperación**. Sin copias no hay
// reversión, así que «la recuperación por la vía COMPLETAR lo da por bueno» incluye conservar el plano
// de reversión de una transacción que sí publicó y sí tiene recibo.
//
// LA PROPIEDAD QUE FIJA EL TEST (mecanismo-agnóstica, y por eso vale para un crash real)
//
//   si tras el `SIGKILL` y la recuperación el canónico es el borde RESULTADO,
//   entonces existe un recibo para ese `changeSetId` y `change_revert` con él devuelve el canónico,
//   byte a byte, al borde ORIGINAL.
//
// El recibo se busca por la vía del agente (`workspace_status.receipts`, E23-H11), no por el disco:
// la historia deja abierto el «registro durable equivalente» y aquí solo importa que sea utilizable.
//
// La recuperación se dispara con un `change_plan` —que la ejecuta (`App::recover_if_pending`, E24-H03)
// y **no** publica nada—, no con el `change_apply` que usa el test de arriba: aplicar movería la
// `WorkspaceRevision` y el `change_revert` posterior fallaría con `WRITE_CONFLICT` por una razón
// ajena al defecto.
//
// CÓMO SE APUNTA EL `SIGKILL` A ESA VENTANA (y por qué no con un `sleep`)
//
// El test de arriba escalona retrasos a ciegas, y para esta ventana eso no sirve: en esta máquina un
// `change_apply` del escenario tarda ~1,7 s y el tramo `[último rename, recibo)` son unos pocos
// milisegundos —un 1 % del total—, así que doce retrasos fijos no aciertan ninguno (medido: 5 de 12
// mataban con el apply ya terminado y 7 durante el staging; **cero** en la ventana). El disparador es
// por tanto el **estado durable del journal**: el test lo sondea desde fuera y manda el `SIGKILL` en
// cuanto lo ve en `applied`, que es —por definición de `mark_all_applied`, E13-H05— el instante
// inmediatamente posterior al último rename y anterior al sellado. Es observación de disco desde otro
// proceso, no un seam: el binario que muere es el de release del repo, sin features de test, y no sabe
// que lo están mirando.
//
// ROJO ESPERADO HOY: el `SIGKILL` cae con el canónico publicado y sin recibo, así que
// `workspace_status.receipts` no tiene el recibo del `changeSetId` y el criterio falla en su primera
// aserción. (Cuando el recibo se persista con el journal, el segundo filo sigue vivo: la vía COMPLETAR
// tiene que dejar reversible lo que completó.)
// ---------------------------------------------------------------------------

/// El `txnId` con el que la fachada nombra el recibo y el material de una transacción: el hash desnudo
/// del `changeSetId` (misma convención que `lodestar_workspace::transaction_id`).
fn txn_id_de(change_set_id: &str) -> &str {
    change_set_id
        .strip_prefix("changeset:")
        .unwrap_or(change_set_id)
}

/// Estado global del único write-ahead journal presente bajo `.lodestar/runtime/journal/`, si lo hay.
///
/// Es el **punto de mira** del `SIGKILL` de esta historia (`applied` = último rename hecho y anotado,
/// sellado aún no) y la instrumentación con la que el test cuenta si acertó. No es una aserción: lo
/// que se asevera se mira por la superficie MCP.
fn estado_del_journal(root: &Path) -> Option<String> {
    let dir = root.join(".lodestar/runtime/journal");
    for e in std::fs::read_dir(&dir).ok()?.flatten() {
        let p = e.path();
        if p.extension().is_some_and(|x| x == "json") {
            let raw = std::fs::read_to_string(&p).ok()?;
            let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
            return v["state"].as_str().map(|s| s.to_string());
        }
    }
    None
}

/// `true` si el plano de control ya tiene un recibo con el nombre de esta transacción. Igual que
/// [`estado_del_journal`], es instrumentación: la aserción se hace por `workspace_status`.
fn hay_recibo_en_disco(root: &Path, txn_id: &str) -> bool {
    root.join(".lodestar/runtime/receipts")
        .join(format!("{txn_id}.json"))
        .exists()
}

/// Las operaciones del escenario: mover el hub dentro de `notas/` reescribiendo los 60 enlaces
/// entrantes. Toca los 61 `.md`, de modo que la publicación dura lo suficiente para que un `SIGKILL`
/// pueda caer dentro.
fn ops_del_escenario() -> serde_json::Value {
    serde_json::json!({
        "operations": [{"op": "move", "from": "hub.md", "to": "notas/hub.md",
                        "rewriteInboundLinks": true}],
        "policy": {"requireValidResult": true, "allowWarnings": true}
    })
}

/// Cuántas veces se repite el escenario. El disparador es determinista (ver la cabecera), así que no
/// hace falta barrer nada: las repeticiones solo cubren el jitter del sondeo.
const REPETICIONES: usize = 4;

/// Cota superior del sondeo del journal. Solo tiene que distinguir «no llega» de «lento»: un
/// `change_apply` completo de este escenario tarda ~2 s en un `debug` local, así que 30 s dan holgura
/// de sobra a un runner cargado sin que un fallo del disparador cueste minutos de CI.
const LIMITE_SONDEO: std::time::Duration = std::time::Duration::from_secs(30);

/// **E25-H04** · Criterio 4 — **Dado** un `SIGKILL` real entre el último rename y el sellado,
/// **Cuando** se reabre y se recupera, **Entonces** hay recibo y la transacción es reversible.
#[test]
fn crash_tras_publicar_deja_transaccion_reversible() {
    let mut publicados = 0usize;
    let mut en_la_ventana = 0usize;
    let mut sin_recibo = 0usize;

    for i in 0..REPETICIONES {
        let dir = tempfile::tempdir().unwrap();
        construye(dir.path());
        let original = instantanea(dir.path());

        let mut s = Sesion::abrir(dir.path());
        let plan = s.tool("change_plan", ops_del_escenario());
        let cs = plan["changeSetId"]
            .as_str()
            .expect("changeSetId")
            .to_string();
        let txn = txn_id_de(&cs).to_string();

        // Se lanza el apply SIN leer la respuesta y se sondea el journal desde fuera: en cuanto declara
        // `applied` —último rename hecho y anotado, sellado aún no— llega el `SIGKILL`.
        s.envia(
            "tools/call",
            serde_json::json!({"name": "change_apply", "arguments": {"changeSetId": cs}}),
        );
        let t0 = std::time::Instant::now();
        while t0.elapsed() < LIMITE_SONDEO
            && estado_del_journal(dir.path()).as_deref() != Some("applied")
        {
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        s.matar();

        // Instrumentación: ¿se acertó la ventana? `applied` en disco = se mató tras el último rename;
        // y sin recibo todavía = el estado exacto del defecto (una publicación irreversible).
        let journal_al_morir = estado_del_journal(dir.path());
        let recibo_al_morir = hay_recibo_en_disco(dir.path(), &txn);
        if journal_al_morir.as_deref() == Some("applied") {
            en_la_ventana += 1;
            if !recibo_al_morir {
                sin_recibo += 1;
            }
        }
        eprintln!(
            "E25-H04 (repetición {i}): al morir → journal={journal_al_morir:?}, \
             recibo={recibo_al_morir}"
        );

        // Se reabre y se deja que el motor recupere, SIN tocar el canónico: `change_plan` dispara la
        // recuperación (E24-H03) y no publica nada.
        let mut s2 = Sesion::abrir(dir.path());
        let _ = s2.tool(
            "change_plan",
            serde_json::json!({
                "operations": [{"op": "create", "path": "testigo.md", "body": "# Testigo\n"}],
                "policy": {"requireValidResult": false, "allowWarnings": true}
            }),
        );

        // ¿A qué borde convergió? (planificar no escribe canónico: `testigo.md` no existe).
        let tras_recuperar = instantanea(dir.path());
        let publico =
            !tras_recuperar.contains_key("hub.md") && tras_recuperar.contains_key("notas/hub.md");
        if !publico {
            assert_eq!(
                tras_recuperar.keys().collect::<Vec<_>>(),
                original.keys().collect::<Vec<_>>(),
                "repetición {i}: si no publicó, la recuperación tiene que haber restaurado el borde \
                 ORIGINAL (E24-H14); nunca un estado parcial"
            );
            s2.cerrar();
            continue;
        }
        publicados += 1;

        // (1) HAY RECIBO, y se encuentra por donde lo busca un agente que perdió el `receiptId`.
        let estado = s2.tool("workspace_status", serde_json::json!({}));
        let recibos = estado["receipts"].as_array().cloned().unwrap_or_default();
        let receipt_id = recibos
            .iter()
            .find(|r| r["changeSetId"].as_str() == Some(cs.as_str()))
            .and_then(|r| r["receiptId"].as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| {
                panic!(
                    "repetición {i}: el canónico quedó PUBLICADO tras el SIGKILL y no hay recibo de \
                     «{cs}» ({} recibo(s) en workspace_status). La transacción es irreversible para \
                     siempre: `change_revert` responderá PLAN_EXPIRED y un segundo `change_apply` del \
                     mismo plan, PLAN_STALE. Al morir: journal={journal_al_morir:?}, \
                     recibo={recibo_al_morir}",
                    recibos.len()
                )
            });

        // (2) …y la transacción es REVERSIBLE: es la única prueba de que el recibo sirve.
        let r = s2.tool_cruda(
            "change_revert",
            serde_json::json!({"receiptId": receipt_id}),
        );
        assert!(
            !r["isError"].as_bool().unwrap_or(false),
            "repetición {i}: `change_revert` del recibo «{receipt_id}» de una transacción publicada \
             tiene que revertir. Si falla por copias ausentes, la recuperación por la vía COMPLETAR se \
             ha llevado el plano de reversión de una transacción que sí publicó: {r}"
        );

        // (3) Y el canónico vuelve al borde ORIGINAL byte a byte (el movido y los 60 enlaces
        //     reescritos incluidos).
        assert_eq!(
            instantanea(dir.path()),
            original,
            "repetición {i}: revertir tiene que devolver el conocimiento al estado anterior al apply"
        );

        s2.cerrar();
    }

    // Controles anti-vacuos del ARNÉS (no del criterio): si el `SIGKILL` no llegó a caer nunca en la
    // ventana, este test no ha ejercido nada de lo que la historia cierra y no puede pasar en silencio.
    assert!(
        en_la_ventana > 0,
        "el arnés perdió su punto de mira: en {REPETICIONES} repeticiones ningún `SIGKILL` cayó con el \
         journal en `applied` (último rename hecho, sellado pendiente), que es la ventana del criterio. \
         Si el journal ya no pasa de forma observable por ese estado, re-apunta el disparador"
    );
    assert!(
        publicados > 0,
        "ninguna repetición dejó el canónico en el borde RESULTADO: la propiedad del test («si publicó, \
         hay recibo y es reversible») no se ha ejercido ni una vez"
    );
    // Y si en todas ellas el recibo YA estaba escrito al morir, el escenario exacto del defecto no se
    // ha reproducido: se hace visible sin romper el build (con el recibo persistido junto al journal,
    // que es el arreglo, esto pasa a ser lo NORMAL — y entonces lo que sigue vivo es el segundo filo:
    // que la vía COMPLETAR deje reversible lo que completó).
    if sin_recibo == 0 {
        eprintln!(
            "AVISO (E25-H04): en las {en_la_ventana} muertes dentro de la ventana el recibo ya estaba \
             en disco. Si esto ocurre ANTES del arreglo, el disparador está llegando tarde"
        );
    }
}

// ---------------------------------------------------------------------------
// E25-H05 — Revertir también implica recibo, también cuando el proceso muere
// (`requirements/epica-25-endurecimiento-escritura.md`, bloque B, defecto (c)). Fase ROJA.
//
// El test de arriba cubre el camino del apply. Este cubre **el espejo**, que es el hallazgo MAYOR-2
// del juez ciego de E25-H04: la variante de reversión de entonces —`Workspace::revert_transaction`,
// que delegaba con `recibo: None` y se retiró en E31-H01— NO escribía ningún registro
// durable antes de su punto de no retorno, y el `write_receipt` de la inversa salía por `?` en la
// fachada (`crates/lodestar-app/src/lib.rs:1880-1882`) **después** de que el canónico ya haya vuelto
// atrás. Un `SIGKILL` entre el último rename de la inversa y su recibo deja el conocimiento
// restaurado y **sin registro** de que eso ocurrió: como el recibo es el criterio de «vivo» del GC
// (`journal/ ∪ receipts/`), el árbol `recovery/<txnId>-revert/` queda huérfano y se purga.
//
// LA PROPIEDAD QUE FIJA EL TEST (mecanismo-agnóstica, y por eso vale para un crash real)
//
//   si tras el `SIGKILL` y la recuperación el canónico volvió al borde ORIGINAL (la inversa se
//   completó), entonces existe el recibo de la inversa — el registro de que el *undo* ocurrió.
//
// EL DISPARADOR es el mismo que el del test de arriba y por la misma razón: el **estado durable del
// journal**, sondeado desde fuera. Durante la reversión el único journal en `journal/` es el de la
// inversa (el del apply se selló al terminar), así que `applied` significa aquí «último rename de la
// inversa hecho y anotado, sellado pendiente»: exactamente la ventana del defecto. Es observación de
// disco desde otro proceso, no un seam — el binario que muere es el del repo, sin features de test.
//
// ROJO ESPERADO HOY: la reversión se completa (por la vía COMPLETAR de la recuperación) y no hay
// recibo de la inversa por ninguna parte, así que `workspace_status.receipts` no la lista y la
// primera aserción del criterio falla.
// ---------------------------------------------------------------------------

/// Repeticiones del escenario de reversión. El disparador es determinista (estado del journal), así
/// que solo cubren el jitter del sondeo; cada una arranca dos servidores y publica 61 `.md`, de modo
/// que subirlas cuesta CI sin comprar garantía.
const REPETICIONES_REVERT: usize = 3;

/// **E25-H05** · criterio del crash — **Dado** un `SIGKILL` real entre la publicación de la inversa
/// y su recibo, **Cuando** se reabre y se recupera, **Entonces** la reversión completada tiene su
/// recibo.
#[test]
fn crash_durante_revert_deja_inversa_reversible() {
    let mut en_la_ventana = 0usize;
    let mut restaurados = 0usize;
    let mut sin_recibo = 0usize;

    for i in 0..REPETICIONES_REVERT {
        let dir = tempfile::tempdir().unwrap();
        construye(dir.path());
        let original = instantanea(dir.path());

        let mut s = Sesion::abrir(dir.path());
        let plan = s.tool("change_plan", ops_del_escenario());
        let cs = plan["changeSetId"]
            .as_str()
            .expect("changeSetId")
            .to_string();
        let txn = txn_id_de(&cs).to_string();
        let id_inversa = format!("{txn}-revert");

        // Precondición: el apply publica de verdad (61 `.md` tocados) y devuelve su recibo.
        let aplicado = s.tool(
            "change_apply",
            serde_json::json!({"changeSetId": cs.clone()}),
        );
        let receipt_id = aplicado["receiptId"]
            .as_str()
            .unwrap_or_else(|| panic!("change_apply debe devolver receiptId: {aplicado}"))
            .to_string();
        let publicado = instantanea(dir.path());
        assert!(
            publicado.contains_key("notas/hub.md") && !publicado.contains_key("hub.md"),
            "repetición {i}: precondición, el apply tiene que haber publicado el movimiento"
        );

        // Se lanza el revert SIN leer la respuesta y se sondea el journal de la INVERSA desde fuera:
        // en cuanto declara `applied` —último rename hecho y anotado, sellado aún no— llega el
        // `SIGKILL`.
        s.envia(
            "tools/call",
            serde_json::json!({"name": "change_revert", "arguments": {"receiptId": receipt_id}}),
        );
        let t0 = std::time::Instant::now();
        while t0.elapsed() < LIMITE_SONDEO
            && estado_del_journal(dir.path()).as_deref() != Some("applied")
        {
            std::thread::sleep(std::time::Duration::from_micros(100));
        }
        s.matar();

        // Instrumentación: ¿se acertó la ventana? (no es la aserción; esa se hace por MCP).
        let journal_al_morir = estado_del_journal(dir.path());
        let recibo_al_morir = hay_recibo_en_disco(dir.path(), &id_inversa);
        if journal_al_morir.as_deref() == Some("applied") {
            en_la_ventana += 1;
            if !recibo_al_morir {
                sin_recibo += 1;
            }
        }
        eprintln!(
            "E25-H05 (repetición {i}): al morir revirtiendo → journal={journal_al_morir:?}, \
             recibo de la inversa={recibo_al_morir}"
        );

        // Se reabre y se deja que el motor recupere, SIN tocar el canónico (`change_plan` dispara la
        // recuperación y no publica nada).
        let mut s2 = Sesion::abrir(dir.path());
        let _ = s2.tool(
            "change_plan",
            serde_json::json!({
                "operations": [{"op": "create", "path": "testigo.md", "body": "# Testigo\n"}],
                "policy": {"requireValidResult": false, "allowWarnings": true}
            }),
        );

        // ¿A qué borde convergió la INVERSA?
        let tras_recuperar = instantanea(dir.path());
        if tras_recuperar != original {
            assert_eq!(
                tras_recuperar.keys().collect::<Vec<_>>(),
                publicado.keys().collect::<Vec<_>>(),
                "repetición {i}: si la reversión no se completó, la recuperación tiene que haber \
                 devuelto el canónico al borde POST-APPLY; jamás a un estado parcial"
            );
            s2.cerrar();
            continue;
        }
        restaurados += 1;

        // (1) EXISTE EL RECIBO DE LA INVERSA, y se encuentra por donde lo busca un agente que perdió
        //     el `receiptId` (`workspace_status.receipts`, E23-H11).
        let estado = s2.tool("workspace_status", serde_json::json!({}));
        let recibos = estado["receipts"].as_array().cloned().unwrap_or_default();
        let ids: Vec<&str> = recibos
            .iter()
            .filter_map(|r| r["receiptId"].as_str())
            .collect();
        assert!(
            ids.contains(&id_inversa.as_str()),
            "repetición {i}: el canónico volvió al borde ORIGINAL tras el SIGKILL —la reversión se \
             completó— y no hay recibo «{id_inversa}» ({} recibo(s): {ids:?}). Sin él, el GC ve el \
             árbol `recovery/{id_inversa}/` como basura y lo purga: deshacer el *undo* deja de ser \
             posible, y el agente que preguntó por el estado no tiene forma de saber que su reversión \
             ocurrió. Al morir: journal={journal_al_morir:?}, recibo={recibo_al_morir}",
            recibos.len()
        );

        s2.cerrar();
    }

    // Controles anti-vacuos del ARNÉS (no del criterio).
    assert!(
        en_la_ventana > 0,
        "el arnés perdió su punto de mira: en {REPETICIONES_REVERT} repeticiones ningún `SIGKILL` \
         cayó con el journal de la inversa en `applied` (último rename hecho, sellado pendiente), que \
         es la ventana del criterio. Si la reversión ya no pasa de forma observable por ese estado, \
         re-apunta el disparador"
    );
    assert!(
        restaurados > 0,
        "ninguna repetición dejó el canónico en el borde ORIGINAL: la propiedad del test («si la \
         inversa se completó, existe su recibo») no se ha ejercido ni una vez"
    );
    if sin_recibo == 0 {
        eprintln!(
            "AVISO (E25-H05): en las {en_la_ventana} muertes dentro de la ventana el recibo de la \
             inversa ya estaba en disco. Si esto ocurre ANTES del arreglo, el disparador llega tarde"
        );
    }
}
