//! **E21-H02** — Selecciones masivas por consulta (`ARCHITECTURE.md §20.11`,
//! `REFACTOR_PHASE_2 §Fase 12 (Operaciones masivas basadas en consulta)`). Fase ROJA.
//!
//! ## La forma del wire que fijan estos tests
//!
//! `change_plan` gana una forma de **selección** además del array de operaciones sueltas. Cuando el
//! valor de operaciones es un OBJETO
//!
//! ```json
//! { "selection": { "where": "<consulta E19>" | "filter": { … } },
//!   "operation":  { "<op universal>": { <parámetros> } } }
//! ```
//!
//! la consulta E19 (`§Fase 5`) selecciona documentos y la `operation` se expande a **una
//! `NormalizedOperation` por documento seleccionado** (el array suelto `[ {op…}, … ]` sigue
//! valiendo tal cual). La `operation` codifica el tipo como CLAVE (`{patch_frontmatter: {…}}`),
//! según el ejemplo de la historia; solo las ops con sentido en masa (`patch_frontmatter`,
//! `replace_text`, `delete`, `apply_fix`) — `create` no aplica a documentos existentes, así que
//! estos tests solo ejercen `patch_frontmatter`. El valor de `patch_frontmatter` es el merge-patch
//! (RFC 7386) que se aplica a cada documento que casa.
//!
//! ## Por qué son ROJOS hoy
//!
//! `App::change_plan` exige que las operaciones sean un ARRAY (`raw_ops.as_array()`), así que un
//! objeto `{selection, operation}` se rechaza hoy con `Err(InvalidSchema)`. Los tres tests fallan
//! por esa razón (el `.expect()` sobre `change_plan` entra en pánico) hasta que E21-H02 enseñe a
//! `change_plan` a interpretar la selección.
//!
//! `seleccion_captura_revisiones` fija además una clave de wire NUEVA en el plan: `capturedRevisions`
//! (objeto `path → "blake3:…"`), donde el plan registra la revisión de cada documento seleccionado
//! (`§Fase 12`: query → documentos → **snapshot de revisiones** → … → change plan). Se observa
//! serializando el `PlanResult` a JSON, de modo que el test no depende de un símbolo Rust concreto
//! del struct — pero SÍ fija que esa clave debe existir y su forma.

use std::path::Path;

use lodestar_app::App;
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::DocumentRef;
use lodestar_core::types::RelPath;
use serde_json::{json, Value};

/// Escribe un `.md` dentro del workspace temporal, creando los directorios intermedios.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Política permisiva: el criterio a probar es la EXPANSIÓN de la selección, no el veredicto de
/// conformidad — sin esto un plan podría fallar por una razón distinta de la que se fija.
fn policy_permisiva() -> PlanPolicy {
    PlanPolicy {
        require_conformant_result: false,
        allow_warnings: true,
    }
}

/// Referencia a un documento por su path (identidad v2, `id: None`).
fn doc_ref(path: &str) -> DocumentRef {
    DocumentRef {
        path: RelPath::new(path).unwrap(),
        id: None,
    }
}

/// Workspace con 5 documentos donde la consulta `type = "decision" and status = "draft"` casa
/// EXACTAMENTE dos (`d1.md`, `d2.md`): el resto queda fuera por `type` (`index.md`, `n1.md`) o por
/// `status` (`d3.md`), de modo que la selección no puede colar ni saltarse ninguno.
fn app_con_decisiones() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Índice\n---\n\n# Índice\n",
    );
    escribe(
        dir.path(),
        "d1.md",
        "---\ntype: decision\nstatus: draft\ntitle: D1\n---\n\n# D1\n",
    );
    escribe(
        dir.path(),
        "d2.md",
        "---\ntype: decision\nstatus: draft\ntitle: D2\n---\n\n# D2\n",
    );
    escribe(
        dir.path(),
        "d3.md",
        "---\ntype: decision\nstatus: accepted\ntitle: D3\n---\n\n# D3\n",
    );
    escribe(
        dir.path(),
        "n1.md",
        "---\ntype: note\nstatus: draft\ntitle: N1\n---\n\n# N1\n",
    );
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");
    (dir, app)
}

/// El wire de una selección masiva `patch_frontmatter` con la consulta `where` dada.
fn seleccion_patch(where_expr: &str) -> Value {
    json!({
        "selection": { "where": where_expr },
        "operation": { "patch_frontmatter": { "status": "review" } }
    })
}

