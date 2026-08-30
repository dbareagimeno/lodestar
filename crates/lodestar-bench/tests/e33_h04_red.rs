//! Fase roja independiente de E33-H04.
//!
//! Estos tests hablan únicamente con el ejecutable privado del banco.  El banco aún es un
//! scaffold, por lo que esta suite debe compilar y fallar por ausencia de su capacidad.

use serde_json::{Map, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use tempfile::TempDir;

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

fn bench() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"));
    command.env("RUST_BACKTRACE", "1");
    command
}

fn write_fixture(root: &Path) {
    fs::create_dir_all(root).expect("fixture root");
    fs::write(
        root.join("control.md"),
        "---\ntags: [h04, control]\nservice: bench\n---\n# Control\nmarker-search-h04\n[child](child.md)\n[missing](missing.md)\n",
    )
    .expect("control fixture");
    fs::write(
        root.join("child.md"),
        "---\ntags: [child]\nservice: bench\n---\n# Child\nmarker-get-h04\n[leaf](leaf.md)\n",
    )
    .expect("child fixture");
    fs::write(
        root.join("leaf.md"),
        "---\ntags: [leaf]\nservice: bench\n---\n# Leaf\nmarker-impact-h04\n",
    )
    .expect("leaf fixture");
    // A malformed frontmatter value gives knowledge_check a real diagnostic to inspect.
    fs::write(root.join("broken.md"), "---\ntags: [\n---\n# Broken\n").expect("diagnostic fixture");
}

fn json_stdout(output: std::process::Output, context: &str) -> Value {
    assert!(
        output.status.success(),
        "{context}: el banco debe terminar correctamente; status={} stderr={} stdout={}",
        output.status,
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    assert!(
        !stdout.trim().is_empty(),
        "{context}: stdout no puede estar vacío"
    );
    serde_json::from_str(stdout.trim()).unwrap_or_else(|error| {
        panic!("{context}: stdout debe ser un único informe JSON: {error}; stdout={stdout}")
    })
}

fn object<'a>(value: &'a Value, key: &str, context: &str) -> &'a Map<String, Value> {
    value
        .get(key)
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{context}: falta el objeto JSON {key:?}"))
}

fn array<'a>(value: &'a Value, key: &str, context: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("{context}: se esperaba array JSON {key:?}"))
}

fn required<'a>(value: &'a Value, key: &str, context: &str) -> &'a Value {
    value
        .get(key)
        .unwrap_or_else(|| panic!("{context}: falta la clave JSON {key:?}"))
}

fn non_empty_real(value: &Value, context: &str) {
    match value {
        Value::Null => panic!("{context}: resultado nulo"),
        Value::String(value) => assert!(!value.is_empty(), "{context}: string vacío"),
        Value::Array(value) => assert!(!value.is_empty(), "{context}: array vacío"),
        Value::Object(value) => assert!(!value.is_empty(), "{context}: objeto vacío"),
        Value::Bool(_) | Value::Number(_) => {}
    }
}

fn metric<'a>(metric: &'a Value, context: &str) -> &'a Map<String, Value> {
    let metric = metric
        .as_object()
        .unwrap_or_else(|| panic!("{context}: métrica debe ser objeto"));
    let sample_count = metric
        .get("sample_count")
        .unwrap_or_else(|| panic!("{context}: falta sample_count"))
        .as_u64()
        .unwrap_or_else(|| panic!("{context}: sample_count debe ser entero JSON"));
    assert!(sample_count > 0, "{context}: sample_count debe ser > 0");
    let p50 = metric
        .get("p50_ns")
        .unwrap_or_else(|| panic!("{context}: falta p50_ns"))
        .as_u64()
        .unwrap_or_else(|| panic!("{context}: p50_ns debe ser entero JSON >= 0"));
    let p95 = metric
        .get("p95_ns")
        .unwrap_or_else(|| panic!("{context}: falta p95_ns"))
        .as_u64()
        .unwrap_or_else(|| panic!("{context}: p95_ns debe ser entero JSON >= 0"));
    assert!(p95 >= p50, "{context}: p95 debe ser >= p50");
    let payload = metric
        .get("payload_bytes")
        .unwrap_or_else(|| panic!("{context}: falta payload_bytes"))
        .as_u64()
        .unwrap_or_else(|| panic!("{context}: payload_bytes debe ser entero JSON >= 0"));
    assert!(payload > 0, "{context}: payload_bytes debe ser > 0");
    let result = metric
        .get("result")
        .unwrap_or_else(|| panic!("{context}: falta result"));
    let encoded = serde_json::to_vec(result).expect("result serializable");
    assert_eq!(
        payload as usize,
        encoded.len(),
        "{context}: payload_bytes debe ser serde_json::to_vec(result).len()"
    );
    metric
}

