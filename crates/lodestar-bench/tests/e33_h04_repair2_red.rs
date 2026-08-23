//! Fase roja fresca de reparación de E33-H04.
//!
//! Estos guards fijan los cuatro defectos de la revisión: entrada portable de la suite, A6 sobre
//! el manifest real, cadena wire derivada de los raw versionados (incluido el payload legado) y
//! columnas Markdown cuya unidad no puede inferirse del nombre.

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

/// A6 must have a deterministic entry point for the actual app manifest.  Requiring a caller to
/// point at an arbitrary manifest permits a green check against an unrelated clean crate.
#[test]
fn a6_dependency_guard_defaults_to_real_lodestar_app_manifest() {
    let output = bench()
        .arg("--check-a6-dependencies")
        .output()
        .expect("ejecutar guard A6 sobre lodestar-app");
    assert!(
        output.status.success(),
        "A6 debe inspeccionar crates/lodestar-app/Cargo.toml real; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

/// Both wire payloads must be recomputed from each raw stdout and the legacy corrida field is
/// `legacy=stdout`.  Independent mutations are deliberately attempted.
#[test]
fn wire_calibration_chain_derives_both_payloads_and_rejects_legacy_mutation() {
    let root = repo_root();
    let evidence = root.join("docs/qa/e33-h04-wire-evidencia-2026-08-22.json");
    let official = root.join("crates/lodestar-bench/tests/fixtures/e33_h04_full_format.json");
    assert!(evidence.is_file(), "falta evidencia wire versionada");
    assert!(official.is_file(), "falta corrida oficial versionada");

    let temp = TempDir::new().expect("scratch de mutaciones wire");
    let evidence_copy = temp.path().join("wire-evidencia.json");
    let official_copy = temp.path().join("corrida.json");
    copy_json(&evidence, &evidence_copy);
    copy_json(&official, &official_copy);

    let valid = bench()
        .args([
            "--validate-wire-calibration-chain",
            "--wire-evidence",
            evidence_copy.to_str().expect("evidence UTF-8"),
            "--official-report",
            official_copy.to_str().expect("official UTF-8"),
        ])
        .output()
        .expect("validar cadena wire válida");
    assert!(
        valid.status.success(),
        "la cadena válida debe pasar; stdout={} stderr={}",
        String::from_utf8_lossy(&valid.stdout),
        String::from_utf8_lossy(&valid.stderr)
    );

    let original_evidence: Value =
        serde_json::from_str(&fs::read_to_string(&evidence_copy).expect("leer evidencia wire"))
            .expect("evidencia JSON");
    for field in ["payload_bytes_stdout", "payload_bytes_structured_content"] {
        let mut mutated = original_evidence.clone();
        let value = mutated["observations"][0][field]
            .as_u64()
            .expect("payload wire entero");
        mutated["observations"][0][field] = Value::from(value + 1);
        fs::write(
            &evidence_copy,
            serde_json::to_vec_pretty(&mutated).expect("serializar evidencia mutada"),
        )
        .expect("escribir evidencia mutada");
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
                .expect("validar payload wire mutado"),
            field,
        );
    }

    // Restore the evidence so the following failure is attributable solely to the legacy field,
    // rather than to a stale mutation from the previous negative cases.
    fs::write(
        &evidence_copy,
        serde_json::to_vec_pretty(&original_evidence).expect("restaurar evidencia wire"),
    )
    .expect("restaurar evidencia");
    let mut legacy = serde_json::from_str::<Value>(
        &fs::read_to_string(&official_copy).expect("leer corrida oficial"),
    )
    .expect("corrida oficial JSON");
    let payload = legacy["wire_calibration"]["results"][0]["payload_bytes"]
        .as_u64()
        .expect("payload legado entero");
    legacy["wire_calibration"]["results"][0]["payload_bytes"] = Value::from(payload + 1);
    fs::write(
        &official_copy,
        serde_json::to_vec_pretty(&legacy).expect("serializar corrida mutada"),
    )
    .expect("escribir corrida mutada");
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
            .expect("validar payload legado mutado"),
        "payload_bytes legado",
    );
}

/// The generated Markdown must state units in every performance column, including the wire
/// envelope distinction.  A bare `p50 ns`/`Payload bytes` heading is too easy to misread.
#[test]
fn renderer_markdown_regenerates_explicit_unit_columns() {
    let temp = TempDir::new().expect("fixture root");
    fs::write(temp.path().join("control.md"), "# control\n").expect("control fixture");
    let markdown = temp.path().join("h04.md");
    let output = bench()
        .args([
            "--smoke",
            "--seed",
            "33",
            "--root",
            temp.path().to_str().expect("root UTF-8"),
            "--markdown-output",
            markdown.to_str().expect("markdown UTF-8"),
        ])
        .output()
        .expect("generar Markdown H04");
    assert!(
        output.status.success(),
        "renderer debe completar smoke; stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let text = fs::read_to_string(&markdown).expect("Markdown generado");
    assert!(
        text.contains("| Perfil | Escala | Variante | Tool | Muestras | p50 (ns) | p95 (ns) | Payload (bytes) |"),
        "renderer debe regenerar cabecera con unidades explícitas; Markdown:\n{text}"
    );
}
