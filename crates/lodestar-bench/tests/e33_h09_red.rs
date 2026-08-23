//! Fase roja de E33-H09.
//!
//! Esta suite fija la interfaz de la sonda extrema sin materializar 100k documentos durante CI.
//! Las invocaciones positivas usan una escala mínima; el formato se verifica con una fixture
//! pequeña y la corrida Realista/100k se acredita mediante el manifiesto oficial. Mientras falta
//! la implementación, las pruebas deben fallar por ausencia de `--extreme`/su informe, nunca
//! por un corpus vacío o una aserción vacía.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::process::{Command, Output};
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

fn combined(output: &Output) -> String {
    format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn run_extreme(dir: &TempDir, scale: u64, iterations: u64) -> Output {
    let json = dir.path().join("extreme.json");
    let markdown = dir.path().join("extreme.md");
    bench()
        .args([
            "--extreme",
            "--confirm-extreme",
            "--profile",
            "realista",
            "--scale",
            &scale.to_string(),
            "--iterations",
            &iterations.to_string(),
            "--json-output",
            json.to_str().expect("json path"),
            "--markdown-output",
            markdown.to_str().expect("markdown path"),
        ])
        .output()
        .expect("ejecutar sonda extrema")
}

fn fixture_root() -> TempDir {
    let dir = tempfile::tempdir().expect("fixture root");
    fs::write(
        dir.path().join("control.md"),
        "---\nservice: bench\ntags: [h09]\n---\n# Control\nmarker-h09\n",
    )
    .expect("control fixture");
    dir
}

fn json_stdout(output: &Output, context: &str) -> Value {
    assert!(
        output.status.success(),
        "{context}: la sonda debe terminar correctamente: {}",
        combined(output)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let text = text.trim();
    assert!(!text.is_empty(), "{context}: stdout no puede estar vacío");
    serde_json::from_str(text).unwrap_or_else(|error| {
        panic!("{context}: stdout no es JSON válido: {error}; stdout={text}")
    })
}

fn object<'a>(value: &'a Value, key: &str, context: &str) -> &'a serde_json::Map<String, Value> {
    value
        .get(key)
        .and_then(Value::as_object)
        .unwrap_or_else(|| panic!("{context}: falta objeto JSON {key:?}"))
}

fn array<'a>(value: &'a Value, key: &str, context: &str) -> &'a [Value] {
    value
        .get(key)
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_else(|| panic!("{context}: falta array JSON {key:?}"))
}

fn positive_u64(value: &Value, key: &str, context: &str) -> u64 {
    let number = value
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{context}: {key} debe ser entero positivo"));
    assert!(number > 0, "{context}: {key} debe ser > 0");
    number
}

fn metric<'a>(value: &'a Value, context: &str, iterations: u64) -> &'a Value {
    let metric = value
        .as_object()
        .unwrap_or_else(|| panic!("{context}: métrica debe ser objeto"));
    let sample_count = positive_u64(&Value::Object(metric.clone()), "sample_count", context);
    assert_eq!(
        sample_count, iterations,
        "{context}: sample_count debe coincidir con --iterations"
    );
    let samples = metric
        .get("sample_elapsed_ns")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}: falta sample_elapsed_ns"));
    assert_eq!(
        samples.len() as u64,
        iterations,
        "{context}: sample_elapsed_ns debe conservar cada muestra"
    );
    assert!(
        samples
            .iter()
            .all(|sample| sample.as_u64().unwrap_or(0) > 0),
        "{context}: las muestras deben ser tiempos positivos"
    );
    assert!(metric.contains_key("p50_ns"), "{context}: falta p50_ns");
    assert!(metric.contains_key("p95_ns"), "{context}: falta p95_ns");
    assert!(metric.contains_key("result"), "{context}: falta result");
    value
}

fn measurements<'a>(report: &'a Value, context: &str) -> Vec<&'a Value> {
    let variants = array(report, "variants", context);
    let got: BTreeSet<_> = variants.iter().filter_map(Value::as_str).collect();
    assert_eq!(
        got,
        VARIANTS.iter().copied().collect(),
        "{context}: las tres variantes deben estar nombradas"
    );
    let tools = array(report, "tools", context);
    let got_tools: BTreeSet<_> = tools.iter().filter_map(Value::as_str).collect();
    assert_eq!(
        got_tools,
        TOOLS.iter().copied().collect(),
        "{context}: las siete lecturas deben estar nombradas"
    );
    let rows = array(report, "measurements", context);
    assert_eq!(
        rows.len(),
        VARIANTS.len(),
        "{context}: una fila por variante"
    );
    rows.iter().collect()
}