fn row_tool<'a>(row: &'a Value, tool: &str, context: &str) -> &'a Value {
    let tools = object(row, "tools", context);
    tools
        .get(tool)
        .unwrap_or_else(|| panic!("{context}: falta tool {tool}"))
}

fn measurement_matrix(report: &Value) -> Vec<(String, Value)> {
    let variants = array(report, "variants", "matriz H04");
    assert_eq!(
        variants.len(),
        3,
        "matriz H04: variants debe tener exactamente 3 elementos"
    );
    let expected: BTreeSet<_> = VARIANTS.iter().copied().collect();
    let got: BTreeSet<_> = variants.iter().filter_map(Value::as_str).collect();
    assert_eq!(
        got, expected,
        "matriz H04: las variantes deben ser disco, SQLite-raw y RAM-memoizado"
    );

    let tools = array(report, "tools", "matriz H04");
    assert_eq!(
        tools.len(),
        7,
        "matriz H04: tools debe tener exactamente 7 elementos"
    );
    let got_tools: BTreeSet<_> = tools.iter().filter_map(Value::as_str).collect();
    assert_eq!(
        got_tools,
        TOOLS.iter().copied().collect(),
        "matriz H04: las siete lecturas deben estar nombradas"
    );

    let rows = array(report, "measurements", "matriz H04");
    assert_eq!(
        rows.len(),
        3,
        "matriz H04: measurements debe tener exactamente 3 filas"
    );
    let mut result = Vec::new();
    for row in rows {
        let variant = required(row, "variant", "matriz H04")
            .as_str()
            .unwrap_or_else(|| panic!("matriz H04: variant debe ser string"))
            .to_owned();
        assert!(
            expected.contains(variant.as_str()),
            "matriz H04: variante desconocida {variant}"
        );
        let document_count = required(row, "document_count", "matriz H04")
            .as_u64()
            .unwrap_or_else(|| panic!("matriz H04: document_count debe ser entero"));
        assert!(
            document_count > 0,
            "matriz H04: document_count debe ser > 0"
        );
        for tool in TOOLS {
            metric(
                row_tool(row, tool, "matriz H04"),
                &format!("{variant}/{tool}"),
            );
        }
        metric(
            required(row, "cold_open", "matriz H04"),
            &format!("{variant}/cold_open"),
        );
        result.push((variant, row.clone()));
    }
    let row_names: BTreeSet<_> = result.iter().map(|(variant, _)| variant.as_str()).collect();
    assert_eq!(
        row_names, expected,
        "matriz H04: measurements debe contener una fila única por variante"
    );
    result
}

fn result_for<'a>(row: &'a Value, tool: &str, context: &str) -> &'a Value {
    required(row_tool(row, tool, context), "result", context)
}

fn assert_error(value: &Value, context: &str) {
    let error = object(value, "error", context);
    let code = error
        .get("code")
        .unwrap_or_else(|| panic!("{context}: falta error.code"))
        .as_str()
        .unwrap_or_else(|| panic!("{context}: error.code debe ser string"));
    let message = error
        .get("message")
        .unwrap_or_else(|| panic!("{context}: falta error.message"))
        .as_str()
        .unwrap_or_else(|| panic!("{context}: error.message debe ser string"));
    assert!(
        !code.is_empty(),
        "{context}: error.code no puede estar vacío"
    );
    assert!(
        !message.is_empty(),
        "{context}: error.message no puede estar vacío"
    );
}

