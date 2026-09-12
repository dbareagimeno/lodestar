//! Fase roja de la tercera revisión E33-H09.
//!
//! Las pruebas usan escala 3 para los caminos ejecutables y leen el artefacto 100k ya versionado.
//! No se materializan 100k/1M en CI.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn combined(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn json_report(output: &Output, context: &str) -> Value {
    assert!(
        output.status.success(),
        "{context}: la ejecución debe terminar correctamente: {}",
        combined(output)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    serde_json::from_str(text.trim()).unwrap_or_else(|error| {
        panic!("{context}: stdout no es un informe JSON: {error}; stdout={text}")
    })
}

fn extreme_command() -> Command {
    let mut command = bench();
    command.args([
        "--extreme",
        "--profile",
        "realista",
        "--scale",
        "3",
        "--iterations",
        "1",
    ]);
    command
}

fn run_small(output_dir: &Path) -> (Output, PathBuf, PathBuf) {
    let json = output_dir.join("small.json");
    let markdown = output_dir.join("small.md");
    let mut command = extreme_command();
    command.arg("--confirm-extreme");
    command.args([
        "--json-output",
        json.to_str().expect("json path"),
        "--markdown-output",
        markdown.to_str().expect("markdown path"),
    ]);
    (command.output().expect("ejecutar escala 3"), json, markdown)
}

fn rows<'a>(report: &'a Value, context: &str) -> Vec<&'a Value> {
    let rows = report
        .get("measurements")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}: falta measurements"));
    assert_eq!(rows.len(), 3, "{context}: deben existir tres filas");
    rows.iter().collect()
}

fn assert_metric_percentiles(metric: &Value, expected_samples: usize, context: &str) {
    let mut samples: Vec<u64> = metric["sample_elapsed_ns"]
        .as_array()
        .unwrap_or_else(|| panic!("{context}: falta sample_elapsed_ns"))
        .iter()
        .map(|sample| sample.as_u64().expect("muestra entera"))
        .collect();
    assert_eq!(
        samples.len(),
        expected_samples,
        "{context}: número de muestras"
    );
    samples.sort_unstable();
    let p50 = samples[samples.len() / 2];
    let p95_index = ((samples.len() * 95).saturating_sub(1) / 100).min(samples.len() - 1);
    assert_eq!(
        metric["p50_ns"].as_u64(),
        Some(p50),
        "{context}: p50 derivado"
    );
    assert_eq!(
        metric["p95_ns"].as_u64(),
        Some(samples[p95_index]),
        "{context}: p95 derivado"
    );
}

fn artifact_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/e33_h09_extreme_format.json")
}

#[cfg(unix)]
fn fake_df(temp: &Path, body: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let bin = temp.join("fake-bin");
    fs::create_dir_all(&bin).expect("fake df bin");
    let script = bin.join("df");
    let log = temp.join("df.log");
    fs::write(&script, body).expect("escribir fake df");
    let mut permissions = fs::metadata(&script)
        .expect("metadata fake df")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("hacer ejecutable fake df");
    (bin, log)
}