/// `seleccion_masiva_patch` — **Dado** `{selection:{where:"type = \"decision\" and status =
/// \"draft\""}, operation:{patch_frontmatter:{status:"review"}}}`, **Cuando** se planifica,
/// **Entonces** el plan tiene una op por documento que casa la consulta (d1, d2), cada una un
/// `patch_frontmatter` que fija `status: review`.
#[test]
fn seleccion_masiva_patch() {
    let (_dir, app) = app_con_decisiones();

    let plan = app
        .change_plan(
            None,
            &seleccion_patch("type = \"decision\" and status = \"draft\""),
            policy_permisiva(),
        )
        .expect("una selección masiva válida debe producir un plan");

    // Se observa la forma normalizada por su serialización JSON (evita depender de `serde_yaml` en
    // el binario de test): una op por documento que casa, y ninguna más.
    let ops = serde_json::to_value(&plan.normalized_operations).unwrap();
    let arr = ops
        .as_array()
        .unwrap_or_else(|| panic!("normalizedOperations debe ser un array: {ops}"));
    assert_eq!(
        arr.len(),
        2,
        "la selección debe expandirse a UNA op por documento que casa (d1, d2), no {}: {ops}",
        arr.len(),
    );

    let mut paths: Vec<String> = Vec::new();
    for op in arr {
        assert_eq!(
            op["op"], "patch_frontmatter",
            "cada op de la selección debe ser un `patch_frontmatter`: {op}",
        );
        // DISCRIMINADOR: el patch fija de verdad `status: review` (una expansión vacua que no
        // llevara el patch pasaría el conteo pero fallaría aquí).
        assert_eq!(
            op["patch"]["status"], "review",
            "cada op debe aplicar el patch `status: review` de la selección: {op}",
        );
        paths.push(
            op["path"]
                .as_str()
                .unwrap_or_else(|| panic!("la op debe llevar `path`: {op}"))
                .to_string(),
        );
    }
    paths.sort();
    assert_eq!(
        paths,
        vec!["d1.md".to_string(), "d2.md".to_string()],
        "la selección debe expandirse EXACTAMENTE sobre los documentos que casan (d1, d2)",
    );
}

/// `seleccion_vacia` — **Dado** una selección que no casa ningún documento, **Cuando** se
/// planifica, **Entonces** el plan es vacío (sin cambios), SIN error.
#[test]
fn seleccion_vacia() {
    let (_dir, app) = app_con_decisiones();

    let plan = app
        .change_plan(
            None,
            // Ningún documento tiene `type = "inexistente"`.
            &seleccion_patch("type = \"inexistente\""),
            policy_permisiva(),
        )
        .expect("una selección que no casa nada debe planificar SIN error (plan vacío)");

    assert!(
        plan.normalized_operations.is_empty(),
        "una selección vacía debe dar un plan sin operaciones: {:?}",
        plan.normalized_operations,
    );
    assert_eq!(
        plan.impact.affected_count, 0,
        "una selección vacía no afecta a ningún documento: {:?}",
        plan.impact,
    );
}

/// `seleccion_captura_revisiones` — **Dado** una selección masiva, **Cuando** se planifica,
/// **Entonces** cada documento del plan lleva su `DocumentRevision` capturada.
///
/// La captura se fija como una clave de wire del plan: `capturedRevisions`, un objeto `path →
/// "blake3:…"` con una entrada por documento seleccionado, igual a su revisión ACTUAL en disco (la
/// misma que reporta `knowledge_get`). Se lee de la serialización JSON del `PlanResult`.
#[test]
fn seleccion_captura_revisiones() {
    let (_dir, app) = app_con_decisiones();

    let plan = app
        .change_plan(
            None,
            &seleccion_patch("type = \"decision\" and status = \"draft\""),
            policy_permisiva(),
        )
        .expect("una selección masiva válida debe producir un plan");

    let plan_json = serde_json::to_value(&plan).unwrap();
    let captured = plan_json
        .get("capturedRevisions")
        .and_then(Value::as_object)
        .unwrap_or_else(|| {
            panic!(
                "el plan de una selección masiva debe llevar `capturedRevisions` (objeto \
                 path→revisión): {plan_json}"
            )
        });

    for p in ["d1.md", "d2.md"] {
        let esperada = app
            .knowledge_get(&doc_ref(p), &[], None)
            .expect("el documento seleccionado debe existir")
            .revision;
        let capturada = captured.get(p).and_then(Value::as_str).unwrap_or_else(|| {
            panic!("`capturedRevisions` debe tener entrada para {p}: {plan_json}")
        });
        assert_eq!(
            capturada, esperada.0,
            "la revisión capturada de {p} debe ser su DocumentRevision actual",
        );
    }
    assert_eq!(
        captured.len(),
        2,
        "`capturedRevisions` debe tener exactamente una entrada por documento seleccionado (d1, d2): \
         {plan_json}",
    );
}