fn contains_string(value: &Value, needle: &str) -> bool {
    match value {
        Value::String(value) => value.contains(needle),
        Value::Array(values) => values.iter().any(|value| contains_string(value, needle)),
        Value::Object(values) => values.values().any(|value| contains_string(value, needle)),
        Value::Null | Value::Bool(_) | Value::Number(_) => false,
    }
}

fn normalize_temporal_values(value: &Value) -> Value {
    fn visit(value: &Value, change_context: bool) -> Value {
        match value {
            Value::Array(values) => Value::Array(
                values
                    .iter()
                    .map(|value| visit(value, change_context))
                    .collect(),
            ),
            Value::Object(values) => Value::Object(
                values
                    .iter()
                    .map(|(key, value)| {
                        let child_change_context = change_context
                            || matches!(
                                key.as_str(),
                                "change_cycle" | "receipts" | "receipt" | "apply"
                            );
                        // Keep every key and array position. `_ns` metrics are the only
                        // globally temporal values. Identity, receipt, timestamp, and
                        // revision fields are temporal only inside the exact change-cycle
                        // subtrees; functional revisions (notably knowledge_check) stay real.
                        let dynamic = key.ends_with("_ns")
                            || (change_context
                                && (key == "id"
                                    || matches!(
                                        key.as_str(),
                                        "timestamp"
                                            | "started_at"
                                            | "finished_at"
                                            | "receiptId"
                                            | "changeSetId"
                                            | "previousWorkspaceRevision"
                                            | "previousRevision"
                                            | "resultRevision"
                                            | "workspaceRevision"
                                            | "receipt_path"
                                    )));
                        (
                            key.clone(),
                            if dynamic {
                                Value::String("<temporal-sentinel>".into())
                            } else {
                                visit(value, child_change_context)
                            },
                        )
                    })
                    .collect(),
            ),
            _ => value.clone(),
        }
    }

    visit(value, false)
}

#[test]
fn informe_completo_y_variantes_equivalentes_en_smoke() {
    let root = TempDir::new().expect("temp root");
    write_fixture(root.path());
    let report = json_stdout(
        bench()
            .args([
                "--smoke",
                "--seed",
                "33",
                "--root",
                root.path().to_str().unwrap(),
            ])
            .output()
            .expect("run smoke"),
        "BDD-1 smoke",
    );
    let rows = measurement_matrix(&report);
    let counts: BTreeSet<_> = rows
        .iter()
        .map(|(_, row)| required(row, "document_count", "BDD-1").as_u64().unwrap())
        .collect();
    assert_eq!(
        counts.len(),
        1,
        "BDD-1: document_count debe ser igual entre variantes"
    );
    let change = object(&report, "change_cycle", "BDD-1 ciclo de cambio");
    assert_eq!(
        required(&Value::Object(change.clone()), "variant", "BDD-1"),
        "app/disk"
    );
    metric(
        required(&Value::Object(change.clone()), "metric", "BDD-1"),
        "BDD-1 change_cycle",
    );
}

