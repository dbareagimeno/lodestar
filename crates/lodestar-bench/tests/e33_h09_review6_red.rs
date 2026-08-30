//! Fase roja R6 de E33-H09: acoplamiento del plan full y orden observable de las mediciones.
//!
//! Los seams exigidos son internos al banco. Ninguna prueba ejecuta full, 100k ni 1M:
//! - `--internal-test-full-plan` debe respetar `LODESTAR_BENCH_TEST_ITERATIONS` y exponer
//!   `full_execution_config`, la configuración compartible con el full real;
//! - el sampler recibe `LODESTAR_BENCH_TEST_RSS_PHASE` en cada frontera RSS;
//! - `LODESTAR_BENCH_TEST_SQLITE_TIMING_LOG` registra fases y consumo de la traza R5.

use serde_json::{json, Value};
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

fn report(output: &Output, context: &str) -> Value {
    assert!(
        output.status.success(),
        "{context}: falta el seam interno esperado: {}",
        combined(output)
    );
    serde_json::from_slice(&output.stdout)
        .unwrap_or_else(|error| panic!("{context}: stdout no es JSON: {error}"))
}

fn full_plan(iterations: &str) -> Value {
    let output = bench()
        .arg("--internal-test-full-plan")
        .env("LODESTAR_BENCH_TEST_ITERATIONS", iterations)
        .output()
        .expect("observar plan full interno");
    report(&output, "plan full barato")
}

fn tiny_root(directory: &Path) -> PathBuf {
    let root = directory.join("tiny-root");
    fs::create_dir_all(root.join(".lodestar")).expect("crear fixture pequeña y plano de control");
    fs::write(
        root.join("control.md"),
        "---\nservice: bench\ntags: [h09-r6]\n---\n# Control\nmarker-r6\n",
    )
    .expect("escribir Markdown de control");
    root
}

fn worker(root: &Path, variant: &str, iterations: &str) -> Command {
    let mut command = bench();
    command.args([
        "--extreme-worker",
        "--profile",
        "realista",
        "--scale",
        "1",
        "--iterations",
        iterations,
        "--root",
        root.to_str().expect("root UTF-8"),
        "--worker-variant",
        variant,
    ]);
    command
}

fn metric_samples(metric: &Value, context: &str) -> Vec<u64> {
    metric["sample_elapsed_ns"]
        .as_array()
        .unwrap_or_else(|| panic!("{context}: falta sample_elapsed_ns"))
        .iter()
        .map(|sample| {
            sample
                .as_u64()
                .unwrap_or_else(|| panic!("{context}: muestra no entera"))
        })
        .collect()
}

#[test]
fn full_plan_con_iteraciones_tres_deriva_jobs_y_config_compartida() {
    let plan = full_plan("3");
    assert_eq!(plan["mode"], "full");
    assert_eq!(
        plan["iterations"], 3,
        "el observador del plan debe usar las mismas iteraciones que el full real"
    );

    let jobs = plan["jobs"]
        .as_array()
        .expect("anti-vacuidad: seis jobs del full");
    assert_eq!(jobs.len(), 6, "dos perfiles por tres escalas");
    let got: BTreeSet<_> = jobs
        .iter()
        .map(|job| {
            assert_eq!(
                job["iterations"], 3,
                "cada job debe derivar sus iteraciones de la configuración full"
            );
            assert_eq!(job["variants"], json!(VARIANTS));
            assert_eq!(job["tools"], json!(TOOLS));
            (
                job["profile"].as_str().expect("profile de job"),
                job["scale"].as_u64().expect("scale de job"),
            )
        })
        .collect();
    assert_eq!(
        got,
        [
            ("plano", 100),
            ("plano", 1_000),
            ("plano", 10_000),
            ("realista", 100),
            ("realista", 1_000),
            ("realista", 10_000),
        ]
        .into_iter()
        .collect(),
        "el seam barato no puede inventar otra matriz"
    );

    let config = plan["full_execution_config"].as_object().expect(
        "falta full_execution_config: arquitectura necesita una config/fingerprint compartible con el full real",
    );
    assert_eq!(config["schema_version"], plan["schema_version"]);
    assert_eq!(config["runtime_profile"], plan["runtime_profile"]);
    assert_eq!(config["wire_calibration"], plan["wire_calibration"]);
    assert_eq!(config["output_formats"], plan["output_formats"]);
    assert_eq!(config["iterations"], 3);
    let fingerprint = config["fingerprint"]
        .as_str()
        .expect("fingerprint estable de la configuración full");
    assert!(
        fingerprint.len() >= 16 && fingerprint.bytes().all(|byte| byte.is_ascii_hexdigit()),
        "anti-vacuidad: fingerprint hexadecimal no trivial: {fingerprint:?}"
    );

    let other = full_plan("4");
    assert_eq!(other["iterations"], 4);
    assert!(other["jobs"]
        .as_array()
        .expect("jobs con segunda configuración")
        .iter()
        .all(|job| job["iterations"] == 4));
    assert_ne!(
        other["full_execution_config"]["fingerprint"], config["fingerprint"],
        "el fingerprint debe identificar la configuración efectiva, no ser una constante"
    );
}