#[cfg(unix)]
fn path_with_fake_df(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

#[cfg(unix)]
#[test]
fn preflight_df_no_disponible_consulta_falla_y_confirmada_marca_unverified() {
    let temp = tempfile::tempdir().expect("tempdir");
    let script = "#!/bin/sh\necho invoked >> \"$LODESTAR_H09_DF_LOG\"\nexit 1\n";
    let (bin, log) = fake_df(temp.path(), script);
    let root = temp.path().join("unverified-root");
    let path = path_with_fake_df(&bin);

    let mut without_confirmation = extreme_command();
    without_confirmation
        .env("PATH", &path)
        .env("LODESTAR_H09_DF_LOG", &log)
        .arg("--root")
        .arg(&root);
    let output = without_confirmation
        .output()
        .expect("preflight sin confirmación");
    assert!(
        !output.status.success(),
        "df no disponible exige confirmación"
    );
    assert!(combined(&output).contains("confirm"));
    assert!(!root.exists(), "preflight debe fallar antes de crear root");
    assert!(
        !fs::read_to_string(&log)
            .unwrap_or_default()
            .trim()
            .is_empty(),
        "df debe haberse consultado antes del error"
    );

    let confirmed_root = temp.path().join("confirmed-root");
    let json = temp.path().join("confirmed.json");
    let mut confirmed = extreme_command();
    confirmed
        .env("PATH", &path)
        .env("LODESTAR_H09_DF_LOG", &log)
        .args(["--confirm-extreme", "--root"])
        .arg(&confirmed_root)
        .args(["--json-output"])
        .arg(&json);
    let confirmed_output = confirmed.output().expect("preflight confirmado");
    let report = json_report(&confirmed_output, "preflight confirmado");
    assert_eq!(report["preflight"]["status"], "unverified");
    assert_eq!(report["preflight"]["confirmed"], true);
    assert_ne!(report["preflight"]["status"], "space_checked");
    assert!(
        !confirmed_root.exists(),
        "root temporal explícito debe limpiarse"
    );
}

#[cfg(unix)]
#[test]
fn preflight_df_suficiente_marca_checked() {
    let temp = tempfile::tempdir().expect("tempdir");
    let script = "#!/bin/sh\necho invoked >> \"$LODESTAR_H09_DF_LOG\"\necho 'Filesystem 1024-blocks Used Available Capacity Mounted on'\necho '/fake 1 1 1000000000 1% /'\n";
    let (bin, log) = fake_df(temp.path(), script);
    let path = path_with_fake_df(&bin);
    let mut command = extreme_command();
    command
        .env("PATH", &path)
        .env("LODESTAR_H09_DF_LOG", &log)
        .args(["--json-output"])
        .arg(temp.path().join("checked.json"));
    let output = command.output().expect("preflight suficiente");
    let report = json_report(&output, "preflight suficiente");
    assert_eq!(report["preflight"]["status"], "checked");
    assert_eq!(report["preflight"]["confirmed"], false);
    assert!(!fs::read_to_string(log)
        .unwrap_or_default()
        .trim()
        .is_empty());
}

#[test]
fn workers_rss_y_sqlite_exponen_identidad_y_unidades_reconciliables() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (output, _, _) = run_small(temp.path());
    let report = json_report(&output, "workers y RSS");
    let coordinator = report["coordinator_pid"]
        .as_u64()
        .expect("coordinator_pid entero");
    let mut worker_pids = BTreeSet::new();
    let platform = report["platform"]["os"].as_str().unwrap_or("<missing>");
    for row in rows(&report, "workers y RSS") {
        let variant = row["variant"].as_str().unwrap_or("<missing>");
        let worker = row["worker_pid"].as_u64().expect("worker_pid entero");
        assert_ne!(
            worker, coordinator,
            "{variant}: worker separado del coordinador"
        );
        assert!(
            worker_pids.insert(worker),
            "worker_pid debe ser único por variante"
        );
        let rss = row["rss"].as_object().expect("rss objeto");
        if matches!(platform, "macos" | "linux" | "windows") {
            assert_eq!(rss["status"], "available");
            let raw = rss["raw_value"].as_u64().expect("raw_value entero");
            let absolute = rss["absolute_bytes"]
                .as_u64()
                .expect("absolute_bytes entero");
            let units = rss["raw_units"].as_str().expect("raw_units string");
            if matches!(platform, "macos" | "windows") {
                assert_eq!(units, "bytes");
                assert_eq!(absolute, raw);
            } else {
                assert_eq!(units, "KiB");
                assert_eq!(absolute, raw * 1024);
            }
        } else {
            assert_eq!(rss["status"], "unavailable", "{variant}: RSS honesto");
            assert!(rss["reason"]
                .as_str()
                .is_some_and(|reason| !reason.is_empty()));
            for key in [
                "raw_value",
                "absolute_bytes",
                "baseline_bytes",
                "delta_bytes",
            ] {
                assert!(rss.get(key).is_none(), "{variant}: {key} no es una medida");
            }
        }
    }
    assert_eq!(worker_pids.len(), 3);
    assert!(report["sqlite"]["main_bytes"].as_u64().unwrap_or(0) > 0);
}

