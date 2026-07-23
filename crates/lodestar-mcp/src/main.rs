//! Servidor MCP de lodestar (`ARCHITECTURE.md §7.2`).
//!
//! Superficie de arranque (`ARCHITECTURE.md §20.5`, E15-H06):
//! `lodestar-mcp [--root <dir>] [--profile readonly|standard]`. Sin `--root` la raíz es el `cwd`,
//! y **cualquier** directorio vale — no se exige `index.md`, `.lodestar/` ni `lodestar init`.
//!
//! **Logs solo a stderr; stdout = JSON-RPC.** Bucle de líneas JSON-RPC sobre stdio que despacha
//! a los handlers de [`tools`]. La integración con el transporte oficial `rmcp` (handshake completo,
//! resources, streaming) es el paso de producción de E7; este bucle implementa el subconjunto
//! necesario (`initialize`/`tools/list`/`tools/call`) para usarse desde Claude Code.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use lodestar_app::{App, Profile};
use serde_json::{json, Value};

mod tools;

/// Instrucciones del servidor (`instructions` de la respuesta `initialize`, `ARCHITECTURE.md
/// §19.6`): orientan al agente con el **flujo recomendado de 10 pasos**, mencionando las 10 tools
/// en el orden en que se espera usarlas. Los nombres de tool son identificadores (no se traducen);
/// el resto va en español, el idioma del repo (E14-H03).
const SERVER_INSTRUCTIONS: &str = "\
Motor headless de integridad semántica para agentes. Flujo recomendado en cada sesión \
(10 pasos, en orden):

1. `workspace_status`: oriéntate primero — config activa, capacidades del perfil, conformidad y \
recuento agregado del workspace.
2. `knowledge_search`: localiza conceptos por texto y filtros (snippets y revisión, nunca cuerpos \
completos).
3. `knowledge_get`: lee un concepto concreto con `include` selectivo y secciones acotadas.
4. `schema_inspect`: descubre el catálogo de tipos y sus reglas (`.lodestar/schema.yaml`) antes de \
proponer cambios.
5. `graph_query`: consulta el grafo (backlinks, huérfanos, vecindario, caminos) para entender el \
contexto de un concepto.
6. `impact_analyze`: evalúa el impacto de un cambio hipotético (afectados, bloqueos, riesgo) antes \
de proponerlo.
7. `change_plan`: planifica el cambio SIN escribir — normaliza, simula en memoria y valida; \
devuelve un change set con su hash determinista.
8. `change_apply`: aplica el plan calculado con todas las salvaguardas transaccionales; devuelve el \
recibo.
9. `knowledge_check`: audita el conocimiento tras aplicar para confirmar que sigue conforme.
10. `change_revert`: si algo salió mal, revierte la última transacción al estado anterior.

Perfil `readonly`: solo los pasos de lectura y verificación (las tools de cambio no están \
disponibles). Perfil `standard` (por defecto): el flujo completo.";

/// Texto de uso (a stderr: stdout es JSON-RPC puro y nada más).
const USAGE: &str = "\
Uso: lodestar-mcp [--root <dir>] [--profile readonly|standard]

  --root <dir>       Raíz del workspace. Por defecto: el directorio actual (`cwd`).
  --profile <perfil> «standard» (por defecto) o «readonly» (sin las tools de cambio).
  -h, --help         Muestra esta ayuda.";

