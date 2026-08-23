//! Fase roja final de E33-H09: preflight trazable, workers/RSS del artefacto y escala abierta.
//!
//! Los comandos positivos usan escalas pequeñas. El informe 100k se inspecciona como evidencia
//! versionada; esta suite nunca lo vuelve a materializar.

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

fn parse_report(output: &Output, context: &str) -> Value {
    assert!(
        output.status.success(),
        "{context}: ejecución fallida: {}",
        combined(output)
    );
    serde_json::from_str(String::from_utf8_lossy(&output.stdout).trim())
        .unwrap_or_else(|error| panic!("{context}: JSON inválido: {error}"))
}

fn rows<'a>(report: &'a Value, context: &str) -> Vec<&'a Value> {
    let rows = report["measurements"]
        .as_array()
        .unwrap_or_else(|| panic!("{context}: falta measurements"));
    assert_eq!(rows.len(), 3, "{context}: deben existir tres filas");
    rows.iter().collect()
}

fn artifact_path() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/e33_h09_extreme_format.json")
}

#[cfg(unix)]
fn fake_df(temp: &Path, available_blocks: u64) -> (PathBuf, PathBuf) {
    use std::os::unix::fs::PermissionsExt;
    let bin = temp.join("fake-bin");
    fs::create_dir_all(&bin).expect("fake df bin");
    let script = bin.join("df");
    let log = temp.join("df.log");
    let body = format!(
        "#!/bin/sh\necho invoked >> \"$LODESTAR_H09_DF_LOG\"\necho 'Filesystem 1024-blocks Used Available Capacity Mounted on'\necho '/fake 1 0 {available_blocks} 0% /'\n"
    );
    fs::write(&script, body).expect("fake df");
    let mut permissions = fs::metadata(&script)
        .expect("fake df metadata")
        .permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&script, permissions).expect("fake df executable");
    (bin, log)
}

#[cfg(unix)]
fn fake_path(bin: &Path) -> String {
    format!(
        "{}:{}",
        bin.display(),
        std::env::var("PATH").unwrap_or_default()
    )
}

#[cfg(unix)]
#[test]
fn preflight_checked_expone_bytes_exactos_y_coherentes() {
    let temp = tempfile::tempdir().expect("tempdir");
    let available_blocks = 500_000;
    let (bin, log) = fake_df(temp.path(), available_blocks);
    let json = temp.path().join("checked.json");
    let mut command = bench();
    command
        .env("PATH", fake_path(&bin))
        .env("LODESTAR_H09_DF_LOG", &log)
        .args([
            "--extreme",
            "--profile",
            "realista",
            "--scale",
            "3",
            "--iterations",
            "1",
            "--json-output",
            json.to_str().expect("json path"),
        ]);
    let report = parse_report(
        &command.output().expect("ejecutar preflight checked"),
        "preflight checked",
    );
    let available = available_blocks * 1024;
    assert_eq!(report["preflight"]["status"], "checked");
    let required = report["preflight"]["required_bytes"]
        .as_u64()
        .expect("required_bytes entero");
    assert!(required > 0, "required_bytes positivo");
    assert_eq!(report["preflight"]["available_bytes"], available);
    assert!(available >= required);
    assert!(!fs::read_to_string(log)
        .unwrap_or_default()
        .trim()
        .is_empty());
}

