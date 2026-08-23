//! Fase roja fresca de revisión/reparación E33-H05.
//!
//! Los informes son sintéticos y deterministas: cada negativo cambia una sola propiedad del
//! fixture válido, de modo que un gate que acepte una variante dé un rojo conductual explícito.

use serde_json::{json, Map, Value};
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

fn report_with_values(values: &Map<String, Value>, cold_open_ns: u64, schema: &str) -> Value {
    report_with_profile(values, cold_open_ns, schema, "realista")
}

fn report_with_profile(
    values: &Map<String, Value>,
    cold_open_ns: u64,
    schema: &str,
    profile: &str,
) -> Value {
    json!({
        "schema_version": schema,
        "machine": ABSOLUTE_MACHINE,
        "profiles": [profile],
        "scales": [10000],
        "runs": [{
            "profile": profile,
            "scale": 10000,
            "measurements": [{
                "variant": "disk-reparseo",
                "document_count": 10000,
                "tools": values,
                "cold_open": metric(cold_open_ns)
            }]
        }]
    })
}

fn report(p95_ns: u64, cold_open_ns: u64) -> Value {
    let tools = TOOLS
        .into_iter()
        .map(|tool| (tool.to_owned(), metric(p95_ns)))
        .collect::<Map<_, _>>();
    report_with_values(&tools, cold_open_ns, "e33-h04-v2-full")
}

fn baseline(machine_id: &str, p95_ns: u64, cold_open_ns: u64) -> Value {
    baseline_with_profile(machine_id, p95_ns, cold_open_ns, "realista")
}

