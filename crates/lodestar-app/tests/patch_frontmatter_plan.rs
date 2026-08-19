use std::path::Path;

use lodestar_app::App;
use lodestar_core::plan::PlanPolicy;
use serde_json::json;

fn write(root: &Path, relative: &str, content: &str) {
    let path = root.join(relative);
    std::fs::write(path, content).expect("escribir fixture Markdown");
}

fn policy() -> PlanPolicy {
    PlanPolicy {
        require_valid_result: false,
        allow_warnings: true,
    }
}

#[test]
fn patch_sin_efecto_anidado_es_no_op_y_no_publica_escritura() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", "# Index\n");
    write(
        dir.path(),
        "doc.md",
        "---\ntype: Note\nnested:\n  keep: same\n  other: survives\n---\n\n# Body\n",
    );
    let before = std::fs::read_to_string(dir.path().join("doc.md")).unwrap();
    let app = App::open(dir.path()).unwrap();

    let plan = app
        .change_plan(
            None,
            &json!([
                {
                    "op": "patch_frontmatter",
                    "path": "doc.md",
                    "patch": {"nested": {"keep": "same"}}
                }
            ]),
            policy(),
        )
        .expect("un patch semánticamente vacío debe planificar");

    assert_eq!(
        plan.normalized_operations.len(),
        1,
        "la operación debe seguir visible aunque sea no-op"
    );
    assert_eq!(
        plan.no_op_operations.len(),
        1,
        "el no-op debe quedar señalado"
    );
    assert_eq!(plan.no_op_operations[0].index, 0);
    assert_eq!(plan.no_op_operations[0].op, "patch_frontmatter");
    assert!(
        plan.semantic_diff.modified.is_empty(),
        "un merge-patch idéntico no debe inventar un cambio semántico: {:?}",
        plan.semantic_diff
    );

    let applied = app
        .change_apply(&plan.change_set_id, None)
        .expect("el apply de un plan no-op debe completarse");
    assert!(
        applied.changed_paths.is_empty(),
        "un patch sin efecto no debe publicar rutas: {:?}",
        applied.changed_paths
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("doc.md")).unwrap(),
        before,
        "el documento canónico debe quedar byte a byte intacto"
    );
}
