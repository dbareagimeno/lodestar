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
        Sesion {
            proc,
            stdin: Some(stdin),
            stdout,
            id: 0,
        }
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
        s2.envia(
            "initialize",
            serde_json::json!({"protocolVersion": "2025-06-18", "capabilities": {}}),
        );
        let _ = s2.lee();
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
