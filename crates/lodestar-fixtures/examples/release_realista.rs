//! Genera el root canónico que usa la calibración wire de un release.
//!
//! El perfil, tamaño y semilla son deliberadamente explícitos para que el root que mide el
//! arnés sea el mismo corpus Realista/10k que define el banco H04. El overlay es el control
//! pequeño que el bench interno añade a cada root y que hace determinista la búsqueda de
//! marker-search-h04.

use std::env;
use std::path::{Path, PathBuf};
use std::process;

use lodestar_fixtures::escala::{self, Perfil};

fn usage() -> ! {
    eprintln!("uso: release_realista <DEST> [SEED]");
    process::exit(2);
}

fn main() {
    let mut args = env::args().skip(1);
    let destination = args.next().map(PathBuf::from).unwrap_or_else(|| usage());
    let seed = args
        .next()
        .map(|raw| raw.parse::<u64>().unwrap_or_else(|_| usage()))
        .unwrap_or(33);
    if args.next().is_some() {
        usage();
    }

    if let Err(error) = generate(&destination, seed) {
        eprintln!(
            "error generando Realista/10000 en {}: {error}",
            destination.display()
        );
        process::exit(1);
    }
}

fn generate(destination: &Path, seed: u64) -> std::io::Result<()> {
    if destination.exists()
        && std::fs::read_dir(destination)?
            .next()
            .transpose()?
            .is_some()
    {
        return Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "el destino debe estar vacío para conservar el corpus canónico",
        ));
    }
    std::fs::create_dir_all(destination)?;
    escala::genera(destination, Perfil::Realista, 10_000, seed)?;

    // Debe permanecer byte a byte alineado con lodestar-bench::overlay_control.
    write(
        destination,
        "control.md",
        "---\ntags: [h04, control]\nservice: bench\n---\n# Control\nmarker-search-h04\n[child](child.md)\n[missing](missing.md)\n",
    )?;
    write(
        destination,
        "child.md",
        "---\ntags: [child]\nservice: bench\n---\n# Child\nmarker-get-h04\n[leaf](leaf.md)\n",
    )?;
    write(
        destination,
        "leaf.md",
        "---\ntags: [leaf]\nservice: bench\n---\n# Leaf\nmarker-impact-h04\n",
    )?;
    write(destination, "broken.md", "---\ntags: [\n---\n# Broken\n")
}

fn write(root: &Path, relative: &str, content: &str) -> std::io::Result<()> {
    std::fs::write(root.join(relative), content)
}
