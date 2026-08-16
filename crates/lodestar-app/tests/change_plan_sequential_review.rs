//! Reparaciones de cobertura surgidas de la revisión fresca del planner secuencial.
//!
//! Complementan la reproducción roja original con observaciones independientes de cada efecto,
//! del booleano del aplicador canónico y de la ausencia de publicación al rechazar planes legacy.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use lodestar_app::App;
use lodestar_core::plan::{self, PlanPolicy};
use lodestar_core::types::{ErrorCode, FileMap, NormalizedOperation, RelPath};
use serde_json::{json, Value};

fn write(root: &Path, rel: &str, body: &str) {
    let path = root.join(rel);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, body).unwrap();
}

fn read(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

fn policy() -> PlanPolicy {
    PlanPolicy {
        require_valid_result: false,
        allow_warnings: true,
    }
}

fn seed(root: &Path, body: &str) {
    write(root, "index.md", "# Index\n");
    write(
        root,
        "doc.md",
        &format!("---\ntype: Note\ntitle: Doc\n---\n\n{body}"),
    );
}

fn plan_files(root: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(root.join(".lodestar/runtime/plans"))
        .unwrap()
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect()
}

#[test]
fn patch_y_replace_conservan_ambos_efectos_en_ambos_ordenes() {
    for (name, operations) in [
        (
            "patch_then_replace",
            json!([
                {"op":"patch_frontmatter","path":"doc.md","patch":{"status":"review"}},
                {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB"}
            ]),
        ),
        (
            "replace_then_patch",
            json!([
                {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB"},
                {"op":"patch_frontmatter","path":"doc.md","patch":{"status":"review"}}
            ]),
        ),
    ] {
        let dir = tempfile::tempdir().unwrap();
        seed(dir.path(), "AAA\n");
        let app = App::open(dir.path()).unwrap();
        let plan = app
            .change_plan(None, &operations, policy())
            .unwrap_or_else(|error| panic!("{name}: el plan debía ser válido: {error:?}"));
        app.change_apply(&plan.change_set_id, None)
            .unwrap_or_else(|error| panic!("{name}: el apply debía publicar: {error:?}"));

        let final_raw = read(dir.path(), "doc.md");
        assert!(
            final_raw.contains("status: review"),
            "{name}: se perdió el patch: {final_raw}"
        );
        assert!(
            final_raw.contains("BBB"),
            "{name}: se perdió el replace_text: {final_raw}"
        );
        assert!(
            !final_raw.contains("AAA"),
            "{name}: sobrevivió el texto reemplazado: {final_raw}"
        );
    }
}

#[test]
fn dos_edit_section_conservan_ambas_secciones_y_contenidos() {
    let dir = tempfile::tempdir().unwrap();
    seed(
        dir.path(),
        "# Doc\n\n## First\n\nold-first\n\n## Second\n\nold-second\n",
    );
    let app = App::open(dir.path()).unwrap();
    let plan = app
        .change_plan(
            None,
            &json!([
                {"op":"edit_section","path":"doc.md","headingPath":["First"],"mode":"replace","content":"one"},
                {"op":"edit_section","path":"doc.md","headingPath":["Second"],"mode":"replace","content":"two"}
            ]),
            policy(),
        )
        .unwrap();
    app.change_apply(&plan.change_set_id, None).unwrap();

    let final_raw = read(dir.path(), "doc.md");
    for expected in ["## First", "one", "## Second", "two"] {
        assert!(
            final_raw.contains(expected),
            "falta {expected:?} tras componer las secciones: {final_raw}"
        );
    }
    assert!(!final_raw.contains("old-first"));
    assert!(!final_raw.contains("old-second"));
}

#[test]
fn aplicador_unitario_reporta_efecto_y_el_fold_aplica_la_segunda_terminal() {
    let a = RelPath::new("a.md").unwrap();
    let b = RelPath::new("b.md").unwrap();
    let mut base: FileMap = BTreeMap::new();
    base.insert(a.clone(), "old\n".to_string());

    let identical = NormalizedOperation::ReplaceBody {
        path: a.clone(),
        body: "old\n".to_string(),
    };
    let mut stepwise = base.clone();
    assert!(!plan::apply_normalized_operation(&mut stepwise, &identical).unwrap());
    assert_eq!(stepwise, base);

    let first = NormalizedOperation::ReplaceBody {
        path: a.clone(),
        body: "one\n".to_string(),
    };
    let second = NormalizedOperation::ReplaceBody {
        path: a.clone(),
        body: "two\n".to_string(),
    };
    assert!(plan::apply_normalized_operation(&mut stepwise, &first).unwrap());
    assert!(plan::apply_normalized_operation(&mut stepwise, &second).unwrap());
    assert_eq!(stepwise.get(&a).map(String::as_str), Some("two\n"));

    let folded = plan::apply_normalized_ops(&base, &[first, second]).unwrap();
    assert_eq!(folded, stepwise);
    assert_eq!(folded.get(&a).map(String::as_str), Some("two\n"));

    let same_path = NormalizedOperation::Move {
        from: a.clone(),
        to: a.clone(),
        rewrite_inbound_links: false,
    };
    let mut same_files = base.clone();
    assert!(!plan::apply_normalized_operation(&mut same_files, &same_path).unwrap());
    assert_eq!(same_files, base);

    let real_move = NormalizedOperation::Move {
        from: a.clone(),
        to: b.clone(),
        rewrite_inbound_links: false,
    };
    let mut moved = base.clone();
    assert!(plan::apply_normalized_operation(&mut moved, &real_move).unwrap());
    assert!(!moved.contains_key(&a));
    assert_eq!(moved.get(&b).map(String::as_str), Some("old\n"));
}

#[test]
fn other_files_se_conserva_en_el_veredicto_posterior_de_change_apply() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", "# Index\n");
    write(
        dir.path(),
        "README.md",
        "# README\n\nImplementado en [código](src/main.rs).\n",
    );
    write(dir.path(), "src/main.rs", "fn main() {}\n");
    let app = App::open(dir.path()).unwrap();
    let plan = app
        .change_plan(
            None,
            &json!([{"op":"replace_text","path":"README.md","find":"README","replace":"Guide"}]),
            policy(),
        )
        .unwrap();
    assert_eq!(plan.diagnostics_after.warnings, 0);

    let result = app.change_apply(&plan.change_set_id, None).unwrap();
    assert!(result.validation.valid);
    assert_eq!(result.validation.errors, 0);
    assert_eq!(result.validation.warnings, 0);
    let final_raw = read(dir.path(), "README.md");
    assert!(final_raw.contains("# Guide"));
    assert!(final_raw.contains("src/main.rs"));
}

#[test]
fn selection_delete_remove_links_publica_las_dos_bajas_y_la_reescritura_acumulada() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", "# Index\n");
    write(
        dir.path(),
        "source.md",
        "# Source\n\n[a](a.md) y [b](b.md)\n",
    );
    write(dir.path(), "a.md", "---\ntype: Target\n---\n# A\n");
    write(dir.path(), "b.md", "---\ntype: Target\n---\n# B\n");
    let app = App::open(dir.path()).unwrap();
    let plan = app
        .change_plan(
            None,
            &json!({
                "selection":{"where":"type = \"Target\""},
                "operation":{"delete":{"inboundLinksPolicy":"remove_links"}}
            }),
            policy(),
        )
        .unwrap();
    assert_eq!(plan.captured_revisions.len(), 2);

    let result = app.change_apply(&plan.change_set_id, None).unwrap();
    assert!(result.applied);
    assert!(!dir.path().join("a.md").exists());
    assert!(!dir.path().join("b.md").exists());
    let source = read(dir.path(), "source.md");
    assert!(source.contains("# Source"));
    assert!(
        !source.contains("a.md"),
        "se restauró el primer enlace: {source}"
    );
    assert!(
        !source.contains("b.md"),
        "se restauró el segundo enlace: {source}"
    );
}

#[derive(Debug, PartialEq, Eq)]
struct TreeSnapshot {
    directories: BTreeSet<PathBuf>,
    files: BTreeMap<PathBuf, Vec<u8>>,
}

fn tree_snapshot_without_audit(root: &Path) -> TreeSnapshot {
    fn visit(root: &Path, dir: &Path, snapshot: &mut TreeSnapshot) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let entry = entry.unwrap();
            let path = entry.path();
            let rel = path.strip_prefix(root).unwrap().to_path_buf();
            if rel == Path::new(".lodestar/runtime/audit.jsonl") {
                continue;
            }
            if entry.file_type().unwrap().is_dir() {
                snapshot.directories.insert(rel);
                visit(root, &path, snapshot);
            } else {
                snapshot.files.insert(rel, std::fs::read(path).unwrap());
            }
        }
    }

    let mut snapshot = TreeSnapshot {
        directories: BTreeSet::new(),
        files: BTreeMap::new(),
    };
    visit(root, root, &mut snapshot);
    snapshot
}

