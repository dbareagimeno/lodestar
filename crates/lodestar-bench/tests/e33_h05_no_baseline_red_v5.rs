//! Fase roja H05: una corrida H04 oficial multi-perfil sin baseline propia no es ambigua.
//!
//! La máquina de esta prueba es deliberadamente distinta de la máquina release de la baseline
//! oficial. El caso debe seleccionar el modo tendencia sin juzgar umbrales absolutos ni inventar
//! una comparación.

use serde_json::Value;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

const FOREIGN_MACHINE: &str = "ci-runner-e33-h05-no-baseline-red-v5";
const RELEASE_MACHINE: &str = "release-macbook-2026-08";
const REQUIRED_TOOLS: [&str; 7] = [
    "workspace_status",
    "knowledge_search",
    "knowledge_get",
    "metadata_inspect",
    "graph_query",
    "impact_analyze",
    "knowledge_check",
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn official_path(relative: &str) -> PathBuf {
    repo_root().join(relative)
}

fn read_json(path: &Path) -> Value {
    serde_json::from_str(
        &fs::read_to_string(path)
            .unwrap_or_else(|error| panic!("leer artefacto oficial {}: {error}", path.display())),
    )
    .unwrap_or_else(|error| panic!("parsear JSON oficial {}: {error}", path.display()))
}

fn combined_output(output: &std::process::Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn gate_oficial_h04_multiperfil_sin_baseline_propia_declara_tendencia_sin_comparacion() {
    let report_path =
        official_path("crates/lodestar-bench/tests/fixtures/e33_h05_gate_report.json");
    let thresholds_path = official_path("docs/qa/testbench/umbrales.json");
    let baseline_path = official_path("docs/qa/e33-h05-baseline-release-macbook-2026-08.json");
    for path in [&report_path, &thresholds_path, &baseline_path] {
        assert!(path.is_file(), "falta artefacto oficial {}", path.display());
    }

    let report = read_json(&report_path);
    assert_eq!(report["schema_version"], "e33-h04-v2-full");
    assert_eq!(report["profiles"], serde_json::json!(["plano", "realista"]));
    assert_eq!(report["scales"], serde_json::json!([100, 1000, 10000]));
    let eligible_runs = report["runs"]
        .as_array()
        .expect("runs oficiales")
        .iter()
        .filter(|run| run["scale"] == 10000)
        .flat_map(|run| {
            run["measurements"]
                .as_array()
                .expect("measurements oficiales")
                .iter()
                .filter(|measurement| measurement["variant"] == "disk-reparseo")
                .map(move |measurement| (run["profile"].clone(), measurement))
        })
        .collect::<Vec<_>>();
    assert_eq!(
        eligible_runs.len(),
        2,
        "la corrida oficial debe aportar exactamente una medición disk-reparseo por perfil"
    );
    for (profile, measurement) in &eligible_runs {
        assert!(
            profile == "plano" || profile == "realista",
            "perfil H04 inesperado: {profile}"
        );
        assert_eq!(measurement["document_count"], 10004);
        for tool in REQUIRED_TOOLS {
            assert!(
                measurement["tools"][tool]["p95_ns"].as_u64().unwrap_or(0) > 0,
                "fixture oficial no vacía la métrica {profile}.{tool}"
            );
        }
        assert!(measurement["cold_open"]["p95_ns"].as_u64().unwrap_or(0) > 0);
    }

    let thresholds = read_json(&thresholds_path);
    assert_eq!(thresholds["schema_version"], "e33-h05-thresholds-v1");
    assert_eq!(thresholds["absolute_machine_id"], RELEASE_MACHINE);
    assert_eq!(thresholds["p95_ns"], 1_000_000_000_u64);
    assert_eq!(thresholds["cold_open_ns"], 5_000_000_000_u64);

    let baseline = read_json(&baseline_path);
    assert_eq!(baseline["schema_version"], "e33-h05-baseline-v1");
    assert_eq!(baseline["absolute_machine_id"], RELEASE_MACHINE);
    let machines = baseline["machines"]
        .as_object()
        .expect("machines oficiales");
    assert!(machines.contains_key(RELEASE_MACHINE));
    assert!(
        !machines.contains_key(FOREIGN_MACHINE),
        "la máquina bajo prueba debe carecer de entrada propia"
    );

    let output = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"))
        .args([
            "--gate",
            "--report",
            report_path.to_str().expect("ruta report oficial"),
            "--thresholds",
            thresholds_path.to_str().expect("ruta thresholds oficial"),
            "--baseline",
            baseline_path.to_str().expect("ruta baseline oficial"),
            "--machine-id",
            FOREIGN_MACHINE,
        ])
        .output()
        .expect("ejecutar gate oficial H05");
    let text = combined_output(&output);

    assert!(
        output.status.success(),
        "sin baseline propia no debe fallar por ambigüedad ni juzgar absolutos: {text}"
    );
    assert!(
        text.contains("mode=tendencia") || text.contains("mode=trend"),
        "debe declarar modo tendencia: {text}"
    );
    assert!(
        text.contains("sin baseline propia") || text.contains("no own baseline"),
        "debe declarar baseline no disponible: {text}"
    );
    assert!(
        text.contains("no hay comparación disponible") || text.contains("comparison unavailable"),
        "debe declarar comparación no disponible: {text}"
    );
    assert!(
        !text.contains("mode=absolute"),
        "no debe emitir veredicto absoluto: {text}"
    );
    assert!(
        !text.contains("FAIL umbral"),
        "no debe emitir FAIL absoluto: {text}"
    );
    assert!(
        !text.contains("múltiples measurements"),
        "no debe declarar ambigüedad: {text}"
    );
    assert!(
        !text.contains("multiple measurements"),
        "no debe declarar ambigüedad: {text}"
    );
}