#[cfg(unix)]
#[test]
fn rss_sampler_observa_baseline_app_open_load_y_peak_en_orden() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("directorio temporal");
    let root = tiny_root(temp.path());
    let sampler = temp.path().join("rss-sampler.py");
    let log = root.join(".lodestar/rss-events.log");
    fs::write(
        &sampler,
        r#"#!/usr/bin/env python3
import os
from pathlib import Path

phase = os.environ.get("LODESTAR_BENCH_TEST_RSS_PHASE", "<missing-phase>")
log = Path(os.environ["LODESTAR_BENCH_TEST_RSS_LOG"])
with log.open("a") as stream:
    stream.write(phase + "\n")
values = {
    "baseline": 111111,
    "app-open-start": 222222,
    "app-open-end": 333333,
    "load-start": 444444,
    "load-end": 555555,
    "peak": 999999,
}
if phase not in values:
    raise SystemExit("missing or unknown LODESTAR_BENCH_TEST_RSS_PHASE: " + phase)
print(values[phase])
"#,
    )
    .expect("escribir sampler RSS determinista");
    let mut permissions = fs::metadata(&sampler)
        .expect("metadata sampler")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&sampler, permissions).expect("hacer sampler ejecutable");

    let output = worker(&root, "disk-reparseo", "1")
        .env("LODESTAR_BENCH_TEST_RSS_SAMPLER", &sampler)
        .env("LODESTAR_BENCH_TEST_RSS_LOG", &log)
        .output()
        .expect("ejecutar worker con sampler RSS por fases");
    let row = report(&output, "sampler RSS por fases");
    let events = fs::read_to_string(&log)
        .unwrap_or_else(|_| panic!("el worker no publicó eventos RSS deterministas"));
    assert_eq!(
        events.lines().collect::<Vec<_>>(),
        [
            "baseline",
            "app-open-start",
            "app-open-end",
            "load-start",
            "load-end",
            "peak",
        ],
        "baseline debe preceder apertura/carga completas, y el pico debe ser posterior"
    );

    let rss = &row["rss"];
    assert_eq!(rss["status"], "available");
    assert_eq!(rss["baseline_bytes"], 111_111);
    assert_eq!(rss["absolute_bytes"], 999_999);
    assert_eq!(rss["delta_bytes"], 888_888);
    assert!(
        row["document_count"].as_u64().unwrap_or(0) > 0,
        "anti-vacuidad: la carga observada debe producir un corpus no vacío"
    );
}

#[cfg(unix)]
fn fake_insufficient_df(temp: &Path) -> (PathBuf, PathBuf, String) {
    use std::os::unix::fs::PermissionsExt;

    let bin = temp.join("fake-bin");
    fs::create_dir_all(&bin).expect("crear bin de df falso");
    let script = bin.join("df");
    let log = temp.join("df.log");
    fs::write(
        &script,
        "#!/bin/sh\necho invoked >> \"$LODESTAR_H09_DF_LOG\"\necho 'Filesystem 1024-blocks Used Available Capacity Mounted on'\necho '/fake 1 0 1 0% /'\n",
    )
    .expect("escribir df insuficiente");
    let mut permissions = fs::metadata(&script).expect("metadata de df").permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("hacer df ejecutable");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    (bin, log, path)
}

#[cfg(unix)]
fn parse_required(error: &str) -> u64 {
    let suffix = error
        .split("requerido=")
        .nth(1)
        .unwrap_or_else(|| panic!("el error no comunica requerido=: {error}"));
    let digits = suffix
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>();
    assert!(!digits.is_empty(), "required no es parseable: {error}");
    digits
        .parse()
        .unwrap_or_else(|parse_error| panic!("required fuera de u64: {parse_error}: {error}"))
}

