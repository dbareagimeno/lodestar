//! Fase roja de reparación de revisión para E33-H04.
//!
//! Estos tests fijan dos guardas ejecutables que faltan en el banco.  La interfaz es deliberadamente
//! interna: el implementador debe añadirla al binario, pero esta fase solo registra el contrato de
//! prueba.  Los casos positivos y negativos viven en el mismo test para que una implementación
//! vacía no pueda quedar verde por no ejecutar el camino relevante.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use tempfile::TempDir;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("repo root")
}

fn bench() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"));
    command.env("RUST_BACKTRACE", "1");
    command
}

fn run(output: Output, context: &str) -> Output {
    assert!(
        output.status.success(),
        "{context}: la guarda ejecutable debe aceptar el caso válido; status={} stderr={} stdout={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    output
}

fn copy_file(source: &Path, target: &Path) {
    fs::copy(source, target).unwrap_or_else(|error| {
        panic!(
            "copiar fixture {} a {}: {error}",
            source.display(),
            target.display()
        )
    });
}

/// Reproducción de la revisión: stdout/transcript versionados y el bloque oficial deben formar
/// una cadena verificable.  En particular, `payload_bytes_stdout` y
/// `payload_bytes_structured_content` tienen unidades inequívocas y ambos se derivan de raw.
#[test]
fn wire_calibration_chain_guard_derives_official_block_and_rejects_mutation() {
    let root = repo_root();
    let evidence = root.join("docs/qa/e33-h04-wire-evidencia-2026-08-22.json");
    let official = root.join("crates/lodestar-bench/tests/fixtures/e33_h04_full_format.json");
    assert!(evidence.is_file(), "falta evidencia wire versionada");
    assert!(official.is_file(), "falta corrida oficial versionada");

    let temp = TempDir::new().expect("directorio de mutaciones wire");
    let evidence_copy = temp.path().join("wire-evidencia.json");
    let official_copy = temp.path().join("corrida.json");
    copy_file(&evidence, &evidence_copy);
    copy_file(&official, &official_copy);

    let valid = bench()
        .args([
            "--validate-wire-calibration-chain",
            "--wire-evidence",
            evidence_copy.to_str().expect("evidence path UTF-8"),
            "--official-report",
            official_copy.to_str().expect("official path UTF-8"),
        ])
        .output()
        .expect("ejecutar guarda wire válida");
    run(valid, "wire chain válida");

    let mut mutated: Value = serde_json::from_str(
        &fs::read_to_string(&official_copy).expect("leer corrida oficial mutada"),
    )
    .expect("corrida oficial JSON");
    let result = mutated["wire_calibration"]["results"][0]
        .as_object_mut()
        .expect("resultado workspace_status");
    let payload = result
        .get_mut("payload_bytes_structured_content")
        .expect("payload estructurado explícito en bloque oficial");
    *payload = Value::from(payload.as_u64().expect("payload entero") + 1);
    fs::write(
        &official_copy,
        serde_json::to_vec_pretty(&mutated).expect("serializar mutación oficial"),
    )
    .expect("escribir mutación oficial");

    let rejected = bench()
        .args([
            "--validate-wire-calibration-chain",
            "--wire-evidence",
            evidence_copy.to_str().expect("evidence path UTF-8"),
            "--official-report",
            official_copy.to_str().expect("official path UTF-8"),
        ])
        .output()
        .expect("ejecutar guarda wire mutada");
    assert!(
        !rejected.status.success(),
        "wire chain mutada: la guarda debe rechazar payload estructurado alterado; stdout={} stderr={}",
        String::from_utf8_lossy(&rejected.stdout),
        String::from_utf8_lossy(&rejected.stderr)
    );
}

fn manifest(temp: &TempDir, body: &str) -> PathBuf {
    let path = temp.path().join("Cargo.toml");
    let prefix = "[package]\nname = \"synthetic-a6\"\nversion = \"0.0.0\"\nedition = \"2021\"\n\n";
    fs::write(&path, format!("{prefix}{body}")).expect("manifest sintético");
    fs::create_dir(temp.path().join("src")).expect("src sintético");
    fs::write(temp.path().join("src/main.rs"), "fn main() {}\n").expect("main sintético");
    path
}

fn check_a6(path: &Path) -> Output {
    let cwd = path.parent().expect("directorio del manifest");
    let git_probe = Command::new("git")
        .args(["rev-parse", "--show-toplevel"])
        .current_dir(cwd)
        .output()
        .expect("comprobar cwd fuera de Git");
    assert!(
        !git_probe.status.success(),
        "anti-vacuidad: --manifest debe ejercitarse desde TempDir fuera de Git; cwd={}",
        cwd.display()
    );
    bench()
        .args([
            "--check-a6-dependencies",
            "--manifest",
            path.to_str().expect("manifest path UTF-8"),
        ])
        .current_dir(cwd)
        .output()
        .expect("ejecutar guarda A6")
}

/// La guarda A6 debe inspeccionar el package real, no una clave textual concreta: cargo metadata
/// representa una dependencia normal con `kind: null`, y Cargo permite alias y tablas target.
#[test]
fn a6_dependency_guard_rejects_normal_alias_and_target_direct_store_dependencies() {
    let store = repo_root().join("crates/lodestar-store");
    let store = store.to_str().expect("store path UTF-8");

    let clean_temp = TempDir::new().expect("manifest limpio");
    let clean = manifest(&clean_temp, "[dependencies]\n");
    run(check_a6(&clean), "A6 manifest limpio");

    let normal_temp = TempDir::new().expect("manifest normal");
    let normal = manifest(
        &normal_temp,
        &format!("[dependencies]\nlodestar-store = {{ path = \"{store}\" }}\n"),
    );
    assert!(
        !check_a6(&normal).status.success(),
        "A6 debe morder dependencia normal"
    );

    let alias_temp = TempDir::new().expect("manifest alias");
    let alias = manifest(
        &alias_temp,
        &format!(
            "[dependencies]\nstore_facade = {{ package = \"lodestar-store\", path = \"{store}\" }}\n"
        ),
    );
    assert!(
        !check_a6(&alias).status.success(),
        "A6 debe morder alias package=lodestar-store"
    );

    let target_temp = TempDir::new().expect("manifest target");
    let target = manifest(
        &target_temp,
        &format!(
            "[target.'cfg(unix)'.dependencies]\nstore_target = {{ package = \"lodestar-store\", path = \"{store}\" }}\n"
        ),
    );
    assert!(
        !check_a6(&target).status.success(),
        "A6 debe morder dependencia target-specific"
    );
}
