//! Implementación de los subcomandos. Shells finos sobre `lodestar-core`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use lodestar_core::types::{Mutation, RelPath, Severity};
use lodestar_core::Bundle;

use crate::bundle_io::{self, load_bundle};
use crate::sarif;

const DRIFT: u8 = 4;

/// `lodestar check`: analiza y decide conformidad (exit 0/1), con la strictness de `lodestar.toml`.
pub fn check(root: &Path, json: bool, sarif_out: bool) -> anyhow::Result<ExitCode> {
    let files = load_bundle(root)?;
    let bundle = Bundle::from_files(files);
    // Un `lodestar.toml` inválido es exit 3, NO defaults silenciosos: con `block_warnings=true`
    // y un typo TOML, la puerta de CI se relajaría sin ningún aviso.
    let blocked = lodestar_workspace::Config::load(root)
        .map_err(|e| anyhow::anyhow!(e))?
        .gate_blocked(bundle.analyze());
    render_analysis(bundle.analyze(), json, sarif_out, blocked)
}

/// Imprime un `Analysis` en el formato pedido y devuelve el exit code (0 conforme / 1 bloqueado).
/// `blocked` lo decide la strictness de `lodestar.toml`. Reutilizado por `check`, `--staged` y `--rev`.
pub fn render_analysis(
    analysis: &lodestar_core::types::Analysis,
    json: bool,
    sarif_out: bool,
    blocked: bool,
) -> anyhow::Result<ExitCode> {
    if json {
        println!("{}", serde_json::to_string_pretty(analysis)?);
    } else if sarif_out {
        println!("{}", sarif::to_sarif(analysis)?);
    } else {
        print_human(analysis, blocked);
    }
    if blocked {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn print_human(a: &lodestar_core::types::Analysis, blocked: bool) {
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
        if blocked {
            // Cubre también el gate estricto: no imprimir «CONFORME» con exit code 1.
            if a.hard_fail == 0 {
                "NO CONFORME (avisos bloqueados por lodestar.toml)"
            } else {
                "NO CONFORME"
            }
        } else {
            "CONFORME"
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

/// `lodestar init [dir]`: scaffold de un bundle (index raíz + `.gitignore`) + `git init` + commit inicial.
pub fn init(dir: PathBuf) -> anyhow::Result<ExitCode> {
    // El scaffold vive UNA vez en la workspace (lo comparte el first-run del escritorio).
    let had_git = dir.join(".git").exists();
    lodestar_workspace::Workspace::init_bundle(&dir).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    if !had_git {
        println!("git inicializado (commit inicial creado)");
    }
    println!("bundle inicializado en {}", dir.display());
    Ok(ExitCode::SUCCESS)
}

/// `lodestar import <source>`: migra un export del prototipo (un `.zip` de `path→.md`, o un
/// directorio de `.md`) al bundle. `RelPath` es el chokepoint anti zip-slip (`§12` seguridad).
pub fn import(root: &Path, source: PathBuf) -> anyhow::Result<ExitCode> {
    let imported = if source.is_dir() {
        import_dir(root, &source)?
    } else {
        import_zip(root, &source)?
    };
    println!("importados {imported} ficheros a {}", root.display());
    Ok(ExitCode::SUCCESS)
}

fn import_zip(root: &Path, zip_path: &Path) -> anyhow::Result<usize> {
    let file = std::fs::File::open(zip_path)
        .with_context(|| format!("abriendo {}", zip_path.display()))?;
    let mut archive = zip::ZipArchive::new(file).context("leyendo el .zip")?;
    let mut count = 0;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        if !entry.is_file() {
            continue;
        }
        let name = entry.name().replace('\\', "/");
        if !name.ends_with(".md") {
            continue;
        }
        // Chokepoint anti zip-slip: rutas absolutas / `..` se rechazan.
        let rp = match RelPath::new(&name) {
            Ok(rp) => rp,
            Err(_) => {
                eprintln!("aviso: se ignora una ruta no segura del zip: {name}");
                continue;
            }
        };
        let mut content = String::new();
        std::io::Read::read_to_string(&mut entry, &mut content)
            .with_context(|| format!("leyendo {name} del zip"))?;
        bundle_io::write_atomic(root, &rp, &content)?;
        count += 1;
    }
    Ok(count)
}

fn import_dir(root: &Path, dir: &Path) -> anyhow::Result<usize> {
    let files = load_bundle(dir)?;
    let mut count = 0;
    for (rp, content) in &files {
        bundle_io::write_atomic(root, rp, content)?;
        count += 1;
    }
    Ok(count)
}
