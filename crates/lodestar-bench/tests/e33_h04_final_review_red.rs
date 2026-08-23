//! Revisión roja final de E33-H04 (adenda §22.4/§22.5).
//!
//! Snapshot de alcance: `target/agent-state/e33-h04-v2-red9-final-review/pre-red.json`.
//! Los oráculos son relojes externos, bytes del filesystem, recibos cargados por `App`, el
//! arnés MCP real y un `git archive` de `develop`; no se aceptan informes autoconsistentes.

use lodestar_app::App;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};
use tempfile::TempDir;

const ITERATIONS_ENV: &str = "LODESTAR_BENCH_TEST_ITERATIONS";
const CHANGE_PARENT_ENV: &str = "LODESTAR_BENCH_TEST_CHANGE_PARENT";
const TOOLS: [&str; 7] = [
    "workspace_status",
    "knowledge_search",
    "knowledge_get",
    "metadata_inspect",
    "graph_query",
    "impact_analyze",
    "knowledge_check",
];
const VARIANTS: [&str; 3] = ["disk-reparseo", "sqlite-raw", "ram-memoizado"];
const PROFILES: [&str; 2] = ["plano", "realista"];
const SCALES: [u64; 3] = [100, 1_000, 10_000];

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("raíz")
}

fn canonical_develop_candidates(github_base_ref: Option<&str>) -> [&'static str; 2] {
    if github_base_ref == Some("develop") {
        ["origin/develop", "develop"]
    } else {
        ["develop", "origin/develop"]
    }
}

fn resolve_develop_ref_with<F>(
    github_base_ref: Option<&str>,
    mut resolve: F,
) -> Result<(String, String), String>
where
    F: FnMut(&str) -> Result<String, String>,
{
    let candidates = canonical_develop_candidates(github_base_ref);
    let mut attempts = Vec::new();
    for reference in candidates {
        match resolve(reference) {
            Ok(commit) if !commit.trim().is_empty() => {
                return Ok((reference.to_owned(), commit.trim().to_owned()));
            }
            Ok(_) => attempts.push(format!(
                "{reference}: git rev-parse devolvió un commit vacío"
            )),
            Err(error) => attempts.push(format!("{reference}: {error}")),
        }
    }
    let context = format!(
        "no se pudo resolver la referencia canónica develop (GITHUB_BASE_REF={github_base_ref:?})"
    );
    Err(format!("{context}; intentos={attempts:?}; no se usa HEAD"))
}

fn resolve_develop_ref(root: &Path) -> Result<(String, String), String> {
    let github_base_ref = std::env::var("GITHUB_BASE_REF").ok();
    resolve_develop_ref_with(github_base_ref.as_deref(), |reference| {
        let revspec = format!("{reference}^{{commit}}");
        let output = Command::new("git")
            .args(["rev-parse", "--verify", "--quiet"])
            .arg(&revspec)
            .current_dir(root)
            .output()
            .map_err(|error| format!("no se pudo ejecutar git rev-parse: {error}"))?;
        if output.status.success() {
            Ok(String::from_utf8_lossy(&output.stdout).into_owned())
        } else {
            let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
            Err(if stderr.is_empty() {
                format!("referencia ausente (status {})", output.status)
            } else {
                format!("git rev-parse falló (status {}): {stderr}", output.status)
            })
        }
    })
}

fn archive_resolved_develop(
    root: &Path,
    archive: &Path,
    resolved: &(String, String),
) -> std::io::Result<Output> {
    Command::new("git")
        .args(["archive", "--format=tar"])
        .arg(&resolved.1)
        .args(["-o"])
        .arg(archive)
        .current_dir(root)
        .output()
}

fn write_fixture(root: &Path, documents: usize) {
    fs::create_dir_all(root).expect("raíz fixture");
    for index in 0..documents {
        fs::write(
            root.join(format!("document-{index:03}.md")),
            format!("---\ntags: [review, scale]\nservice: bench\n---\n# Document {index}\n"),
        )
        .expect("document fixture");
    }
    fs::write(root.join("control.md"), "---\ntags: [h04, control]\nservice: bench\n---\n# Control\nmarker-search-h04\n[child](child.md)\n[missing](missing.md)\n").expect("control");
    fs::write(
        root.join("child.md"),
        "---\ntags: [child]\nservice: bench\n---\n# Child\nmarker-get-h04\n[leaf](leaf.md)\n",
    )
    .expect("child");
    fs::write(
        root.join("leaf.md"),
        "---\ntags: [leaf]\nservice: bench\n---\n# Leaf\nmarker-impact-h04\n",
    )
    .expect("leaf");
    fs::write(root.join("broken.md"), "---\ntags: [\n---\n# Broken\n").expect("diagnostic");
}

