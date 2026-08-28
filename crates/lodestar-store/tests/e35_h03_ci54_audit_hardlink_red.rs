//! E35-H03 CI54 — el seam de auditoría no puede aliasar físicamente Markdown canónico.
//!
//! La variable del seam es global al proceso. El padre crea los dos casos y ejecuta cada uno en
//! un subprocess: un `.lodestar/audit.ndjson` que es hardlink de `control.md`, y un sidecar regular
//! legítimo que debe conservar los eventos de lectura. Así se observan los bytes reales sin que un
//! caso contamine el entorno del otro.

use lodestar_core::types::RelPath;
use lodestar_core::DocumentStore;
use lodestar_store::Store;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TEST_NAME: &str =
    "ci54_rebuild_ignora_audit_hardlink_de_markdown_y_conserva_sidecar_legitimo";
const CHILD_MODE: &str = "LODESTAR_CI54_CHILD_MODE";
const CHILD_ROOT: &str = "LODESTAR_CI54_CHILD_ROOT";
const READ_AUDIT: &str = "LODESTAR_H03_TEST_READ_AUDIT";

fn write_workspace(root: &Path) -> BTreeMap<String, Vec<u8>> {
    fs::create_dir_all(root.join(".lodestar")).expect("CI54 crear plano de control");
    let markdown = BTreeMap::from([
        (
            "control.md".to_owned(),
            b"---\ntags: [ci54, hardlink]\n---\n# Canonical control\nbytes-must-never-change\n"
                .to_vec(),
        ),
        (
            "peer.md".to_owned(),
            b"---\ntags: [ci54, peer]\n---\n# Peer\nrebuild-must-remain-safe\n".to_vec(),
        ),
    ]);
    for (name, bytes) in &markdown {
        fs::write(root.join(name), bytes).expect("CI54 escribir Markdown canónico");
    }
    markdown
}

fn markdown_bytes(root: &Path) -> BTreeMap<String, Vec<u8>> {
    ["control.md", "peer.md"]
        .into_iter()
        .map(|name| {
            (
                name.to_owned(),
                fs::read(root.join(name)).expect("CI54 leer Markdown canónico"),
            )
        })
        .collect()
}

fn assert_hardlink_identity(markdown: &Path, audit: &Path) {
    let markdown_metadata = fs::metadata(markdown).expect("CI54 metadata Markdown");
    let audit_metadata = fs::metadata(audit).expect("CI54 metadata audit hardlink");
    assert!(
        markdown_metadata.is_file() && audit_metadata.is_file(),
        "CI54 anti-vacuidad: ambos nombres deben resolver a un fichero regular"
    );
    assert_eq!(
        fs::read(markdown).expect("CI54 leer origen del hardlink"),
        fs::read(audit).expect("CI54 leer nombre alternativo del hardlink"),
        "CI54 anti-vacuidad: el alias físico parte de los mismos bytes"
    );

    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;

        assert_eq!(
            (markdown_metadata.dev(), markdown_metadata.ino()),
            (audit_metadata.dev(), audit_metadata.ino()),
            "CI54 anti-vacuidad: Markdown y audit deben compartir (dev, ino)"
        );
        assert!(
            markdown_metadata.nlink() >= 2 && audit_metadata.nlink() >= 2,
            "CI54 anti-vacuidad: el inode debe tener al menos dos enlaces"
        );
    }
}

fn assert_published_snapshot(root: &Path, expected: &BTreeMap<String, Vec<u8>>) {
    let reopened = Store::open(root).expect("CI54 reabrir índice publicado");
    assert_eq!(
        reopened.paths(),
        vec![
            RelPath::new("control.md").expect("CI54 RelPath control"),
            RelPath::new("peer.md").expect("CI54 RelPath peer"),
        ],
        "CI54 rebuild seguro: la generación publicada contiene el corpus exacto"
    );
    for (name, bytes) in expected {
        let path = RelPath::new(name).expect("CI54 RelPath de snapshot");
        let expected_text = String::from_utf8(bytes.clone()).expect("CI54 fixture UTF-8");
        assert_eq!(
            reopened.raw(&path).as_deref(),
            Some(expected_text.as_str()),
            "CI54 rebuild seguro: SQLite conserva el snapshot exacto de {name}"
        );
    }
}

