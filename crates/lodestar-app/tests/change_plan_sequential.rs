//! Fase roja de la semántica secuencial de `change_plan`.
//!
//! Cada operación cruda debe materializarse contra el `working` inmediatamente anterior,
//! conservando a la vez las revisiones esperadas del snapshot `base`.  Estas pruebas ejercen la
//! frontera pública de `lodestar-app` y, en el último caso, el aplicador puro del core.

use std::collections::BTreeMap;
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
        &format!("---\ntype: Note\ntitle: Doc\n---\n\n{}", body),
    );
}

fn app_with_body(body: &str) -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), body);
    let app = App::open(dir.path()).unwrap();
    (dir, app)
}

fn apply(app: &App, ops: Value) {
    let plan = app
        .change_plan(None, &ops, policy())
        .unwrap_or_else(|e| panic!("la secuencia debía planificar: {e:?}"));
    app.change_apply(&plan.change_set_id, None)
        .unwrap_or_else(|e| panic!("la secuencia debía aplicar: {e:?}"));
}

fn body_on(root: &Path, rel: &str) -> String {
    std::fs::read_to_string(root.join(rel)).unwrap()
}

fn revision(app: &App, path: &str) -> String {
    app.knowledge_get(
        &lodestar_core::types::DocumentRef {
            path: RelPath::new(path).unwrap(),
            id: None,
        },
        &["revision".to_string()],
        None,
    )
    .unwrap()
    .revision
    .0
    .clone()
}

fn replace_bodies(plan: &lodestar_app::PlanResult) -> Vec<String> {
    plan.normalized_operations
        .iter()
        .filter_map(|op| match op {
            NormalizedOperation::ReplaceBody { body, .. } => Some(body.clone()),
            _ => None,
        })
        .collect()
}

fn plan_files(root: &Path) -> Vec<PathBuf> {
    let dir = root.join(".lodestar/runtime/plans");
    std::fs::read_dir(dir)
        .unwrap()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect()
}

#[test]
fn tres_replace_text_independientes_son_acumulativos() {
    let (dir, app) = app_with_body("AAA BBB CCC\n");
    let plan = app
        .change_plan(
            None,
            &json!([
                {"op":"replace_text","path":"doc.md","find":"AAA","replace":"A1"},
                {"op":"replace_text","path":"doc.md","find":"BBB","replace":"B1"},
                {"op":"replace_text","path":"doc.md","find":"CCC","replace":"C1"}
            ]),
            policy(),
        )
        .unwrap();
    let bodies = replace_bodies(&plan);
    assert_eq!(
        bodies.len(),
        3,
        "las tres ops deben normalizarse a ReplaceBody"
    );
    assert!(bodies[0].contains("A1") && bodies[0].contains("BBB"));
    assert!(bodies[1].contains("A1") && bodies[1].contains("B1"));
    assert!(bodies[2].contains("A1") && bodies[2].contains("B1") && bodies[2].contains("C1"));
    app.change_apply(&plan.change_set_id, None).unwrap();
    let final_body = body_on(dir.path(), "doc.md");
    assert!(final_body.contains("A1") && final_body.contains("B1") && final_body.contains("C1"));
    assert!(!final_body.contains("AAA BBB CCC"));
}

