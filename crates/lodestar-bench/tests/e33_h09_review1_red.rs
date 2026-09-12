//! Fase roja de reparación de la revisión E33-H09.
//!
//! Fija dos detalles del contrato de la sonda extrema que no deben quedar implícitos: el
//! desglose físico de SQLite y la limpieza de un `--root` explícito creado por la propia corrida.

use serde_json::Value;
use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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
        "{context}: la sonda debe terminar correctamente: {}",
        combined(output)
    );
    let text = String::from_utf8_lossy(&output.stdout);
    let text = text.trim();
    assert!(!text.is_empty(), "{context}: stdout no puede estar vacío");
    serde_json::from_str(text).unwrap_or_else(|error| {
        panic!("{context}: stdout debe ser un informe JSON: {error}; stdout={text}")
    })
}

fn run_without_root(output_dir: &Path) -> Output {
    let json = output_dir.join("extreme.json");
    let markdown = output_dir.join("extreme.md");
    bench()
        .args([
            "--extreme",
            "--confirm-extreme",
            "--profile",
            "realista",
            "--scale",
            "3",
            "--iterations",
            "1",
            "--json-output",
            json.to_str().expect("json path"),
            "--markdown-output",
            markdown.to_str().expect("markdown path"),
        ])
        .output()
        .expect("ejecutar sonda extrema")
}

fn run_args(args: &[&str]) -> Output {
    bench()
        .args(args)
        .output()
        .expect("ejecutar lodestar-bench")
}

fn extreme_report(output_dir: &Path, scale: &str, iterations: &str) -> (Output, PathBuf, PathBuf) {
    let json = output_dir.join("extreme.json");
    let markdown = output_dir.join("extreme.md");
    let output = bench()
        .args([
            "--extreme",
            "--confirm-extreme",
            "--profile",
            "realista",
            "--scale",
            scale,
            "--iterations",
            iterations,
            "--json-output",
            json.to_str().expect("json path"),
            "--markdown-output",
            markdown.to_str().expect("markdown path"),
        ])
        .output()
        .expect("ejecutar sonda extrema");
    (output, json, markdown)
}

fn measurements<'a>(report: &'a Value, context: &str) -> Vec<&'a Value> {
    let rows = report
        .get("measurements")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{context}: falta measurements"));
    assert_eq!(rows.len(), 3, "{context}: deben existir tres filas");
    rows.iter().collect()
}

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

#[test]
fn sqlite_footprint_expone_desglose_wal_shm_y_total_consistente() {
    let output_dir = tempfile::tempdir().expect("directorio de salidas");
    let output = run_without_root(output_dir.path());
    let report = report(&output, "footprint SQLite");
    let sqlite = report
        .get("sqlite")
        .and_then(Value::as_object)
        .expect("informe: falta objeto sqlite");

    for key in [
        "main_bytes",
        "wal_bytes",
        "shm_bytes",
        "auxiliary_bytes",
        "total_bytes",
    ] {
        assert!(
            sqlite.get(key).and_then(Value::as_u64).is_some(),
            "sqlite: {key} debe ser un contador entero medido"
        );
    }
    let main = sqlite["main_bytes"].as_u64().expect("main_bytes entero");
    let wal = sqlite["wal_bytes"].as_u64().expect("wal_bytes entero");
    let shm = sqlite["shm_bytes"].as_u64().expect("shm_bytes entero");
    let auxiliary = sqlite["auxiliary_bytes"]
        .as_u64()
        .expect("auxiliary_bytes entero");
    let total = sqlite["total_bytes"].as_u64().expect("total_bytes entero");
    assert_eq!(
        auxiliary,
        wal + shm,
        "sqlite: auxiliary_bytes debe ser exactamente wal_bytes + shm_bytes"
    );
    assert_eq!(
        total,
        main + auxiliary,
        "sqlite: total_bytes debe ser exactamente main_bytes + auxiliary_bytes"
    );
}

#[test]
fn root_explicito_creado_por_sonda_se_elimina_y_salidas_sobreviven() {
    let output_dir = tempfile::tempdir().expect("directorio de salidas");
    let root = output_dir.path().join("generated-root");
    let json = output_dir.path().join("outside.json");
    let markdown = output_dir.path().join("outside.md");
    assert!(
        !root.exists(),
        "la precondición exige root inicialmente inexistente"
    );

    let output = bench()
        .args([
            "--extreme",
            "--confirm-extreme",
            "--profile",
            "realista",
            "--scale",
            "3",
            "--iterations",
            "1",
            "--root",
            root.to_str().expect("root path"),
            "--json-output",
            json.to_str().expect("json path"),
            "--markdown-output",
            markdown.to_str().expect("markdown path"),
        ])
        .output()
        .expect("ejecutar sonda con root explícito");
    assert!(
        output.status.success(),
        "root inexistente debe ser un workspace temporal de la corrida: {}",
        combined(&output)
    );
    assert!(
        json.is_file(),
        "la salida JSON fuera del root debe sobrevivir"
    );
    assert!(
        markdown.is_file(),
        "la salida Markdown fuera del root debe sobrevivir"
    );
    assert!(
        fs::metadata(&json).expect("metadata JSON").len() > 0,
        "JSON no puede quedar vacío"
    );
    assert!(
        fs::metadata(&markdown).expect("metadata Markdown").len() > 0,
        "Markdown no puede quedar vacío"
    );
    assert!(
        !root.exists(),
        "la sonda debe eliminar el root explícito que creó al terminar"
    );
}