#[test]
fn modo_extremo_exige_parametros_y_no_altera_full_smoke() {
    let root = fixture_root();
    let smoke = bench()
        .args([
            "--smoke",
            "--root",
            root.path().to_str().expect("root path"),
        ])
        .output()
        .expect("ejecutar smoke");
    assert!(
        smoke.status.success(),
        "smoke existente debe seguir verde: {}",
        combined(&smoke)
    );
    let smoke_report: Value = serde_json::from_slice(&smoke.stdout).expect("smoke JSON");
    assert_eq!(smoke_report["schema_version"], "e33-h04-v2");
    assert!(
        smoke_report["measurements"][0]["document_count"]
            .as_u64()
            .unwrap_or(0)
            > 0,
        "smoke existente debe conservar un corpus no vacío"
    );

    let missing = bench()
        .args(["--extreme"])
        .output()
        .expect("validar parámetros extremos");
    assert!(
        !missing.status.success(),
        "sin parámetros debe fallar antes del corpus"
    );
    let text = combined(&missing);
    assert!(
        text.contains("profile")
            || text.contains("perfil")
            || text.contains("scale")
            || text.contains("escala")
            || text.contains("iterations")
            || text.contains("iteraciones"),
        "el error debe nombrar el parámetro ausente, no solo un argumento desconocido: {text}"
    );
}

#[test]
fn scale_acepta_entero_positivo_sin_whitelist() {
    let dir = tempfile::tempdir().expect("tempdir");
    let small = run_extreme(&dir, 3, 1);
    assert!(
        small.status.success(),
        "una escala positiva fuera de las escalas históricas debe aceptarse: {}",
        combined(&small)
    );

    // La validación de 1M no materializa el corpus en CI: iterations=0 obliga a validar escala
    // primero y fallar por iteraciones, nunca por no pertenecer a una whitelist.
    let million = bench()
        .args([
            "--extreme",
            "--profile",
            "realista",
            "--scale",
            "1000000",
            "--iterations",
            "0",
        ])
        .output()
        .expect("validar escala 1M");
    assert!(!million.status.success());
    let million_text = combined(&million);
    assert!(
        million_text.contains("iterations")
            || million_text.contains("iteraciones")
            || million_text.contains("positivo"),
        "1M no debe rechazarse por whitelist: {million_text}"
    );

    for invalid in ["0", "-1", "18446744073709551616"] {
        let output = bench()
            .args([
                "--extreme",
                "--profile",
                "realista",
                "--scale",
                invalid,
                "--iterations",
                "1",
            ])
            .output()
            .expect("validar escala inválida");
        assert!(!output.status.success(), "scale={invalid} debe fallar");
        let text = combined(&output);
        assert!(
            text.contains("scale") || text.contains("escala"),
            "scale={invalid}: el error debe nombrar scale: {text}"
        );
    }
}

#[test]
fn variantes_extremas_conservan_equivalencia_funcional() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = run_extreme(&dir, 3, 1);
    let report = json_stdout(&output, "equivalencia extrema");
    let rows = measurements(&report, "equivalencia extrema");
    let baseline = object(rows[0], "tools", "equivalencia extrema");
    for row in rows {
        let variant = row["variant"].as_str().unwrap_or("<sin-variante>");
        let tools = object(row, "tools", "equivalencia extrema");
        for tool in TOOLS {
            let expected = baseline[tool]
                .get("result")
                .unwrap_or_else(|| panic!("{variant}/{tool}: falta result baseline"));
            let actual = tools[tool]
                .get("result")
                .unwrap_or_else(|| panic!("{variant}/{tool}: falta result"));
            assert_eq!(
                actual, expected,
                "divergencia funcional en variante={variant}, tool={tool}"
            );
        }
    }
}

