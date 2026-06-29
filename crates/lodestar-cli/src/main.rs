//! `lodestar` — fachada CLI (`ARCHITECTURE.md §7.3`). Puerta de CI con exit codes congelados.
//!
//! Cada subcomando resuelve el root → construye el `Bundle` (efímero, sobre el core) → serializa.
//! **Cero lógica OKF aquí**: toda la semántica vive en `lodestar-core`.
//!
//! Exit codes (congelados): `0` conforme · `1` hard-fail · `2` uso · `3` runtime/IO · `4` drift de generadores.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::{Parser, Subcommand};

mod bundle_io;
mod commands;
mod git;
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
    /// Inicializa un bundle nuevo (index raíz + `.gitignore`).
    Init { dir: Option<PathBuf> },
    /// La puerta de CI: ¿es conforme el bundle? (exit 0/1).
    Check {
        /// Salida JSON (el `Analysis` serializado).
        #[arg(long)]
        json: bool,
        /// Salida SARIF 2.1.0 (para integraciones de CI).
        #[arg(long)]
        sarif: bool,
        /// Juzga el árbol staged en git (pendiente E4).
        #[arg(long)]
        staged: bool,
        /// Juzga el árbol de un commit (pendiente E4).
        #[arg(long)]
        rev: Option<String>,
        /// Juzga un rango de commits (pendiente E4).
        #[arg(long)]
        range: Option<String>,
    },
    /// Genera el `index.md` de un directorio (o `--check` para detectar drift, exit 4).
    Index {
        dir: Option<String>,
        #[arg(long)]
        check: bool,
    },
    /// Genera/purga los índices de tags (o `--check`, exit 4).
    Tags {
        #[arg(long)]
        check: bool,
    },
    /// Exporta el bundle a un `.zip`.
    Export {
        #[arg(long)]
        out: Option<PathBuf>,
    },
    /// Reconstruye la cache (pendiente E5: requiere la workspace).
    Reindex,
    /// Importa un bundle del prototipo (localStorage) — pendiente E8.
    Import { source: Option<PathBuf> },
    /// Historial de commits (con conformidad).
    Log {
        #[arg(long, default_value_t = 20)]
        limit: usize,
    },
    /// Último commit conforme.
    LastConforming,
    /// Lista las ramas locales.
    Branch,
    /// Pull del upstream (`git pull --ff-only`).
    Pull,
    /// Push al upstream configurado.
    Push,
    /// Diff entre dos revisiones (pendiente E6: render del OkfDiff).
    Diff,
    /// Merge de una rama (pendiente: switch/merge target en vcs).
    Merge,
    /// Instala los hooks de git (pendiente).
    Hooks,
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
        Command::Init { dir } => commands::init(dir.unwrap_or(root)),
        Command::Check {
            json,
            sarif,
            staged,
            rev,
            range,
        } => {
            if staged || rev.is_some() || range.is_some() {
                return runtime_err("check --staged/--rev/--range se implementa en E4 (vcs)");
            }
            commands::check(&root, json, sarif)
        }
        Command::Index { dir, check } => commands::index(&root, dir.unwrap_or_default(), check),
        Command::Tags { check } => commands::tags(&root, check),
        Command::Export { out } => commands::export(&root, out),
        Command::Reindex => {
            eprintln!("reindex: pendiente E5 (requiere la workspace).");
            Ok(ExitCode::SUCCESS)
        }
        Command::Import { .. } => {
            eprintln!("import: pendiente E8 (migración del prototipo).");
            Ok(ExitCode::SUCCESS)
        }
        Command::Log { limit } => git::log(&root, limit),
        Command::LastConforming => git::last_conforming(&root),
        Command::Branch => git::branch(&root),
        Command::Pull => git::sync(&root, false),
        Command::Push => git::sync(&root, true),
        Command::Diff | Command::Merge | Command::Hooks => {
            eprintln!("Subcomando de git: pendiente (ver requirements E4/E6).");
            Ok(ExitCode::SUCCESS)
        }
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