#[test]
fn siete_tools_conservan_semantica_observable_del_fixture() {
    let temp = tempfile::tempdir().expect("tempdir");
    let (output, _, _) = run_small(temp.path());
    let report = json_report(&output, "semántica tools");
    for row in rows(&report, "semántica tools") {
        let variant = row["variant"].as_str().unwrap_or("<missing>");
        let tools = row["tools"].as_object().expect("tools objeto");
        assert_eq!(tools.len(), 7, "{variant}: siete tools exactas");
        let status = &tools["workspace_status"]["result"];
        assert!(status["counts"]["documents"].as_u64().unwrap_or(0) > 0);
        let search = &tools["knowledge_search"]["result"];
        assert!(serde_json::to_string(search)
            .expect("search serializable")
            .contains("marker-search-h04"));
        let get = &tools["knowledge_get"]["result"];
        assert!(get["body"]
            .as_str()
            .unwrap_or("")
            .contains("marker-get-h04"));
        assert_eq!(get["path"], "child.md");
        let metadata = &tools["metadata_inspect"]["result"];
        assert!(metadata["presentIn"].as_u64().unwrap_or(0) > 0);
        assert!(metadata["values"]
            .as_array()
            .is_some_and(|values| !values.is_empty()));
        let graph = &tools["graph_query"]["result"];
        assert!(graph["edges"]
            .as_array()
            .is_some_and(|edges| !edges.is_empty()));
        assert!(graph["nodes"]
            .as_array()
            .is_some_and(|nodes| !nodes.is_empty()));
        let impact = &tools["impact_analyze"]["result"];
        assert!(impact["affectedDocuments"]
            .as_array()
            .is_some_and(|documents| !documents.is_empty()));
        assert!(impact["summary"].is_object());
        let check = &tools["knowledge_check"]["result"];
        assert!(check["diagnostics"]
            .as_array()
            .is_some_and(|diagnostics| !diagnostics.is_empty()));
        assert!(check["summary"].is_object());
        for tool in TOOLS {
            assert!(
                !tools[tool]["result"].is_null(),
                "{variant}/{tool}: result no nulo"
            );
        }
    }
}

#[test]
fn roots_existentes_outputs_internos_y_fallos_de_escritura_no_dejan_datos() {
    let temp = tempfile::tempdir().expect("tempdir");
    let existing = temp.path().join("existing");
    fs::create_dir_all(&existing).expect("existing root");
    let sentinel = existing.join("sentinel.txt");
    fs::write(&sentinel, "keep").expect("sentinel");
    let mut reject_existing = extreme_command();
    reject_existing.arg("--root").arg(&existing);
    let output = reject_existing.output().expect("root existente");
    assert!(!output.status.success());
    assert_eq!(
        fs::read_to_string(&sentinel).expect("sentinel preserved"),
        "keep"
    );

    let internal = temp.path().join("internal-output-root");
    let internal_json = internal.join("report.json");
    let mut reject_internal_output = extreme_command();
    reject_internal_output
        .arg("--root")
        .arg(&internal)
        .args(["--json-output"])
        .arg(&internal_json);
    let output = reject_internal_output.output().expect("output dentro root");
    assert!(!output.status.success());
    assert!(
        !internal.exists(),
        "output dentro del root se rechaza antes de crear"
    );

    let failed_root = temp.path().join("failed-output-root");
    let bad_output = temp.path().join("output-directory");
    fs::create_dir_all(&bad_output).expect("directorio output inválido");
    let mut reject_write = extreme_command();
    reject_write
        .arg("--root")
        .arg(&failed_root)
        .args(["--json-output"])
        .arg(&bad_output);
    let output = reject_write.output().expect("fallo de escritura");
    assert!(!output.status.success());
    assert!(
        !failed_root.exists(),
        "fallo de escritura debe limpiar root materializado"
    );
}