#[test]
fn replace_text_aaa_a_bbb_seguido_de_bbb_a_ccc_termina_en_ccc() {
    let (dir, app) = app_with_body("AAA\n");
    apply(
        &app,
        json!([
            {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB"},
            {"op":"replace_text","path":"doc.md","find":"BBB","replace":"CCC"}
        ]),
    );
    let final_body = body_on(dir.path(), "doc.md");
    assert!(final_body.contains("CCC"));
    assert!(!final_body.contains("AAA") && !final_body.contains("BBB"));
}

#[test]
fn expected_occurrences_se_cuenta_sobre_el_working_anterior() {
    let (dir, app) = app_with_body("AAA\n");
    apply(
        &app,
        json!([
            {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB","expectedOccurrences":1},
            {"op":"replace_text","path":"doc.md","find":"BBB","replace":"CCC","expectedOccurrences":1}
        ]),
    );
    assert!(body_on(dir.path(), "doc.md").contains("CCC"));
}

#[test]
fn no_op_es_solo_la_sustitucion_vacia_despues_de_un_cambio_efectivo() {
    let (_dir, app) = app_with_body("AAA\n");
    let plan = app
        .change_plan(
            None,
            &json!([
                {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB"},
                {"op":"replace_text","path":"doc.md","find":"ZZZ","replace":"CCC"}
            ]),
            policy(),
        )
        .unwrap();
    assert_eq!(plan.normalized_operations.len(), 2);
    assert_eq!(plan.no_op_operations.len(), 1);
    assert_eq!(plan.no_op_operations[0].index, 1);
}

#[test]
fn secuencia_net_zero_aaa_bbb_aaa_no_declara_no_ops() {
    let (_dir, app) = app_with_body("AAA\n");
    let plan = app
        .change_plan(
            None,
            &json!([
                {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB"},
                {"op":"replace_text","path":"doc.md","find":"BBB","replace":"AAA"}
            ]),
            policy(),
        )
        .unwrap();
    assert!(plan.semantic_diff.modified.is_empty());
    assert!(plan.semantic_diff.body_changes.is_empty());
    assert!(plan.no_op_operations.is_empty());
}

#[test]
fn dos_ops_con_la_misma_expected_revision_de_base_siguen_validas_y_acumuladas() {
    let (dir, app) = app_with_body("AAA CCC\n");
    let expected = revision(&app, "doc.md");
    let plan = app
        .change_plan(
            None,
            &json!([
                {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB","expectedRevision":expected},
                {"op":"replace_text","path":"doc.md","find":"CCC","replace":"DDD","expectedRevision":expected}
            ]),
            policy(),
        )
        .unwrap();
    let bodies = replace_bodies(&plan);
    assert_eq!(bodies.len(), 2);
    assert!(bodies[1].contains("BBB") && bodies[1].contains("DDD"));
    app.change_apply(&plan.change_set_id, None).unwrap();
    let final_body = body_on(dir.path(), "doc.md");
    assert!(final_body.contains("BBB") && final_body.contains("DDD"));
}

fn assert_content_composition(
    name: &str,
    body: &str,
    ops: Value,
    required: &str,
    forbidden: &[&str],
) {
    let (dir, app) = app_with_body(body);
    let plan = app
        .change_plan(None, &ops, policy())
        .unwrap_or_else(|e| panic!("{name}: las operaciones deben componerse, error {e:?}"));
    app.change_apply(&plan.change_set_id, None).unwrap();
    let final_raw = body_on(dir.path(), "doc.md");
    assert!(
        final_raw.contains(required),
        "{name}: falta {required:?}: {final_raw}"
    );
    for old in forbidden {
        assert!(
            !final_raw.contains(old),
            "{name}: sobrevivió {old:?}: {final_raw}"
        );
    }
}

#[test]
fn replace_body_y_replace_text_componen() {
    assert_content_composition(
        "replace_body_replace_text",
        "# Doc\n\nAAA\n\n## Section\n\nold\n",
        json!([
            {"op":"replace_body","path":"doc.md","body":"# Doc\n\nBBB\n"},
            {"op":"replace_text","path":"doc.md","find":"BBB","replace":"CCC"}
        ]),
        "CCC",
        &["AAA"],
    );
}

#[test]
fn replace_text_y_edit_section_componen() {
    assert_content_composition(
        "replace_text_edit_section",
        "# Doc\n\nAAA\n\n## Section\n\nold\n",
        json!([
            {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB"},
            {"op":"edit_section","path":"doc.md","headingPath":["Section"],"mode":"replace","content":"new"}
        ]),
        "BBB",
        &["old"],
    );
}

#[test]
fn dos_edit_section_componen_sin_perder_la_otra_seccion() {
    assert_content_composition(
        "edit_section_edit_section",
        "# Doc\n\n## First\n\nold-first\n\n## Second\n\nold-second\n",
        json!([
            {"op":"edit_section","path":"doc.md","headingPath":["First"],"mode":"replace","content":"one"},
            {"op":"edit_section","path":"doc.md","headingPath":["Second"],"mode":"replace","content":"two"}
        ]),
        "one",
        &["old-second"],
    );
}

#[test]
fn patch_frontmatter_y_replace_text_componen() {
    assert_content_composition(
        "patch_frontmatter_replace_text",
        "# Doc\n\nAAA\n\n## Section\n\nold\n",
        json!([
            {"op":"patch_frontmatter","path":"doc.md","patch":{"status":"review"}},
            {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB"}
        ]),
        "status: review",
        &["AAA"],
    );
}

#[test]
fn replace_text_y_patch_frontmatter_componen() {
    assert_content_composition(
        "replace_text_patch_frontmatter",
        "# Doc\n\nAAA\n\n## Section\n\nold\n",
        json!([
            {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB"},
            {"op":"patch_frontmatter","path":"doc.md","patch":{"status":"review"}}
        ]),
        "BBB",
        &["AAA"],
    );
}

#[test]
fn create_y_replace_text_ve_el_documento_creado() {
    let (dir, app) = app_with_body("AAA\n");
    let plan = app
        .change_plan(
            None,
            &json!([
                {"op":"create","path":"nuevo.md","body":"# Nuevo\n\nAAA\n"},
                {"op":"replace_text","path":"nuevo.md","find":"AAA","replace":"BBB"}
            ]),
            policy(),
        )
        .unwrap();
    app.change_apply(&plan.change_set_id, None).unwrap();
    assert!(body_on(dir.path(), "nuevo.md").contains("BBB"));
}

#[test]
fn move_y_replace_text_ve_el_destino_recien_movido() {
    let (dir, app) = app_with_body("AAA\n");
    let plan = app
        .change_plan(
            None,
            &json!([
                {"op":"move","from":"doc.md","to":"moved.md"},
                {"op":"replace_text","path":"moved.md","find":"AAA","replace":"BBB"}
            ]),
            policy(),
        )
        .unwrap();
    app.change_apply(&plan.change_set_id, None).unwrap();
    assert!(body_on(dir.path(), "moved.md").contains("BBB"));
    assert!(!dir.path().join("doc.md").exists());
}

#[test]
fn delete_y_replace_text_falla_sin_resucitar_el_origen() {
    let (_dir, app) = app_with_body("AAA\n");
    let err = app
        .change_plan(
            None,
            &json!([
                {"op":"delete","path":"doc.md","inboundLinksPolicy":"remove_links"},
                {"op":"replace_text","path":"doc.md","find":"AAA","replace":"BBB"}
            ]),
            policy(),
        )
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::DocumentNotFound);
}

fn assert_rewrite_after_edit(borrar: bool) {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", "# Index\n");
    write(dir.path(), "target.md", "# Target\n");
    write(dir.path(), "source.md", "# Source\n\n[enlace](target.md)\n");
    let app = App::open(dir.path()).unwrap();
    let op_estructural = if borrar {
        json!({"op":"delete","path":"target.md","inboundLinksPolicy":"remove_links"})
    } else {
        json!({"op":"move","from":"target.md","to":"renamed.md","rewriteInboundLinks":true})
    };
    let plan = app
        .change_plan(
            None,
            &json!([
                {"op":"replace_text","path":"source.md","find":"Source","replace":"Edited"},
                op_estructural
            ]),
            policy(),
        )
        .unwrap();
    app.change_apply(&plan.change_set_id, None).unwrap();
    let source = body_on(dir.path(), "source.md");
    assert!(
        source.contains("Edited"),
        "la edición previa no debe perderse: {source}"
    );
    if borrar {
        assert!(!source.contains("target.md"));
    } else {
        assert!(source.contains("renamed.md"));
    }
}

#[test]
fn move_despues_de_editar_reescribe_enlaces_desde_el_working() {
    assert_rewrite_after_edit(false);
}

#[test]
fn delete_despues_de_editar_reescribe_enlaces_desde_el_working() {
    assert_rewrite_after_edit(true);
}

#[test]
fn selection_remove_links_elimina_dos_targets_y_captura_revisiones_base() {
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
    let a_rev = revision(&app, "a.md");
    let b_rev = revision(&app, "b.md");
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
    assert_eq!(
        plan.captured_revisions[&RelPath::new("a.md").unwrap()].0,
        a_rev
    );
    assert_eq!(
        plan.captured_revisions[&RelPath::new("b.md").unwrap()].0,
        b_rev
    );
    let source_rewrites: Vec<String> = replace_bodies(&plan)
        .into_iter()
        .filter(|body| body.starts_with("# Source"))
        .collect();
    assert!(
        !source_rewrites.is_empty(),
        "la selección debe producir una reescritura del origen"
    );
    assert!(
        source_rewrites
            .iter()
            .any(|body| !body.contains("a.md") && !body.contains("b.md")),
        "las dos eliminaciones deben componerse contra el mismo working: {source_rewrites:?}"
    );
}

#[test]
fn plan_y_apply_en_procesos_distintos_persisten_tres_sustituciones() {
    let dir = tempfile::tempdir().unwrap();
    seed(dir.path(), "AAA BBB CCC\n");
    let plan = {
        let app = App::open(dir.path()).unwrap();
        app.change_plan(
            None,
            &json!([
                {"op":"replace_text","path":"doc.md","find":"AAA","replace":"A1"},
                {"op":"replace_text","path":"doc.md","find":"BBB","replace":"B1"},
                {"op":"replace_text","path":"doc.md","find":"CCC","replace":"C1"}
            ]),
            policy(),
        )
        .unwrap()
    };
    let app_2 = App::open(dir.path()).unwrap();
    app_2.change_apply(&plan.change_set_id, None).unwrap();
    let final_raw = body_on(dir.path(), "doc.md");
    assert!(final_raw.contains("A1") && final_raw.contains("B1") && final_raw.contains("C1"));
}

fn stale_version_case(version: Option<Value>) {
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
    let path = &files[0];
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    if let Some(version) = version {
        value["plannerSemanticsVersion"] = version;
    } else {
        value
            .as_object_mut()
            .unwrap()
            .remove("plannerSemanticsVersion");
    }
    std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    let before = body_on(dir.path(), "doc.md");
    let err = app.change_apply(&plan.change_set_id, None).unwrap_err();
    assert_eq!(err.code, ErrorCode::PlanStale);
    assert_eq!(body_on(dir.path(), "doc.md"), before);
}

#[test]
fn plan_sin_planner_semantics_version_se_rechaza_stale_sin_escrituras() {
    stale_version_case(None);
}

#[test]
fn plan_con_version_legacy_se_rechaza_stale_sin_escrituras() {
    stale_version_case(Some(json!(1)));
}

#[test]
fn runtime_plan_lleva_version_interna_y_planresult_no_la_expone() {
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
    let public: Value = serde_json::to_value(&plan).unwrap();
    assert!(
        public.get("plannerSemanticsVersion").is_none(),
        "la versión es interna y no debe aparecer en PlanResult/MCP: {public}"
    );
    let files = plan_files(dir.path());
    assert_eq!(
        files.len(),
        1,
        "precondición: debe existir un plan runtime persistido"
    );
    let runtime: Value =
        serde_json::from_str(&std::fs::read_to_string(&files[0]).unwrap()).unwrap();
    assert_eq!(
        runtime.get("plannerSemanticsVersion"),
        Some(&json!(plan::PLAN_SEMANTICS_VERSION)),
        "el plan runtime nuevo debe fijar la semántica interna vigente: {runtime}"
    );
}

#[test]
fn plan_actual_sin_no_op_operations_sigue_deserializando_y_aplicando() {
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
    let path = &files[0];
    let mut value: Value = serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap();
    value.as_object_mut().unwrap().remove("noOpOperations");
    std::fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
    app.change_apply(&plan.change_set_id, None).unwrap();
    assert!(body_on(dir.path(), "doc.md").contains("BBB"));
}

#[test]
fn rebase_conserva_other_files_y_no_emite_link_target_missing() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "index.md", "# Index\n");
    write(
        dir.path(),
        "README.md",
        "# README\n\nImplementado en [código](src/main.rs).\n",
    );
    write(dir.path(), "src/main.rs", "fn main() {}\n");
    let app = App::open(dir.path()).unwrap();
    let before = app
        .change_plan(
            None,
            &json!([{"op":"replace_text","path":"README.md","find":"README","replace":"Guide"}]),
            policy(),
        )
        .unwrap();
    assert_eq!(before.diagnostics_before.warnings, 0);
    assert_eq!(before.diagnostics_after.warnings, 0);
    app.change_apply(&before.change_set_id, None).unwrap();
    assert!(body_on(dir.path(), "README.md").contains("Guide"));
}

#[test]
fn aplicador_unitario_detecta_noops_moves_y_fold_secuencial() {
    let a = RelPath::new("a.md").unwrap();
    let b = RelPath::new("b.md").unwrap();
    let mut files: FileMap = BTreeMap::new();
    files.insert(a.clone(), "old\n".to_string());
    let noop = NormalizedOperation::ReplaceBody {
        path: a.clone(),
        body: "old\n".to_string(),
    };
    let unchanged = plan::apply_normalized_ops(&files, std::slice::from_ref(&noop)).unwrap();
    assert_eq!(unchanged, files, "ReplaceBody idéntico debe ser no-op");

    let moved = NormalizedOperation::Move {
        from: a.clone(),
        to: b.clone(),
        rewrite_inbound_links: false,
    };
    let same_path = NormalizedOperation::Move {
        from: b.clone(),
        to: b.clone(),
        rewrite_inbound_links: false,
    };
    let folded = plan::apply_normalized_ops(&files, &[moved.clone(), same_path.clone()]).unwrap();
    let once = plan::apply_normalized_ops(&files, std::slice::from_ref(&moved)).unwrap();
    let twice = plan::apply_normalized_ops(&once, std::slice::from_ref(&same_path)).unwrap();
    assert_eq!(
        folded, twice,
        "el fold debe equivaler a aplicar una por una"
    );
    assert_eq!(folded.get(&b).map(String::as_str), Some("old\n"));
    assert!(!folded.contains_key(&a));
}
