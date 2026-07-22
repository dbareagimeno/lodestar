//! Implementación de los subcomandos. Shells finos sobre `lodestar-core`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use anyhow::Context;
use lodestar_core::types::{Mutation, RelPath, Severity};
use lodestar_core::Bundle;

use crate::bundle_io::{self, load_bundle};
use crate::sarif;

const DRIFT: u8 = 4;

/// `lodestar check`: la puerta de CI sobre el **working tree**. Juzga la conformidad **completa**
/// (OKF + `SCHEMA-*` + `REL-*` + refs externas) con el **mismo motor** que
/// [`lodestar_app::App::knowledge_check`] scope `workspace` (invariante #3 — una sola verdad
/// computada): ambos comparten la fusión OKF+schema-driven de
/// `App::schema_diagnostics_by_path`. La CLI es una fachada fina y **no** recompone la validación
/// schema-driven a mano.
///
/// La salida (`--json`/`--sarif`/humano) se renderiza desde un **único** `Analysis` completo
/// ([`lodestar_app::App::full_analysis`], un solo `analyze()`): así los diagnósticos `SCHEMA-*`/
/// `REL-*` que disparan el fallo se **surface an** en el wire, no solo el veredicto. El veredicto
/// `conformant` (== sin `Err` entre los conceptos, misma semántica que `knowledge_check`) decide el
/// bloqueo; además, `lodestar.toml` puede endurecer la puerta bloqueando también avisos
/// (`gate.block_warnings`). Exit codes congelados: `0` conforme · `1` bloqueado · `3` runtime/IO
/// (error del servicio o `lodestar.toml` inválido).
pub fn check(root: &Path, json: bool, sarif_out: bool) -> anyhow::Result<ExitCode> {
    // Motor completo (OKF + schema-driven) en `App`, computado UNA sola vez: `full_analysis` corre
    // `analyze()` y fusiona los `SCHEMA-*`/`REL-*` en `per_file` con la misma lógica que
    // `knowledge_check`. De ese `Analysis` sale tanto el veredicto como la salida.
    let app = lodestar_app::App::open(root).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let analysis = app
        .full_analysis()
        .map_err(|e| anyhow::anyhow!(e.as_str().to_string()))?;

    // Veredicto == sin `Err` entre los conceptos, misma semántica que `knowledge_check` scope
    // `workspace` (que también itera `analysis.concepts`): así la fachada no reimplementa el motor,
    // solo lee el mismo `Analysis`.
    let conformant = !analysis
        .concepts
        .iter()
        .filter_map(|p| analysis.per_file.get(p))
        .flatten()
        .any(|c| c.level == Severity::Err);

    // Un `lodestar.toml` inválido es exit 3, NO defaults silenciosos: con `block_warnings=true`
    // y un typo TOML, la puerta de CI se relajaría sin ningún aviso. El motor ya cubre los `Err`
    // (incluidos los schema-driven) vía `conformant`; `lodestar.toml` solo puede endurecer la
    // puerta para que los avisos OKF también bloqueen.
    let blocked = !conformant
        || lodestar_workspace::Config::load(root)
            .map_err(|e| anyhow::anyhow!(e))?
            .gate_blocked(&analysis);
    render_analysis(&analysis, conformant, json, sarif_out, blocked)
}

/// Imprime un `Analysis` en el formato pedido y devuelve el exit code (0 conforme / 1 bloqueado).
/// `conformant` es el veredicto del motor completo (misma semántica que `knowledge_check`), que en
/// `--json` se emite de forma ADITIVA junto a los campos históricos del `Analysis`; los `SCHEMA-*`/
/// `REL-*` viajan dentro de `per_file` (y por tanto en `--sarif`/humano) al haberse fusionado en
/// [`lodestar_app::App::full_analysis`]. `blocked` decide el exit code: lo endurece la strictness de
/// `lodestar.toml` (avisos OKF) sobre el veredicto del motor.
pub fn render_analysis(
    analysis: &lodestar_core::types::Analysis,
    conformant: bool,
    json: bool,
    sarif_out: bool,
    blocked: bool,
) -> anyhow::Result<ExitCode> {
    if json {
        // Aditivo: serializar el `Analysis` (conserva `concepts`/`hardFail`/… en el wire) y añadir
        // el veredicto `conformant` del motor completo sin tocar el resto del objeto.
        let mut value = serde_json::to_value(analysis)?;
        if let serde_json::Value::Object(map) = &mut value {
            map.insert(
                "conformant".to_string(),
                serde_json::Value::Bool(conformant),
            );
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
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
        // `errs` cuenta TODOS los diagnósticos `Err` de `per_file` (OKF + `SCHEMA-*`/`REL-*`
        // fusionados en `full_analysis`), no solo `a.hard_fail` (OKF): así el resumen humano es
        // coherente con las líneas `✗` de arriba.
        errs,
        warns,
        if blocked {
            // Cubre también el gate estricto: no imprimir «CONFORME» con exit code 1.
            if errs == 0 {
                "NO CONFORME (avisos bloqueados por lodestar.toml)"
            } else {
                "NO CONFORME"
            }
        } else {
            "CONFORME"
        }
    );
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

/// `lodestar reindex`: reconstruye la cache `.lodestar/index.db` desde disco. No es un subcomando
/// git (`§13`): la cache es una vista derivada del bundle, independiente de si hay repo git o no.
pub fn reindex(root: &Path) -> anyhow::Result<ExitCode> {
    let mut ws =
        lodestar_workspace::Workspace::open(root).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    ws.enable_cache()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!(
        "cache reconstruida en {}",
        root.join(".lodestar/index.db").display()
    );
    Ok(ExitCode::SUCCESS)
}