#[test]
fn extremo_rechaza_cada_parametro_obligatorio_ausente_y_perfil_desconocido() {
    let cases = [
        (
            ["--extreme", "--scale", "3", "--iterations", "1"].as_slice(),
            "profile",
        ),
        (
            ["--extreme", "--profile", "realista", "--iterations", "1"].as_slice(),
            "scale",
        ),
        (
            ["--extreme", "--profile", "realista", "--scale", "3"].as_slice(),
            "iterations",
        ),
        (
            [
                "--extreme",
                "--profile",
                "desconocido",
                "--scale",
                "3",
                "--iterations",
                "1",
            ]
            .as_slice(),
            "perfil desconocido",
        ),
    ];
    for (args, expected) in cases {
        let output = run_args(args);
        assert!(!output.status.success(), "caso {expected} debe fallar");
        let text = combined(&output).to_lowercase();
        assert!(
            text.contains(expected),
            "caso {expected}: el error debe nombrar la causa: {text}"
        );
    }
}

#[test]
fn variantes_unicas_exponen_siete_resultados_no_null_marcador_y_equivalencia_exacta() {
    let dir = tempfile::tempdir().expect("directorio temporal");
    let (output, _, _) = extreme_report(dir.path(), "3", "1");
    let report = report(&output, "equivalencia exacta");
    assert_eq!(report["functional_equivalence"], true);
    let variants: BTreeSet<_> = measurements(&report, "equivalencia exacta")
        .iter()
        .map(|row| row["variant"].as_str().unwrap_or("<missing>"))
        .collect();
    assert_eq!(
        variants,
        VARIANTS.iter().copied().collect(),
        "las variantes deben ser únicas y exactamente las tres ratificadas"
    );
    let baseline = measurements(&report, "equivalencia exacta")[0];
    for row in measurements(&report, "equivalencia exacta") {
        let variant = row["variant"].as_str().unwrap_or("<missing>");
        let tools = row["tools"]
            .as_object()
            .unwrap_or_else(|| panic!("{variant}: falta objeto tools"));
        assert_eq!(tools.len(), TOOLS.len(), "{variant}: deben ser siete tools");
        let baseline_tools = baseline["tools"].as_object().expect("baseline tools");
        for tool in TOOLS {
            let metric = tools
                .get(tool)
                .unwrap_or_else(|| panic!("{variant}/{tool}: falta lectura"));
            assert!(
                !metric["result"].is_null(),
                "{variant}/{tool}: result no puede ser null"
            );
            assert_eq!(
                metric["result"], baseline_tools[tool]["result"],
                "{variant}/{tool}: resultado normalizado divergente"
            );
        }
        let search_text = serde_json::to_string(&tools["knowledge_search"]["result"])
            .expect("search result serializable");
        assert!(
            search_text.contains("marker-search-h04"),
            "{variant}/knowledge_search: falta marcador semántico conocido"
        );
    }
}

#[test]
fn p50_y_p95_son_derivados_exactos_de_sample_elapsed_ns() {
    let dir = tempfile::tempdir().expect("directorio temporal");
    let (output, _, _) = extreme_report(dir.path(), "3", "3");
    let report = report(&output, "percentiles");
    assert_eq!(report["iterations"], 3);
    for row in measurements(&report, "percentiles") {
        let variant = row["variant"].as_str().unwrap_or("<missing>");
        let tools = row["tools"].as_object().expect("tools");
        for tool in TOOLS {
            let metric = &tools[tool];
            let mut samples: Vec<u64> = metric["sample_elapsed_ns"]
                .as_array()
                .unwrap_or_else(|| panic!("{variant}/{tool}: falta sample_elapsed_ns"))
                .iter()
                .map(|sample| sample.as_u64().expect("sample entero"))
                .collect();
            assert_eq!(samples.len(), 3, "{variant}/{tool}: tres muestras");
            samples.sort_unstable();
            let p50 = samples[samples.len() / 2];
            let p95_index = ((samples.len() * 95).saturating_sub(1) / 100).min(samples.len() - 1);
            assert_eq!(
                metric["p50_ns"].as_u64(),
                Some(p50),
                "{variant}/{tool}: p50"
            );
            assert_eq!(
                metric["p95_ns"].as_u64(),
                Some(samples[p95_index]),
                "{variant}/{tool}: p95"
            );
        }
    }
}

