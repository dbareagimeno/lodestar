//! Implementación de los subcomandos. Shells finos sobre `lodestar-core`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use lodestar_core::types::{Mutation, RelPath, Severity};
use lodestar_core::Bundle;

use crate::bundle_io::{self, load_bundle};
use crate::sarif;

const DRIFT: u8 = 4;

/// `lodestar check`: analiza y decide conformidad (exit 0/1).
pub fn check(root: &Path, json: bool, sarif_out: bool) -> anyhow::Result<ExitCode> {
    let files = load_bundle(root)?;
    let bundle = Bundle::from_files(files);
    let analysis = bundle.analyze();

    if json {
        println!("{}", serde_json::to_string_pretty(analysis)?);
    } else if sarif_out {
        println!("{}", sarif::to_sarif(analysis)?);
    } else {
        print_human(analysis);
    }

    // Strictness por defecto: solo Err bloquea. (La lectura de lodestar.toml es E8-H01.)
    if analysis.hard_fail > 0 {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn print_human(a: &lodestar_core::types::Analysis) {
    let mut errs = 0usize;
    let mut warns = 0usize;
    for (path, checks) in &a.per_file {
        for c in checks {
            match c.level {
                Severity::Err => {
                    errs += 1;
                    println!("  ✗ [{}] {}: {}", c.code.as_str(), path, c.msg);
                }
                Severity::Warn => {
                    warns += 1;
                    println!("  ! [{}] {}: {}", c.code.as_str(), path, c.msg);
                }
                _ => {}
            }
        }
    }
    println!(
        "\n{} concepts · {} con errores · {} avisos · {}",
        a.concepts.len(),
        a.hard_fail,
        warns,
        if a.hard_fail == 0 {
            "CONFORME"
        } else {
            "NO CONFORME"
        }
    );
    let _ = errs;
}

/// `lodestar index [dir] [--check]`.
pub fn index(root: &Path, dir: String, check: bool) -> anyhow::Result<ExitCode> {
    let dir = if dir.is_empty() || dir.ends_with('/') {
        dir
    } else {
        format!("{dir}/")
    };
    let bundle = Bundle::from_files(load_bundle(root)?);
    let mutation = bundle.gen_index(&dir);
    apply_or_check(root, &mutation, check)
}

/// `lodestar tags [--check]`.
pub fn tags(root: &Path, check: bool) -> anyhow::Result<ExitCode> {
    let bundle = Bundle::from_files(load_bundle(root)?);
    let mutation = bundle.gen_tag_indexes();
    apply_or_check(root, &mutation, check)
}

/// Aplica una `Mutation` o, con `--check`, devuelve exit 4 si hay drift contra disco.
fn apply_or_check(root: &Path, mutation: &Mutation, check: bool) -> anyhow::Result<ExitCode> {
    if check {
        let mut drift = false;
        for (path, content) in &mutation.writes {
            let on_disk = std::fs::read_to_string(root.join(path.as_str())).ok();
            if on_disk.as_deref() != Some(content.as_str()) {
                eprintln!("drift: {} está desactualizado", path);
                drift = true;
            }
        }
        for path in &mutation.deletes {
            if root.join(path.as_str()).exists() {
                eprintln!("drift: {} debería eliminarse", path);
                drift = true;
            }
        }
        return Ok(if drift {
            ExitCode::from(DRIFT)
        } else {
            ExitCode::SUCCESS
        });
    }
    for (path, content) in &mutation.writes {
        bundle_io::write_atomic(root, path, content)?;
        println!("escrito {}", path);
    }
    for path in &mutation.deletes {
        bundle_io::delete(root, path)?;
        println!("eliminado {}", path);
    }
    Ok(ExitCode::SUCCESS)
}

/// `lodestar export [--out file.zip]`.
pub fn export(root: &Path, out: Option<PathBuf>) -> anyhow::Result<ExitCode> {
    let bundle = Bundle::from_files(load_bundle(root)?);
    let out = out.unwrap_or_else(|| root.join("bundle.zip"));
    let file = std::fs::File::create(&out).with_context(|| format!("creando {}", out.display()))?;
    bundle
        .export_zip(file)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!("exportado a {}", out.display());
    Ok(ExitCode::SUCCESS)
}

/// `lodestar init [dir]`: scaffold mínimo de un bundle (index raíz + `.gitignore`).
///
/// El `git init` + commit inicial se completa en E4 (vcs); aquí solo el scaffold de ficheros.
pub fn init(dir: PathBuf) -> anyhow::Result<ExitCode> {
    std::fs::create_dir_all(&dir).with_context(|| format!("creando {}", dir.display()))?;
    let index = dir.join("index.md");
    if !index.exists() {
        std::fs::write(&index, "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n")?;
    }
    let gitignore = dir.join(".gitignore");
    if !gitignore.exists() {
        std::fs::write(&gitignore, "/.lodestar/\n*.db\n*.db-shm\n*.db-wal\n")?;
    }
    let _ = RelPath::new("index.md"); // documenta el invariante de paths
    println!("bundle inicializado en {}", dir.display());
    eprintln!("nota: `git init` + commit inicial se cablea en E4 (vcs).");
    Ok(ExitCode::SUCCESS)
}
