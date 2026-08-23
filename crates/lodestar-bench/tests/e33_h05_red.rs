//! Fase roja independiente de E33-H05.
//!
//! El gate se ejercita contra informes JSON sintéticos: los tests no dependen del reloj ni de
//! la máquina que los ejecuta.  La interfaz que fijan para la implementación es
//! `--gate --report PATH --thresholds PATH --baseline PATH --machine-id ID`.

use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
use tempfile::TempDir;

const ABSOLUTE_MACHINE: &str = "release-macbook-2026-08";
const TREND_MACHINE: &str = "ci-runner-17";
const TOOLS: [&str; 7] = [
    "workspace_status",
    "knowledge_search",
    "knowledge_get",
    "metadata_inspect",
    "graph_query",
    "impact_analyze",
    "knowledge_check",
];

fn bench() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"));
    command.env("RUST_BACKTRACE", "1");
    command
}

fn write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("fixture JSON"),
    )
    .expect("write JSON");
}

fn metric(p95_ns: u64) -> Value {
    json!({
        "sample_count": 2,
        "sample_elapsed_ns": [p95_ns / 2, p95_ns],
        "p50_ns": p95_ns / 2,
        "p95_ns": p95_ns,
        "payload_bytes": 1,
        "result": {"ok": true}
    })
}

fn report(p95_ns: u64, cold_open_ns: u64) -> Value {
    let tools = TOOLS
        .into_iter()
        .map(|tool| (tool.to_owned(), metric(p95_ns)))
        .collect::<serde_json::Map<_, _>>();
    json!({
        "schema_version": "e33-h04-v2-full",
        "machine": ABSOLUTE_MACHINE,
        "profiles": ["plano"],
        "scales": [10000],
        "runs": [{
            "profile": "plano",
            "scale": 10000,
            "measurements": [{
                "variant": "disk-reparseo",
                "document_count": 10000,
                "tools": tools,
                "cold_open": metric(cold_open_ns)
            }]
        }]
    })
}

fn thresholds(p95_ns: u64, cold_open_ns: u64) -> Value {
    json!({
        "schema_version": "e33-h05-thresholds-v1",
        "ratified_on": "2026-08-22",
        "reference": "E33-H05 D4",
        "absolute_machine_id": ABSOLUTE_MACHINE,
        "variant": "disk-reparseo",
        "scale": 10000,
        "p95_ns": p95_ns,
        "cold_open_ns": cold_open_ns
    })
}

fn baseline(records: &[(&str, u64, u64)]) -> Value {
    let machines = records
        .iter()
        .map(|(machine_id, p95_ns, cold_open_ns)| {
            (
                (*machine_id).to_owned(),
                json!({
                    "machine_id": machine_id,
                    "report": report(*p95_ns, *cold_open_ns)
                }),
            )
        })
        .collect::<serde_json::Map<_, _>>();
    json!({
        "schema_version": "e33-h05-baseline-v1",
        "absolute_machine_id": ABSOLUTE_MACHINE,
        "machines": machines
    })
}

fn fixture_dir() -> TempDir {
    tempfile::tempdir().expect("fixture tempdir")
}

fn run_gate(
    dir: &TempDir,
    current_report: &Value,
    limits: &Value,
    baselines: &Value,
    machine_id: &str,
) -> Output {
    let report_path = dir.path().join(format!("report-{machine_id}.json"));
    let thresholds_path = dir.path().join("thresholds.json");
    let baseline_path = dir.path().join("baseline.json");
    write_json(&report_path, current_report);
    write_json(&thresholds_path, limits);
    write_json(&baseline_path, baselines);

    bench()
        .args([
            "--gate",
            "--report",
            report_path.to_str().expect("report path"),
            "--thresholds",
            thresholds_path.to_str().expect("threshold path"),
            "--baseline",
            baseline_path.to_str().expect("baseline path"),
            "--machine-id",
            machine_id,
        ])
        .output()
        .expect("ejecutar gate")
}