#[test]
fn artefacto_100k_declara_preflight_pids_rss_y_equivalencia_semantica() {
    let path = artifact_path();
    assert!(path.is_file(), "falta artefacto 100k exacto");
    let text = fs::read_to_string(&path).expect("leer artefacto 100k");
    let report: Value = serde_json::from_str(&text).expect("artefacto JSON válido");
    assert_eq!(report["preflight"]["status"], "checked");
    let required = report["preflight"]["required_bytes"]
        .as_u64()
        .expect("required_bytes positivo");
    let available = report["preflight"]["available_bytes"]
        .as_u64()
        .expect("available_bytes positivo");
    assert!(required > 0 && available >= required);
    let coordinator = report["coordinator_pid"].as_u64().expect("coordinator_pid");
    let mut worker_pids = BTreeSet::new();
    let rows = rows(&report, "artefacto 100k");
    let baseline = rows[0]["tools"].as_object().expect("baseline tools");
    for row in rows {
        let variant = row["variant"].as_str().unwrap_or("<missing>");
        let worker = row["worker_pid"].as_u64().expect("worker_pid");
        assert_ne!(worker, coordinator, "{variant}: worker separado");
        assert!(worker_pids.insert(worker), "worker_pid único por variante");
        let rss = row["rss"].as_object().expect("rss objeto");
        assert_eq!(rss["status"], "available");
        assert!(rss["method"]
            .as_str()
            .is_some_and(|v| v.contains("getrusage")));
        let raw = rss["raw_value"].as_u64().expect("RSS raw_value");
        let absolute = rss["absolute_bytes"].as_u64().expect("RSS absolute_bytes");
        let units = rss["raw_units"].as_str().expect("RSS raw_units");
        match report["platform"]["os"].as_str() {
            Some("macos") => {
                assert_eq!(units, "bytes");
                assert_eq!(absolute, raw);
            }
            Some("linux") => {
                assert_eq!(units, "KiB");
                assert_eq!(absolute, raw * 1024);
            }
            other => panic!("plataforma RSS no soportada: {other:?}"),
        }
        let tools = row["tools"].as_object().expect("tools objeto");
        assert_eq!(tools.len(), 7);
        for tool in TOOLS {
            assert!(
                !tools[tool]["result"].is_null(),
                "{variant}/{tool} no vacío"
            );
            assert_eq!(tools[tool]["result"], baseline[tool]["result"]);
        }
        let status = &row["tools"]["workspace_status"]["result"];
        assert!(status["counts"]["documents"].as_u64().unwrap_or(0) > 0);
        assert!(
            serde_json::to_string(&row["tools"]["knowledge_search"]["result"])
                .expect("search JSON")
                .contains("marker-search-h04")
        );
        assert!(row["tools"]["knowledge_get"]["result"]["body"]
            .as_str()
            .unwrap_or("")
            .contains("marker-get-h04"));
        assert_eq!(row["tools"]["knowledge_get"]["result"]["path"], "child.md");
        assert!(
            row["tools"]["metadata_inspect"]["result"]["presentIn"]
                .as_u64()
                .unwrap_or(0)
                > 0
        );
        assert!(row["tools"]["graph_query"]["result"]["edges"]
            .as_array()
            .is_some_and(|v| !v.is_empty()));
        assert!(
            row["tools"]["impact_analyze"]["result"]["affectedDocuments"]
                .as_array()
                .is_some_and(|v| !v.is_empty())
        );
        assert!(row["tools"]["knowledge_check"]["result"]["diagnostics"]
            .as_array()
            .is_some_and(|v| !v.is_empty()));
    }
    assert_eq!(worker_pids.len(), 3);
    assert_eq!(report["functional_equivalence"], true);
    let variants: BTreeSet<_> = report["variants"]
        .as_array()
        .expect("variants")
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(variants, VARIANTS.iter().copied().collect());
}

#[test]
fn provenance_commit_del_artefacto_es_sha1_resoluble() {
    let report: Value =
        serde_json::from_str(&fs::read_to_string(artifact_path()).expect("leer artefacto"))
            .expect("JSON artefacto");
    let commit = report["provenance"]["commit"]
        .as_str()
        .expect("commit provenance");
    assert_eq!(commit.len(), 40, "commit debe parecer SHA-1");
    assert!(commit
        .chars()
        .all(|character| character.is_ascii_hexdigit()));
    let object = format!("{commit}^{{commit}}");
    let output = Command::new("git")
        .args(["cat-file", "-e", &object])
        .output()
        .expect("git cat-file");
    assert!(
        output.status.success(),
        "commit provenance resoluble: {}",
        combined(&output)
    );
}

#[test]
fn escalas_positivas_fuera_de_historicas_no_son_whitelist() {
    for scale in ["17", "257"] {
        let output = tempfile::tempdir().expect("tempdir");
        let json = output.path().join(format!("scale-{scale}.json"));
        let mut command = bench();
        command.args([
            "--extreme",
            "--profile",
            "realista",
            "--scale",
            scale,
            "--iterations",
            "1",
            "--json-output",
            json.to_str().expect("json path"),
        ]);
        let report = parse_report(&command.output().expect("escala abierta"), scale);
        assert_eq!(report["scale"], scale.parse::<u64>().expect("scale entero"));
        assert_eq!(rows(&report, scale).len(), 3);
    }
}
