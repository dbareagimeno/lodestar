//! `lodestar` — fachada CLI (`ARCHITECTURE.md §7.3`). Puerta de CI con exit codes congelados.
//!
//! Cada subcomando resuelve el root → construye el `DocumentSet` (efímero, sobre el core) → serializa.
//! **Cero lógica de dominio aquí**: toda la semántica vive en `lodestar-core`.
//!
//! Exit codes (congelados): `0` conforme · `1` hard-fail · `2` uso · `3` runtime/IO. El `4` (drift
//! de generadores) se retiró con los generadores en E15-H02: sin `index`/`tags` no hay drift.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod commands;
mod sarif;

/// Motor de integridad semántica para bases de conocimiento Markdown — línea de comandos.
#[derive(Parser)]
#[command(name = "lodestar", version, about)]
struct Cli {
    /// Raíz del workspace (por defecto: el directorio actual; nunca se asciende a los ancestros).
    #[arg(long, global = true)]
    path: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// La puerta de CI: ¿es conforme el workspace? (exit 0/1). Juzga siempre el **working tree**
    /// (scope workspace) — git salió de la superficie en E9-H02 y del repo en E15-H01, así que no
    /// hay `--staged`/`--rev`/`--range` que valgan.
    Check {
        /// Salida JSON (el `Analysis` serializado).
        #[arg(long, conflicts_with = "sarif")]
        json: bool,
        /// Salida SARIF 2.1.0 (para integraciones de CI).
        #[arg(long)]
        sarif: bool,
    },
    /// Reconstruye la cache `.lodestar/index.db` desde los `.md`.
    Reindex,
}

/// Resuelve la raíz del workspace: `--path` si se da, si no el **cwd tal cual**
/// (`ARCHITECTURE.md §20.5`, E15-H06).
///
/// **No asciende por los ancestros**: el ascenso buscando `index.md`/`.lodestar` desaparece con la
/// unidad «workspace» — la raíz es el directorio donde se invoca, y lo que se juzga es ese directorio,
/// nunca un proyecto que lo contenga.
fn resolve_root(explicit: Option<&Path>) -> PathBuf {
    match explicit {
        Some(p) => p.to_path_buf(),
        None => std::env::current_dir().unwrap_or_else(|_| PathBuf::from(".")),
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    let root = resolve_root(cli.path.as_deref());
    let result = match cli.command {
        Command::Check { json, sarif } => commands::check(&root, json, sarif),
        Command::Reindex => commands::reindex(&root),
    };
    match result {
        Ok(code) => code,
        Err(e) => runtime_err(&e.to_string()),
    }
}

/// Imprime un error de runtime/IO y devuelve el exit code 3.
fn runtime_err(msg: &str) -> ExitCode {
    eprintln!("error: {msg}");
    ExitCode::from(3)
}