#[test]
fn extremo_registra_muestras_y_rebuild_separado() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = run_extreme(&dir, 3, 2);
    let report = json_stdout(&output, "muestras y rebuild");
    assert_eq!(report["iterations"], 2);
    for row in measurements(&report, "muestras y rebuild") {
        let variant = row["variant"].as_str().unwrap_or("<sin-variante>");
        let tools = object(row, "tools", "muestras y rebuild");
        for tool in TOOLS {
            metric(&tools[tool], &format!("{variant}/{tool}"), 2);
        }
        metric(
            row.get("cold_open")
                .unwrap_or_else(|| panic!("{variant}: falta cold_open")),
            &format!("{variant}/cold_open"),
            2,
        );
        if variant == "sqlite-raw" {
            let rebuild = row
                .get("rebuild")
                .unwrap_or_else(|| panic!("{variant}: rebuild debe ser independiente"));
            metric(rebuild, "sqlite-raw/rebuild", 1);
            assert!(
                row.get("percentiles_includes_rebuild") == Some(&Value::Bool(false)),
                "rebuild no puede entrar en percentiles SQLite"
            );
        }
    }
}

#[test]
fn extremo_registra_tamanos_y_rss_honesto() {
    let dir = tempfile::tempdir().expect("tempdir");
    let output = run_extreme(&dir, 3, 1);
    let report = json_stdout(&output, "footprint extremo");
    let corpus = object(&report, "corpus", "footprint extremo");
    assert!(positive_u64(&Value::Object(corpus.clone()), "document_count", "corpus") > 0);
    assert!(positive_u64(&Value::Object(corpus.clone()), "bytes", "corpus") > 0);
    let sqlite = object(&report, "sqlite", "footprint extremo");
    assert!(
        sqlite.contains_key("main_bytes"),
        "sqlite: falta main_bytes"
    );
    assert!(
        sqlite.contains_key("auxiliary_bytes"),
        "sqlite: falta auxiliary_bytes"
    );
    assert!(
        sqlite.contains_key("total_bytes"),
        "sqlite: falta total_bytes"
    );
    for row in measurements(&report, "footprint extremo") {
        let variant = row["variant"].as_str().unwrap_or("<sin-variante>");
        let rss = object(row, "rss", &format!("{variant}/rss"));
        let status = rss.get("status").and_then(Value::as_str).unwrap_or("");
        if status == "available" {
            assert!(
                rss.get("absolute_bytes")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    > 0,
                "{variant}/rss: absolute_bytes debe ser > 0"
            );
            assert!(rss.get("method").and_then(Value::as_str).is_some());
            assert!(rss.get("units").and_then(Value::as_str).is_some());
            assert!(rss.get("scope").and_then(Value::as_str).is_some());
        } else {
            assert_eq!(status, "unavailable", "{variant}: RSS debe ser honesto");
            assert!(
                rss.get("reason")
                    .and_then(Value::as_str)
                    .is_some_and(|reason| !reason.is_empty()),
                "{variant}: RSS unavailable exige motivo"
            );
        }
    }
}

#[test]
fn preflight_extremo_falla_sin_parciales_y_1m_exige_confirmacion_si_es_incierto() {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path().join("corpus");
    let output = bench()
        .args([
            "--extreme",
            "--profile",
            "realista",
            "--scale",
            "1000000000",
            "--iterations",
            "1",
            "--root",
            root.to_str().expect("root path"),
        ])
        .output()
        .expect("preflight extremo");
    assert!(
        !output.status.success(),
        "preflight sin recursos verificables debe fallar"
    );
    let text = combined(&output);
    assert!(
        text.contains("dispon")
            || text.contains("required")
            || text.contains("requer")
            || text.contains("confirm"),
        "debe comunicar disponible/requerido o confirmación explícita: {text}"
    );
    assert!(
        !root.exists()
            || fs::read_dir(&root)
                .map(|entries| entries.count())
                .unwrap_or(0)
                == 0,
        "el fallo de preflight no puede dejar corpus parcial"
    );

    let uncertain = bench()
        .args([
            "--extreme",
            "--profile",
            "realista",
            "--scale",
            "1000000",
            "--iterations",
            "1",
        ])
        .output()
        .expect("preflight 1M");
    assert!(!uncertain.status.success());
    let uncertain_text = combined(&uncertain);
    assert!(
        uncertain_text.contains("confirm")
            || uncertain_text.contains("confirmación")
            || uncertain_text.contains("recurso")
            || uncertain_text.contains("dispon"),
        "1M incierto debe exigir confirmación o explicar recursos: {uncertain_text}"
    );
}

