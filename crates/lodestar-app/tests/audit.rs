//! Tests de integración de E13-H10: auditoría local `.lodestar/runtime/audit.jsonl`.
//!
//! `App::change_apply` (E13-H08) YA orquesta y publica la transacción, pero NO deja rastro de
//! auditoría. Esta historia añade que, por cada operación de escritura (`change_apply` /
//! `change_revert`), se **anexe una línea JSON** al fichero de auditoría runtime con la forma:
//!
//! ```jsonc
//! { "timestamp", "client", "tool", "changeSetId",
//!   "baseRevision", "resultRevision", "paths", "result" }
//! ```
//!
//! La auditoría es **runtime** (gitignored, FUERA de `WorkspaceRevision`, no indexada — vive bajo
//! `.lodestar/runtime/`, igual que `plans/`/`receipts/`) y es un registro **local** (no se expone
//! por tool MCP). Es JSONL: una línea JSON por evento, anexada; el fichero crece, no se reescribe.
//!
//! ## Dónde asumo que se escribe la auditoría y qué forma tiene la línea
//! - **Ruta**: `.lodestar/runtime/audit.jsonl` (constante [`AUDIT_REL`]).
//! - **Clave de operación** `tool`: el nombre de la operación de escritura; para un apply, la cadena
//!   `"change_apply"` (misma etiqueta que la tool MCP).
//! - **Clave `changeSetId`**: referencia el change set aplicado; se asevera de forma robusta (contiene
//!   el hash desnudo del `ChangeSetId`, sin fijar si conserva o sanea el prefijo `changeset:`).
//! - **Clave `result`**: `"success"` en éxito; en fallo, cualquier valor distinto de `"success"`
//!   (p. ej. `"error"` o el código wire del `ErrorCode`) — el test no fija el literal exacto del
//!   fallo, solo que registra un fallo.
//! - **Claves `baseRevision`/`resultRevision`**: las [`WorkspaceRevision`] antes/después del apply
//!   (`"blake3:…"`); en éxito deben coincidir con `previous`/`result` del [`ApplyResult`].
//!
//! ## Fase ROJA (documentada)
//! `change_apply` YA existe y compila, así que estos tests **compilan**; el rojo es de aserción
//! (regla 2): tras un apply, `.lodestar/runtime/audit.jsonl` **no existe** todavía (nadie lo
//! escribe), de modo que la lectura del fichero falla y el test hace panic con el mensaje de
//! «el fichero de auditoría debe existir». Cuando E13-H10 implemente la anexión, la línea aparecerá
//! y las aserciones de contenido (`result`, `tool`, `changeSetId`, revisiones) fijarán el contrato.
//!
//! NO se toca producción: la implementación de la auditoría es trabajo del implementador.

use std::path::{Path, PathBuf};

use lodestar_app::App;
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::{ChangeSetId, ErrorCode, WorkspaceRevision};

/// Ruta relativa (dentro del bundle) del fichero de auditoría runtime.
const AUDIT_REL: [&str; 3] = [".lodestar", "runtime", "audit.jsonl"];

/// Escribe un `.md` (creando los directorios intermedios) dentro del bundle temporal.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Monta un `App` sobre un bundle temporal con un index raíz + un concept conforme (`alfa.md`).
/// El `TempDir` se devuelve para mantener el directorio vivo mientras dure el test.
fn app_con_bundle() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Alfa](alfa.md)\n",
    );
    escribe(
        dir.path(),
        "alfa.md",
        "---\ntype: Concept\ntitle: Alfa\ndescription: Primer concept\n---\n\n# Resumen\n\ncuerpo\n",
    );
    let app = App::open(dir.path()).expect("el bundle temporal debe abrir");
    (dir, app)
}

/// Una propuesta mínima pero real: un `patch_frontmatter` inocuo sobre `alfa.md`. Basta para que
/// `change_plan` produzca un plan aplicable con una operación normalizada.
fn una_operacion() -> serde_json::Value {
    serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "alfa.md" },
          "patch": { "description": "alfa actualizada por el apply" } },
    ])
}

/// Política permisiva: no exige resultado conforme y admite warnings, para que el plan sea
/// siempre aplicable (el criterio no depende del veredicto de conformidad).
fn policy_permisiva() -> PlanPolicy {
    PlanPolicy {
        require_conformant_result: false,
        allow_warnings: true,
    }
}

/// El hash desnudo (sin el prefijo `changeset:`) de un `ChangeSetId`.
fn hash_desnudo(id: &ChangeSetId) -> String {
    id.0.strip_prefix("changeset:").unwrap_or(&id.0).to_string()
}

/// Ruta absoluta del fichero de auditoría del bundle.
fn ruta_audit(root: &Path) -> PathBuf {
    let mut p = root.to_path_buf();
    for seg in AUDIT_REL {
        p.push(seg);
    }
    p
}