fn baseline_with_profile(machine_id: &str, p95_ns: u64, cold_open_ns: u64, profile: &str) -> Value {
    let mut machines = Map::new();
    machines.insert(
        ABSOLUTE_MACHINE.to_owned(),
        json!({
            "machine_id": ABSOLUTE_MACHINE,
            "report": report_with_profile(&values(100), 100, "e33-h04-v2-full", profile)
        }),
    );
    if machine_id != ABSOLUTE_MACHINE {
        machines.insert(
            machine_id.to_owned(),
            json!({
                "machine_id": machine_id,
                "report": report_with_profile(
                    &values(p95_ns),
                    cold_open_ns,
                    "e33-h04-v2-full",
                    profile
                )
            }),
        );
    } else {
        machines.insert(
            ABSOLUTE_MACHINE.to_owned(),
            json!({
                "machine_id": ABSOLUTE_MACHINE,
                "report": report_with_profile(
                    &values(p95_ns),
                    cold_open_ns,
                    "e33-h04-v2-full",
                    profile
                )
            }),
        );
    }
    json!({
        "schema_version": "e33-h05-baseline-v1",
        "absolute_machine_id": ABSOLUTE_MACHINE,
        "machines": machines
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

fn fixture_dir() -> TempDir {
    tempfile::tempdir().expect("fixture tempdir")
}

fn write_json(path: &Path, value: &Value) {
    fs::write(
        path,
        serde_json::to_vec_pretty(value).expect("fixture JSON"),
    )
    .expect("escribir fixture JSON");
}

fn assert_non_vacuous_report(value: &Value) {
    let run = &value["runs"][0];
    assert_eq!(run["profile"], "realista");
    assert_eq!(run["scale"], 10000);
    let measurement = &run["measurements"][0];
    assert_eq!(measurement["variant"], "disk-reparseo");
    assert_eq!(measurement["document_count"], 10000);
    for tool in TOOLS {
        assert!(
            measurement["tools"][tool]["p95_ns"].as_u64().unwrap_or(0) > 0,
            "fixture debe medir {tool}"
        );
    }
    assert!(measurement["cold_open"]["p95_ns"].as_u64().unwrap_or(0) > 0);
}

fn run_gate_values(
    dir: &TempDir,
    report_value: &Value,
    thresholds_value: &Value,
    baseline_value: &Value,
    machine_id: &str,
) -> Output {
    let report_path = dir.path().join("report.json");
    let thresholds_path = dir.path().join("thresholds.json");
    let baseline_path = dir.path().join("baseline.json");
    write_json(&report_path, report_value);
    write_json(&thresholds_path, thresholds_value);
    write_json(&baseline_path, baseline_value);
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

fn run_gate_raw_report(
    dir: &TempDir,
    report_raw: &str,
    thresholds_value: &Value,
    baseline_value: &Value,
    machine_id: &str,
) -> Output {
    let report_path = dir.path().join("report.json");
    let thresholds_path = dir.path().join("thresholds.json");
    let baseline_path = dir.path().join("baseline.json");
    fs::write(&report_path, report_raw).expect("escribir report raw");
    write_json(&thresholds_path, thresholds_value);
    write_json(&baseline_path, baseline_value);
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

fn output_text(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn values(default: u64) -> Map<String, Value> {
    TOOLS
        .into_iter()
        .map(|tool| (tool.to_owned(), metric(default)))
        .collect()
}

macro_rules! isolated_absolute_tool_test {
    ($name:ident, $tool:literal) => {
        #[test]
        fn $name() {
            let dir = fixture_dir();
            let mut tool_values = values(100);
            tool_values.insert($tool.to_owned(), metric(1_000_000_001));
            let current = report_with_values(&tool_values, 100, "e33-h04-v2-full");
            assert_non_vacuous_report(&current);
            let output = run_gate_values(
                &dir,
                &current,
                &thresholds(1_000_000_000, 5_000_000_000),
                &baseline(ABSOLUTE_MACHINE, 100, 100),
                ABSOLUTE_MACHINE,
            );
            let text = output_text(&output);
            assert!(
                !output.status.success(),
                "{} sobre límite debe fallar: {}",
                $tool,
                text
            );
            assert!(text.contains("p95"), "debe nombrar p95: {text}");
            assert!(text.contains($tool), "debe nombrar {}: {}", $tool, text);
            assert!(
                text.contains("1000000001"),
                "debe nombrar la medición: {text}"
            );
            assert!(
                text.contains("1000000000"),
                "debe nombrar el límite: {text}"
            );
        }
    };
}

isolated_absolute_tool_test!(gate_falla_por_workspace_status_aislado, "workspace_status");
isolated_absolute_tool_test!(gate_falla_por_knowledge_search_aislado, "knowledge_search");
isolated_absolute_tool_test!(gate_falla_por_knowledge_get_aislado, "knowledge_get");
isolated_absolute_tool_test!(gate_falla_por_metadata_inspect_aislado, "metadata_inspect");
isolated_absolute_tool_test!(gate_falla_por_graph_query_aislado, "graph_query");
isolated_absolute_tool_test!(gate_falla_por_impact_analyze_aislado, "impact_analyze");
isolated_absolute_tool_test!(gate_falla_por_knowledge_check_aislado, "knowledge_check");

#[test]
fn gate_falla_por_cold_open_aislado() {
    let dir = fixture_dir();
    let current = report(100, 5_000_000_001);
    assert_non_vacuous_report(&current);
    let output = run_gate_values(
        &dir,
        &current,
        &thresholds(1_000_000_000, 5_000_000_000),
        &baseline(ABSOLUTE_MACHINE, 100, 100),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "cold-open sobre límite debe fallar: {text}"
    );
    assert!(text.contains("cold"), "debe nombrar cold-open: {text}");
    assert!(
        text.contains("5000000001"),
        "debe nombrar la medición: {text}"
    );
    assert!(
        text.contains("5000000000"),
        "debe nombrar el límite: {text}"
    );
}

#[test]
fn gate_pasa_en_igualdad_exacta_de_limites_p95_y_cold_open() {
    let dir = fixture_dir();
    let current = report(1_000_000_000, 5_000_000_000);
    assert_non_vacuous_report(&current);
    let output = run_gate_values(
        &dir,
        &current,
        &thresholds(1_000_000_000, 5_000_000_000),
        &baseline(ABSOLUTE_MACHINE, 1_000_000_000, 5_000_000_000),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "igualdad debe pasar con <=: {text}"
    );
    assert!(text.contains("PASS"), "debe declarar PASS: {text}");
}

#[test]
fn gate_pasa_en_tendencia_con_igualdad_exacta_a_baseline() {
    let dir = fixture_dir();
    let current = report(1000, 5000);
    assert_non_vacuous_report(&current);
    let output = run_gate_values(
        &dir,
        &current,
        &thresholds(1, 1),
        &baseline(TREND_MACHINE, 1000, 5000),
        TREND_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "igualdad de tendencia debe pasar: {text}"
    );
    assert!(
        text.contains("tendencia") || text.contains("trend"),
        "debe declarar tendencia: {text}"
    );
}

macro_rules! isolated_trend_tool_test {
    ($name:ident, $tool:literal) => {
        #[test]
        fn $name() {
            let dir = fixture_dir();
            let mut current_values = values(100);
            current_values.insert($tool.to_owned(), metric(101));
            let current = report_with_values(&current_values, 100, "e33-h04-v2-full");
            assert_non_vacuous_report(&current);
            let output = run_gate_values(
                &dir,
                &current,
                &thresholds(1, 1),
                &baseline(TREND_MACHINE, 100, 100),
                TREND_MACHINE,
            );
            let text = output_text(&output);
            assert!(
                !output.status.success(),
                "degradación aislada de {} debe fallar: {}",
                $tool,
                text
            );
            assert!(
                text.contains("degrad") || text.contains("regress"),
                "debe nombrar degradación: {text}"
            );
            assert!(text.contains($tool), "debe nombrar {}: {}", $tool, text);
            assert!(text.contains("101"), "debe nombrar la medición: {text}");
        }
    };
}

isolated_trend_tool_test!(
    gate_falla_por_degradacion_workspace_status_aislada,
    "workspace_status"
);
isolated_trend_tool_test!(
    gate_falla_por_degradacion_knowledge_search_aislada,
    "knowledge_search"
);
isolated_trend_tool_test!(
    gate_falla_por_degradacion_knowledge_get_aislada,
    "knowledge_get"
);
isolated_trend_tool_test!(
    gate_falla_por_degradacion_metadata_inspect_aislada,
    "metadata_inspect"
);
isolated_trend_tool_test!(
    gate_falla_por_degradacion_graph_query_aislada,
    "graph_query"
);
isolated_trend_tool_test!(
    gate_falla_por_degradacion_impact_analyze_aislada,
    "impact_analyze"
);
isolated_trend_tool_test!(
    gate_falla_por_degradacion_knowledge_check_aislada,
    "knowledge_check"
);

#[test]
fn gate_falla_por_degradacion_cold_open_aislada() {
    let dir = fixture_dir();
    let current = report(100, 101);
    assert_non_vacuous_report(&current);
    let output = run_gate_values(
        &dir,
        &current,
        &thresholds(1, 1),
        &baseline(TREND_MACHINE, 100, 100),
        TREND_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "degradación de cold-open debe fallar: {text}"
    );
    assert!(text.contains("cold-open"), "debe nombrar cold-open: {text}");
    assert!(text.contains("101"), "debe nombrar la medición: {text}");
}

#[test]
fn gate_fuera_de_maquina_sin_baseline_declara_no_disponible_y_exit0() {
    let dir = fixture_dir();
    let current = report(100, 100);
    assert_non_vacuous_report(&current);
    let output = run_gate_values(
        &dir,
        &current,
        &thresholds(1, 1),
        &baseline(ABSOLUTE_MACHINE, 100, 100),
        TREND_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "sin baseline propia no debe juzgar absolutos: {text}"
    );
    assert!(
        text.contains("sin baseline propia"),
        "debe declarar no disponible: {text}"
    );
}

#[test]
fn gate_rechaza_schema_version_desconocido_del_report_actual() {
    let dir = fixture_dir();
    let current = report(100, 100);
    assert_non_vacuous_report(&current);
    let mut malformed = current.clone();
    malformed["schema_version"] = json!("e33-h99-future");
    let output = run_gate_values(
        &dir,
        &malformed,
        &thresholds(1000, 1000),
        &baseline(ABSOLUTE_MACHINE, 100, 100),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "schema desconocido del report debe rechazarse: {text}"
    );
    assert!(
        text.contains("schema_version"),
        "debe nombrar schema_version: {text}"
    );
}

#[test]
fn gate_rechaza_schema_version_desconocido_del_report_embebido_en_baseline() {
    let dir = fixture_dir();
    let current = report(100, 100);
    assert_non_vacuous_report(&current);
    let mut malformed_baseline = baseline(ABSOLUTE_MACHINE, 100, 100);
    malformed_baseline["machines"][ABSOLUTE_MACHINE]["report"]["schema_version"] =
        json!("e33-h99-future");
    let output = run_gate_values(
        &dir,
        &current,
        &thresholds(1000, 1000),
        &malformed_baseline,
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "schema desconocido de baseline debe rechazarse: {text}"
    );
    assert!(
        text.contains("schema_version"),
        "debe nombrar schema_version: {text}"
    );
}

#[test]
fn gate_rechaza_schema_version_desconocido_de_thresholds() {
    let dir = fixture_dir();
    let current = report(100, 100);
    let mut malformed = thresholds(1000, 1000);
    malformed["schema_version"] = json!("e33-h99-future");
    let output = run_gate_values(
        &dir,
        &current,
        &malformed,
        &baseline(ABSOLUTE_MACHINE, 100, 100),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "schema de thresholds desconocido debe rechazarse: {text}"
    );
    assert!(
        text.contains("schema_version"),
        "debe nombrar schema_version: {text}"
    );
}

#[test]
fn gate_rechaza_schema_version_desconocido_de_baseline() {
    let dir = fixture_dir();
    let current = report(100, 100);
    let mut malformed = baseline(ABSOLUTE_MACHINE, 100, 100);
    malformed["schema_version"] = json!("e33-h99-future");
    let output = run_gate_values(
        &dir,
        &current,
        &thresholds(1000, 1000),
        &malformed,
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "schema de baseline desconocido debe rechazarse: {text}"
    );
    assert!(
        text.contains("schema_version"),
        "debe nombrar schema_version: {text}"
    );
}

#[test]
fn gate_rechaza_ratified_on_ausente() {
    let dir = fixture_dir();
    let current = report(100, 100);
    let mut malformed = thresholds(1000, 1000);
    malformed
        .as_object_mut()
        .expect("thresholds object")
        .remove("ratified_on");
    let output = run_gate_values(
        &dir,
        &current,
        &malformed,
        &baseline(ABSOLUTE_MACHINE, 100, 100),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "ratified_on ausente debe rechazarse: {text}"
    );
    assert!(
        text.contains("ratified_on"),
        "debe nombrar ratified_on: {text}"
    );
}

#[test]
fn gate_rechaza_reference_ausente() {
    let dir = fixture_dir();
    let current = report(100, 100);
    let mut malformed = thresholds(1000, 1000);
    malformed
        .as_object_mut()
        .expect("thresholds object")
        .remove("reference");
    let output = run_gate_values(
        &dir,
        &current,
        &malformed,
        &baseline(ABSOLUTE_MACHINE, 100, 100),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "reference ausente debe rechazarse: {text}"
    );
    assert!(text.contains("reference"), "debe nombrar reference: {text}");
}

#[test]
fn gate_rechaza_variant_threshold_incorrecta() {
    let dir = fixture_dir();
    let current = report(100, 100);
    let mut malformed = thresholds(1000, 1000);
    malformed["variant"] = json!("plano");
    let output = run_gate_values(
        &dir,
        &current,
        &malformed,
        &baseline(ABSOLUTE_MACHINE, 100, 100),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "variant incorrecta debe rechazarse: {text}"
    );
    assert!(text.contains("variant"), "debe nombrar variant: {text}");
}

#[test]
fn gate_rechaza_scale_threshold_incorrecta() {
    let dir = fixture_dir();
    let current = report(100, 100);
    let mut malformed = thresholds(1000, 1000);
    malformed["scale"] = json!(1000);
    let output = run_gate_values(
        &dir,
        &current,
        &malformed,
        &baseline(ABSOLUTE_MACHINE, 100, 100),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "scale incorrecta debe rechazarse: {text}"
    );
    assert!(text.contains("scale"), "debe nombrar scale: {text}");
}

#[test]
fn gate_rechaza_tool_ausente() {
    let dir = fixture_dir();
    let mut tool_values = values(100);
    tool_values.remove("graph_query");
    let current = report_with_values(&tool_values, 100, "e33-h04-v2-full");
    let output = run_gate_values(
        &dir,
        &current,
        &thresholds(1000, 1000),
        &baseline(ABSOLUTE_MACHINE, 100, 100),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "tool ausente debe rechazarse: {text}"
    );
    assert!(
        text.contains("graph_query"),
        "debe nombrar tool ausente: {text}"
    );
}

#[test]
fn gate_rechaza_tool_duplicada() {
    let dir = fixture_dir();
    let current = report(100, 100);
    assert_non_vacuous_report(&current);
    let tools = current["runs"][0]["measurements"][0]["tools"]
        .as_object()
        .expect("tools");
    let entries = TOOLS
        .into_iter()
        .map(|tool| {
            let encoded = serde_json::to_string(&tools[tool]).expect("metric JSON");
            format!("\"{tool}\":{encoded}")
        })
        .collect::<Vec<_>>();
    let valid_tools = serde_json::to_string(tools).expect("tools JSON");
    let duplicate_entry = entries
        .iter()
        .map(|entry| entry.as_str())
        .chain(std::iter::once(entries[0].as_str()))
        .collect::<Vec<_>>()
        .join(",");
    let raw = serde_json::to_string(&current)
        .expect("report JSON")
        .replace(&valid_tools, &format!("{{{duplicate_entry}}}"));
    let output = run_gate_raw_report(
        &dir,
        &raw,
        &thresholds(1000, 1000),
        &baseline(ABSOLUTE_MACHINE, 100, 100),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "tool duplicada debe rechazarse: {text}"
    );
    assert!(
        text.contains("duplic") || text.contains("ambig"),
        "debe explicar duplicidad: {text}"
    );
}

#[test]
fn gate_selecciona_perfil_plano_de_baseline_con_realista_coexistente() {
    let dir = fixture_dir();
    let realista_measurement =
        report_with_profile(&values(1_000_000_001), 100, "e33-h04-v2-full", "realista")["runs"][0]
            ["measurements"][0]
            .clone();
    let plano_measurement = report_with_profile(&values(100), 100, "e33-h04-v2-full", "plano")
        ["runs"][0]["measurements"][0]
        .clone();
    let current = json!({
        "schema_version": "e33-h04-v2-full",
        "machine": ABSOLUTE_MACHINE,
        "profiles": ["realista", "plano"],
        "scales": [10000],
        "runs": [
            {"profile": "realista", "scale": 10000, "measurements": [realista_measurement]},
            {"profile": "plano", "scale": 10000, "measurements": [plano_measurement]}
        ]
    });
    assert_eq!(current["runs"].as_array().expect("runs").len(), 2);
    assert_eq!(current["runs"][0]["profile"], "realista");
    assert_eq!(current["runs"][1]["profile"], "plano");
    assert!(
        current["runs"][0]["measurements"][0]["tools"]["workspace_status"]["p95_ns"]
            .as_u64()
            .expect("realista p95")
            > 1_000_000_000
    );
    assert_eq!(
        current["runs"][1]["measurements"][0]["tools"]["workspace_status"]["p95_ns"],
        100
    );
    let output = run_gate_values(
        &dir,
        &current,
        &thresholds(1_000_000_000, 5_000_000_000),
        &baseline_with_profile(ABSOLUTE_MACHINE, 100, 100, "plano"),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "baseline propia plano debe seleccionar plano aunque coexista realista: {text}"
    );
    assert!(
        text.contains("mode=absolute"),
        "debe declarar modo absoluto: {text}"
    );
}

#[test]
fn gate_rechaza_dos_measurements_disk_reparseo_del_mismo_perfil_seleccionado() {
    let dir = fixture_dir();
    let measurement = report_with_profile(&values(100), 100, "e33-h04-v2-full", "plano")["runs"][0]
        ["measurements"][0]
        .clone();
    let current = json!({
        "schema_version": "e33-h04-v2-full",
        "machine": ABSOLUTE_MACHINE,
        "profiles": ["plano"],
        "scales": [10000],
        "runs": [{
            "profile": "plano",
            "scale": 10000,
            "measurements": [measurement.clone(), measurement]
        }]
    });
    assert_eq!(current["runs"].as_array().expect("runs").len(), 1);
    assert_eq!(current["runs"][0]["profile"], "plano");
    assert_eq!(
        current["runs"][0]["measurements"]
            .as_array()
            .expect("measurements")
            .len(),
        2
    );
    let output = run_gate_values(
        &dir,
        &current,
        &thresholds(1_000_000_000, 5_000_000_000),
        &baseline_with_profile(ABSOLUTE_MACHINE, 100, 100, "plano"),
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "dos measurements del mismo perfil seleccionado deben rechazarse: {text}"
    );
    assert!(
        text.contains("múltiples") || text.contains("multiple") || text.contains("ambig"),
        "debe explicar duplicidad dentro del perfil seleccionado: {text}"
    );
}

#[test]
fn gate_rechaza_absolute_machine_id_incoherente_entre_thresholds_y_baseline() {
    let dir = fixture_dir();
    let current = report(100, 100);
    let mut malformed_baseline = baseline(ABSOLUTE_MACHINE, 100, 100);
    malformed_baseline["absolute_machine_id"] = json!("other-release-machine");
    let output = run_gate_values(
        &dir,
        &current,
        &thresholds(1000, 1000),
        &malformed_baseline,
        ABSOLUTE_MACHINE,
    );
    let text = output_text(&output);
    assert!(
        !output.status.success(),
        "machine id incoherente debe rechazarse: {text}"
    );
    assert!(
        text.contains("absolute_machine_id"),
        "debe nombrar incoherencia: {text}"
    );
}

#[test]
fn gate_oficial_versionado_h04_pasa_en_modo_absoluto() {
    let crate_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let report_path = crate_dir.join("tests/fixtures/e33_h04_full_format.json");
    let thresholds_path = crate_dir.join("../../docs/qa/testbench/umbrales.json");
    let baseline_path =
        crate_dir.join("../../docs/qa/e33-h05-baseline-release-macbook-2026-08.json");
    for path in [&report_path, &thresholds_path, &baseline_path] {
        assert!(
            path.is_file(),
            "falta artefacto oficial versionado: {}",
            path.display()
        );
    }
    let thresholds_text = fs::read_to_string(&thresholds_path).expect("leer umbrales oficiales");
    let thresholds_value: Value = serde_json::from_str(&thresholds_text).expect("JSON umbrales");
    assert_eq!(thresholds_value["ratified_on"], "2026-08-22");
    assert!(thresholds_value["reference"]
        .as_str()
        .unwrap_or("")
        .contains("E33-H05"));
    assert_eq!(thresholds_value["variant"], "disk-reparseo");
    assert_eq!(thresholds_value["scale"], 10000);
    assert_eq!(thresholds_value["p95_ns"], 1_000_000_000_u64);
    assert_eq!(thresholds_value["cold_open_ns"], 5_000_000_000_u64);
    assert_eq!(thresholds_value["absolute_machine_id"], ABSOLUTE_MACHINE);

    let baseline_text = fs::read_to_string(&baseline_path).expect("leer baseline oficial");
    let baseline_value: Value = serde_json::from_str(&baseline_text).expect("JSON baseline");
    assert_eq!(baseline_value["schema_version"], "e33-h05-baseline-v1");
    let report = &baseline_value["machines"][ABSOLUTE_MACHINE]["report"];
    assert_eq!(report["schema_version"], "e33-h04-v2-full");
    assert_eq!(report["profiles"][0], "realista");
    assert_eq!(report["scales"][0], 10000);
    let measurement = &report["runs"][0]["measurements"][0];
    assert_eq!(measurement["variant"], "disk-reparseo");
    assert!(measurement["document_count"].as_u64().unwrap_or(0) >= 10000);
    let expected_h04 = [
        ("workspace_status", 265405167_u64),
        ("knowledge_search", 288724209_u64),
        ("knowledge_get", 481312208_u64),
        ("metadata_inspect", 201016375_u64),
        ("graph_query", 243586125_u64),
        ("impact_analyze", 481542291_u64),
        ("knowledge_check", 251015708_u64),
    ];
    for (tool, expected) in expected_h04 {
        assert_eq!(
            measurement["tools"][tool]["p95_ns"], expected,
            "cifra H04 de {tool}"
        );
    }
    assert_eq!(measurement["cold_open"]["p95_ns"], 248825583_u64);

    let workflow = fs::read_to_string(crate_dir.join("../../.github/workflows/ci.yml"))
        .expect("leer workflow CI");
    let smoke_lines = workflow
        .lines()
        .map(str::trim)
        .filter(|line| line.starts_with("run: cargo run -p lodestar-bench --"))
        .collect::<Vec<_>>();
    assert!(
        smoke_lines.contains(&"run: cargo run -p lodestar-bench -- --smoke"),
        "CI debe tener el step smoke exacto: {smoke_lines:?}"
    );
    assert!(
        smoke_lines.iter().all(|line| !line.contains("--gate")),
        "el smoke CI no debe juzgar gate absoluto: {smoke_lines:?}"
    );

    let output = bench()
        .args([
            "--gate",
            "--report",
            report_path.to_str().expect("report path"),
            "--thresholds",
            thresholds_path.to_str().expect("thresholds path"),
            "--baseline",
            baseline_path.to_str().expect("baseline path"),
            "--machine-id",
            ABSOLUTE_MACHINE,
        ])
        .output()
        .expect("ejecutar gate oficial versionado");
    let text = output_text(&output);
    assert!(
        output.status.success(),
        "corrida oficial H04 debe pasar el gate: {text}"
    );
    assert!(
        text.contains("mode=absolute"),
        "debe declarar modo absoluto: {text}"
    );
    if text.contains("profile=") || text.contains("perfil=") {
        assert!(
            text.contains("realista"),
            "si declara perfil debe ser realista: {text}"
        );
    }
}