fn assert_legacy_is_stale_without_publication(version: Option<Value>) {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "AAA\n");
    let app = App::open(dir.path()).unwrap();
    let plan = app
        .change_plan(
            None,
            &json!([{"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB"}]),
            policy(),
        )
        .unwrap();
    let files = plan_files(dir.path());
    assert_eq!(files.len(), 1);
    let plan_path = &files[0];
    let mut persisted: Value = serde_json::from_slice(&std::fs::read(plan_path).unwrap()).unwrap();
    let expected_legacy_version = version.as_ref().and_then(Value::as_u64).unwrap_or(1);
    if let Some(version) = version {
        persisted["plannerSemanticsVersion"] = version;
    } else {
        persisted
            .as_object_mut()
            .unwrap()
            .remove("plannerSemanticsVersion");
    }
    std::fs::write(plan_path, serde_json::to_vec_pretty(&persisted).unwrap()).unwrap();

    let before = tree_snapshot_without_audit(dir.path());
    let error = app.change_apply(&plan.change_set_id, None).unwrap_err();
    assert_eq!(error.code, ErrorCode::PlanStale);
    assert!(error
        .message
        .contains("semántica anterior del planificador"));
    assert!(
        error
            .message
            .contains(&format!("versión {expected_legacy_version}")),
        "la ausencia de versión debe identificarse como v1 legacy: {error:?}"
    );
    assert_eq!(
        tree_snapshot_without_audit(dir.path()),
        before,
        "un plan legacy no puede publicar documentos ni crear recibos, journal, staging o recovery"
    );
}

#[test]
fn plan_sin_version_se_rechaza_antes_de_cualquier_publicacion() {
    assert_legacy_is_stale_without_publication(None);
}

#[test]
fn plan_con_version_uno_se_rechaza_antes_de_cualquier_publicacion() {
    assert_legacy_is_stale_without_publication(Some(json!(1)));
}

#[test]
fn expected_occurrences_incorrecto_reporta_el_conteo_del_working() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "AAA\n");
    let app = App::open(dir.path()).unwrap();
    let error = app
        .change_plan(
            None,
            &json!([
                {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB BBB","expectedOccurrences":1},
                {"op":"replace_text","path":"doc.md","find":"BBB","replace":"CCC","expectedOccurrences":1}
            ]),
            policy(),
        )
        .unwrap_err();
    assert_eq!(error.code, ErrorCode::InvalidSchema);
    assert!(
        error
            .message
            .contains("se esperaban 1 coincidencias pero se encontraron 2"),
        "el diagnóstico debe contar sobre working: {error:?}"
    );
}