#[test]
fn formato_de_informe_estable_conserva_claves_temporales() {
    let root = TempDir::new().expect("temp root");
    write_fixture(root.path());
    let args = [
        "--smoke",
        "--seed",
        "33",
        "--root",
        root.path().to_str().unwrap(),
    ];
    let reports: Vec<_> = (1..=3)
        .map(|index| {
            json_stdout(
                bench().args(args).output().expect("run smoke estable"),
                &format!("formato corrida {index}"),
            )
        })
        .collect();
    let first = &reports[0];
    let normalized_first = normalize_temporal_values(first);
    let first_commit = required(first, "commit", "estabilidad commit").clone();
    let first_machine = required(first, "machine", "estabilidad machine").clone();
    let first_workspace_revision = object(first, "app_results", "estabilidad app_results")
        .get("knowledge_check")
        .unwrap_or_else(|| panic!("estabilidad: falta app_results.knowledge_check"))
        .get("workspaceRevision")
        .cloned()
        .unwrap_or_else(|| panic!("estabilidad knowledge_check: falta workspaceRevision"));
    assert_eq!(
        normalized_first["commit"], first_commit,
        "commit es provenance estable, no tiempo"
    );
    assert_eq!(
        normalized_first["machine"], first_machine,
        "machine es provenance estable, no tiempo"
    );
    assert_eq!(
        normalized_first["app_results"]["knowledge_check"]["workspaceRevision"],
        first_workspace_revision,
        "knowledge_check.workspaceRevision es funcional y no debe normalizarse"
    );
    for (index, report) in reports.iter().enumerate().skip(1) {
        assert_eq!(
            key_set(first),
            key_set(report),
            "la estructura de informe debe ser estable en corrida {index}"
        );
        assert_eq!(
            normalized_first,
            normalize_temporal_values(report),
            "misma semilla debe conservar valores funcionales tras normalizar solo dinámica de corrida {index}"
        );
    }
    for report in &reports {
        for row in array(report, "measurements", "estabilidad") {
            for tool in TOOLS {
                let metric = row_tool(row, tool, "estabilidad").as_object().unwrap();
                assert!(
                    metric.contains_key("payload_bytes") && metric.contains_key("sample_count"),
                    "la estabilidad no puede eliminar la clave temporal/métrica"
                );
            }
        }
    }
}

fn key_set(value: &Value) -> BTreeSet<String> {
    match value {
        Value::Array(values) => values.iter().flat_map(key_set).collect(),
        Value::Object(values) => values
            .iter()
            .flat_map(|(key, value)| std::iter::once(key.clone()).chain(key_set(value)))
            .collect(),
        _ => BTreeSet::new(),
    }
}

#[test]
fn las_siete_lecturas_conservan_resultado_y_error_de_app() {
    let root = TempDir::new().expect("temp root");
    write_fixture(root.path());
    let report = json_stdout(
        bench()
            .args([
                "--smoke",
                "--seed",
                "33",
                "--root",
                root.path().to_str().unwrap(),
            ])
            .output()
            .expect("run smoke"),
        "BDD-A1",
    );
    let app = object(&report, "app_results", "BDD-A1 App frente a seam interno");
    let seam = object(&report, "seam_results", "BDD-A1 App frente a seam interno");
    for tool in TOOLS {
        let app_result = app
            .get(tool)
            .unwrap_or_else(|| panic!("BDD-A1: falta {tool}"));
        let seam_result = seam
            .get(tool)
            .unwrap_or_else(|| panic!("BDD-A1: falta {tool}"));
        assert_eq!(
            app_result, seam_result,
            "BDD-A1 Result JSON completo divergente para {tool}"
        );
    }
    let negatives = object(&report, "negative_results", "negativos H04");
    let names: Vec<_> = negatives.keys().collect();
    assert!(
        names.len() >= 2,
        "negativos H04: se requieren al menos dos tools distintas"
    );
    for (tool, pair) in negatives {
        let pair = pair
            .as_object()
            .unwrap_or_else(|| panic!("negativo {tool}: pair debe ser objeto"));
        let app = pair
            .get("app")
            .unwrap_or_else(|| panic!("negativo {tool}: falta app"));
        let seam = pair
            .get("seam")
            .unwrap_or_else(|| panic!("negativo {tool}: falta seam"));
        assert_error(app, &format!("negativo {tool}/app"));
        assert_error(seam, &format!("negativo {tool}/seam"));
        assert_eq!(
            app, seam,
            "negativo {tool}: Result JSON completo debe ser idéntico"
        );
    }
}

