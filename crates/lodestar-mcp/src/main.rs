//! Servidor MCP de lodestar (`ARCHITECTURE.md §7.2`).
//!
//! **Logs solo a stderr; stdout = JSON-RPC.** Bucle de líneas JSON-RPC sobre stdio que despacha
//! a los handlers de [`tools`]. La integración con el transporte oficial `rmcp` (handshake completo,
//! resources, streaming) es el paso de producción de E7; este bucle implementa el subconjunto
//! necesario (`initialize`/`tools/list`/`tools/call`) para usarse desde Claude Code.

use std::io::{BufRead, Write};
use std::path::PathBuf;

use lodestar_workspace::Workspace;
use serde_json::{json, Value};

mod tools;

fn main() {
    let root = std::env::args()
        .nth(1)
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));

    let ws = match Workspace::open(&root) {
        Ok(ws) => ws,
        Err(e) => {
            eprintln!("lodestar-mcp: no se pudo abrir el bundle: {e}");
            std::process::exit(3);
        }
    };
    eprintln!(
        "lodestar-mcp: escuchando JSON-RPC en stdio (root={})",
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
        let req: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("lodestar-mcp: JSON inválido: {e}");
                continue;
            }
        };
        let resp = handle(&ws, &req);
        if let Some(resp) = resp {
            let mut out = stdout.lock();
            let _ = writeln!(out, "{resp}");
            let _ = out.flush();
        }
    }
}

/// Despacha un mensaje JSON-RPC. Devuelve `None` para notificaciones (sin `id`).
fn handle(ws: &Workspace, req: &Value) -> Option<Value> {
    let id = req.get("id").cloned();
    let method = req.get("method").and_then(Value::as_str).unwrap_or("");
    let params = req.get("params").cloned().unwrap_or(Value::Null);

    // Notificaciones (sin id) no llevan respuesta.
    let id = id?;

    let result: Result<Value, (i64, String)> = match method {
        "initialize" => Ok(json!({
            "protocolVersion": "2024-11-05",
            "serverInfo": { "name": "lodestar-mcp", "version": env!("CARGO_PKG_VERSION") },
            "capabilities": { "tools": {} }
        })),
        "tools/list" => Ok(json!({ "tools": tools::list() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            match tools::call(ws, name, &args) {
                Ok(v) => Ok(json!({
                    "content": [{ "type": "text", "text": v.to_string() }],
                    "structuredContent": v
                })),
                Err(e) => Err((-32000, e)),
            }
        }
        other => Err((-32601, format!("método no soportado: {other}"))),
    };

    Some(match result {
        Ok(value) => json!({ "jsonrpc": "2.0", "id": id, "result": value }),
        Err((code, message)) => {
            json!({ "jsonrpc": "2.0", "id": id, "error": { "code": code, "message": message } })
        }
    })
}
