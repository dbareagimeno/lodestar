//! Análisis de un plan (`ChangeSet`): riesgo (E12-H02) y diff semántico (E12-H03).
//!
//! Función pura [`assess_risk`]: dado un conjunto de [`NormalizedOperation`] ya resueltas y el
//! `Bundle` anterior al cambio, deriva un [`RiskAssessment`] con razones en español. No hace I/O;
//! toda la verdad la da el core (`Bundle::backlinks`, invariante #3 de `CLAUDE.md`).
//!
//! Función pura [`semantic_diff`]: dado el `Bundle` antes/después de un plan y el `Schema`,
//! deriva un [`SemanticDiff`] reusando [`crate::diff::diff_snap`] y las validaciones de esquema
//! (`validate_schema`/`validate_relations`) — ver la doc de la función para el detalle.
//!
//! ## Heurística (documentada, no normativa — H02 solo fija el orden de magnitud)
//!
//! Para cada operación que **encoge** el grafo de conceptos — deprecar (`TransitionStatus{to:
//! "deprecated"}` o un `PatchFrontmatter` cuyo patch pone `status: deprecated`), borrar
//! (`Delete`) o mover (`Move`) — se mide su *blast radius*: los backlinks entrantes
//! (`Bundle::backlinks(&path).inbound`) que el concepto afectado tenía en el bundle **antes**
//! del cambio (después de deprecar/borrar/mover, esos backlinks quedan apuntando a algo
//! deprecado, roto o movido). Umbral:
//!
//! - `0` backlinks → sin factor de riesgo (no añade razón ni sube el nivel).
//! - `1..=4` backlinks → factor `Medium`.
//! - `>=5` backlinks → factor `High`.
//!
//! El `level` final es el máximo de los factores detectados entre todas las operaciones; sin
//! factores, `Low` con `reasons` vacío. Operaciones que no encogen nada (p. ej. un
//! `patch_frontmatter` que no toca `status` a `deprecated`, o cualquier operación sobre un
//! concepto sin backlinks) no generan factor de riesgo — de ahí que un cambio aislado sea
//! siempre `Low`.
//!
//! `bundle_after` se recibe por coherencia de firma con el resto del pipeline de plan (E12-H03
//! `SemanticDiff`, E12-H04 `ValidationReport` sí lo necesitan); esta heurística de riesgo solo
//! necesita el bundle *antes* del cambio para medir backlinks del concepto afectado.

use std::collections::BTreeSet;

use crate::diff::{diff_snap, BodyHunk, ChangeKind};
use crate::schema::{validate_relations, validate_schema, Schema};
use crate::types::{
    Check, CheckCode, FrontmatterPatch, NormalizedOperation, RelPath, RiskAssessment, RiskLevel,
    SemanticDiff, Severity,
};
use crate::Bundle;

/// A partir de este número de backlinks entrantes, el factor de riesgo es `High` (por debajo,
/// `Medium`, siempre que haya al menos uno).
const HIGH_BACKLINKS_THRESHOLD: usize = 5;

/// Evalúa el riesgo de aplicar `ops` sobre `bundle_before`. Ver heurística en el módulo.
pub fn assess_risk(
    ops: &[NormalizedOperation],
    bundle_before: &Bundle,
    _bundle_after: &Bundle,
) -> RiskAssessment {
    let mut level = RiskLevel::Low;
    let mut reasons = Vec::new();

    for op in ops {
        let Some(path) = shrinking_path(op) else {
            continue;
        };
        let backlinks = bundle_before.backlinks(&path).inbound.len();
        if backlinks == 0 {
            continue;
        }

        let factor_level = if backlinks >= HIGH_BACKLINKS_THRESHOLD {
            RiskLevel::High
        } else {
            RiskLevel::Medium
        };
        if factor_level > level {
            level = factor_level;
        }

        reasons.push(format!(
            "{} «{}» afecta a {} concepto{} que lo referencian.",
            accion(op),
            path.as_str(),
            backlinks,
            if backlinks == 1 { "" } else { "s" },
        ));
    }

    RiskAssessment { level, reasons }
}

/// Si `op` "encoge" el grafo de conceptos (deprecar/borrar/mover), devuelve el path del concepto
/// cuyos backlinks entrantes hay que medir. `None` para el resto de operaciones (no generan
/// factor de riesgo bajo esta heurística).
fn shrinking_path(op: &NormalizedOperation) -> Option<RelPath> {
    match op {
        NormalizedOperation::TransitionStatus { path, to } if to == "deprecated" => {
            Some(path.clone())
        }
        NormalizedOperation::Delete { path, .. } => Some(path.clone()),
        NormalizedOperation::Move { from, .. } => Some(from.clone()),
        NormalizedOperation::PatchFrontmatter { path, patch }
            if patches_status_to_deprecated(patch) =>
        {
            Some(path.clone())
        }
        _ => None,
    }
}

/// `true` si el patch de frontmatter pone `status: deprecated` explícitamente.
fn patches_status_to_deprecated(patch: &FrontmatterPatch) -> bool {
    matches!(
        patch.0.get("status"),
        Some(Some(serde_yaml::Value::String(s))) if s == "deprecated"
    )
}

