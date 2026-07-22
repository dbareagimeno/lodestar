//! Servidor MCP de lodestar (`ARCHITECTURE.md §7.2`).
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

/// Parsea `<bundle> [--profile readonly|standard]`: el bundle es el primer argumento
/// posicional (sin tocar); `--profile` es una flag adicional, `standard` por defecto
/// (`ARCHITECTURE.md §19.6`).
fn parse_args() -> (PathBuf, Profile) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut root = None;
    let mut profile = Profile::Standard;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
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
            other if root.is_none() => root = Some(PathBuf::from(other)),
            _ => {}
        }
        i += 1;
    }
    let root =
        root.unwrap_or_else(|| std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")));
    (root, profile)
}

fn main() {
    let (root, profile) = parse_args();

    // Un bundle de verdad tiene `index.md` o `.lodestar/`: sin esta comprobación el servidor
    // arrancaría "feliz" sobre un directorio arbitrario y `create_concept` escribiría donde caiga.
    if !root.join("index.md").is_file() && !root.join(".lodestar").is_dir() {
        eprintln!(
            "lodestar-mcp: {} no es un bundle lodestar (falta index.md o .lodestar/)",
            root.display()
        );
        std::process::exit(3);
    }
    let app = match App::open(&root) {
        Ok(app) => app,
        Err(e) => {
            eprintln!("lodestar-mcp: no se pudo abrir el bundle: {e}");
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
                "capabilities": { "tools": {} }
            }))
        }
        // El spec obliga a responder a ping con result vacío ("MUST respond promptly").
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({ "tools": tools::list() })),
        "tools/call" => {
            let name = params.get("name").and_then(Value::as_str).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(Value::Null);
            if !tools::exists(name) {
                // Tool desconocida = error de protocolo (Invalid params, según el spec MCP).
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