/// Parsea `[--root <dir>] [--profile readonly|standard]` (`ARCHITECTURE.md §20.5`).
///
/// **No hay argumento posicional**: la raíz es `--root` si se da y el `cwd` si no
/// (`§20.1`, «arranque sin ceremonia»: `cd my-project && lodestar-mcp` funciona). Cualquier otro
/// argumento es error de uso (exit 2).
fn parse_args() -> (PathBuf, Profile) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut root = None;
    let mut profile = Profile::Standard;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--root" => {
                i += 1;
                match args.get(i) {
                    Some(dir) => root = Some(PathBuf::from(dir)),
                    None => {
                        eprintln!("lodestar-mcp: --root necesita un directorio\n\n{USAGE}");
                        std::process::exit(2);
                    }
                }
            }
            "--profile" => {
                i += 1;
                profile = match args.get(i).map(String::as_str) {
                    Some("readonly") => Profile::Readonly,
                    Some("standard") => Profile::Standard,
                    other => {
                        eprintln!(
                            "lodestar-mcp: --profile inválido «{}» (usa «readonly» o «standard»)",
                            other.unwrap_or("")
                        );
                        std::process::exit(2);
                    }
                };
            }
            "-h" | "--help" => {
                eprintln!("{USAGE}");
                std::process::exit(0);
            }
            other => {
                eprintln!("lodestar-mcp: argumento no reconocido «{other}»\n\n{USAGE}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let root =
        root.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    (root, profile)
}

fn main() {
    let (root, profile) = parse_args();

    // La raíz se canonicaliza UNA sola vez al arrancar y queda fija toda la sesión
    // (`ARCHITECTURE.md §20.5`): todas las rutas públicas son relativas a ella, así que no puede
    // depender del `cwd` del proceso ni cambiar a mitad de sesión.
    let root = match std::fs::canonicalize(&root) {
        Ok(p) => p,
        Err(e) => {
            eprintln!(
                "lodestar-mcp: no se pudo resolver la raíz {}: {e}",
                root.display()
            );
            std::process::exit(3);
        }
    };
    // Cualquier directorio es un workspace válido: no hace falta `index.md`, ni `.lodestar/`, ni
    // `lodestar init` (`§20.1`). El gate de «esto no es un bundle» se retiró en E15-H06.
    let app = match App::open(&root) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("lodestar-mcp: no se pudo abrir el workspace: {e}");
            std::process::exit(3);
        }
    };
    eprintln!(
        "lodestar-mcp: escuchando JSON-RPC en stdio (root={}, profile={profile:?})",
        root.display()
    );

    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    for line in stdin.lock().lines() {
        let line = match line {
            Ok(l) if !l.trim().is_empty() => l,
            Ok(_) => continue,
            Err(_) => break,
        };
        // JSON-RPC: el JSON imparseable exige responder -32700 con id null (si no, el cliente
        // se queda esperando la respuesta de ese id para siempre).
        let resp = match serde_json::from_str::<Value>(&line) {
            Ok(v) => handle(&app, profile, &v),
            Err(e) => Some(rpc_error(Value::Null, -32700, &format!("Parse error: {e}"))),
        };
        if let Some(resp) = resp {
            let mut out = stdout.lock();
            let _ = writeln!(out, "{resp}");
            let _ = out.flush();
        }
    }
}

fn rpc_error(id: Value, code: i64, message: &str) -> Value {
    json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
}

/// Despacha un mensaje JSON-RPC. Devuelve `None` para notificaciones (sin `id`).
fn handle(app: &App, profile: Profile, req: &Value) -> Option<Value> {
    // Un mensaje que no es un objeto (array de batch, string, número…) es un request
    // inválido: -32600, no un descarte silencioso que cuelga al cliente.
    if !req.is_object() {
        return Some(rpc_error(
            Value::Null,
            -32600,
            "Invalid Request: se esperaba un objeto JSON-RPC (batch no soportado)",
        ));
    }
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    // Notificaciones (sin id) no llevan respuesta.
    let id = id?;

    let result: Result<Value, (i64, String)> = match method {
        "initialize" => {
            // Ecoa la versión pedida por el cliente si la conocemos; si no, la nuestra.
            let version = params
                .get("protocolVersion")
                .and_then(Value::as_str)
                .filter(|v| matches!(*v, "2024-11-05" | "2025-03-26" | "2025-06-18"))
                .unwrap_or("2024-11-05");
            Ok(json!({
                "protocolVersion": version,
                "serverInfo": { "name": "lodestar-mcp", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "tools": {} },
                "instructions": SERVER_INSTRUCTIONS
            }))
        }
        // El spec obliga a responder a ping con result vacío ("MUST respond promptly").
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::available_tools(profile) })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            if !tools::available(profile, name) {
                // Tool no disponible bajo este perfil = error de protocolo (`-32602`): tool
                // desconocida, o tool de cambio invocada bajo `readonly`. Ocultarla de
                // `tools/list` no basta — un cliente que la llame igualmente NO debe ejecutarla
                // (E14-H03). El código `-32602` la deja fuera del despacho antes de `call()`.
                Err((-32602, format!("tool desconocida: {name}")))
            } else {
                match tools::call(app, profile, name, &args) {
                    // `structuredContent` debe ser un objeto: las tools ya devuelven objetos.
                    Ok(v) => Ok(json!({
                        "content": [{ "type": "text", "text": v.to_string() }],
                        "structuredContent": v
                    })),
                    // Error de EJECUCIÓN de la tool: va en el result con isError, no como error
                    // JSON-RPC — así el modelo lo ve y puede corregir, sin que el cliente lo
                    // trate como fallo de transporte.
                    Err(e) => Ok(json!({
                        "content": [{ "type": "text", "text": e }],
                        "isError": true
                    })),
                }
            }
        }
        other => Err((-32601, format!("método no soportado: {other}"))),
    };

    Some(match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err((code, message)) => rpc_error(id, code, &message),
    })
}
