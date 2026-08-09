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
mod validacion;

/// Versiones de `protocolVersion` que el servidor soporta de verdad, en el orden que declara
/// `contracts/mcp.yml` (`meta.protocolo.protocol_versions_aceptadas`). Única fuente de esta lista:
/// tanto el rechazo (`initialize` con una versión ausente de aquí → `-32602`) como el mensaje que
/// las enumera se generan a partir de esta constante.
const PROTOCOL_VERSIONS: [&str; 3] = ["2024-11-05", "2025-03-26", "2025-06-18"];

/// Versión por defecto que responde el servidor cuando el cliente no pide ninguna.
const DEFAULT_PROTOCOL_VERSION: &str = "2024-11-05";

/// Preámbulo fijo de las instrucciones del servidor (no depende del perfil).
const SERVER_INSTRUCTIONS_PREAMBULO: &str = "\
Motor headless de integridad semántica para agentes. Opera sobre la red de documentos Markdown de \
un proyecto cualquiera: no exige estructura previa, ningún nombre de fichero activa reglas \
especiales, el frontmatter es YAML arbitrario tuyo y todas las rutas son relativas a la raíz.";

/// Un paso del flujo recomendado: la tool que introduce y el texto que lo describe (sin numerar —
/// la numeración se genera al componer, para que sobreviva a que un perfil filtre pasos).
struct Paso {
    tool: &'static str,
    texto: &'static str,
}

/// Los 10 pasos del flujo recomendado, en orden, cada uno atado a la tool que introduce.
///
/// **Fuente única de qué tools se nombran** (invariante #3, criterio estructural de E29-H09): el
/// texto de `instructions` se compone filtrando esta tabla por [`tools::available`] — el MISMO
/// predicado que decide `tools/list` (`tools::available_tools`) — así que el conjunto de tools
/// nombradas nunca puede divergir del servido. No hay una segunda lista de nombres escrita a mano.
const PASOS: [Paso; 10] = [
    Paso { tool: "workspace_status", texto: "oriéntate primero — config activa, capacidades del perfil, validez y \
recuento agregado del workspace, recuperación pendiente y los recibos disponibles para revertir." },
    Paso { tool: "knowledge_search", texto: "localiza documentos por texto libre y por consulta tipada (`where`/`filter`); \
con `include: [\"frontmatter.<campo>\"]` proyectas metadata de cada resultado sin pedir el documento \
entero. Devuelve snippets y revisión, nunca cuerpos completos." },
    Paso { tool: "knowledge_get", texto: "lee un documento concreto con `include` selectivo y secciones acotadas por \
`headingPath`." },
    Paso { tool: "metadata_inspect", texto: "descubre las convenciones de metadata de la base (qué campos existen, de qué \
tipos y qué valores toman) sin necesitar un schema, antes de proponer cambios." },
    Paso { tool: "graph_query", texto: "consulta el grafo de enlaces — operaciones `backlinks`, `outgoing`, \
`neighborhood`, `isolated`, `dangling`, `path_between`, `cycles`, `components`." },
    Paso { tool: "impact_analyze", texto: "evalúa el impacto de un cambio hipotético (afectados directos y transitivos, \
riesgo) antes de proponerlo." },
    Paso { tool: "change_plan", texto: "planifica el cambio SIN escribir — normaliza las operaciones, simula en memoria y \
valida el resultado; devuelve un change set con su hash determinista." },
    Paso { tool: "change_apply", texto: "aplica el plan calculado con todas las salvaguardas transaccionales; devuelve el \
recibo." },
    Paso { tool: "knowledge_check", texto: "audita el conocimiento tras aplicar para confirmar que sigue siendo \
interpretable y que sus enlaces siguen resolviendo." },
    Paso { tool: "change_revert", texto: "si algo salió mal, revierte al estado anterior la transacción del `receiptId` \
que te dio `change_apply` (o el que listó `workspace_status`)." },
];

/// Nota final del perfil, apéndice del texto compuesto.
fn nota_de_perfil(profile: Profile) -> &'static str {
    if profile.writes_enabled() {
        "Perfil `standard` (por defecto): el flujo completo."
    } else {
        "Perfil `readonly`: solo los pasos de lectura y verificación (las tools de cambio no están \
disponibles)."
    }
}