#[test]
fn artefacto_100k_tiene_provenance_footprint_equivalencia_y_rss_no_vacuos() {
    let json_path = artifact_path();
    let markdown_path = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/qa/e33-h09-realista-100k-2026-08-23.md");
    assert!(json_path.is_file(), "falta fixture H09 pequeña");
    assert!(markdown_path.is_file(), "falta resumen H09 compañero");
    let text = fs::read_to_string(&json_path).expect("leer JSON 100k");
    for forbidden in ["/private/", "/tmp/", "/Users/", "/home/", r"C:\Users\"] {
        assert!(!text.contains(forbidden), "artefacto filtra {forbidden}");
    }
    let report: Value = serde_json::from_str(&text).expect("JSON 100k válido");
    assert!(report["provenance"]["commit"]
        .as_str()
        .is_some_and(|v| !v.is_empty()));
    assert!(report["provenance"]["working_tree_clean"].is_boolean());
    let corpus_count = report["corpus"]["document_count"]
        .as_u64()
        .expect("corpus count");
    assert!(corpus_count > 0 && report["corpus"]["bytes"].as_u64().unwrap_or(0) > 0);
    let sqlite = &report["sqlite"];
    let main = sqlite["main_bytes"].as_u64().expect("sqlite main");
    let wal = sqlite["wal_bytes"].as_u64().expect("sqlite wal");
    let shm = sqlite["shm_bytes"].as_u64().expect("sqlite shm");
    assert!(main > 0);
    assert_eq!(sqlite["auxiliary_bytes"].as_u64(), Some(wal + shm));
    assert_eq!(sqlite["total_bytes"].as_u64(), Some(main + wal + shm));
    assert_eq!(report["functional_equivalence"], true);
    let rows = rows(&report, "artefacto 100k");
    let variants: BTreeSet<_> = rows
        .iter()
        .map(|row| row["variant"].as_str().unwrap())
        .collect();
    assert_eq!(variants, VARIANTS.iter().copied().collect());
    let baseline = rows[0]["tools"].as_object().expect("baseline tools");
    for row in rows {
        assert_eq!(row["document_count"].as_u64(), Some(corpus_count));
        let rss = row["rss"].as_object().expect("rss");
        for key in ["status", "method", "units", "platform", "scope"] {
            assert!(
                rss[key].as_str().is_some_and(|v| !v.is_empty()),
                "rss {key}"
            );
        }
        assert!(rss["absolute_bytes"].as_u64().unwrap_or(0) > 0);
        assert!(rss["worker_isolated"].as_bool().unwrap_or(false));
        let tools = row["tools"].as_object().expect("tools");
        assert_eq!(tools.len(), 7);
        for tool in TOOLS {
            assert!(!tools[tool]["result"].is_null());
            assert_eq!(tools[tool]["result"], baseline[tool]["result"]);
        }
    }
}

#[test]
fn cold_open_y_rebuild_reportan_percentiles_derivados() {
    let temp = tempfile::tempdir().expect("tempdir");
    let json = temp.path().join("iterations.json");
    let mut command = bench();
    command.args([
        "--extreme",
        "--profile",
        "realista",
        "--scale",
        "3",
        "--iterations",
        "3",
        "--json-output",
        json.to_str().expect("json path"),
    ]);
    let output = command.output().expect("iteraciones 3");
    let report = json_report(&output, "cold/rebuild");
    for row in rows(&report, "cold/rebuild") {
        let variant = row["variant"].as_str().unwrap_or("<missing>");
        assert_metric_percentiles(&row["cold_open"], 3, &format!("{variant}/cold_open"));
        if variant == "sqlite-raw" {
            assert_metric_percentiles(&row["rebuild"], 1, "sqlite-raw/rebuild");
            assert_eq!(row["percentiles_includes_rebuild"], false);
        }
    }
}