fn combined_output(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_report_fixture_is_non_vacuous(value: &Value) {
    let run = &value["runs"][0];
    assert_eq!(run["scale"], 10000, "fixture debe juzgar escala 10k");
    let row = &run["measurements"][0];
    assert_eq!(row["variant"], "disk-reparseo");
    assert_eq!(row["document_count"], 10000);
    for tool in TOOLS {
        assert!(row["tools"][tool]["p95_ns"].as_u64().unwrap_or(0) > 0);
    }
    assert!(row["cold_open"]["p95_ns"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn gate_falla_con_umbral_violado() {
    let dir = fixture_dir();
    let current = report(42_000_000, 84_000_000);
    assert_report_fixture_is_non_vacuous(&current);
    let limits = thresholds(1, 1);
    assert_eq!(limits["absolute_machine_id"], ABSOLUTE_MACHINE);
    let baselines = baseline(&[(ABSOLUTE_MACHINE, 42_000_000, 84_000_000)]);

    let output = run_gate(&dir, &current, &limits, &baselines, ABSOLUTE_MACHINE);
    assert!(
        !output.status.success(),
        "umbral imposible debe dar exit != 0: {}",
        combined_output(&output)
    );
    let text = combined_output(&output);
    assert!(text.contains("p95"), "debe nombrar la clase p95: {text}");
    assert!(
        text.contains("workspace_status"),
        "debe nombrar la tool: {text}"
    );
    assert!(text.contains("42000000"), "debe nombrar lo medido: {text}");
    assert!(
        text.contains("limit") || text.contains("límite"),
        "debe nombrar el límite: {text}"
    );
    assert!(
        text.contains("cold") || text.contains("cold-open"),
        "debe nombrar cold-open: {text}"
    );
    assert!(
        text.contains("84000000"),
        "debe nombrar lo medido en cold-open: {text}"
    );
}

#[test]
fn gate_pasa_con_umbrales_holgados() {
    let dir = fixture_dir();
    let current = report(42_000_000, 84_000_000);
    assert_report_fixture_is_non_vacuous(&current);
    let limits = thresholds(1_000_000_000_000, 1_000_000_000_000);
    let baselines = baseline(&[(ABSOLUTE_MACHINE, 42_000_000, 84_000_000)]);

    let output = run_gate(&dir, &current, &limits, &baselines, ABSOLUTE_MACHINE);
    assert!(
        output.status.success(),
        "umbrales holgados deben dar exit 0: {}",
        combined_output(&output)
    );
    let text = combined_output(&output);
    assert!(
        text.contains("PASS") || text.contains("pass"),
        "debe declarar PASS: {text}"
    );
    assert!(
        text.contains("margin") || text.contains("margen"),
        "debe registrar márgenes: {text}"
    );
    assert!(
        text.contains("workspace_status"),
        "el margen debe identificar la tool: {text}"
    );
    assert!(
        text.contains("cold") || text.contains("cold-open"),
        "el margen debe identificar cold-open: {text}"
    );
}

#[test]
fn gate_degrada_a_tendencia_fuera_de_la_maquina_baseline() {
    let dir = fixture_dir();
    let limits = thresholds(1, 1);
    let baselines = baseline(&[
        (ABSOLUTE_MACHINE, 42_000_000, 84_000_000),
        (TREND_MACHINE, 100_000_000, 200_000_000),
    ]);
    assert_eq!(baselines["absolute_machine_id"], ABSOLUTE_MACHINE);
    assert!(baselines["machines"][TREND_MACHINE]["machine_id"].is_string());

    // Mejora respecto de la baseline propia: aun con umbrales absolutos imposibles, el gate no
    // puede juzgar absolutos en esta máquina ni convertir una mejora en un FAIL.
    let improved = report(90_000_000, 180_000_000);
    assert_report_fixture_is_non_vacuous(&improved);
    let output = run_gate(&dir, &improved, &limits, &baselines, TREND_MACHINE);
    assert!(
        output.status.success(),
        "una mejora de tendencia debe pasar: {}",
        combined_output(&output)
    );
    let text = combined_output(&output);
    assert!(
        text.contains("tendencia") || text.contains("trend"),
        "debe declarar modo tendencia: {text}"
    );
    assert!(
        !text.contains("umbral violado") && !text.contains("threshold violated"),
        "no debe juzgar absolutos: {text}"
    );

    // Una degradación respecto de la baseline propia sí es determinista y debe detectarse.
    let degraded = report(120_000_000, 240_000_000);
    let output = run_gate(&dir, &degraded, &limits, &baselines, TREND_MACHINE);
    assert!(
        !output.status.success(),
        "la degradación de tendencia debe dar exit != 0: {}",
        combined_output(&output)
    );
    let text = combined_output(&output);
    assert!(
        text.contains("tendencia") || text.contains("trend"),
        "debe conservar modo tendencia: {text}"
    );
    assert!(
        text.contains("degrad") || text.contains("regress"),
        "debe nombrar la degradación: {text}"
    );
    assert!(
        text.contains("120000000"),
        "debe nombrar el valor medido: {text}"
    );
}