fn run_smoke(measured: &Path, cycle: &Path, iterations: usize) -> (Value, Duration) {
    let started = Instant::now();
    let output = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"))
        .args([
            "--smoke",
            "--seed",
            "33",
            "--root",
            measured.to_str().expect("UTF-8"),
        ])
        .env(ITERATIONS_ENV, iterations.to_string())
        .env(CHANGE_PARENT_ENV, cycle)
        .output()
        .expect("ejecutar bench smoke");
    let elapsed = started.elapsed();
    assert!(
        output.status.success(),
        "smoke falló: status={} stdout={} stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(!stdout.trim().is_empty(), "stdout smoke vacío");
    (
        serde_json::from_str(stdout.trim()).expect("JSON smoke"),
        elapsed,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    kind: String,
    bytes: Vec<u8>,
}

fn tree(root: &Path) -> BTreeMap<String, Entry> {
    fn walk(root: &Path, current: &Path, out: &mut BTreeMap<String, Entry>) {
        for item in fs::read_dir(current).expect("leer árbol") {
            let path = item.expect("entrada árbol").path();
            let relative = path
                .strip_prefix(root)
                .expect("ruta relativa")
                .to_string_lossy()
                .replace('\\', "/");
            let file_type = fs::symlink_metadata(&path).expect("metadata").file_type();
            if file_type.is_dir() {
                out.insert(
                    relative.clone(),
                    Entry {
                        kind: "dir".into(),
                        bytes: vec![],
                    },
                );
                walk(root, &path, out);
            } else if file_type.is_file() {
                out.insert(
                    relative,
                    Entry {
                        kind: "file".into(),
                        bytes: fs::read(&path).expect("bytes"),
                    },
                );
            } else if file_type.is_symlink() {
                out.insert(
                    relative,
                    Entry {
                        kind: "symlink".into(),
                        bytes: fs::read_link(&path)
                            .expect("link")
                            .to_string_lossy()
                            .as_bytes()
                            .to_vec(),
                    },
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

fn metric_samples(metric: &Value, label: &str) -> Vec<u64> {
    let samples = metric["sample_elapsed_ns"]
        .as_array()
        .unwrap_or_else(|| panic!("{label}: falta sample_elapsed_ns"));
    assert!(!samples.is_empty(), "{label}: samples vacías");
    let values: Vec<u64> = samples
        .iter()
        .map(|value| value.as_u64().expect("sample entero"))
        .collect();
    assert!(
        values.iter().all(|value| *value > 0),
        "{label}: muestra cero"
    );
    assert_eq!(metric["sample_count"].as_u64(), Some(values.len() as u64));
    let mut sorted = values.clone();
    sorted.sort_unstable();
    let expected_p50 = sorted[sorted.len() / 2];
    let expected_p95 = sorted[((sorted.len() * 95).saturating_sub(1) / 100).min(sorted.len() - 1)];
    assert_eq!(
        metric["p50_ns"].as_u64(),
        Some(expected_p50),
        "{label}: p50 no se recalcula de sample_elapsed_ns"
    );
    assert_eq!(
        metric["p95_ns"].as_u64(),
        Some(expected_p95),
        "{label}: p95 no se recalcula de sample_elapsed_ns"
    );
    assert!(expected_p50 > 0, "{label}: p50 cero");
    assert!(expected_p95 > 0, "{label}: p95 cero");
    assert!(expected_p95 >= expected_p50, "{label}: p95 < p50");
    assert!(metric["payload_bytes"]
        .as_u64()
        .is_some_and(|value| value > 0));
    let encoded = serde_json::to_vec(&metric["result"]).expect("result serializable");
    assert!(!encoded.is_empty(), "{label}: payload vacío");
    assert_eq!(metric["payload_bytes"].as_u64(), Some(encoded.len() as u64));
    values
}

fn assert_semantic_non_empty(tool: &str, result: &Value, label: &str) {
    let object = result
        .as_object()
        .unwrap_or_else(|| panic!("{label}: result no es objeto"));
    assert!(!object.is_empty(), "{label}: result vacío");
    match tool {
        "workspace_status" => {
            assert!(
                object
                    .get("counts")
                    .and_then(|counts| counts.get("documents"))
                    .and_then(Value::as_u64)
                    .is_some_and(|n| n > 0),
                "{label}: documents vacío"
            );
            assert!(
                object.get("counts").is_some_and(Value::is_object),
                "{label}: counts ausente"
            );
            assert!(
                object.get("valid").is_some_and(Value::is_boolean),
                "{label}: valid ausente/no booleano"
            );
        }
        "knowledge_search" => assert!(
            object
                .get("results")
                .and_then(Value::as_array)
                .is_some_and(|a| !a.is_empty()),
            "{label}: search vacío"
        ),
        "knowledge_get" => {
            assert!(
                object
                    .get("path")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty()),
                "{label}: path vacío"
            );
            assert!(
                object.get("body").and_then(Value::as_str).is_some(),
                "{label}: body ausente/no string"
            );
            assert!(
                !object.contains_key("document"),
                "{label}: shape obsoleto document"
            );
        }
        "metadata_inspect" => {
            assert!(
                object
                    .get("presentIn")
                    .and_then(Value::as_u64)
                    .is_some_and(|n| n > 0),
                "{label}: metadata vacío"
            );
            assert!(
                object.get("inferredTypes").is_some_and(Value::is_object),
                "{label}: inferredTypes ausente"
            );
        }
        "graph_query" => {
            assert!(
                object
                    .get("nodes")
                    .and_then(Value::as_array)
                    .is_some_and(|a| !a.is_empty()),
                "{label}: graph vacío"
            );
            assert!(
                object.get("edges").is_some_and(Value::is_array),
                "{label}: edges ausente"
            );
            assert!(
                object.get("summary").is_some_and(Value::is_object),
                "{label}: summary ausente"
            );
        }
        "impact_analyze" => {
            assert!(
                object
                    .get("summary")
                    .and_then(|summary| summary.get("directlyAffected"))
                    .and_then(Value::as_u64)
                    .is_some_and(|n| n > 0),
                "{label}: impact vacío"
            );
            assert!(
                object.get("affectedDocuments").is_some_and(Value::is_array),
                "{label}: affectedDocuments ausente"
            );
            assert!(
                object.get("recommendations").is_some_and(Value::is_array),
                "{label}: recommendations ausente"
            );
        }
        "knowledge_check" => {
            assert!(
                object.get("valid").is_some_and(Value::is_boolean),
                "{label}: valid ausente/no booleano"
            );
            assert!(
                object
                    .get("summary")
                    .and_then(Value::as_object)
                    .is_some_and(|o| !o.is_empty()),
                "{label}: check vacío"
            );
            assert!(
                object.get("diagnostics").is_some_and(Value::is_array),
                "{label}: diagnostics ausente"
            );
            assert!(
                object
                    .get("workspaceRevision")
                    .and_then(Value::as_str)
                    .is_some_and(|s| !s.is_empty()),
                "{label}: workspaceRevision ausente"
            );
        }
        _ => unreachable!(),
    }
}

fn assert_wire_semantic_non_empty(tool: &str, result: &Value, label: &str) {
    let payload = if tool == "knowledge_get" {
        let envelope = result
            .as_object()
            .unwrap_or_else(|| panic!("{label}: knowledge_get wire no es objeto"));
        assert_eq!(
            envelope.len(),
            1,
            "{label}: knowledge_get wire debe tener solo el envelope document"
        );
        envelope
            .get("document")
            .unwrap_or_else(|| panic!("{label}: knowledge_get wire falta document"))
    } else {
        result
    };
    assert_semantic_non_empty(tool, payload, label);
}

fn result_map<'a>(object: &'a Value, key: &str) -> &'a serde_json::Map<String, Value> {
    object[key]
        .as_object()
        .unwrap_or_else(|| panic!("falta {key} como objeto"))
}

#[test]
fn suite_manifest_portable_y_declara_los_tests_contemporaneos() {
    // The suite contract is an ordinary repository fixture.  It must not rely on the
    // orchestrator's ignored `target/agent-state` scratch tree.
    let manifest: Value =
        serde_json::from_str(include_str!("fixtures/e33_h04_suite_manifest.json"))
            .expect("suite manifest JSON");
    let got: BTreeSet<_> = manifest["suite_tests"]
        .as_array()
        .expect("suite_tests")
        .iter()
        .map(|value| value.as_str().expect("suite test path"))
        .collect();
    let expected: BTreeSet<_> = [
        "crates/lodestar-bench/tests/e33_h04_red.rs",
        "crates/lodestar-bench/tests/a6_dependencias.rs",
        "crates/lodestar-bench/tests/e33_h04_repair_red2.rs",
        "crates/lodestar-bench/tests/e33_h04_review_repair_red.rs",
        "crates/lodestar-bench/tests/e33_h04_final_review_red.rs",
        "crates/lodestar-bench/tests/e33_h04_repair2_red.rs",
        "crates/lodestar-bench/tests/e33_h04_review_repair3_red.rs",
        "crates/lodestar-bench/tests/a6_ci_portabilidad.rs",
    ]
    .into_iter()
    .collect();
    assert_eq!(
        got, expected,
        "suite H04 debe incluir todos los targets actuales"
    );
    assert_eq!(
        manifest["owned_test"],
        "crates/lodestar-bench/tests/e33_h04_review_repair3_red.rs"
    );
    assert!(
        manifest["criteria_tests"]["wire"]
            .as_str()
            .is_some_and(|value| value.contains("wire_calibration_chain")),
        "suite declara la cadena wire fresca"
    );
}

#[test]
fn smoke_muestras_y_trace_cruzan_reloj_externo_y_no_aceptan_ceros() {
    let source = TempDir::new().expect("root medido canónico");
    let cycle = TempDir::new().expect("root ciclo");
    write_fixture(source.path(), 40);
    write_fixture(cycle.path(), 17);
    let before = tree(source.path());
    let (report, wall) = run_smoke(source.path(), cycle.path(), 3);
    assert!(wall > Duration::from_millis(0), "reloj externo cero");
    let mut measured_ns = 0u128;
    let mut all_samples = Vec::new();
    for row in report["measurements"].as_array().expect("measurements") {
        let variant = row["variant"].as_str().expect("variant");
        let mut variant_samples = BTreeSet::new();
        for tool in TOOLS {
            let metric = &row["tools"][tool];
            let samples = metric_samples(metric, &format!("{variant}/{tool}"));
            measured_ns += samples.iter().map(|value| *value as u128).sum::<u128>();
            all_samples.extend(samples.iter().copied());
            variant_samples.extend(samples.iter().copied());
            let trace = report["acquisition_trace"][variant][tool]
                .as_array()
                .unwrap_or_else(|| panic!("trace {variant}/{tool}"));
            assert_eq!(trace.len(), samples.len());
            for (index, item) in trace.iter().enumerate() {
                assert!(item["elapsed_ns"].as_u64().is_some_and(|value| value > 0));
                assert_eq!(item["elapsed_ns"].as_u64(), Some(samples[index]));
                assert_semantic_non_empty(
                    tool,
                    &item["result"],
                    &format!("trace/{variant}/{tool}"),
                );
            }
            assert_semantic_non_empty(tool, &metric["result"], &format!("metric/{variant}/{tool}"));
        }
        let cold_open = metric_samples(&row["cold_open"], &format!("{variant}/cold_open"));
        measured_ns += cold_open.iter().map(|value| *value as u128).sum::<u128>();
        all_samples.extend(cold_open.iter().copied());
        variant_samples.extend(cold_open.iter().copied());
        if variant == "sqlite-raw" {
            let rebuild = metric_samples(&row["rebuild"], "sqlite-raw/rebuild");
            measured_ns += rebuild.iter().map(|value| *value as u128).sum::<u128>();
            all_samples.extend(rebuild.iter().copied());
            variant_samples.extend(rebuild.iter().copied());
        }
        assert!(
            variant_samples.len() >= 2,
            "{variant}: las muestras de la variante deben contener al menos dos duraciones distintas"
        );
    }
    assert!(measured_ns > 0, "suma de samples cero");
    assert!(
        measured_ns.saturating_mul(1_000) >= wall.as_nanos(),
        "las muestras internas no cruzan la magnitud del reloj externo"
    );
    assert!(
        measured_ns <= wall.as_nanos(),
        "las mediciones secuenciales no pueden exceder el reloj externo del proceso"
    );
    assert!(
        all_samples.windows(2).any(|pair| pair[0] != pair[1]),
        "toda la matriz de muestras es un sentinel constante"
    );
    assert_eq!(tree(source.path()), before, "root medido alterado");
}

#[test]
fn ciclo_app_disk_posee_root_captura_cambio_unico_y_recibo_cargable() {
    let measured = TempDir::new().expect("root medido canónico");
    let cycle = TempDir::new().expect("root ciclo");
    write_fixture(measured.path(), 40);
    write_fixture(cycle.path(), 17);
    let measured_before = tree(measured.path());
    let cycle_before = tree(cycle.path());
    let (report, _) = run_smoke(measured.path(), cycle.path(), 1);
    assert_eq!(
        tree(measured.path()),
        measured_before,
        "root medido alterado"
    );
    assert_eq!(
        fs::read_to_string(cycle.path().join("control.md")).expect("control"),
        "---\ntags: [h04, control]\nservice: bench\n---\n# before-state\n\nafter-state-1\n"
    );
    let cycle_report = &report["change_cycle"];
    assert_eq!(cycle_report["source"], "app/disk");
    assert_eq!(cycle_report["iterations"], 1);
    assert_eq!(cycle_report["changed_paths"], json!(["control.md"]));
    let receipts = cycle_report["receipts"].as_array().expect("receipts");
    assert_eq!(receipts.len(), 1, "debe haber un receipt real");
    let receipt_path = receipts[0]["receipt_path"].as_str().expect("receipt_path");
    let relative = Path::new(receipt_path);
    assert!(
        !relative.is_absolute()
            && !relative
                .components()
                .any(|component| matches!(component, std::path::Component::ParentDir))
    );
    let receipt_file = cycle.path().join(relative);
    assert!(
        receipt_file.is_file(),
        "receipt_path no existe bajo CHANGE_PARENT_ENV"
    );
    let receipt_value: Value =
        serde_json::from_slice(&fs::read(&receipt_file).expect("receipt bytes"))
            .expect("receipt JSON");
    assert_eq!(receipt_value, receipts[0]["receipt"]);
    let receipt_id = serde_json::from_value(receipt_value["id"].clone()).expect("ReceiptId");
    let app = App::open(cycle.path()).expect("App ciclo");
    let loaded = app
        .workspace()
        .load_receipt(&receipt_id)
        .expect("load_receipt real");
    assert_eq!(
        serde_json::to_value(loaded).expect("receipt serializable"),
        receipt_value
    );
    let receipt_files: Vec<_> = fs::read_dir(cycle.path().join(".lodestar/runtime/receipts"))
        .expect("receipts dir")
        .filter_map(Result::ok)
        .filter(|e| e.path().extension().is_some_and(|x| x == "json"))
        .collect();
    assert_eq!(receipt_files.len(), 1, "solo un receipt persistido");
    assert_eq!(cycle_before["child.md"], tree(cycle.path())["child.md"]);
    let changed: Vec<_> = tree(cycle.path())
        .iter()
        .filter_map(|(path, entry)| (cycle_before.get(path) != Some(entry)).then_some(path.clone()))
        .collect();
    assert!(changed.iter().any(|path| path == "control.md"));
    assert!(
        changed.iter().all(|path| path == "control.md"
            || path == ".gitignore"
            || path == receipt_path
            || path == ".lodestar"
            || path.starts_with(".lodestar/")),
        "mutación ajena al ciclo: {changed:?}"
    );
}

fn assert_matrix_run(run: &Value, context: &str) {
    let rows = run["measurements"]
        .as_array()
        .unwrap_or_else(|| panic!("{context}: measurements"));
    assert_eq!(rows.len(), 3);
    let names: BTreeSet<_> = rows
        .iter()
        .map(|row| row["variant"].as_str().expect("variant"))
        .collect();
    assert_eq!(names, VARIANTS.into_iter().collect());
    let app_results = result_map(run, "app_results");
    let seam_results = result_map(run, "seam_results");
    for row in rows {
        let variant = row["variant"].as_str().expect("variant");
        assert!(row["document_count"].as_u64().is_some_and(|n| n > 0));
        let tools = row["tools"].as_object().expect("tools");
        assert_eq!(tools.len(), 7);
        for tool in TOOLS {
            let metric = &tools[tool];
            let _ = metric_samples(metric, &format!("{context}/{variant}/{tool}"));
            assert_semantic_non_empty(
                tool,
                &metric["result"],
                &format!("{context}/{variant}/{tool}"),
            );
            assert_eq!(
                &metric["result"], &app_results[tool],
                "{context}/{variant}/{tool} != app_results"
            );
            assert_eq!(
                &metric["result"], &seam_results[tool],
                "{context}/{variant}/{tool} != seam_results"
            );
        }
        let _ = metric_samples(&row["cold_open"], &format!("{context}/{variant}/cold_open"));
        if variant == "sqlite-raw" {
            let _ = metric_samples(&row["rebuild"], &format!("{context}/{variant}/rebuild"));
        }
    }
}

#[test]
fn artefact_full_conserva_2x3x3x7_y_resultados_exactos_no_vacios() {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/e33_h04_full_format.json");
    let report: Value = serde_json::from_str(&fs::read_to_string(path).expect("artefacto full"))
        .expect("JSON full");
    assert_eq!(report["profiles"], json!(PROFILES));
    assert_eq!(report["scales"], json!(SCALES));
    let runs = report["runs"].as_array().expect("runs");
    assert_eq!(runs.len(), 6);
    let mut seen = BTreeSet::new();
    for run in runs {
        let profile = run["profile"].as_str().expect("profile");
        let scale = run["scale"].as_u64().expect("scale");
        assert!(seen.insert((profile, scale)), "run duplicado");
        assert_matrix_run(run, &format!("{profile}/{scale}"));
    }
    assert_eq!(seen.len(), 6);
}

#[test]
fn artefact_negativos_conserva_error_real_y_equivalente_app_seam() {
    let report: Value = serde_json::from_str(
        &fs::read_to_string(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("tests/fixtures/e33_h04_full_format.json"),
        )
        .expect("artefacto full"),
    )
    .expect("JSON full");
    for run in report["runs"].as_array().expect("runs") {
        let negatives = run["negative_results"]
            .as_object()
            .expect("negative_results");
        for tool in ["knowledge_get", "metadata_inspect"] {
            let row = negatives[tool].as_object().expect("negative tool");
            assert_eq!(row["app"], row["seam"], "negative {tool} diverge app/seam");
            let error = row["app"]["error"].as_object().expect("error object");
            assert!(!error["code"].as_str().unwrap_or_default().is_empty());
            assert!(!error["message"].as_str().unwrap_or_default().is_empty());
        }
        let negative_names: BTreeSet<_> = negatives.keys().map(String::as_str).collect();
        assert_eq!(
            negative_names,
            ["knowledge_get", "metadata_inspect"].into_iter().collect(),
            "la baseline ratificada solo cubre los dos negativos donde aplica"
        );
    }
}

#[test]
fn wire_live_ejecuta_las_siete_tools_y_valida_semantica_real() {
    let root = TempDir::new().expect("root wire");
    write_fixture(root.path(), 8);
    let build_target = TempDir::new().expect("target MCP efímero");
    let build = Command::new("cargo")
        .args([
            "build",
            "--offline",
            "-p",
            "lodestar-mcp",
            "--bin",
            "lodestar-mcp",
        ])
        .env("CARGO_NET_OFFLINE", "true")
        .env("CARGO_TARGET_DIR", build_target.path())
        .current_dir(repo_root())
        .output()
        .expect("compilar lodestar-mcp offline");
    assert!(
        build.status.success(),
        "wire live no pudo construir lodestar-mcp offline: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let binary = build_target
        .path()
        .join("debug")
        .join(format!("lodestar-mcp{}", std::env::consts::EXE_SUFFIX));
    assert!(
        binary.is_file(),
        "build offline no produjo {}",
        binary.display()
    );
    let args: BTreeMap<&str, Value> = BTreeMap::from([
        ("workspace_status", json!({})),
        (
            "knowledge_search",
            json!({"text":"marker-search-h04", "where":r#"service = "bench""#}),
        ),
        (
            "knowledge_get",
            json!({"ref":{"path":"child.md"}, "include":["body","frontmatter","outgoingLinks","backlinks"]}),
        ),
        ("metadata_inspect", json!({"mode":"field", "field":"tags"})),
        ("graph_query", json!({"operation":"components"})),
        (
            "impact_analyze",
            json!({"ref":{"path":"child.md"}, "proposedOperation":{"kind":"delete"}}),
        ),
        (
            "knowledge_check",
            json!({"scope":{"kind":"workspace"}, "minimumSeverity":"info", "includeSuggestedFixes":true}),
        ),
    ]);
    for tool in TOOLS {
        let started = Instant::now();
        let output = Command::new("python3")
            .args(["docs/qa/testbench/lodestar_harness.py", "--root"])
            .arg(root.path())
            .args(["--profile", "readonly", "--binary"])
            .arg(&binary)
            .args(["--call", tool])
            .arg(serde_json::to_string(&args[tool]).expect("args JSON"))
            .current_dir(repo_root())
            .output()
            .expect("harness live");
        let wall = started.elapsed();
        assert!(
            output.status.success(),
            "wire {tool} falló: stdout={} stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let observation: Value =
            serde_json::from_slice(&output.stdout).expect("stdout harness JSON");
        assert!(wall > Duration::ZERO, "wire {tool}: reloj externo cero");
        assert!(
            !output.stdout.is_empty(),
            "wire {tool}: payload externo vacío"
        );
        assert_eq!(observation["kind"], "call");
        assert_eq!(observation["tool"], tool);
        assert_eq!(observation["is_error"], false, "error MCP {tool}");
        assert_wire_semantic_non_empty(tool, &observation["structured"], &format!("wire/{tool}"));
        assert!(!observation["text"].as_str().unwrap_or_default().is_empty());
        assert!(observation["text"]
            .as_str()
            .is_some_and(|text| !text.is_empty()));
    }
}

fn sha256(path: &Path) -> String {
    let output = Command::new("python3")
        .args([
            "-c",
            "import hashlib,pathlib,sys; print(hashlib.sha256(pathlib.Path(sys.argv[1]).read_bytes()).hexdigest())",
        ])
        .arg(path)
        .output()
        .expect("python3 para SHA-256");
    assert!(
        output.status.success(),
        "SHA-256 falló: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .expect("hash")
        .to_owned()
}

fn transcript_wall_seconds(path: &Path) -> Vec<f64> {
    let text = fs::read_to_string(path).expect("transcript tiempo");
    let values: Vec<f64> = text
        .lines()
        .filter_map(|line| line.strip_prefix("real "))
        .map(|value| value.trim().parse::<f64>().expect("real parseable"))
        .collect();
    assert!(
        !values.is_empty(),
        "transcript sin línea real: {}",
        path.display()
    );
    assert!(
        values.iter().all(|value| *value > 0.0),
        "transcript con real no positivo"
    );
    values
}

fn assert_command_has(command: &Value, token: &str, context: &str) {
    let rendered = command
        .as_str()
        .map(str::to_owned)
        .or_else(|| serde_json::to_string(command).ok())
        .unwrap_or_default();
    assert!(
        rendered.contains(token),
        "{context}: comando no contiene {token:?}"
    );
}

#[test]
fn wire_historico_exige_stdout_transcript_versionados_y_hashes_verificables() {
    let root = repo_root();
    let path = root.join("docs/qa/e33-h04-wire-evidencia-2026-08-22.json");
    assert!(path.is_file(), "falta evidencia wire histórica cruda");
    let evidence: Value = serde_json::from_str(&fs::read_to_string(&path).expect("wire histórico"))
        .expect("wire histórico JSON");
    let observations = evidence["observations"].as_array().expect("observations");
    assert_eq!(
        observations.len(),
        10,
        "wire histórico: exactamente 5 status + 5 search"
    );
    let official_rel = evidence["official_artifact"]
        .as_str()
        .expect("official_artifact");
    let official_path = root.join(official_rel);
    let official: Value =
        serde_json::from_str(&fs::read_to_string(&official_path).expect("artifact oficial wire"))
            .expect("artifact oficial wire JSON");
    let official_results = official["results"]
        .as_array()
        .expect("resultados wire oficiales");
    assert_eq!(official_results.len(), 2, "artifact oficial: status/search");
    let mut seen = BTreeSet::new();
    for observation in observations {
        let tool = observation["tool"].as_str().expect("tool histórica");
        assert!(matches!(tool, "workspace_status" | "knowledge_search"));
        let index = observation["index"].as_u64().expect("index histórica");
        assert!((1..=5).contains(&index), "index histórica fuera de 1..=5");
        assert!(
            seen.insert((tool, index)),
            "observación histórica duplicada"
        );
        assert_eq!(observation["profile"].as_str(), Some("readonly"));
        assert!(!observation["binary"]
            .as_str()
            .unwrap_or_default()
            .is_empty());
        assert!(observation["binary"]
            .as_str()
            .unwrap()
            .contains("lodestar-mcp"));
        assert!(!observation["root"].as_str().unwrap_or_default().is_empty());
        let args = observation["args"].as_object().expect("args histórica");
        let expected_args = match tool {
            "workspace_status" => json!({}),
            "knowledge_search" => {
                json!({"text":"marker-search-h04", "where":"service = \"bench\""})
            }
            _ => unreachable!(),
        };
        assert_eq!(
            Value::Object(args.clone()),
            expected_args,
            "args históricas"
        );
        assert_command_has(
            &observation["command"],
            "lodestar_harness.py",
            "wire histórico",
        );
        assert_command_has(&observation["command"], "--profile", "wire histórico");
        assert_command_has(&observation["command"], "--binary", "wire histórico");
        assert_command_has(&observation["command"], tool, "wire histórico");
        let stdout = root.join(observation["raw_stdout"].as_str().expect("raw_stdout"));
        let transcript = root.join(
            observation["time_transcript"]
                .as_str()
                .expect("time_transcript"),
        );
        assert!(
            stdout.is_file(),
            "stdout no versionado: {}",
            stdout.display()
        );
        assert!(
            transcript.is_file(),
            "transcript no versionado: {}",
            transcript.display()
        );
        let raw: Value = serde_json::from_str(&fs::read_to_string(&stdout).expect("raw stdout"))
            .expect("raw stdout JSON parseable");
        assert_eq!(raw["kind"], "call");
        assert_eq!(raw["tool"], tool);
        assert_eq!(raw["is_error"], false);
        assert_wire_semantic_non_empty(tool, &raw["structured"], "wire histórico structured");
        assert!(raw["text"].as_str().is_some_and(|text| !text.is_empty()));
        let wall = transcript_wall_seconds(&transcript);
        assert_eq!(wall.len(), 1, "cada observación exige una medida wall");
        assert_eq!(observation["wall_seconds"].as_f64(), Some(wall[0]));
        let payload = observation["payload_bytes"]
            .as_u64()
            .expect("payload histórico");
        assert!(payload > 0, "payload histórico cero");
        assert!(
            observation["result_check"].is_object(),
            "result_check histórico ausente"
        );
        assert_eq!(
            observation["sha256_stdout"].as_str(),
            Some(sha256(&stdout).as_str())
        );
        assert_eq!(
            observation["sha256_transcript"].as_str(),
            Some(sha256(&transcript).as_str())
        );
    }
    assert_eq!(seen.len(), 10);
    for official_row in official_results {
        let tool = official_row["tool"].as_str().expect("tool oficial");
        let rows: Vec<&Value> = observations
            .iter()
            .filter(|row| row["tool"] == tool)
            .collect();
        assert_eq!(
            rows.len(),
            5,
            "artifact oficial: cinco observaciones de {tool}"
        );
        let mut elapsed = Vec::new();
        for row in rows {
            let index = row["index"].as_u64().unwrap() as usize;
            let time = row["wall_seconds"].as_f64().unwrap();
            elapsed.push(time);
            let expected_official = official_row["real_seconds"][index - 1]
                .as_f64()
                .expect("real oficial");
            assert!(
                (time - expected_official).abs() <= 1e-9,
                "observación/payload temporal no cruza artifact"
            );
            assert_eq!(
                row["payload_bytes"], official_row["payload_bytes"],
                "payload no cruza artifact"
            );
            assert_eq!(
                row["result_check"], official_row["result_check"],
                "result_check no cruza artifact"
            );
        }
        let mut sorted = elapsed.clone();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p50 = sorted[sorted.len() / 2];
        let p95 = sorted[((sorted.len() * 95).saturating_sub(1) / 100).min(sorted.len() - 1)];
        assert_eq!(official_row["sample_count"].as_u64(), Some(5));
        assert_eq!(official_row["p50_seconds"].as_f64(), Some(p50));
        assert_eq!(official_row["p95_seconds"].as_f64(), Some(p95));
        assert!(p50 > 0.0 && p95 > 0.0 && p95 >= p50);
    }
}

fn run_api_snapshot(
    script: &Path,
    manifest: &Path,
    output: &Path,
    target: &Path,
    cwd: &Path,
) -> std::process::Output {
    Command::new("python3")
        .arg(script)
        .args(["--manifest-path"])
        .arg(manifest)
        .args(["--output"])
        .arg(output)
        .args(["--target-dir"])
        .arg(target)
        .env("CARGO_NET_OFFLINE", "true")
        .env("RUSTC_BOOTSTRAP", "1")
        .current_dir(cwd)
        .output()
        .expect("public-api snapshot")
}

#[test]
fn resolver_develop_en_pr_prefiere_origin_develop_y_no_head() {
    let mut seen = Vec::new();
    let resolved = resolve_develop_ref_with(Some("develop"), |reference| {
        seen.push(reference.to_owned());
        match reference {
            "origin/develop" => Ok("origin-commit".to_owned()),
            _ => Err("no debe consultarse en presencia del remoto".to_owned()),
        }
    })
    .expect("origin/develop debe ser la primera opción en PR");

    assert_eq!(seen, ["origin/develop"]);
    assert_eq!(
        resolved,
        ("origin/develop".to_owned(), "origin-commit".to_owned())
    );
    assert!(!seen.iter().any(|reference| reference == "HEAD"));
}

#[test]
fn resolver_develop_en_pr_hace_fallback_a_develop_local_sin_head() {
    let mut seen = Vec::new();
    let resolved = resolve_develop_ref_with(Some("develop"), |reference| {
        seen.push(reference.to_owned());
        match reference {
            "origin/develop" => Err("remote-tracking ref ausente".to_owned()),
            "develop" => Ok("local-commit".to_owned()),
            _ => Err("referencia inesperada".to_owned()),
        }
    })
    .expect("develop local debe ser fallback de PR");

    assert_eq!(seen, ["origin/develop", "develop"]);
    assert_eq!(resolved, ("develop".to_owned(), "local-commit".to_owned()));
    assert!(!seen.iter().any(|reference| reference == "HEAD"));
}

#[test]
fn resolver_develop_fuera_de_pr_hace_fallback_a_origin_develop_sin_head() {
    let mut seen = Vec::new();
    let resolved = resolve_develop_ref_with(None, |reference| {
        seen.push(reference.to_owned());
        match reference {
            "develop" => Err("rama local ausente".to_owned()),
            "origin/develop" => Ok("remote-commit".to_owned()),
            _ => Err("referencia inesperada".to_owned()),
        }
    })
    .expect("origin/develop debe ser fallback fuera de PR");

    assert_eq!(seen, ["develop", "origin/develop"]);
    assert_eq!(
        resolved,
        ("origin/develop".to_owned(), "remote-commit".to_owned())
    );
    assert!(!seen.iter().any(|reference| reference == "HEAD"));
}

#[test]
fn resolver_develop_sin_referencias_falla_con_diagnostico_util_y_no_head() {
    let mut seen = Vec::new();
    let error = resolve_develop_ref_with(Some("develop"), |reference| {
        seen.push(reference.to_owned());
        Err("ref ausente".to_owned())
    })
    .expect_err("sin develop ni origin/develop debe fallar");

    assert_eq!(seen, ["origin/develop", "develop"]);
    assert!(error.contains("GITHUB_BASE_REF"));
    assert!(error.contains("origin/develop"));
    assert!(error.contains("develop"));
    assert!(error.contains("intentos"));
    assert!(error.contains("no se usa HEAD"));
    assert!(!seen.iter().any(|reference| reference == "HEAD"));
}

#[test]
fn archive_develop_usa_el_sha_resuelto_y_no_la_referencia_simbolica() {
    let repo = TempDir::new().expect("repo Git temporal");
    let git = |args: &[&str]| {
        let output = Command::new("git")
            .args(args)
            .current_dir(repo.path())
            .output()
            .expect("ejecutar Git temporal");
        assert!(
            output.status.success(),
            "git {args:?} falló: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    };

    git(&["init", "--quiet"]);
    git(&["config", "user.email", "bench@example.invalid"]);
    git(&["config", "user.name", "Lodestar Bench"]);
    fs::write(repo.path().join("marker.txt"), "commit fijado\n").expect("marker A");
    git(&["add", "marker.txt"]);
    git(&["commit", "--quiet", "-m", "commit fijado"]);
    let fixed_commit = git(&["rev-parse", "HEAD"]);

    fs::write(repo.path().join("marker.txt"), "ref avanzada\n").expect("marker B");
    git(&["commit", "--quiet", "-am", "avanza ref"]);
    let moved_ref_commit = git(&["rev-parse", "HEAD"]);
    git(&[
        "update-ref",
        "refs/remotes/origin/develop",
        &moved_ref_commit,
    ]);
    assert_ne!(fixed_commit, moved_ref_commit);

    let archive = repo.path().join("develop.tar");
    let resolved = ("origin/develop".to_owned(), fixed_commit.clone());
    let archived = archive_resolved_develop(repo.path(), &archive, &resolved)
        .expect("archivar commit develop resuelto");
    assert!(
        archived.status.success(),
        "git archive falló: {}",
        String::from_utf8_lossy(&archived.stderr)
    );

    let archived_commit = Command::new("git")
        .arg("get-tar-commit-id")
        .stdin(Stdio::from(fs::File::open(&archive).expect("abrir tar")))
        .output()
        .expect("leer commit del tar");
    assert!(archived_commit.status.success());
    assert_eq!(
        String::from_utf8_lossy(&archived_commit.stdout).trim(),
        fixed_commit
    );
}

fn semantic_mentions(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(text) => text.contains(needle),
        Value::Array(values) => values.iter().any(|value| semantic_mentions(value, needle)),
        Value::Object(values) => values
            .values()
            .any(|value| semantic_mentions(value, needle)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn assert_enum_variants_are_stable(semantic: &[Value], context: &str) {
    for record in semantic
        .iter()
        .filter(|record| record.get("kind").and_then(Value::as_str) == Some("enum"))
    {
        let path = record["path"].as_str().unwrap_or("<enum sin path>");
        for variant in record["variants"]
            .as_array()
            .unwrap_or_else(|| panic!("{context}/{path}: variants debe ser array"))
        {
            let name = variant["name"].as_str().unwrap_or("<variante sin nombre>");
            match &variant["kind"] {
                Value::String(kind) => assert_eq!(
                    kind, "plain",
                    "{context}/{path}/{name}: kind string desconocido"
                ),
                Value::Object(kind) if kind.contains_key("tuple") => assert!(
                    kind["tuple"]
                        .as_array()
                        .is_some_and(|fields| fields.iter().all(Value::is_string)),
                    "{context}/{path}/{name}: tuple debe contener tipos estables, no IDs de rustdoc"
                ),
                Value::Object(kind) if kind.contains_key("struct") => assert!(
                    kind["struct"]["fields"].as_array().is_some_and(|fields| {
                        fields.iter().all(|field| {
                            field.get("name").is_some_and(Value::is_string)
                                && field.get("type").is_some_and(Value::is_string)
                        })
                    }),
                    "{context}/{path}/{name}: struct debe contener nombres/tipos estables, no IDs de rustdoc"
                ),
                other => panic!("{context}/{path}/{name}: kind no canónico: {other}"),
            }
        }
    }
}

fn assert_public_api_snapshot_shape(snapshot: &Value, context: &str) {
    let semantic = snapshot
        .get("semantic")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}: semantic debe ser array"));
    assert!(
        !semantic.is_empty(),
        "{context}: semantic no puede estar vacío"
    );
    for symbol in [
        "App",
        "AppError",
        "workspace_status",
        "knowledge_get",
        "change_apply",
    ] {
        assert!(
            semantic_mentions(&Value::Array(semantic.clone()), symbol),
            "{context}: semantic debe conservar el símbolo/firma {symbol}"
        );
    }
    assert!(
        !semantic_mentions(&Value::Array(semantic.clone()), "ReadServices"),
        "{context}: ReadServices es implementación interna y no puede aparecer en API default"
    );
    assert_enum_variants_are_stable(semantic, context);

    let metadata = snapshot
        .get("metadata")
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{context}: metadata debe ser objeto"));
    let package = metadata
        .get("package")
        .and_then(Value::as_str)
        .or_else(|| metadata.get("package_name").and_then(Value::as_str));
    assert_eq!(
        package,
        Some("lodestar-app"),
        "{context}: metadata debe identificar package lodestar-app"
    );

    let default_features = metadata
        .get("features")
        .and_then(|features| match features {
            Value::Object(features) => features.get("default").and_then(Value::as_array),
            Value::Array(features) => Some(features),
            _ => None,
        })
        .or_else(|| metadata.get("default_features").and_then(Value::as_array))
        .unwrap_or_else(|| panic!("{context}: metadata debe declarar features default"));
    assert!(
        default_features.iter().all(Value::is_string),
        "{context}: features default debe ser array de nombres reales (también puede estar vacío)"
    );

    let manifest_processed = metadata
        .get("manifest_processed")
        .and_then(Value::as_bool)
        .is_some_and(|processed| processed)
        || ["manifest", "manifest_path"]
            .into_iter()
            .filter_map(|key| metadata.get(key).and_then(Value::as_str))
            .any(|path| !path.is_empty() && path.ends_with("Cargo.toml"));
    assert!(
        manifest_processed,
        "{context}: metadata debe demostrar que procesó el manifest Cargo.toml"
    );
}

#[test]
fn public_api_variant_tuple_preserva_el_orden_posicional() {
    let root = repo_root();
    let fixture = TempDir::new().expect("fixture API tuple");
    fs::create_dir_all(fixture.path().join("src")).expect("src fixture");
    fs::write(
        fixture.path().join("Cargo.toml"),
        "[package]\nname = \"variant-fixture\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .expect("manifest fixture");
    fs::write(
        fixture.path().join("src/lib.rs"),
        "pub enum Pair { Values(u32, String) }\n",
    )
    .expect("lib fixture");
    let output = fixture.path().join("snapshot.json");
    let run = run_api_snapshot(
        &root.join("scripts/public-api-snapshot.py"),
        &fixture.path().join("Cargo.toml"),
        &output,
        &fixture.path().join("target"),
        fixture.path(),
    );
    assert!(
        run.status.success(),
        "snapshot de tuple falló: {}",
        String::from_utf8_lossy(&run.stderr)
    );
    let snapshot: Value =
        serde_json::from_str(&fs::read_to_string(output).expect("leer snapshot de tuple"))
            .expect("snapshot de tuple JSON");
    let pair = snapshot["semantic"]
        .as_array()
        .and_then(|semantic| {
            semantic
                .iter()
                .find(|record| record["path"] == "variant_fixture::Pair")
        })
        .expect("enum Pair en snapshot");
    assert_eq!(
        pair["variants"][0]["kind"]["tuple"],
        json!(["u32", "String"]),
        "el orden de una tuple variant forma parte de la API"
    );
}

#[test]
fn public_api_compara_develop_archive_baseline_y_current_con_features_default() {
    let root = repo_root();
    let temp = TempDir::new().expect("temp API");
    let archive = temp.path().join("develop.tar");
    let resolved = resolve_develop_ref(&root).unwrap_or_else(|error| {
        panic!(
            "no se pudo resolver la base develop para archivar API (GITHUB_BASE_REF={:?}): {error}",
            std::env::var("GITHUB_BASE_REF").ok()
        )
    });
    let (develop_ref, develop_commit) = (&resolved.0, &resolved.1);
    let archived = archive_resolved_develop(&root, &archive, &resolved).expect("git archive");
    assert!(
        archived.status.success(),
        "git archive de {develop_ref} ({develop_commit}) falló: {}",
        String::from_utf8_lossy(&archived.stderr)
    );
    let develop = temp.path().join("develop");
    fs::create_dir_all(&develop).expect("develop dir");
    let extracted = Command::new("tar")
        .args(["-xf"])
        .arg(&archive)
        .current_dir(&develop)
        .output()
        .expect("tar");
    assert!(extracted.status.success(), "tar develop falló");
    let script_current = root.join("scripts/public-api-snapshot.py");
    assert!(
        script_current.is_file(),
        "current debe aportar el script public-api-snapshot.py"
    );
    let develop_out = temp.path().join("develop-api.json");
    let current_out = temp.path().join("current-api.json");
    let develop_run = run_api_snapshot(
        &script_current,
        &develop.join("crates/lodestar-app/Cargo.toml"),
        &develop_out,
        &temp.path().join("target-develop"),
        &develop,
    );
    assert!(
        develop_run.status.success(),
        "snapshot develop falló: {}",
        String::from_utf8_lossy(&develop_run.stderr)
    );
    let current_run = run_api_snapshot(
        &script_current,
        &root.join("crates/lodestar-app/Cargo.toml"),
        &current_out,
        &temp.path().join("target-current"),
        &root,
    );
    assert!(
        current_run.status.success(),
        "snapshot current falló: {}",
        String::from_utf8_lossy(&current_run.stderr)
    );
    let develop_value: Value =
        serde_json::from_str(&fs::read_to_string(&develop_out).expect("API develop"))
            .expect("API develop JSON");
    let current_value: Value =
        serde_json::from_str(&fs::read_to_string(&current_out).expect("API current"))
            .expect("API current JSON");
    let baseline: Value = serde_json::from_str(
        &fs::read_to_string(root.join("docs/qa/e33-h04-lodestar-app-public-api.json"))
            .expect("baseline API"),
    )
    .expect("baseline API JSON");
    assert_public_api_snapshot_shape(&develop_value, "API develop");
    assert_public_api_snapshot_shape(&baseline, "API baseline");
    assert_public_api_snapshot_shape(&current_value, "API current");
    assert_eq!(
        baseline["metadata"]["source_commit"].as_str(),
        Some(develop_commit.as_str())
    );
    assert_eq!(develop_value["semantic"], baseline["semantic"]);
    assert_eq!(develop_value["semantic"], current_value["semantic"]);
}