/// Verbo español que describe la operación, para la razón legible.
fn accion(op: &NormalizedOperation) -> &'static str {
    match op {
        NormalizedOperation::Delete { .. } => "Borrar",
        NormalizedOperation::Move { .. } => "Mover",
        _ => "Deprecar",
    }
}

// --- E12-H03: `SemanticDiff` --------------------------------------------------------------

/// Diff semántico entre `before` y `after` — E12-H03.
///
/// `created`/`modified`/`deleted`/`frontmatter_changes`/`body_changes` reusan
/// [`crate::diff::diff_snap`] (la única verdad de diff del core, invariante #3 de `CLAUDE.md`):
/// cada [`crate::diff::FileDiff`] se clasifica por su `kind`, y `frontmatter_changes`/
/// `body_changes` marcan los paths cuyo `FileDiff` trae cambios de frontmatter (`fm` no vacío) o
/// de cuerpo (algún `BodyHunk::Add`/`Remove`, no solo contexto). `moved` queda siempre vacío:
/// `diff_snap` no hace detección de renombres (un `Move` se ve como `Remove` + `Add`), y no hay
/// heurística de renombre en el core que reusar sin inventar semántica nueva — fuera del alcance
/// de esta historia. `relation_changes` se deriva de `links_added`/`links_removed` de cada
/// `FileDiff` (los out-links textuales que ya computa `diff_snap`): es la aproximación más
/// cercana a "cambió una relación" sin reimplementar la resolución de relaciones del `schema`.
///
/// `diagnostics_introduced`/`diagnostics_resolved` comparan el conjunto COMPLETO de diagnósticos
/// de cada bundle bajo `schema`: los 15 checks OKF (`Bundle::analyze().per_file`) más
/// `validate_schema`/`validate_relations` (E10-H07/E11-H03) — el mismo universo que ve
/// `lodestar check`. La identidad de un check para este diff es la tupla `(targets, code, msg)`:
/// dos checks son "el mismo problema" si coinciden en los paths afectados, el código y el
/// mensaje; se ignoran `id`/`range`/`related`/`fixes` (metadatos aditivos sin relevancia para
/// "¿sigue el mismo diagnóstico?"). `diagnostics_introduced` = checks de `after` cuya clave no
/// está en `before`; `diagnostics_resolved` = checks de `before` cuya clave no está en `after`.
/// Se descartan los `Severity::Pass` (no son diagnósticos, son la ausencia de uno).
pub fn semantic_diff(before: &Bundle, after: &Bundle, schema: &Schema) -> SemanticDiff {
    let okf = diff_snap(before.files(), after.files());

    let mut created = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    let mut frontmatter_changes = Vec::new();
    let mut body_changes = Vec::new();
    let mut relation_changes = Vec::new();

    for fd in &okf.files {
        match fd.kind {
            ChangeKind::Add => created.push(fd.path.clone()),
            ChangeKind::Mod => modified.push(fd.path.clone()),
            ChangeKind::Remove => deleted.push(fd.path.clone()),
        }
        if !fd.fm.is_empty() {
            frontmatter_changes.push(fd.path.clone());
        }
        if fd
            .body
            .iter()
            .any(|h| matches!(h, BodyHunk::Add(_) | BodyHunk::Remove(_)))
        {
            body_changes.push(fd.path.clone());
        }
        if !fd.links_added.is_empty() || !fd.links_removed.is_empty() {
            relation_changes.push(fd.path.clone());
        }
    }

    let before_checks = all_checks(before, schema);
    let after_checks = all_checks(after, schema);
    let before_keys: BTreeSet<_> = before_checks.iter().map(check_key).collect();
    let after_keys: BTreeSet<_> = after_checks.iter().map(check_key).collect();

    let diagnostics_introduced = after_checks
        .iter()
        .filter(|c| !before_keys.contains(&check_key(c)))
        .cloned()
        .collect();
    let diagnostics_resolved = before_checks
        .iter()
        .filter(|c| !after_keys.contains(&check_key(c)))
        .cloned()
        .collect();

    SemanticDiff {
        created,
        modified,
        deleted,
        moved: Vec::new(),
        frontmatter_changes,
        body_changes,
        relation_changes,
        diagnostics_introduced,
        diagnostics_resolved,
    }
}

/// Conjunto completo de diagnósticos de `bundle` bajo `schema`: los 15 checks OKF clásicos
/// (`Bundle::analyze`) más las extensiones de esquema (`validate_schema`/`validate_relations`) —
/// el mismo universo que ve `lodestar check`. Descarta `Severity::Pass`: no es un diagnóstico,
/// es la ausencia de uno.
fn all_checks(bundle: &Bundle, schema: &Schema) -> Vec<Check> {
    let mut out: Vec<Check> = bundle
        .analyze()
        .per_file
        .values()
        .flatten()
        .cloned()
        .collect();
    out.extend(validate_schema(bundle, schema));
    out.extend(validate_relations(bundle, schema));
    out.retain(|c| c.level != Severity::Pass);
    out
}

/// Clave de identidad de un check para `semantic_diff`: `(targets, code, msg)`.
fn check_key(c: &Check) -> (Vec<RelPath>, CheckCode, String) {
    (c.targets.clone(), c.code, c.msg.clone())
}
