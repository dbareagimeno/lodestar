//! Fase roja R5 de E33-H09.
//!
//! Los tres seams exigidos son exclusivamente internos al banco y no forman parte de la CLI de
//! producto ni de MCP:
//! - `--internal-test-full-plan` observa el plan oficial sin generar corpus;
//! - `LODESTAR_BENCH_TEST_RSS_SAMPLER` observa las seis fronteras ordenadas mediante
//!   `LODESTAR_BENCH_TEST_RSS_PHASE`;
//! - `LODESTAR_BENCH_TEST_SQLITE_TIMING_TRACE` inyecta duraciones en el worker SQLite real.
//!
//! Ninguna prueba ejecuta full, 100k o 1M.

use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

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

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tiny_root(directory: &Path) -> PathBuf {
    let root = directory.join("tiny-root");
    fs::create_dir(&root).expect("crear fixture pequeña");
    fs::write(
        root.join("control.md"),
        "---\nservice: bench\ntags: [h09-r5]\n---\n# Control\nmarker-r5\n",
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
fn full_oficial_conserva_plan_formato_y_semantica_sin_materializar_corpus() {
    let output = bench()
        .arg("--internal-test-full-plan")
        .output()
        .expect("observar plan full interno");
    let plan = report(&output, "plan full barato");

    assert_eq!(plan["mode"], "full");
    assert_eq!(plan["schema_version"], "e33-h04-v2-full");
    assert_eq!(plan["iterations"], 10);
    assert_eq!(plan["profiles"], json!(["plano", "realista"]));
    assert_eq!(plan["scales"], json!([100, 1000, 10000]));
    assert_eq!(plan["variants"], json!(VARIANTS));
    assert_eq!(plan["tools"], json!(TOOLS));
    assert_eq!(
        plan["output_formats"],
        json!(["stdout-json", "json-file", "markdown-file"]),
        "full conserva las tres salidas canónicas"
    );
    assert_eq!(plan["runtime_profile"], "standard");
    assert_eq!(plan["equivalence"], "exact-normalized-results");
    assert_eq!(plan["cold_open"], "app-open-plus-first-read");
    assert_eq!(plan["sqlite_rebuild"], "separate-from-read-percentiles");
    assert_eq!(plan["wire_calibration"], "full-only");

    let jobs = plan["jobs"]
        .as_array()
        .expect("anti-vacuidad: jobs del full");
    assert_eq!(jobs.len(), 6, "dos perfiles por tres escalas");
    let got: BTreeSet<_> = jobs
        .iter()
        .map(|job| {
            let profile = job["profile"].as_str().expect("profile de job");
            let scale = job["scale"].as_u64().expect("scale de job");
            assert_eq!(job["iterations"], 10, "{profile}/{scale}: iteraciones");
            assert_eq!(job["variants"], json!(VARIANTS));
            assert_eq!(job["tools"], json!(TOOLS));
            (profile, scale)
        })
        .collect();
    let expected: BTreeSet<_> = [
        ("plano", 100),
        ("plano", 1_000),
        ("plano", 10_000),
        ("realista", 100),
        ("realista", 1_000),
        ("realista", 10_000),
    ]
    .into_iter()
    .collect();
    assert_eq!(got, expected, "la matriz oficial no admite jobs extremos");
    assert!(
        jobs.iter().all(|job| job["mode"] == "full"),
        "ningún job full puede convertirse en extreme"
    );
}

#[cfg(unix)]
#[test]
fn rss_baseline_usa_lectura_anterior_al_corpus_y_no_una_constante() {
    use std::os::unix::fs::PermissionsExt;

    let temp = tempfile::tempdir().expect("directorio temporal");
    let root = tiny_root(temp.path());
    let sampler = temp.path().join("rss-sampler.py");
    let log = temp.path().join("rss-sampler.log");
    fs::write(
        &sampler,
        r#"#!/usr/bin/env python3
import os
from pathlib import Path

log = Path(os.environ["LODESTAR_BENCH_TEST_RSS_LOG"])
phase = os.environ.get("LODESTAR_BENCH_TEST_RSS_PHASE", "<missing-phase>")
values = {
    "baseline": 111111,
    "app-open-start": 222222,
    "app-open-end": 333333,
    "load-start": 444444,
    "load-end": 555555,
    "peak": 999999,
}
value = values.get(phase)
with log.open("a") as stream:
    stream.write(f"{phase}:{value if value is not None else 'unavailable'}\n")
if value is None:
    raise SystemExit("missing or unknown LODESTAR_BENCH_TEST_RSS_PHASE: " + phase)
print(value)
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
        .expect("ejecutar worker con sampler RSS");
    let row = report(&output, "sampler RSS del worker");
    let events = fs::read_to_string(&log).unwrap_or_else(|_| {
        panic!("el worker no invocó LODESTAR_BENCH_TEST_RSS_SAMPLER; faltan las seis fases")
    });
    assert_eq!(
        events.lines().collect::<Vec<_>>(),
        [
            "baseline:111111",
            "app-open-start:222222",
            "app-open-end:333333",
            "load-start:444444",
            "load-end:555555",
            "peak:999999",
        ],
        "baseline debe preceder apertura/carga completas, y el pico debe ser posterior"
    );

    let rss = &row["rss"];
    assert_eq!(rss["status"], "available");
    assert_eq!(rss["baseline_bytes"], 111_111);
    assert_eq!(rss["absolute_bytes"], 999_999);
    assert_eq!(rss["delta_bytes"], 888_888);
    assert_ne!(
        rss["baseline_bytes"], rss["absolute_bytes"],
        "anti-vacuidad: una constante repetida no demuestra baseline pre-corpus"
    );
    assert_eq!(row["variant"], "disk-reparseo");
    assert!(
        row["document_count"].as_u64().unwrap_or(0) > 0,
        "el worker debe haber cargado un corpus no vacío entre las lecturas"
    );
}

#[test]
fn sqlite_rebuild_no_contamina_muestras_con_reloj_determinista() {
    let temp = tempfile::tempdir().expect("directorio temporal");
    let root = tiny_root(temp.path());
    let trace = temp.path().join("sqlite-clock.json");
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
        .output()
        .expect("ejecutar worker SQLite con reloj determinista");
    let row = report(&output, "reloj SQLite del worker");
    assert_eq!(row["variant"], "sqlite-raw");
    assert!(
        row["document_count"].as_u64().unwrap_or(0) > 0,
        "anti-vacuidad: la variante SQLite debe leer un corpus"
    );

    let rebuild = &row["rebuild"];
    assert_eq!(rebuild["sample_count"], 1);
    assert_eq!(metric_samples(rebuild, "rebuild"), [rebuild_ns]);
    assert_eq!(rebuild["p50_ns"], rebuild_ns);
    assert_eq!(rebuild["p95_ns"], rebuild_ns);

    let tools = row["tools"]
        .as_object()
        .expect("anti-vacuidad: siete métricas SQLite");
    assert_eq!(tools.len(), TOOLS.len());
    let mut observed = BTreeSet::new();
    for (index, tool) in TOOLS.iter().enumerate() {
        let expected = [101_u64 + index as u64, 201_u64 + index as u64];
        let metric = &tools[*tool];
        assert_eq!(metric["sample_count"], 2, "{tool}: dos lecturas");
        assert_eq!(metric_samples(metric, tool), expected, "{tool}");
        assert_eq!(metric["p50_ns"], expected[1], "{tool}: p50 derivado");
        assert_eq!(metric["p95_ns"], expected[1], "{tool}: p95 derivado");
        for sample in metric_samples(metric, tool) {
            assert_ne!(
                sample, rebuild_ns,
                "{tool}: rebuild no puede entrar temporalmente en lecturas"
            );
            observed.insert(sample);
        }
    }
    assert_eq!(observed.len(), TOOLS.len() * 2, "14 muestras distinguibles");
    assert_eq!(metric_samples(&row["cold_open"], "cold-open"), [31, 41]);
}

fn sha256(bytes: &[u8]) -> String {
    let mut child = Command::new("python3")
        .args([
            "-c",
            "import hashlib,sys; print(hashlib.sha256(sys.stdin.buffer.read()).hexdigest())",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("ejecutar hashlib");
    child
        .stdin
        .take()
        .expect("stdin de hashlib")
        .write_all(bytes)
        .expect("alimentar hashlib");
    let output = child.wait_with_output().expect("esperar hashlib");
    assert!(
        output.status.success(),
        "hashlib debe terminar correctamente"
    );
    String::from_utf8(output.stdout)
        .expect("salida hashlib UTF-8")
        .split_whitespace()
        .next()
        .expect("digest de hashlib")
        .to_owned()
}

#[test]
fn correccion_review3_coincide_con_lock_anterior_salvo_delta_de_adenda_autorizado() {
    const PATH: &str = "crates/lodestar-bench/tests/e33_h09_review3_red.rs";
    const OLD_HASH: &str = "f2bb6f704320aaebbb29beedee2d8619feaaf4a14dc331ddb0e1e664f1cc0653";
    const CURRENT_HASH: &str = "1339624b9adfe80bb167d313355cb5b08d093cba0b88d3085c908182c745ad70";
    const OLD_FORMULA: &str = r#"    let required = 3_u64 * 32_768 + 256 * 1024 * 1024;
    let available = available_blocks * 1024;
    assert_eq!(report["preflight"]["status"], "checked");
    assert_eq!(report["preflight"]["required_bytes"], required);
    assert_eq!(report["preflight"]["available_bytes"], available);
    assert!(available >= required);
"#;
    const NEW_FORMULA: &str = r#"    let available = available_blocks * 1024;
    assert_eq!(report["preflight"]["status"], "checked");
    let required = report["preflight"]["required_bytes"]
        .as_u64()
        .expect("required_bytes entero");
    assert!(required > 0, "required_bytes positivo");
    assert_eq!(report["preflight"]["available_bytes"], available);
    assert!(available >= required);
"#;
    const OLD_PATH: &str = r#"fn artifact_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../docs/qa/e33-h09-realista-100k-2026-08-23.json")
}"#;
    const NEW_PATH: &str = r#"fn artifact_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/e33_h09_extreme_format.json")
}"#;

    let root = workspace_root();
    let source_path = root.join(PATH);
    let source = fs::read_to_string(&source_path)
        .expect("leer test review3 actual")
        .replace("\r\n", "\n");

    assert_ne!(
        OLD_HASH, CURRENT_HASH,
        "debe existir una corrección ratificada"
    );
    assert_eq!(
        source.matches(NEW_FORMULA).count(),
        1,
        "delta de fórmula nuevo exacto una vez"
    );
    assert_eq!(
        source.matches(OLD_FORMULA).count(),
        0,
        "la fórmula fija retirada no puede seguir activa"
    );
    assert_eq!(
        source.matches(NEW_PATH).count(),
        1,
        "delta de ruta nuevo exacto una vez"
    );
    assert_eq!(
        source.matches(OLD_PATH).count(),
        0,
        "la ruta del bruto completo retirada no puede seguir activa"
    );
    assert_eq!(
        sha256(source.as_bytes()),
        CURRENT_HASH,
        "el fichero actual debe coincidir con su lock vigente"
    );

    let reconstructed = source
        .replacen(NEW_FORMULA, OLD_FORMULA, 1)
        .replacen(NEW_PATH, OLD_PATH, 1);
    assert_eq!(
        sha256(reconstructed.as_bytes()),
        OLD_HASH,
        "al revertir ambos deltas autorizados debe reconstruirse exactamente el lock anterior; cualquier otro cambio queda prohibido"
    );
}