fn child_case(mode: &str) {
    let root = PathBuf::from(std::env::var_os(CHILD_ROOT).expect("CI54 child root"));
    let audit = PathBuf::from(std::env::var_os(READ_AUDIT).expect("CI54 child audit"));
    let before_markdown = markdown_bytes(&root);
    let before_audit = fs::read(&audit).expect("CI54 audit inicial");
    assert_eq!(
        before_markdown.keys().cloned().collect::<BTreeSet<_>>(),
        BTreeSet::from(["control.md".to_owned(), "peer.md".to_owned()]),
        "CI54 anti-vacuidad: corpus exacto de dos Markdown"
    );

    if mode == "hardlink" {
        assert_hardlink_identity(&root.join("control.md"), &audit);
    }

    let store = Store::open(&root).expect("CI54 Store::open en subprocess aislado");
    let paths = [
        RelPath::new("control.md").expect("CI54 RelPath control"),
        RelPath::new("peer.md").expect("CI54 RelPath peer"),
    ];
    let rebuild = store.rebuild_from_inventory(&paths, &BTreeSet::new());
    let after_markdown = markdown_bytes(&root);
    let after_audit = fs::read(&audit).expect("CI54 audit final");

    assert_eq!(
        after_markdown, before_markdown,
        "rojo causal CI54: el audit hardlink modificó Markdown canónico; rebuild={rebuild:?}"
    );
    rebuild.expect("CI54: ignorar el alias físico debe permitir un rebuild seguro");
    assert_published_snapshot(&root, &before_markdown);

    match mode {
        "hardlink" => assert_eq!(
            after_audit, before_audit,
            "CI54: el audit que aliasa Markdown debe quedar completamente ignorado"
        ),
        "regular" => {
            assert!(
                after_audit.len() > before_audit.len(),
                "CI54 control anti-vacuidad: el sidecar regular debe recibir eventos"
            );
            let appended = &after_audit[before_audit.len()..];
            let events = std::str::from_utf8(appended)
                .expect("CI54 control NDJSON UTF-8")
                .lines()
                .map(|line| {
                    serde_json::from_str::<serde_json::Value>(line).expect("CI54 evento JSON")
                })
                .collect::<Vec<_>>();
            assert_eq!(
                events.len(),
                2,
                "CI54 control: una lectura auditada por cada Markdown real"
            );
            assert_eq!(
                events
                    .iter()
                    .map(|event| {
                        assert_eq!(event["event"].as_str(), Some("payload_read"));
                        assert_eq!(event["open_count"].as_u64(), Some(1));
                        assert_eq!(event["read_count"].as_u64(), Some(1));
                        event["path"]
                            .as_str()
                            .expect("CI54 evento con path")
                            .to_owned()
                    })
                    .collect::<BTreeSet<_>>(),
                BTreeSet::from(["control.md".to_owned(), "peer.md".to_owned()]),
                "CI54 control: el sidecar legítimo conserva los eventos reales del corpus"
            );
        }
        other => panic!("CI54 child mode desconocido: {other}"),
    }
}

fn spawn_case(mode: &str, root: &Path, audit: &Path) -> Output {
    Command::new(std::env::current_exe().expect("CI54 localizar test executable"))
        .args(["--exact", TEST_NAME, "--nocapture", "--test-threads=1"])
        .env(CHILD_MODE, mode)
        .env(CHILD_ROOT, root)
        .env(READ_AUDIT, audit)
        .output()
        .expect("CI54 ejecutar subprocess")
}

fn diagnostics(label: &str, output: &Output, root: &Path) -> String {
    format!(
        "{label}={{status:{}, stdout:{}, stderr:{}, control:{:?}, audit:{:?}}}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr),
        fs::read(root.join("control.md")),
        fs::read(root.join(".lodestar/audit.ndjson")),
    )
}

fn hardlinks_unavailable(error: &io::Error) -> bool {
    matches!(
        error.kind(),
        io::ErrorKind::Unsupported | io::ErrorKind::PermissionDenied
    )
}

#[test]
fn ci54_rebuild_ignora_audit_hardlink_de_markdown_y_conserva_sidecar_legitimo() {
    if let Some(mode) = std::env::var_os(CHILD_MODE) {
        child_case(&mode.to_string_lossy());
        return;
    }

    let hardlink_case = tempfile::tempdir().expect("CI54 sandbox hardlink");
    let hardlink_root = hardlink_case.path().join("workspace");
    write_workspace(&hardlink_root);
    let hardlink_audit = hardlink_root.join(".lodestar/audit.ndjson");
    let hardlink_output = match fs::hard_link(hardlink_root.join("control.md"), &hardlink_audit) {
        Ok(()) => Some(spawn_case("hardlink", &hardlink_root, &hardlink_audit)),
        Err(error) if hardlinks_unavailable(&error) => {
            eprintln!(
                "CI54: plataforma/filesystem sin hardlinks disponibles; caso omitido: {error}"
            );
            None
        }
        Err(error) => panic!("CI54 crear hardlink de reproducción: {error}"),
    };

    let regular_case = tempfile::tempdir().expect("CI54 sandbox sidecar regular");
    let regular_root = regular_case.path().join("workspace");
    write_workspace(&regular_root);
    let regular_audit = regular_root.join(".lodestar/audit.ndjson");
    fs::write(&regular_audit, b"").expect("CI54 crear sidecar regular independiente");
    let regular_output = spawn_case("regular", &regular_root, &regular_audit);

    let hardlink_ok = hardlink_output
        .as_ref()
        .map(|output| output.status.success())
        .unwrap_or(true);
    assert!(
        hardlink_ok && regular_output.status.success(),
        "CI54 subprocesses deben rechazar alias físico y conservar el control legítimo; {}; {}",
        hardlink_output
            .as_ref()
            .map(|output| diagnostics("hardlink", output, &hardlink_root))
            .unwrap_or_else(|| "hardlink={unsupported}".to_owned()),
        diagnostics("regular", &regular_output, &regular_root),
    );
}
