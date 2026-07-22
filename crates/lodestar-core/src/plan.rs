//! Evaluación de riesgo de un plan (`ChangeSet`) — E12-H02.
//!
//! Función pura [`assess_risk`]: dado un conjunto de [`NormalizedOperation`] ya resueltas y el
//! `Bundle` anterior al cambio, deriva un [`RiskAssessment`] con razones en español. No hace I/O;
//! toda la verdad la da el core (`Bundle::backlinks`, invariante #3 de `CLAUDE.md`).
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

use crate::types::{FrontmatterPatch, NormalizedOperation, RelPath, RiskAssessment, RiskLevel};
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