/// Instrucciones del servidor (`instructions` de la respuesta `initialize`, `ARCHITECTURE.md
/// §19.6`/`§20.1`): orientan al agente con el flujo recomendado, mencionando en orden las tools que
/// el perfil activo realmente sirve. Los nombres de tool son identificadores (no se traducen); el
/// resto va en español, el idioma del repo (E14-H03).
///
/// **Esto es superficie de WIRE, no documentación**: viaja en la respuesta de `initialize` y es lo
/// primero que lee un agente, así que un nombre de operación o un parámetro que ya no existe aquí
/// se convierte en una llamada fallida del cliente. E23-H13 saldó el drift acumulado
/// (`huérfanos` → la operación se llama `isolated` desde E16-H02; `conformidad` → el wire dice
/// `valid` desde E23-H14). E29-H09 saldó un segundo drift —bajo `readonly` el texto seguía
/// describiendo las 3 tools de cambio, invisibles pero mencionadas— generando el texto **por
/// perfil** a partir de [`PASOS`] filtrado por [`tools::available`], en vez de servir una constante
/// única: el conjunto de tools nombradas queda atado estructuralmente al servido por `tools/list`
/// (`tests/mcp.rs::instructions_sin_vocabulario_retirado` e
/// `instructions_readonly_nombra_solo_las_tools_servidas` lo comprueban por los dos perfiles).
fn server_instructions(profile: Profile) -> String {
    let mut out = String::from(SERVER_INSTRUCTIONS_PREAMBULO);
    out.push_str("\n\nFlujo recomendado en esta sesión, en orden:\n\n");
    let mut n = 0usize;
    for paso in &PASOS {
        if !tools::available(profile, paso.tool) {
            continue;
        }
        n += 1;
        out.push_str(&format!("{n}. `{}`: {}\n", paso.tool, paso.texto));
    }
    out.push('\n');
    out.push_str(nota_de_perfil(profile));
    out
}

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
    // `lodestar init` (`§20.1`). El gate de «esto no es un workspace» se retiró en E15-H06.
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

/// El nombre JSON del tipo de un valor, para mensajes de error que rechazan un tipo del wire
/// (E30-H03 seguimiento 10). Nombra el tipo tal como lo escribe el cliente en su JSON —`number`,
/// `null`, `object`…—, no el tipo interno de `serde_json`, porque el destinatario del mensaje es
/// quien redactó ese JSON.
fn tipo_json(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
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
        // El `match` distingue TRES casos, no dos (E30-H03 seguimiento 10): hasta v0.5.0 el
        // `.and_then(Value::as_str)` colapsaba «clave ausente» con «clave presente con un valor que
        // no es string» —número, `null` explícito, objeto—, y el segundo caía al brazo de ausente:
        // handshake con ÉXITO y versión por defecto ante un tipo que el wire no admite. Ahora la
        // presencia se mira antes que el tipo.
        "initialize" => match params.get("protocolVersion") {
            // Ausente: válida, se responde la versión por defecto del servidor (omitir no es
            // pedir algo imposible).
            None => Ok(json!({
                "protocolVersion": DEFAULT_PROTOCOL_VERSION,
                "serverInfo": { "name": "lodestar-mcp", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "tools": {} },
                "instructions": server_instructions(profile)
            })),
            // Presente y soportada: se ecoa.
            Some(Value::String(v)) if PROTOCOL_VERSIONS.contains(&v.as_str()) => Ok(json!({
                "protocolVersion": v,
                "serverInfo": { "name": "lodestar-mcp", "version": env!("CARGO_PKG_VERSION") },
                "capabilities": { "tools": {} },
                "instructions": server_instructions(profile)
            })),
            // Presente, string y NO soportada: el handshake se rechaza (no se normaliza en silencio
            // a la versión por defecto). Un `initialize` fallido es un handshake fallido, no un
            // error de dominio de tool: `-32602` (mismo código que "tool desconocida"/"tool no
            // disponible bajo este perfil"), sin `result`, y el mensaje lista las aceptadas.
            Some(Value::String(v)) => Err((
                -32602,
                format!(
                    "protocolVersion no soportada: «{v}». Versiones aceptadas: {}",
                    PROTOCOL_VERSIONS.join(", ")
                ),
            )),
            // Presente pero NO string (número, booleano, `null` explícito, lista, objeto): tipo
            // incorrecto en el wire, mismo código que la versión no soportada. Un `null` explícito
            // NO es lo mismo que omitir la clave: quien la manda está declarando algo, y lo que
            // declara no es una versión.
            Some(otro) => Err((
                -32602,
                format!(
                    "protocolVersion debe ser una cadena (string) y llegó un {}: {otro}. \
                     Versiones aceptadas: {}",
                    tipo_json(otro),
                    PROTOCOL_VERSIONS.join(", ")
                ),
            )),
        },
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