/// Lee `.lodestar/runtime/audit.jsonl` y devuelve una línea JSON parseada por cada evento. Hace
/// panic (rojo esperado en fase ROJA) si el fichero no existe. Cada línea no vacía debe ser JSON.
fn lineas_audit(root: &Path) -> Vec<serde_json::Value> {
    let ruta = ruta_audit(root);
    let contenido = std::fs::read_to_string(&ruta).unwrap_or_else(|e| {
        panic!(
            "el fichero de auditoría {} debe existir tras una operación de escritura: {e}",
            ruta.display()
        )
    });
    contenido
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| {
            serde_json::from_str(l).unwrap_or_else(|e| {
                panic!("cada línea de auditoría debe ser JSON válido: {e}\n{l}")
            })
        })
        .collect()
}

/// El valor de una clave string de una línea de auditoría, o `""`.
fn campo<'a>(linea: &'a serde_json::Value, clave: &str) -> &'a str {
    linea
        .get(clave)
        .and_then(serde_json::Value::as_str)
        .unwrap_or("")
}

/// Localiza LA línea de auditoría del apply de `change_set_id`: `tool == "change_apply"` y su
/// `changeSetId` referencia el change set (contiene el hash desnudo).
fn linea_del_apply<'a>(lineas: &'a [serde_json::Value], id: &ChangeSetId) -> &'a serde_json::Value {
    let hash = hash_desnudo(id);
    lineas
        .iter()
        .find(|l| campo(l, "tool") == "change_apply" && campo(l, "changeSetId").contains(&hash))
        .unwrap_or_else(|| {
            panic!(
                "debe existir una línea de auditoría del `change_apply` para {id:?}; líneas: {lineas:?}"
            )
        })
}

/// `audit_registra_apply` — Dado un `change_apply` exitoso, Cuando termina, Entonces
/// `.lodestar/runtime/audit.jsonl` tiene una línea con `result:"success"`, el `tool`
/// (`change_apply`), el `changeSetId` y las revisiones base/resultado coherentes con el receipt.
#[test]
fn audit_registra_apply() {
    let (dir, app) = app_con_bundle();
    let plan = app
        .change_plan(None, &una_operacion(), policy_permisiva())
        .expect("el `change_plan` debe tener éxito y producir un plan");

    // Apply exitoso: sin control optimista (None), la base coincide y publica.
    let apply = app
        .change_apply(&plan.change_set_id, None)
        .expect("el `change_apply` debe tener éxito y publicar la transacción");

    // La línea de auditoría del apply existe y registra el éxito.
    let lineas = lineas_audit(dir.path());
    let linea = linea_del_apply(&lineas, &plan.change_set_id);

    assert_eq!(
        campo(linea, "result"),
        "success",
        "un apply exitoso debe registrarse con `result:\"success\"`: {linea}",
    );

    // Revisiones coherentes con el `ApplyResult`: base = previa, resultado = revisión final.
    assert_eq!(
        campo(linea, "baseRevision"),
        apply.previous_workspace_revision.0,
        "`baseRevision` de la auditoría debe ser la revisión previa del apply: {linea}",
    );
    assert_eq!(
        campo(linea, "resultRevision"),
        apply.workspace_revision.0,
        "`resultRevision` de la auditoría debe ser la revisión resultante del apply: {linea}",
    );
}

/// `audit_registra_fallo` — Dado un apply que falla por conflicto, Cuando se procesa, Entonces la
/// línea de auditoría registra el fallo (`result` != `"success"`).
///
/// Cómo se provoca el fallo: se planifica con éxito y luego se llama a `change_apply` con un
/// `expected_workspace_revision` deliberadamente falso (`blake3:0000…`, distinto de la base real).
/// El control optimista de workspace (paso 2 de `change_apply`) detecta la discrepancia y devuelve
/// `Err(RevisionConflict)` sin publicar. La auditoría debe registrar ese intento con un `result`
/// distinto de `"success"` (el `result` es precisamente el campo que distingue éxito de fallo en el
/// registro; un audit trail cubre también los intentos rechazados).
#[test]
fn audit_registra_fallo() {
    let (dir, app) = app_con_bundle();
    let plan = app
        .change_plan(None, &una_operacion(), policy_permisiva())
        .expect("el `change_plan` debe tener éxito y producir un plan");

    // Revisión esperada falsa (distinta de la base real) → conflicto de revisión.
    let revision_falsa = WorkspaceRevision(format!("blake3:{}", "0".repeat(64)));
    assert_ne!(
        revision_falsa, plan.base_workspace_revision,
        "la revisión falsa debe diferir de la base real para forzar el conflicto",
    );

    let err = app
        .change_apply(&plan.change_set_id, Some(revision_falsa))
        .expect_err("el `change_apply` con revisión esperada falsa debe fallar por conflicto");
    assert!(
        matches!(err, ErrorCode::RevisionConflict),
        "el fallo debe ser un conflicto de revisión, fue {err:?}",
    );

    // La línea de auditoría del apply existe y registra el fallo.
    let lineas = lineas_audit(dir.path());
    let linea = linea_del_apply(&lineas, &plan.change_set_id);

    assert_ne!(
        campo(linea, "result"),
        "success",
        "un apply que falla por conflicto debe registrarse con `result` != \"success\": {linea}",
    );
}