#[test]
fn fixture_extrema_es_completa_no_vacia_y_sin_rutas_privadas() {
    let text = include_str!("fixtures/e33_h09_extreme_format.json");
    assert!(
        !text.trim().is_empty(),
        "fixture extrema no puede estar vacía"
    );
    assert!(
        !text.contains("/Users/"),
        "fixture no puede filtrar ruta privada macOS"
    );
    assert!(
        !text.contains("/home/"),
        "fixture no puede filtrar ruta privada Linux"
    );
    let report: Value = serde_json::from_str(text).expect("fixture extrema JSON");
    assert!(
        report["corpus"]["document_count"]
            .as_u64()
            .is_some_and(|count| count > 0),
        "fixture extrema debe conservar un corpus no vacío"
    );
    assert_eq!(
        report["functional_equivalence"], true,
        "fixture extrema debe declarar equivalencia funcional"
    );
    let rows = report["measurements"]
        .as_array()
        .expect("fixture extrema debe declarar measurements");
    assert_eq!(rows.len(), VARIANTS.len(), "fixture exige tres variantes");
    let variants: BTreeSet<_> = rows
        .iter()
        .filter_map(|row| row["variant"].as_str())
        .collect();
    assert_eq!(
        variants,
        VARIANTS.iter().copied().collect(),
        "fixture debe nombrar las tres variantes"
    );
    for row in rows {
        assert!(
            row.get("document_count")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                > 0
        );
        let tools = object(row, "tools", "fixture extrema");
        for tool in TOOLS {
            assert!(
                tools.get(tool).and_then(Value::as_object).is_some(),
                "fixture extrema debe declarar {tool}"
            );
            assert!(
                tools[tool].get("result").is_some(),
                "fixture extrema debe conservar result para {tool}"
            );
        }
        assert!(row.get("rss").and_then(Value::as_object).is_some());
    }
}

#[test]
fn no_regresion_mantiene_h05_10k_ci_y_superficie_publica() {
    let thresholds = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/qa/testbench/umbrales.json"),
    )
    .expect("umbrales H05");
    let thresholds: Value = serde_json::from_str(&thresholds).expect("umbrales JSON");
    assert_eq!(thresholds["scale"], 10000, "H05 solo puede seleccionar 10k");
    assert_eq!(thresholds["variant"], "disk-reparseo");

    let workflow = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows/testbench.yml"),
    )
    .expect("workflow testbench");
    assert!(
        !workflow.contains("--extreme"),
        "CI no debe ejecutar la sonda extrema opt-in"
    );
    let contract =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../contracts/mcp.yml"))
            .expect("contrato MCP");
    assert!(!contract.contains("extreme"), "H09 no cambia contrato MCP");
    let decision = fs::read_to_string(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../decisiones/14-store-sin-consumidor.md"),
    )
    .expect("decisión 14");
    assert!(
        decision.contains("estado: \"abierta\"") || decision.contains("Estado: abierta"),
        "§14 debe continuar abierta"
    );

    // La corrida 100k queda fuera de Git; el manifiesto versionado es la única prueba oficial de
    // existencia y trazabilidad del bruto externo.
    let manifest_path =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/qa/corridas/v0.6.2/manifest.json");
    let manifest_text =
        fs::read_to_string(&manifest_path).expect("manifiesto oficial de la corrida H09");
    assert!(
        !manifest_text.trim().is_empty(),
        "manifiesto oficial no puede estar vacío"
    );
    let manifest: Value = serde_json::from_str(&manifest_text).expect("manifiesto JSON válido");
    let h09 = manifest["results"]
        .as_array()
        .and_then(|results| {
            results
                .iter()
                .find(|result| result["id"] == "e33-h09-rendimiento")
        })
        .expect("manifiesto debe catalogar e33-h09-rendimiento");
    let summary = h09["summary"].as_str().expect("H09 debe enlazar resumen");
    assert!(
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join(summary)
            .is_file(),
        "el resumen oficial H09 debe existir: {summary}"
    );
    let artifact_url = h09["artifact"]["url"]
        .as_str()
        .expect("H09 debe identificar el bruto externo por URL");
    assert!(
        artifact_url.starts_with("https://") && artifact_url.contains("/releases/"),
        "URL H09 debe ser estable y oficial: {artifact_url}"
    );
}
