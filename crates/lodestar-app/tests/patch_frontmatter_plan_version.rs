use std::path::Path;

use lodestar_app::App;
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::ErrorCode;
use serde_json::{json, Value};

fn write(root: &Path, relative: &str, content: &str) {
    std::fs::write(root.join(relative), content).expect("escribir fixture Markdown");
}

fn policy() -> PlanPolicy {
    PlanPolicy {
        require_valid_result: false,
        allow_warnings: true,
    }
}

#[test]
fn change_apply_rechaza_plan_legacy_de_patch_anidado_sin_tocar_el_canonico() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", "# Index\n");
    let original = b"---\ntype: Note\nnested:\n  keep: survives\n  target: old\n  deeper:\n    keep: survives-too\n    target: old-deep\nother: untouched\n---\n\n# Body\n";
    write(
        dir.path(),
        "doc.md",
        std::str::from_utf8(original).expect("fixture UTF-8"),
    );
    let app = App::open(dir.path()).unwrap();

    let plan = app
        .change_plan(
            None,
            &json!([
                {
                    "op": "patch_frontmatter",
                    "path": "doc.md",
                    "patch": {
                        "nested": {
                            "target": "new",
                            "deeper": {"target": "new-deep"}
                        }
                    }
                }
            ]),
            policy(),
        )
        .expect("el patch RFC 7386 anidado debe planificar");

    assert_eq!(plan.normalized_operations.len(), 1);
    assert!(
        plan.no_op_operations.is_empty(),
        "el patch debe cambiar el documento y no ser un caso vacuo"
    );
    assert_eq!(plan.semantic_diff.modified.len(), 1);
    assert_eq!(plan.semantic_diff.modified[0].as_str(), "doc.md");

    let plans_dir = dir.path().join(".lodestar/runtime/plans");
    let plan_files: Vec<_> = std::fs::read_dir(&plans_dir)
        .expect("change_plan debe persistir el plan runtime")
        .map(|entry| entry.expect("entrada de plan legible").path())
        .filter(|path| path.extension().and_then(|ext| ext.to_str()) == Some("json"))
        .collect();
    assert_eq!(
        plan_files.len(),
        1,
        "debe existir exactamente un plan persistido: {plan_files:?}"
    );

    let plan_path = &plan_files[0];
    let mut persisted: Value =
        serde_json::from_slice(&std::fs::read(plan_path).expect("leer plan persistido"))
            .expect("el plan persistido debe ser JSON");
    assert!(
        persisted.get("plannerSemanticsVersion").is_some(),
        "el plan persistido debe declarar su versión de semántica"
    );
    persisted["plannerSemanticsVersion"] = json!(2);
    std::fs::write(
        plan_path,
        serde_json::to_vec_pretty(&persisted).expect("serializar plan legacy"),
    )
    .expect("forjar versión legacy solo en el plan runtime");

    let error = app
        .change_apply(&plan.change_set_id, None)
        .expect_err("un plan con semántica anterior debe rechazarse antes de escribir");
    assert_eq!(error.code, ErrorCode::PlanStale);
    assert_eq!(
        std::fs::read(dir.path().join("doc.md")).expect("leer documento canónico"),
        original,
        "PLAN_STALE no debe modificar el documento canónico ni un byte"
    );
}
