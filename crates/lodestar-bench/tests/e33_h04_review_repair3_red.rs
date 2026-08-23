//! Fase roja fresca de revisión para E33-H04.
//!
//! Snapshot de alcance: `target/agent-state/e33-h04-review-repair3-red-v15/pre-red.json`.
//! Estos guards cubren los tres mutantes concretos de la revisión: el manifest implícito de A6,
//! el payload stdout del resultado oficial y las cabeceras wire del renderer generado.

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

fn copy_json(source: &Path, destination: &Path) {
    fs::copy(source, destination).unwrap_or_else(|error| {
        panic!(
            "copiar {} a {}: {error}",
            source.display(),
            destination.display()
        )
    });
}

fn assert_rejected(output: Output, label: &str) {
    assert!(
        !output.status.success(),
        "{label}: la guarda debe rechazar la mutación; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// BDD-H04-A6: sin `--manifest`, desde un cwd arbitrario, la guarda debe declarar el manifest
/// canónico de lodestar-app. Un default hacia otro Cargo.toml limpio no satisface este criterio.
#[test]
fn a6_sin_manifest_declara_el_manifest_canonico_de_lodestar_app_desde_cwd_arbitrario() {
    let root = repo_root();
    let canonical = root
        .join("crates/lodestar-app/Cargo.toml")
        .canonicalize()
        .expect("manifest canónico de lodestar-app");
    let cwd = TempDir::new().expect("cwd arbitrario");
    let output = bench()
        .arg("--check-a6-dependencies")
        .current_dir(cwd.path())
        .output()
        .expect("ejecutar guard A6 sin manifest explícito");
    assert!(
        output.status.success(),
        "A6 canónico debe pasar; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.trim().is_empty(),
        "A6 debe emitir en stdout una prueba machine-readable o textual del manifest inspeccionado"
    );
    let canonical_display = canonical.to_string_lossy();
    assert!(
        stdout.contains(canonical_display.as_ref())
            || stdout.contains("crates/lodestar-app/Cargo.toml"),
        "A6 debe declarar exactamente crates/lodestar-app/Cargo.toml; stdout={stdout}"
    );
    assert!(
        !stdout.contains(cwd.path().to_string_lossy().as_ref()),
        "A6 no debe declarar un manifest derivado del cwd arbitrario; stdout={stdout}"
    );
}

/// BDD-H04-wire: una mutación únicamente de `wire_calibration.results[0].payload_bytes_stdout`
/// debe rechazarse, conservando evidencia/raw/legacy/structured y el resto de la fila intactos.
#[test]
fn wire_oficial_rechaza_mutacion_aislada_de_payload_bytes_stdout() {
    let root = repo_root();
    let evidence = root.join("docs/qa/e33-h04-wire-evidencia-2026-08-22.json");
    let official = root.join("crates/lodestar-bench/tests/fixtures/e33_h04_full_format.json");
    assert!(evidence.is_file(), "falta evidencia wire versionada");
    assert!(official.is_file(), "falta corrida oficial versionada");

    let temp = TempDir::new().expect("scratch de mutación wire");
    let evidence_copy = temp.path().join("wire-evidencia.json");
    let official_copy = temp.path().join("corrida.json");
    copy_json(&evidence, &evidence_copy);
    copy_json(&official, &official_copy);
    let evidence_before = fs::read(&evidence_copy).expect("bytes de evidencia antes");

    let original: Value =
        serde_json::from_str(&fs::read_to_string(&official_copy).expect("leer corrida oficial"))
            .expect("corrida oficial JSON");
    let original_result = original["wire_calibration"]["results"][0]
        .as_object()
        .expect("resultado wire oficial")
        .clone();
    let original_stdout_payload = original_result["payload_bytes_stdout"]
        .as_u64()
        .expect("payload stdout entero");
    let mut mutated = original.clone();
    mutated["wire_calibration"]["results"][0]["payload_bytes_stdout"] =
        Value::from(original_stdout_payload + 1);
    let mutated_result = mutated["wire_calibration"]["results"][0]
        .as_object()
        .expect("resultado wire mutado");
    for (key, value) in &original_result {
        if key != "payload_bytes_stdout" {
            assert_eq!(
                mutated_result.get(key),
                Some(value),
                "la mutación debe dejar intacto el campo oficial {key}"
            );
        }
    }
    assert_eq!(
        mutated["wire_calibration"]["results"][0]["payload_bytes"],
        original["wire_calibration"]["results"][0]["payload_bytes"],
        "legacy payload_bytes debe permanecer intacto"
    );
    fs::write(
        &official_copy,
        serde_json::to_vec_pretty(&mutated).expect("serializar mutación aislada"),
    )
    .expect("escribir corrida oficial mutada");

    assert_rejected(
        bench()
            .args([
                "--validate-wire-calibration-chain",
                "--wire-evidence",
                evidence_copy.to_str().expect("evidence UTF-8"),
                "--official-report",
                official_copy.to_str().expect("official UTF-8"),
            ])
            .output()
            .expect("validar payload stdout oficial mutado"),
        "payload_bytes_stdout oficial",
    );
    assert_eq!(
        fs::read(&evidence_copy).expect("bytes de evidencia después"),
        evidence_before,
        "la guarda no debe modificar evidencia/raw/structured"
    );
}

/// BDD-H04-renderer: re-renderizar el informe oficial debe conservar la distinción wire entre el
/// payload completo de stdout y el payload de `structuredContent`, con unidades en las cabeceras.
#[test]
fn renderer_oficial_incluye_cabeceras_wire_de_stdout_y_structured_content() {
    let root = repo_root();
    let official = root.join("crates/lodestar-bench/tests/fixtures/e33_h04_full_format.json");
    assert!(official.is_file(), "falta corrida oficial versionada");
    let temp = TempDir::new().expect("scratch renderer");
    let markdown = temp.path().join("corrida.md");
    let output = bench()
        .args([
            "--render-report",
            official.to_str().expect("official UTF-8"),
            "--markdown-output",
            markdown.to_str().expect("markdown UTF-8"),
        ])
        .output()
        .expect("re-renderizar informe oficial");
    assert!(
        output.status.success(),
        "renderer oficial debe aceptar un report existente; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(&markdown).expect("Markdown wire re-renderizado");
    assert!(
        text.contains("Payload stdout (bytes)"),
        "renderer debe incluir cabecera Payload stdout (bytes); Markdown:\n{text}"
    );
    assert!(
        text.contains("Payload structuredContent (bytes)"),
        "renderer debe incluir cabecera Payload structuredContent (bytes); Markdown:\n{text}"
    );
    assert!(
        text.contains("| Wire tool |"),
        "renderer debe incluir la tabla wire, no solo la tabla Rust; Markdown:\n{text}"
    );
}
