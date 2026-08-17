//! Servidor MCP de lodestar (`ARCHITECTURE.md §7.2`).
//!
//! La fachada conserva `--root`/`--profile`, pero el framing y el ciclo de vida pertenecen al
//! transporte oficial rmcp. stdout queda reservado al JSON-RPC; los mensajes de arranque van a
//! stderr.

use std::{error::Error, path::PathBuf};

use lodestar_app::{App, Profile};
use lodestar_mcp::{LodestarMcpServer, LodestarMcpService, SerialExecutor};
use rmcp::ServiceExt;

/// Texto de uso (a stderr: stdout es JSON-RPC puro y nada más).
const USAGE: &str = "\\
Uso: lodestar-mcp [--root <dir>] [--profile readonly|standard]

  --root <dir>       Raíz del workspace. Por defecto: el directorio actual (`cwd`).
  --profile <perfil> «standard» (por defecto) o «readonly» (sin las tools de cambio).
  -h, --help         Muestra esta ayuda.";

/// Parsea `[--root <dir>] [--profile readonly|standard]` (`ARCHITECTURE.md §20.5`).
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    let (root, profile) = parse_args();

    // La raíz se canonicaliza una sola vez al arrancar y queda fija toda la sesión.
    let root = match std::fs::canonicalize(&root) {
        Ok(path) => path,
        Err(error) => {
            eprintln!(
                "lodestar-mcp: no se pudo resolver la raíz {}: {error}",
                root.display()
            );
            std::process::exit(3);
        }
    };
    let app = match App::open(&root) {
        Ok(app) => app,
        Err(error) => {
            eprintln!("lodestar-mcp: no se pudo abrir el workspace: {error}");
            std::process::exit(3);
        }
    };
    eprintln!(
        "lodestar-mcp: escuchando JSON-RPC en stdio (root={}, profile={profile:?})",
        root.display()
    );

    let service = LodestarMcpService::new(app, profile);
    let server: SerialExecutor<LodestarMcpService> = SerialExecutor::new(service);
    let server = LodestarMcpServer::new(server);
    server
        .serve(rmcp::transport::stdio())
        .await?
        .waiting()
        .await?;
    Ok(())
}