#[test]
fn equivalencia_exacta_por_tool_en_tres_variantes() {
    let root = TempDir::new().expect("temp root");
    write_fixture(root.path());
    let report = json_stdout(
        bench()
            .args([
                "--smoke",
                "--seed",
                "33",
                "--root",
                root.path().to_str().unwrap(),
            ])
            .output()
            .expect("run smoke"),
        "BDD-A2",
    );
    let rows = measurement_matrix(&report);
    let baseline = &rows[0].1;
    for tool in TOOLS {
        let expected = result_for(baseline, tool, "BDD-A2");
        for (variant, row) in &rows[1..] {
            assert_eq!(
                expected,
                result_for(row, tool, "BDD-A2"),
                "BDD-A2 Result JSON completo divergente en {tool} entre disco y {variant}"
            );
        }
    }
}

#[test]
fn corpus_control_ejercita_las_siete_lecturas() {
    let root = TempDir::new().expect("temp root");
    write_fixture(root.path());
    let report = json_stdout(
        bench()
            .args([
                "--smoke",
                "--seed",
                "33",
                "--root",
                root.path().to_str().unwrap(),
            ])
            .output()
            .expect("run smoke"),
        "BDD-A3 corpus",
    );
    let rows = measurement_matrix(&report);
    assert!(
        rows.iter()
            .any(|(_, row)| required(row, "document_count", "BDD-A3").as_u64().unwrap() >= 3),
        "BDD-A3 corpus no vacío: requiere >=3 documentos"
    );
    let row = &rows[0].1;
    let status = result_for(row, "workspace_status", "BDD-A3");
    let counts = object(status, "counts", "BDD-A3 status");
    assert!(counts.get("documents").and_then(Value::as_u64).unwrap_or(0) > 0);
    assert!(counts.get("links").and_then(Value::as_u64).unwrap_or(0) > 0);
    let search = result_for(row, "knowledge_search", "BDD-A3");
    assert!(
        contains_string(
            required(search, "results", "BDD-A3 search"),
            "marker-search-h04"
        ),
        "BDD-A3: search debe contener su marcador"
    );
    let get = result_for(row, "knowledge_get", "BDD-A3");
    assert_eq!(
        required(get, "path", "BDD-A3 get").as_str(),
        Some("child.md")
    );
    assert!(contains_string(
        required(get, "body", "BDD-A3 get"),
        "marker-get-h04"
    ));
    let metadata = result_for(row, "metadata_inspect", "BDD-A3");
    assert!(
        contains_string(metadata, "tags") && contains_string(metadata, "control"),
        "BDD-A3: metadata debe inspeccionar el campo esperado"
    );
    let graph = result_for(row, "graph_query", "BDD-A3");
    assert!(!array(graph, "nodes", "BDD-A3 graph").is_empty());
    assert!(!array(graph, "edges", "BDD-A3 graph").is_empty());
    let impact = result_for(row, "impact_analyze", "BDD-A3");
    let summary = object(impact, "summary", "BDD-A3 impact");
    assert!(
        summary
            .get("directlyAffected")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
    );
    assert!(
        summary
            .get("transitivelyAffected")
            .and_then(Value::as_u64)
            .unwrap_or(0)
            > 0
    );
    assert!(!array(impact, "affectedDocuments", "BDD-A3 impact").is_empty());
    let check = result_for(row, "knowledge_check", "BDD-A3");
    let diagnostics = array(check, "diagnostics", "BDD-A3 check");
    assert!(!diagnostics.is_empty());
    for diagnostic in diagnostics {
        let code = required(diagnostic, "code", "BDD-A3 diagnostic")
            .as_str()
            .unwrap_or("");
        let message = required(diagnostic, "msg", "BDD-A3 diagnostic")
            .as_str()
            .unwrap_or("");
        assert!(
            !code.is_empty() && !message.is_empty(),
            "BDD-A3 diagnóstico code/message no vacío"
        );
    }
}