#[test]
fn rss_por_variante_declara_worker_aislado_y_metodo_portable() {
    let dir = tempfile::tempdir().expect("directorio temporal");
    let (output, _, _) = extreme_report(dir.path(), "3", "1");
    let report = report(&output, "RSS");
    for row in measurements(&report, "RSS") {
        let variant = row["variant"].as_str().unwrap_or("<missing>");
        let rss = row["rss"]
            .as_object()
            .unwrap_or_else(|| panic!("{variant}: falta rss"));
        match std::env::consts::OS {
            "macos" | "linux" => {
                assert_eq!(
                    rss["status"], "available",
                    "{variant}: RSS debe estar disponible"
                );
                assert!(rss["method"].as_str().unwrap_or("").contains("getrusage"));
                assert!(rss["absolute_bytes"].as_u64().unwrap_or(0) > 0);
            }
            "windows" => {
                assert_eq!(
                    rss["status"], "available",
                    "{variant}: RSS debe estar disponible"
                );
                assert!(rss["absolute_bytes"].as_u64().unwrap_or(0) > 0);
                assert_eq!(rss["raw_units"].as_str(), Some("bytes"));
                assert!(
                    rss["method"]
                        .as_str()
                        .is_some_and(|method| method.contains("GetProcessMemoryInfo")
                            && method.contains("PeakWorkingSetSize")),
                    "{variant}: la procedencia debe identificar la medición nativa de pico Windows"
                );
            }
            _ => {
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
        assert!(rss["units"].as_str().is_some_and(|value| !value.is_empty()));
        assert!(rss["platform"]
            .as_str()
            .is_some_and(|value| !value.is_empty()));
        assert!(rss["scope"].as_str().is_some_and(|value| !value.is_empty()));
        assert!(
            rss.get("worker_isolated")
                .and_then(Value::as_bool)
                .unwrap_or(false),
            "{variant}: RSS debe indicar worker_isolated=true"
        );
    }
}

#[test]
fn outputs_persistidos_coinciden_con_stdout_y_declaran_campos_principales() {
    let dir = tempfile::tempdir().expect("directorio temporal");
    let (output, json_path, markdown_path) = extreme_report(dir.path(), "3", "1");
    let stdout = String::from_utf8(output.stdout).expect("stdout UTF-8");
    let persisted_json = fs::read_to_string(&json_path).expect("JSON persistido");
    let stdout_value: Value = serde_json::from_str(stdout.trim()).expect("stdout JSON");
    let persisted_value: Value = serde_json::from_str(&persisted_json).expect("JSON persistido");
    assert_eq!(
        persisted_value, stdout_value,
        "JSON persistido debe coincidir semánticamente con stdout"
    );
    let value = stdout_value;
    let markdown = fs::read_to_string(&markdown_path).expect("Markdown persistido");
    for key in [
        "profile",
        "scale",
        "iterations",
        "captured_at",
        "functional_equivalence",
    ] {
        assert!(
            markdown.contains(&value[key].to_string()),
            "Markdown debe declarar {key}"
        );
    }
    assert!(markdown.contains("disk-reparseo"));
    assert!(markdown.contains("sqlite-raw"));
    assert!(markdown.contains("ram-memoizado"));
}

#[test]
fn fixture_estructural_extrema_y_md_companero_son_completos_y_portables() {
    let qa = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/qa");
    let json_path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/e33_h09_extreme_format.json");
    let markdown_path = qa.join("e33-h09-realista-100k-2026-08-23.md");
    assert!(
        json_path.is_file(),
        "falta fixture estructural JSON de formato extrema"
    );
    assert!(
        markdown_path.is_file(),
        "falta Markdown compañero de la corrida extrema 2026-08-23"
    );
    let text = fs::read_to_string(&json_path).expect("leer artefacto JSON");
    let forbidden = ["/private/", "/tmp/", "/Users/", "/home/", r"C:\Users\"];
    for route in forbidden {
        assert!(
            !text.contains(route),
            "artefacto filtra ruta privada {route}"
        );
    }
    let value: Value = serde_json::from_str(&text).expect("artefacto JSON válido");
    assert_eq!(value["profile"], "realista");
    assert_eq!(value["scale"], 100000);
    assert_eq!(value["iterations"], 1);
    assert!(value["captured_at"].as_str().is_some_and(|v| !v.is_empty()));
    assert!(value["platform"]["os"]
        .as_str()
        .is_some_and(|v| !v.is_empty()));
    assert!(value["platform"]["arch"]
        .as_str()
        .is_some_and(|v| !v.is_empty()));
    assert!(value["provenance"].as_object().is_some());
    assert_eq!(value["functional_equivalence"], true);
    let corpus_count = value["corpus"]["document_count"]
        .as_u64()
        .expect("corpus count");
    assert!(corpus_count >= 100000);
    let rows = measurements(&value, "fixture estructural de formato extrema");
    let variants: BTreeSet<_> = rows
        .iter()
        .map(|row| row["variant"].as_str().unwrap())
        .collect();
    assert_eq!(variants, VARIANTS.iter().copied().collect());
    for row in rows {
        assert_eq!(row["document_count"].as_u64(), Some(corpus_count));
        assert_eq!(row["tools"].as_object().map(|tools| tools.len()), Some(7));
        assert!(row["rss"].is_object());
    }
    let markdown = fs::read_to_string(markdown_path).expect("leer Markdown compañero");
    assert!(markdown.contains("100000"));
    assert!(!markdown.contains("/private/") && !markdown.contains("/Users/"));
}

#[test]
fn ningun_workflow_ci_ejecuta_la_sonda_extrema() {
    let workflows = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.github/workflows");
    let entries = fs::read_dir(&workflows).expect("listar workflows CI");
    let mut count = 0;
    for entry in entries {
        let path = entry.expect("entrada workflow").path();
        if !matches!(
            path.extension().and_then(|ext| ext.to_str()),
            Some("yml" | "yaml")
        ) {
            continue;
        }
        count += 1;
        let text = fs::read_to_string(&path).expect("leer workflow");
        assert!(
            !text.contains("--extreme"),
            "{} ejecuta --extreme",
            path.display()
        );
    }
    assert!(
        count > 0,
        "debe existir al menos un workflow CI que inspeccionar"
    );
}

#[cfg(unix)]
fn fake_df_dir(temp: &Path, body: &str) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let bin = temp.join("fake-bin");
    fs::create_dir_all(&bin).expect("fake bin");
    let log = temp.join("df-invocations.log");
    let script = bin.join("df");
    fs::write(&script, body).expect("fake df");
    let mut permissions = fs::metadata(&script)
        .expect("fake df metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("fake df executable");
    (bin, log)
}

#[cfg(unix)]
#[test]
fn preflight_df_conocido_falla_tambien_con_confirmacion_y_no_crea_root() {
    let temp = tempfile::tempdir().expect("directorio temporal");
    let script = "#!/bin/sh\necho invoked >> \"$LODESTAR_H09_DF_LOG\"\necho 'Filesystem 1024-blocks Used Available Capacity Mounted on'\necho '/fake 1 1 0 100% /'\n";
    let (bin, log) = fake_df_dir(temp.path(), script);
    let root = temp.path().join("known-insufficient-root");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    for confirm in [false, true] {
        let mut command = bench();
        command
            .env("PATH", &path)
            .env("LODESTAR_H09_DF_LOG", &log)
            .args([
                "--extreme",
                "--profile",
                "realista",
                "--scale",
                "3",
                "--iterations",
                "1",
                "--root",
                root.to_str().expect("root path"),
            ]);
        if confirm {
            command.arg("--confirm-extreme");
        }
        let output = command.output().expect("preflight df insuficiente");
        assert!(
            !output.status.success(),
            "espacio conocido insuficiente debe fallar"
        );
        assert!(combined(&output).contains("insuficiente"));
        assert!(!root.exists(), "preflight no debe crear root parcial");
    }
    let marker = fs::read_to_string(log).expect("marcador df");
    assert!(
        marker.lines().count() >= 2,
        "df debe consultarse en ambos casos"
    );
}

#[cfg(unix)]
#[test]
fn preflight_df_no_verificable_se_consulta_antes_de_exigir_confirmacion() {
    let temp = tempfile::tempdir().expect("directorio temporal");
    let script = "#!/bin/sh\necho invoked >> \"$LODESTAR_H09_DF_LOG\"\nexit 1\n";
    let (bin, log) = fake_df_dir(temp.path(), script);
    let root = temp.path().join("uncertain-root");
    let path = format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    );
    let mut command = bench();
    command
        .env("PATH", path)
        .env("LODESTAR_H09_DF_LOG", &log)
        .args([
            "--extreme",
            "--profile",
            "realista",
            "--scale",
            "1000000",
            "--iterations",
            "1",
            "--root",
            root.to_str().expect("root path"),
        ]);
    let output = command.output().expect("preflight df no verificable");
    assert!(!output.status.success());
    let marker = fs::read_to_string(&log).unwrap_or_default();
    assert!(
        !marker.trim().is_empty(),
        "df debe consultarse antes del error de confirmación"
    );
    assert!(combined(&output).contains("confirm"));
    assert!(!root.exists(), "preflight no verificable no crea root");
}
