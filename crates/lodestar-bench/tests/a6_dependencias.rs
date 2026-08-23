//! Guardas estructurales de E33-H04-A6, independientes de la implementación del bench.

use serde_json::Value;
use std::fs;
use std::path::PathBuf;
use std::process::Command;

fn workspace_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .expect("workspace root")
}

#[test]
fn a6_app_no_tiene_dependencia_directa_store_y_contrato_mcp_es_byte_igual() {
    let root = workspace_root();
    let manifest = fs::read_to_string(root.join("crates/lodestar-app/Cargo.toml"))
        .expect("lodestar-app manifest");
    let mut in_dependencies = false;
    for line in manifest.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_dependencies = trimmed == "[dependencies]";
            continue;
        }
        if in_dependencies {
            assert!(
                !trimmed.starts_with("lodestar-store"),
                "A6: lodestar-app no puede depender directamente de lodestar-store"
            );
        }
    }
    let metadata = Command::new("cargo")
        .args(["metadata", "--no-deps", "--format-version", "1"])
        .current_dir(&root)
        .output()
        .expect("cargo metadata");
    assert!(
        metadata.status.success(),
        "A6: cargo metadata debe funcionar: {}",
        String::from_utf8_lossy(&metadata.stderr)
    );
    let metadata: Value = serde_json::from_slice(&metadata.stdout).expect("cargo metadata JSON");
    let app = metadata["packages"]
        .as_array()
        .unwrap()
        .iter()
        .find(|package| package["name"] == "lodestar-app")
        .expect("lodestar-app metadata");
    let direct_store = app["dependencies"]
        .as_array()
        .unwrap()
        .iter()
        .any(|dep| dep["name"] == "lodestar-store" && dep["kind"] == "normal");
    assert!(
        !direct_store,
        "A6: cargo metadata no debe mostrar dependencia directa app→store"
    );
    // The executable guard owns base-ref discovery.  Keeping that policy in the bench avoids
    // making this integration test assume that a local `develop` branch exists (GitHub Actions
    // pull-request checkouts expose only `origin/develop`).
    let expected = Command::new(env!("CARGO_BIN_EXE_lodestar-bench"))
        .args(["--check-a6-dependencies"])
        .current_dir(&root)
        .output()
        .expect("A6 contract guard");
    assert!(
        expected.status.success(),
        "A6: la guarda de contrato debe funcionar en el checkout actual: stdout={} stderr={}",
        String::from_utf8_lossy(&expected.stdout),
        String::from_utf8_lossy(&expected.stderr)
    );
}
