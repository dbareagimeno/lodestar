//! `lodestar` — fachada CLI (`ARCHITECTURE.md §7.3`). Puerta de CI con exit codes congelados.
//!
//! Cada subcomando resuelve el root → construye el `Bundle` (efímero, sobre el core) → serializa.
//! **Cero lógica OKF aquí**: toda la semántica vive en `lodestar-core`.
//!
//! Exit codes (congelados): `0` conforme · `1` hard-fail · `2` uso · `3` runtime/IO. El `4` (drift
//! de generadores) se retiró con los generadores en E15-H02: sin `index`/`tags` no hay drift.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod commands;
mod sarif;

/// Editor local-first de bases de conocimiento OKF — interfaz de línea de comandos.
#[derive(Parser)]
#[command(name = "lodestar", version, about)]
struct Cli {
    /// Raíz del bundle (por defecto: el directorio actual o el ancestro con `index.md`/`.lodestar`).
    #[arg(long, global = true)]
    path: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// La puerta de CI: ¿es conforme el bundle? (exit 0/1). Juzga siempre el **working tree**
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

/// Resuelve el root del bundle: `--path`, o sube desde el cwd buscando `index.md`/`.lodestar`.
fn resolve_root(explicit: Option<&Path>) -> PathBuf {
    if let Some(p) = explicit {
        return p.to_path_buf();
    }
    let cwd = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let mut cur = cwd.as_path();
    loop {
        if cur.join("index.md").is_file() || cur.join(".lodestar").is_dir() {
            return cur.to_path_buf();
        }
        match cur.parent() {
            Some(p) => cur = p,
            None => return cwd,
        }
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