#[test]
fn cada_muestra_respeta_la_adquisicion_de_su_variante() {
    let root = TempDir::new().expect("temp root");
    write_fixture(root.path());
    let mut child = bench()
        .args(["--probe-acquisition-root", root.path().to_str().unwrap()])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn acquisition probe");
    let stdout = child.stdout.take().expect("probe stdout");
    let mut lines = BufReader::new(stdout).lines();
    let ready = match lines.next() {
        Some(line) => line.expect("BDD-A4: READY UTF-8"),
        None => {
            drop(child.stdin.take());
            let mut stderr = Vec::new();
            child
                .stderr
                .take()
                .expect("BDD-A4 stderr")
                .read_to_end(&mut stderr)
                .expect("BDD-A4 stderr read");
            let status = child.wait().expect("BDD-A4 child wait before READY");
            let stderr = String::from_utf8_lossy(&stderr);
            panic!("BDD-A4: falta READY; el proceso terminó con status={status}; stderr={stderr}");
        }
    };
    let ready: Value = serde_json::from_str(&ready).expect("BDD-A4: READY JSON");
    assert_eq!(required(&ready, "event", "BDD-A4 READY"), "READY");
    fs::write(
        root.path().join("second.md"),
        "---\ntags: [second]\n---\nsecond acquisition\n",
    )
    .expect("BDD-A4 mutation");
    writeln!(child.stdin.as_mut().expect("BDD-A4 stdin"), "continue").expect("BDD-A4 continue");
    drop(child.stdin.take());
    let final_stdout: String = lines
        .map(|line| line.expect("BDD-A4 output UTF-8"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut stderr = Vec::new();
    child
        .stderr
        .take()
        .expect("BDD-A4 stderr")
        .read_to_end(&mut stderr)
        .expect("BDD-A4 stderr read");
    let status = child.wait().expect("BDD-A4 child wait");
    let stderr = String::from_utf8_lossy(&stderr);
    assert!(
        status.success(),
        "BDD-A4: el probe debe terminar con exit 0; status={status}; stderr={stderr}; stdout={final_stdout}"
    );
    let report: Value = serde_json::from_str(final_stdout.trim()).unwrap_or_else(|error| {
        panic!(
            "BDD-A4: salida final debe ser JSON; error={error}; status={status}; stderr={stderr}; stdout={final_stdout}"
        )
    });
    let before = object(&report, "before", "BDD-A4 before");
    let after = object(&report, "after", "BDD-A4 after");
    let before_counts: BTreeMap<_, _> = VARIANTS
        .iter()
        .map(|variant| {
            (
                *variant,
                before
                    .get(*variant)
                    .and_then(Value::as_object)
                    .and_then(|row| row.get("document_count"))
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| {
                        panic!(
                            "BDD-A4 before: document_count ausente/no entero para {variant}; before={before:?}"
                        )
                    }),
            )
        })
        .collect();
    let after_counts: BTreeMap<_, _> = VARIANTS
        .iter()
        .map(|variant| {
            (
                *variant,
                after
                    .get(*variant)
                    .and_then(Value::as_object)
                    .and_then(|row| row.get("document_count"))
                    .and_then(Value::as_u64)
                    .unwrap_or_else(|| {
                        panic!(
                            "BDD-A4 after: document_count ausente/no entero para {variant}; after={after:?}"
                        )
                    }),
            )
        })
        .collect();
    let distinct_before: BTreeSet<_> = before_counts.values().copied().collect();
    assert_eq!(
        distinct_before.len(),
        1,
        "BDD-A4: las tres variantes deben partir de la misma adquisición; before={before_counts:?}; after={after_counts:?}; status={status}; stderr={stderr}"
    );
    for variant in VARIANTS {
        let before_count = before_counts[variant];
        let after_count = after_counts[variant];
        match variant {
            "disk-reparseo" | "sqlite-raw" => assert_eq!(
                after_count,
                before_count + 1,
                "BDD-A4 {variant} debe reflejar exactamente el documento añadido en la segunda adquisición; before={before_counts:?}; after={after_counts:?}"
            ),
            "ram-memoizado" => assert_eq!(
                after_count, before_count,
                "BDD-A4 RAM debe conservar el DocumentSet inicial; before={before_counts:?}; after={after_counts:?}"
            ),
            _ => unreachable!(),
        }
    }
    let rebuild = object(&report, "rebuild", "BDD-A4 rebuild");
    let rebuild_n = rebuild
        .get("sample_count")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let rebuild_p50 = rebuild
        .get("p50_ns")
        .and_then(Value::as_u64)
        .expect("BDD-A4 rebuild p50_ns u64");
    let rebuild_p95 = rebuild
        .get("p95_ns")
        .and_then(Value::as_u64)
        .expect("BDD-A4 rebuild p95_ns u64");
    assert!(rebuild_n > 0);
    assert!(
        rebuild_p95 >= rebuild_p50,
        "BDD-A4 rebuild p95 debe ser >= p50"
    );
    let samples = object(&report, "samples", "BDD-A4 samples");
    for variant in VARIANTS {
        let row = samples
            .get(variant)
            .and_then(Value::as_object)
            .unwrap_or_else(|| panic!("BDD-A4 samples: falta {variant}"));
        assert!(
            !row.contains_key("rebuild"),
            "BDD-A4 rebuild no puede contaminar la muestra {variant}"
        );
    }
}

#[test]
fn ciclo_de_cambio_permanece_en_app_y_escritor_unico() {
    let root = TempDir::new().expect("temp root");
    fs::create_dir_all(root.path()).expect("change root");
    fs::write(root.path().join("control.md"), "# before-state\n").expect("change fixture");
    let report = json_stdout(
        bench()
            .args(["--probe-change-root", root.path().to_str().unwrap()])
            .output()
            .expect("run change probe"),
        "BDD-A5 probe de cambio",
    );
    let source = required(&report, "source", "BDD-A5")
        .as_str()
        .unwrap_or("")
        .to_ascii_lowercase();
    assert_eq!(
        source, "app/disk",
        "BDD-A5: el ciclo debe rotular la fuente App/disco"
    );
    assert!(
        report.get("plan").and_then(Value::as_object).is_some(),
        "BDD-A5: plan real no nulo"
    );
    assert!(
        report.get("apply").and_then(Value::as_object).is_some(),
        "BDD-A5: apply real no nulo"
    );
    let changed_paths = array(&report, "changed_paths", "BDD-A5");
    assert_eq!(changed_paths, &[Value::String("control.md".into())]);
    let markdown = fs::read_to_string(root.path().join("control.md")).expect("BDD-A5 Markdown");
    assert_eq!(
        markdown.matches("after-state").count(),
        1,
        "BDD-A5: el Markdown final debe reflejar exactamente una publicación"
    );
    let receipt_path = required(&report, "receipt_path", "BDD-A5")
        .as_str()
        .expect("receipt_path string");
    let receipt_path = if Path::new(receipt_path).is_absolute() {
        PathBuf::from(receipt_path)
    } else {
        root.path().join(receipt_path)
    };
    assert!(
        receipt_path.is_file(),
        "BDD-A5: receipt_path debe apuntar a un artefacto real"
    );
    let receipt: Value =
        serde_json::from_str(&fs::read_to_string(&receipt_path).expect("receipt read"))
            .expect("BDD-A5 receipt JSON");
    non_empty_real(&receipt, "BDD-A5 receipt");
    assert!(
        contains_string(&receipt, "control.md"),
        "BDD-A5: el recibo real debe identificar el Markdown publicado"
    );
    let receipt_dir = receipt_path.parent().expect("receipt dir");
    let receipts: Vec<_> = fs::read_dir(receipt_dir)
        .expect("BDD-A5 receipts dir")
        .filter_map(Result::ok)
        .filter(|entry| entry.path().extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    assert_eq!(
        receipts.len(),
        1,
        "BDD-A5: debe existir exactamente un recibo JSON real"
    );
    assert!(
        report.get("measurements").is_none(),
        "BDD-A5: el informe de escritura no puede fingir una matriz de variantes"
    );
}
