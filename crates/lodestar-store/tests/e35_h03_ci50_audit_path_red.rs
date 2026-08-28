//! E35-H03 CI50 — el seam de auditoría nunca puede escribir sobre Markdown canónico.
//!
//! La variable del seam es global al proceso. El padre ejecuta cada caso en un subprocess para
//! aislar el entorno y observar bytes reales: un audit que aliasa el propio `.md`, y un control
//! fuera del workspace que debe permanecer ignorado. No se inspecciona la implementación.

use lodestar_core::types::RelPath;
use lodestar_store::Store;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TEST_NAME: &str = "ci50_rebuild_no_escribe_audit_sobre_markdown_canonico";
const CHILD_MODE: &str = "LODESTAR_CI50_CHILD_MODE";
const CHILD_ROOT: &str = "LODESTAR_CI50_CHILD_ROOT";
const OUTSIDE_SENTINEL: &str = "LODESTAR_CI50_OUTSIDE_SENTINEL";
const READ_AUDIT: &str = "LODESTAR_H03_TEST_READ_AUDIT";

fn write_workspace(root: &Path) {
    fs::create_dir(root).expect("CI50 crear workspace exacto");
    fs::write(
        root.join("control.md"),
        "---\ntags: [ci50, control]\n---\n# Canonical control\nbytes-must-never-change\n",
    )
    .expect("CI50 control Markdown");
    fs::write(
        root.join("peer.md"),
        "---\ntags: [ci50, peer]\n---\n# Peer\npeer-also-immutable\n",
    )
    .expect("CI50 peer Markdown");
}

fn markdown_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::read_dir(root)
        .expect("CI50 root legible")
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| path.extension().and_then(|extension| extension.to_str()) == Some("md"))
        .map(|path| {
            let name = path
                .file_name()
                .and_then(|name| name.to_str())
                .expect("CI50 nombre Markdown UTF-8")
                .to_owned();
            let bytes = fs::read(&path).expect("CI50 leer bytes Markdown");
            (name, bytes)
        })
        .collect()
}

fn child_case(mode: &str) {
    let root = PathBuf::from(std::env::var_os(CHILD_ROOT).expect("CI50 child root"));
    let outside =
        PathBuf::from(std::env::var_os(OUTSIDE_SENTINEL).expect("CI50 child sentinel exterior"));
    let before_markdown = markdown_bytes(&root);
    let before_outside = fs::read(&outside).expect("CI50 sentinel exterior inicial");
    assert_eq!(
        before_markdown.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from(["control.md".to_owned(), "peer.md".to_owned()]),
        "CI50 anti-vacuidad: corpus exacto de dos Markdown"
    );

    let store = Store::open(&root).expect("CI50 Store::open en subprocess aislado");
    let paths = [
        RelPath::new("control.md").expect("CI50 RelPath control"),
        RelPath::new("peer.md").expect("CI50 RelPath peer"),
    ];
    let result = store.rebuild_from_inventory(&paths, &BTreeSet::new());
    let after_markdown = markdown_bytes(&root);
    let after_outside = fs::read(&outside).expect("CI50 sentinel exterior final");

    if before_markdown != after_markdown {
        let error = result
            .as_ref()
            .expect_err("CI50: la mutación actual debe hacer abortar el rebuild")
            .to_string();
        assert!(
            error.contains("snapshot changed while indexing"),
            "CI50: la reproducción debe fallar por detectar la mutación causada durante la lectura; error={error}"
        );
    }
    assert_eq!(
        after_markdown,
        before_markdown,
        "rojo causal CI50: READ_AUDIT={:?} modificó Markdown canónico aunque rebuild={result:?}",
        std::env::var_os(READ_AUDIT)
    );
    assert_eq!(
        after_outside, before_outside,
        "CI50 C6: el seam tampoco puede escribir fuera del workspace"
    );
    if mode == "outside-control" {
        assert!(
            result.is_ok(),
            "CI50 control: un audit exterior ignorado no debe alterar el rebuild: {result:?}"
        );
    }
}

fn spawn_case(mode: &str, root: &Path, audit: &Path, outside: &Path) -> Output {
    Command::new(std::env::current_exe().expect("CI50 localizar test executable"))
        .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
        .env(CHILD_MODE, mode)
        .env(CHILD_ROOT, root)
        .env(OUTSIDE_SENTINEL, outside)
        .env(READ_AUDIT, audit)
        .output()
        .expect("CI50 ejecutar subprocess")
}

fn output_diagnostics(label: &str, output: &Output, root: &Path) -> String {
    let entries = fs::read_dir(root)
        .map(|entries| {
            entries
                .filter_map(Result::ok)
                .map(|entry| {
                    let path = entry.path();
                    let size = fs::metadata(&path)
                        .map(|metadata| metadata.len().to_string())
                        .unwrap_or_else(|error| format!("error:{error}"));
                    format!("{}:{size}", path.display())
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|error| vec![format!("read_dir_error:{error}")]);
    format!(
        "{label}={{status:{}, stdout:{}, stderr:{}, root:{}, entries:{entries:?}}}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        root.display()
    )
}

#[test]
fn ci50_rebuild_no_escribe_audit_sobre_markdown_canonico() {
    if let Some(mode) = std::env::var_os(CHILD_MODE) {
        child_case(&mode.to_string_lossy());
        return;
    }

    let alias_case = tempfile::tempdir().expect("CI50 sandbox alias Markdown");
    let alias_root = alias_case.path().join("workspace");
    write_workspace(&alias_root);
    let alias_outside = alias_case.path().join("outside-sentinel.bin");
    fs::write(&alias_outside, b"OUTSIDE-MUST-NOT-CHANGE\n").expect("CI50 sentinel alias case");
    let alias_output = spawn_case(
        "markdown-alias",
        &alias_root,
        &alias_root.join("control.md"),
        &alias_outside,
    );

    let outside_case = tempfile::tempdir().expect("CI50 sandbox audit exterior");
    let outside_root = outside_case.path().join("workspace");
    write_workspace(&outside_root);
    let outside_audit = outside_case.path().join("outside-audit.ndjson");
    fs::write(&outside_audit, b"EXTERNAL-AUDIT-MUST-NOT-CHANGE\n")
        .expect("CI50 audit exterior inicial");
    let outside_output = spawn_case(
        "outside-control",
        &outside_root,
        &outside_audit,
        &outside_audit,
    );

    assert!(
        alias_output.status.success() && outside_output.status.success(),
        "CI50 subprocesses deben preservar todo fichero no autorizado; {}; {}",
        output_diagnostics("markdown_alias", &alias_output, &alias_root),
        output_diagnostics("outside_control", &outside_output, &outside_root)
    );
}