#[cfg(unix)]
#[test]
fn preflight_requerido_es_trazable_y_crece_con_la_escala() {
    let temp = tempfile::tempdir().expect("directorio temporal");
    let (_bin, log, path) = fake_insufficient_df(temp.path());
    let mut observations = Vec::new();
    for scale in ["3", "19"] {
        let root = temp.path().join(format!("scale-{scale}"));
        let output = bench()
            .env("PATH", &path)
            .env("LODESTAR_H09_DF_LOG", &log)
            .args([
                "--extreme",
                "--profile",
                "realista",
                "--scale",
                scale,
                "--iterations",
                "1",
                "--root",
                root.to_str().expect("root UTF-8"),
            ])
            .output()
            .expect("ejecutar preflight insuficiente");
        let text = combined(&output);
        assert!(
            !output.status.success(),
            "df insuficiente debe detener {scale}"
        );
        assert!(
            text.contains("preflight extremo: espacio insuficiente")
                && text.contains(&format!("scale={scale}")),
            "la estimación debe ser trazable hasta su escala: {text}"
        );
        assert!(!root.exists(), "{scale}: no puede quedar corpus parcial");
        observations.push((parse_required(&text), text));
    }

    assert!(
        observations[0].0 > 0,
        "anti-vacuidad: estimación positiva para escala pequeña"
    );
    assert!(
        observations[1].0 > observations[0].0,
        "el requerido debe crecer estrictamente con la escala: {:?}",
        observations
            .iter()
            .map(|(required, _)| required)
            .collect::<Vec<_>>()
    );
    for (_, text) in &observations {
        assert!(
            ["estimador=", "estimator=", "estimate_method="]
                .iter()
                .any(|marker| text.contains(marker)),
            "el error debe identificar el estimador usado, no sólo imprimir un número: {text}"
        );
    }
    assert_eq!(
        fs::read_to_string(log).expect("log de df").lines().count(),
        2,
        "cada escala positiva debe alcanzar exactamente un preflight real"
    );
}

#[test]
fn sqlite_rebuild_termina_antes_de_timers_y_consumos_deterministas() {
    let temp = tempfile::tempdir().expect("directorio temporal");
    let root = tiny_root(temp.path());
    let control = root.join(".lodestar");
    let trace = control.join("sqlite-clock.json");
    let phase_log = control.join("sqlite-phases.log");
    let rebuild_ns = 9_000_000_001_u64;
    let mut tool_elapsed = serde_json::Map::new();
    for (index, tool) in TOOLS.iter().enumerate() {
        tool_elapsed.insert(
            (*tool).to_owned(),
            json!([101_u64 + index as u64, 201_u64 + index as u64]),
        );
    }
    fs::write(
        &trace,
        serde_json::to_vec(&json!({
            "rebuild_elapsed_ns": [rebuild_ns],
            "tool_elapsed_ns": tool_elapsed,
            "cold_open_elapsed_ns": [31, 41]
        }))
        .expect("serializar traza"),
    )
    .expect("escribir traza de reloj");

    let output = worker(&root, "sqlite-raw", "2")
        .env("LODESTAR_BENCH_TEST_SQLITE_TIMING_TRACE", &trace)
        .env("LODESTAR_BENCH_TEST_SQLITE_TIMING_LOG", &phase_log)
        .output()
        .expect("ejecutar worker SQLite con log de fases");
    let row = report(&output, "log de fases SQLite");
    assert_eq!(metric_samples(&row["rebuild"], "rebuild"), [rebuild_ns]);
    for (index, tool) in TOOLS.iter().enumerate() {
        assert_eq!(
            metric_samples(&row["tools"][*tool], tool),
            [101_u64 + index as u64, 201_u64 + index as u64],
            "{tool}: conserva las duraciones distinguibles de R5"
        );
    }
    assert_eq!(metric_samples(&row["cold_open"], "cold-open"), [31, 41]);

    let mut expected = vec![
        "phase:rebuild:start".to_owned(),
        "phase:rebuild:end".to_owned(),
        format!("consume:rebuild:{rebuild_ns}"),
    ];
    for iteration in 1..=2 {
        for (index, tool) in TOOLS.iter().enumerate() {
            let elapsed = if iteration == 1 {
                101_u64 + index as u64
            } else {
                201_u64 + index as u64
            };
            expected.push(format!("phase:tool:{tool}:{iteration}:timer-start"));
            expected.push(format!("phase:tool:{tool}:{iteration}:timer-end"));
            expected.push(format!("consume:tool:{tool}:{iteration}:{elapsed}"));
        }
    }
    for (iteration, elapsed) in [(1, 31_u64), (2, 41_u64)] {
        expected.push(format!("phase:cold-open:{iteration}:timer-start"));
        expected.push(format!("phase:cold-open:{iteration}:timer-end"));
        expected.push(format!("consume:cold-open:{iteration}:{elapsed}"));
    }
    let actual = fs::read_to_string(&phase_log).unwrap_or_else(|_| {
        panic!(
            "falta LODESTAR_BENCH_TEST_SQLITE_TIMING_LOG: no puede probarse el orden rebuild -> timers"
        )
    });
    assert_eq!(
        actual.lines().collect::<Vec<_>>(),
        expected.iter().map(String::as_str).collect::<Vec<_>>(),
        "rebuild debe terminar y consumir su duración antes de iniciar cualquier timer de tool/cold-open"
    );
    assert_eq!(
        expected.len(),
        51,
        "anti-vacuidad: rebuild + 14 tools + 2 cold-open, cada fase con tres eventos"
    );
}
