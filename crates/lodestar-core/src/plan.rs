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
//! Función pura [`validate_result`]: dado el `Bundle` hipotético resultante de un `ChangeSet` y
//! el `Schema`, deriva un [`ValidationReport`] reusando el mismo universo de diagnósticos que
//! `all_checks` (`analyze().per_file` + `validate_schema` + `validate_relations`). Junto con
//! [`PlanPolicy`] y [`can_apply`] (E12-H04) decide si el plan es aplicable.
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

use std::collections::{BTreeMap, BTreeSet};

use crate::diff::{diff_snap, BodyHunk, ChangeKind};
use crate::error::CoreError;
use crate::model;
use crate::schema::{
    rel_target_repairs, relation_targets, target_type_of, validate_relations, validate_schema,
    Schema,
};
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};

use crate::types::{
    Check, CheckCode, EditSectionMode, FileMap, FrontmatterPatch, InboundLinksPolicy,
    NormalizedOperation, ParsedFrontmatter, RelPath, RiskAssessment, RiskLevel, SemanticDiff,
    Severity, ValidationReport, ValidationSummary,
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

// --- E12-H04: `ValidationReport` + `PlanPolicy` -------------------------------------------

/// Valida el `Bundle` hipotético resultante de un `ChangeSet` bajo `schema` — E12-H04.
///
/// Reusa `all_checks` (el mismo universo de diagnósticos que `semantic_diff` y `lodestar
/// check`: los 15 checks OKF de `Bundle::analyze` más `validate_schema`/`validate_relations`,
/// sin `Severity::Pass`). `summary` cuenta por severidad; `conformant` se computa explícitamente
/// como `summary.errors == 0` (no se reusa `ValidationSummary::default().conformant`, que sería
/// `false` por defecto — aquí "conforme" significa "sin errores duros", con o sin warnings).
pub fn validate_result(bundle: &Bundle, schema: &Schema) -> ValidationReport {
    let diagnostics = all_checks(bundle, schema);

    let mut summary = ValidationSummary::default();
    for check in &diagnostics {
        match check.level {
            Severity::Err => summary.errors += 1,
            Severity::Warn => summary.warnings += 1,
            Severity::Info => summary.info += 1,
            Severity::Pass => {}
        }
    }

    ValidationReport {
        conformant: summary.errors == 0,
        summary,
        diagnostics,
    }
}

/// Política de aplicación de un plan — E12-H04. Wire camelCase
/// (`requireConformantResult`/`allowWarnings`) por coherencia con el resto del contrato de plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PlanPolicy {
    /// Si `true`, un [`ValidationReport`] no conforme (`conformant == false`) bloquea `can_apply`.
    pub require_conformant_result: bool,
    /// Si `false`, cualquier warning (`summary.warnings > 0`) bloquea `can_apply`, incluso con
    /// resultado conforme.
    pub allow_warnings: bool,
}

impl Default for PlanPolicy {
    /// Default razonable: exige conformidad y permite warnings (el prototipo no bloquea por
    /// avisos, solo por errores duros).
    fn default() -> Self {
        Self {
            require_conformant_result: true,
            allow_warnings: true,
        }
    }
}

/// Decide si un plan cuyo resultado hipotético dio `report` es aplicable bajo `policy` —
/// E12-H04. `false` si `policy.require_conformant_result` y `!report.conformant`; `false` si
/// `!policy.allow_warnings` y `report.summary.warnings > 0`; `true` en cualquier otro caso.
pub fn can_apply(report: &ValidationReport, policy: &PlanPolicy) -> bool {
    if policy.require_conformant_result && !report.conformant {
        return false;
    }
    if !policy.allow_warnings && report.summary.warnings > 0 {
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Normalización de operaciones de CONTENIDO (E12-H05).
//
// Cada `normalize_*` toma el `Bundle` (fuente de verdad, invariante #1/#3) y una operación de alto
// nivel, y devuelve la [`NormalizedOperation`] YA RESUELTA a path(s) + contenido concreto, lista
// para que la workspace (único escritor) la aplique. Todo es PURO: no toca disco ni reloj.
// Estructura (move/delete) y semántica (relaciones/status) quedan para E12-H06/H07.
// ---------------------------------------------------------------------------

/// El marcador de plantilla que `create` sustituye por el título del concepto (E10-H05/E12-H05).
const TITLE_PLACEHOLDER: &str = "{title}";

/// Normaliza un `create`: resuelve el cuerpo del concepto nuevo.
///
/// - Si `body` es `Some`, se usa tal cual.
/// - Si `body` es `None` y el `DocType` `doctype` (buscado en `schema`) tiene `body_template`, el
///   cuerpo sale de esa plantilla, sustituyendo cada `{title}` por el título dado (o el derivado
///   del path si no se pasa `title`).
/// - Si `body` es `None` y no hay plantilla, se deja `None` (la workspace generará el heading por
///   defecto vía [`Bundle::create_concept`]).
///
/// El `frontmatter` resuelto es el mínimo (`type` + `title`); el resto de campos (status,
/// timestamp) los completa el escritor. Devuelve [`NormalizedOperation::Create`].
///
/// # Errores
/// No falla en esta historia (un `path` ya presente en el bundle no se rechaza aquí; esa política
/// es de la workspace). La firma devuelve `Result` por coherencia con las otras normalizaciones.
pub fn normalize_create(
    _bundle: &Bundle,
    schema: &Schema,
    path: &RelPath,
    doctype: &str,
    title: Option<&str>,
    body: Option<String>,
) -> Result<NormalizedOperation, CoreError> {
    let resolved_title = title
        .map(|s| s.to_string())
        .unwrap_or_else(|| model::derived_title(None, "", path));

    let resolved_body = match body {
        Some(b) => Some(b),
        None => schema
            .types
            .get(doctype)
            .and_then(|dt| dt.body_template.as_ref())
            .map(|tpl| tpl.replace(TITLE_PLACEHOLDER, &resolved_title)),
    };

    let mut fm: BTreeMap<String, Option<serde_yaml::Value>> = BTreeMap::new();
    fm.insert(
        "type".to_string(),
        Some(serde_yaml::Value::String(doctype.to_string())),
    );
    fm.insert(
        "title".to_string(),
        Some(serde_yaml::Value::String(resolved_title)),
    );

    Ok(NormalizedOperation::Create {
        path: path.clone(),
        frontmatter: FrontmatterPatch(fm),
        body: resolved_body,
    })
}

/// Cuerpo (tras el frontmatter) del concepto en `path`, o `Err(NormalizeTargetNotFound)` si el
/// path no tiene fichero en el bundle. Reusa `model::parse_file` (la misma verdad del core).
fn concept_body(bundle: &Bundle, path: &RelPath) -> Result<String, CoreError> {
    let raw = bundle
        .files()
        .get(path)
        .ok_or_else(|| CoreError::NormalizeTargetNotFound(path.as_str().to_string()))?;
    Ok(model::parse_file(path.as_str(), raw).body)
}

/// Normaliza un `replace_text`: sustituye todas las ocurrencias literales de `find` por `replace`
/// en el CUERPO del concepto (tras el frontmatter).
///
/// Si `expected_occurrences` es `Some(n)`, cuenta las coincidencias de `find` en el cuerpo y falla
/// con [`CoreError::ReplaceTextMismatch`] cuando el número real no es `n` (guarda contra ediciones
/// ciegas que tocan más —o menos— de lo previsto). Con `None` no se comprueba el conteo.
///
/// Devuelve [`NormalizedOperation::ReplaceBody`] con el cuerpo entero ya reescrito.
///
/// # Errores
/// - [`CoreError::NormalizeTargetNotFound`] si `path` no existe en el bundle.
/// - [`CoreError::ReplaceTextMismatch`] si el conteo no casa con `expected_occurrences`.
pub fn normalize_replace_text(
    bundle: &Bundle,
    path: &RelPath,
    find: &str,
    replace: &str,
    expected_occurrences: Option<usize>,
) -> Result<NormalizedOperation, CoreError> {
    let body = concept_body(bundle, path)?;

    if let Some(expected) = expected_occurrences {
        let found = if find.is_empty() {
            0
        } else {
            body.matches(find).count()
        };
        if found != expected {
            return Err(CoreError::ReplaceTextMismatch(expected, found));
        }
    }

    let new_body = body.replace(find, replace);
    Ok(NormalizedOperation::ReplaceBody {
        path: path.clone(),
        body: new_body,
    })
}

/// Normaliza un `edit_section`: localiza la subsección acotada por `heading_path` (con
/// [`model::parse_headings`], que ignora los `#` dentro de bloques de código fenceados) y aplica
/// `mode` sobre SU contenido, dejando intactas las secciones hermanas y de otro nivel.
///
/// - [`EditSectionMode::Replace`]: reemplaza el contenido de la sección por `content` (el heading
///   se conserva).
/// - [`EditSectionMode::Append`]: añade `content` al final del contenido de la sección.
/// - [`EditSectionMode::Prepend`]: inserta `content` al principio del contenido de la sección.
///
/// Devuelve [`NormalizedOperation::ReplaceBody`] con el CUERPO COMPLETO reescrito (la sección
/// editada más el resto sin tocar), listo para el único escritor.
///
/// # Errores
/// - [`CoreError::NormalizeTargetNotFound`] si `path` no existe o el `heading_path` no casa con
///   ninguna sección.
pub fn normalize_edit_section(
    bundle: &Bundle,
    path: &RelPath,
    heading_path: &[String],
    mode: EditSectionMode,
    content: &str,
) -> Result<NormalizedOperation, CoreError> {
    let body = concept_body(bundle, path)?;
    let headings = model::parse_headings(&body);
    let (start, end) =
        model::locate_section(&headings, body.len(), heading_path).ok_or_else(|| {
            CoreError::NormalizeTargetNotFound(format!(
                "{}#{}",
                path.as_str(),
                heading_path.join("/")
            ))
        })?;

    let section = &body[start..end];
    let content = content.trim();
    let new_section = match mode {
        EditSectionMode::Replace => format!("\n{content}\n\n"),
        EditSectionMode::Append => format!("{}\n\n{content}\n\n", section.trim_end()),
        EditSectionMode::Prepend => format!("\n{content}\n{}", section.trim_start()),
    };

    let new_body = format!("{}{}{}", &body[..start], new_section, &body[end..]);
    Ok(NormalizedOperation::ReplaceBody {
        path: path.clone(),
        body: new_body,
    })
}

/// `Err(NormalizeTargetNotFound)` si `path` no tiene fichero en el bundle. Punto único de
/// verificación de existencia para los normalizadores que solo necesitan saber que el concepto
/// objetivo existe (sin leer su cuerpo).
fn ensure_exists(bundle: &Bundle, path: &RelPath) -> Result<(), CoreError> {
    if bundle.files().contains_key(path) {
        Ok(())
    } else {
        Err(CoreError::NormalizeTargetNotFound(
            path.as_str().to_string(),
        ))
    }
}

/// Normaliza un `patch_frontmatter`: aplica un merge-patch RFC 7386 al frontmatter de un concepto
/// existente (E12-H05, reserva completada en E12-H08).
///
/// El `patch` (un [`FrontmatterPatch`]) ya es la forma normalizada del merge-patch — `Some(v)`
/// escribe/reemplaza, `None` **borra** la clave (`null` en el wire), clave ausente = no se toca —,
/// así que esta normalización se limita a **validar que el concepto objetivo existe** y envolverlo
/// en [`NormalizedOperation::PatchFrontmatter`]. El merge real sobre el `Frontmatter` lo materializa
/// el aplicador ([`apply_normalized_ops`]) reusando la misma lógica que `Bundle::merge_frontmatter`
/// (un solo camino de merge-patch en el core, invariante #3). **Puro.**
///
/// # Errores
/// [`CoreError::NormalizeTargetNotFound`] si `path` no existe en el bundle.
pub fn normalize_patch_frontmatter(
    bundle: &Bundle,
    path: &RelPath,
    patch: FrontmatterPatch,
) -> Result<NormalizedOperation, CoreError> {
    ensure_exists(bundle, path)?;
    Ok(NormalizedOperation::PatchFrontmatter {
        path: path.clone(),
        patch,
    })
}

/// Normaliza un `replace_body`: reemplaza el cuerpo completo (tras el frontmatter) de un concepto
/// existente por `body` (E12-H05, reserva completada en E12-H08).
///
/// Valida que el concepto objetivo existe y devuelve [`NormalizedOperation::ReplaceBody`] con el
/// cuerpo nuevo tal cual; el frontmatter se conserva al aplicar. **Puro.**
///
/// # Errores
/// [`CoreError::NormalizeTargetNotFound`] si `path` no existe en el bundle.
pub fn normalize_replace_body(
    bundle: &Bundle,
    path: &RelPath,
    body: String,
) -> Result<NormalizedOperation, CoreError> {
    ensure_exists(bundle, path)?;
    Ok(NormalizedOperation::ReplaceBody {
        path: path.clone(),
        body,
    })
}

// ---------------------------------------------------------------------------
// Normalización de operaciones de ESTRUCTURA (E12-H06): `move` y `delete`.
//
// A diferencia de las de contenido, estas producen VARIAS `NormalizedOperation` dentro del mismo
// change set: el rename/borrado estructural MÁS la reescritura/eliminación de los enlaces entrantes.
// Toda la verdad la da el core (`Bundle::backlinks`, `model::resolve_link`); nada de I/O.
// ---------------------------------------------------------------------------

/// Mismo léxico de enlace markdown que [`model::LINK_RE`] (un solo vocabulario de enlaces en el
/// core), pero capturando además el TEXTO del enlace (grupo 1) junto al href (grupo 2). El patrón
/// del interior del paréntesis es idéntico al de `LINK_RE`; el grupo de texto extra permite
/// reescribir el href a un nuevo destino o "desenlazar" a texto plano.
static LINK_REWRITE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#).unwrap());

/// Qué hacer con un enlace entrante cuyo destino resuelto es el concepto afectado.
enum LinkAction<'a> {
    /// Reescribir el href para que apunte a `to` (usado por `move` con `rewriteInboundLinks`).
    Retarget(&'a RelPath),
    /// Quitar el enlace dejando solo su texto (usado por `delete` con `remove_links`).
    Remove,
}

/// Divide un href crudo en `(base, sufijo)`, donde el sufijo empieza en el primer `#` (fragmento) o
/// `?` (query). `model::resolve_link` ignora ese sufijo al resolver; al reescribir lo conservamos.
fn split_href_suffix(href: &str) -> (&str, &str) {
    match href.find(['#', '?']) {
        Some(i) => (&href[..i], &href[i..]),
        None => (href, ""),
    }
}

/// Construye un href relativo desde el directorio de `source_path` hasta `target` (ambos paths de
/// bundle normalizados), reusando el álgebra de directorios del core (`model::dir_of`). Es puro
/// cálculo de rutas: sin `..` cuando comparten prefijo, con `./` cuando el destino queda en el mismo
/// directorio (para que `resolve_link` lo trate inequívocamente como relativo, como el prototipo).
fn relative_href(source_path: &str, target: &str) -> String {
    let from_dir = model::dir_of(source_path);
    let from_parts: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to_parts: Vec<&str> = target.split('/').collect();
    let (to_dirs, file) = to_parts.split_at(to_parts.len() - 1);

    let mut common = 0;
    while common < from_parts.len()
        && common < to_dirs.len()
        && from_parts[common] == to_dirs[common]
    {
        common += 1;
    }

    let mut segs: Vec<&str> = vec![".."; from_parts.len() - common];
    segs.extend_from_slice(&to_dirs[common..]);
    segs.push(file[0]);
    let joined = segs.join("/");

    // Sin `../` y en el mismo directorio → `./fichero.md`, claramente relativo.
    if from_parts.len() == common && !joined.contains('/') {
        format!("./{joined}")
    } else {
        joined
    }
}

/// Reescribe el href de un enlace que apuntaba a `from_target` para que apunte a `to`, conservando
/// el estilo (absoluto `/…` vs relativo) y el sufijo `#fragmento`/`?query`.
fn retarget_href(old_href: &str, source_path: &str, to: &RelPath) -> String {
    let (base, suffix) = split_href_suffix(old_href);
    let new_base = if base.starts_with('/') {
        format!("/{}", to.as_str())
    } else {
        relative_href(source_path, to.as_str())
    };
    format!("{new_base}{suffix}")
}

/// Reescribe el CUERPO de un concepto entrante aplicando `action` SOLO a los enlaces markdown cuyo
/// href resuelve (vía `model::resolve_link`, la misma verdad del core) al concepto `target`. El
/// resto de enlaces y de texto queda intacto — nunca se toca un enlace que apunte a otro sitio.
fn rewrite_body_links(
    body: &str,
    source_path: &str,
    target: &RelPath,
    action: &LinkAction,
) -> String {
    LINK_REWRITE_RE
        .replace_all(body, |caps: &Captures| {
            let full = caps.get(0).map_or("", |m| m.as_str());
            let text = &caps[1];
            let href = &caps[2];
            let resuelve =
                model::resolve_link(href, source_path).and_then(|t| RelPath::new(&t).ok());
            if resuelve.as_ref() != Some(target) {
                return full.to_string();
            }
            match action {
                LinkAction::Retarget(to) => {
                    format!("[{text}]({})", retarget_href(href, source_path, to))
                }
                LinkAction::Remove => text.to_string(),
            }
        })
        .into_owned()
}

/// Normaliza un `move` (rename de un concepto) — E12-H06.
///
/// Produce siempre un [`NormalizedOperation::Move`] `{ from, to }`. Si `rewrite_inbound_links`,
/// añade además, por cada concepto que enlaza a `from` (los entrantes de `Bundle::backlinks(from)`,
/// invariante #3 de `CLAUDE.md`), un [`NormalizedOperation::ReplaceBody`] con el cuerpo del entrante
/// reescrito para que su enlace apunte a `to` (ver `rewrite_body_links` / `retarget_href`). Así,
/// mover un concepto con 30 backlinks y `rewriteInboundLinks:true` da 1 `Move` + 30 `ReplaceBody`,
/// todo dentro del mismo change set.
///
/// # Errores
/// [`CoreError::NormalizeTargetNotFound`] si algún concepto entrante no tiene fichero en el bundle
/// (no debería ocurrir: los entrantes salen del propio bundle).
pub fn normalize_move(
    bundle: &Bundle,
    from: &RelPath,
    to: &RelPath,
    rewrite_inbound_links: bool,
) -> Result<Vec<NormalizedOperation>, CoreError> {
    let mut ops = vec![NormalizedOperation::Move {
        from: from.clone(),
        to: to.clone(),
        rewrite_inbound_links,
    }];

    if rewrite_inbound_links {
        for link in bundle.backlinks(from).inbound {
            let source = link.path;
            let body = concept_body(bundle, &source)?;
            let new_body =
                rewrite_body_links(&body, source.as_str(), from, &LinkAction::Retarget(to));
            ops.push(NormalizedOperation::ReplaceBody {
                path: source,
                body: new_body,
            });
        }
    }

    Ok(ops)
}

/// Normaliza un `delete` (borrado de un concepto) según la política ante enlaces entrantes — E12-H06.
///
/// - [`InboundLinksPolicy::Reject`] (el default): si el concepto tiene entrantes
///   (`Bundle::backlinks(path).inbound`), falla con [`CoreError::InboundLinksExist`] (wire
///   `"INBOUND_LINKS_EXIST"`); sin entrantes, devuelve solo el [`NormalizedOperation::Delete`].
/// - [`InboundLinksPolicy::RemoveLinks`]: devuelve el `Delete` MÁS, por cada entrante, un
///   [`NormalizedOperation::ReplaceBody`] que quita el enlace al concepto borrado dejando su texto
///   plano (ver `rewrite_body_links`).
/// - [`InboundLinksPolicy::Retarget`] / [`InboundLinksPolicy::CreateStub`]: **implementación mínima**
///   en esta historia — devuelven solo el `Delete`, sin manejar los entrantes. E12-H06 no fija
///   criterio para ellas (a qué destino redirigir, qué contenido tendría el stub), así que no se
///   inventa semántica aquí; queda para una historia posterior.
pub fn normalize_delete(
    bundle: &Bundle,
    path: &RelPath,
    policy: InboundLinksPolicy,
) -> Result<Vec<NormalizedOperation>, CoreError> {
    let inbound = bundle.backlinks(path).inbound;
    let delete = NormalizedOperation::Delete {
        path: path.clone(),
        inbound_links_policy: policy,
    };

    match policy {
        InboundLinksPolicy::Reject => {
            if !inbound.is_empty() {
                return Err(CoreError::InboundLinksExist(path.clone()));
            }
            Ok(vec![delete])
        }
        InboundLinksPolicy::RemoveLinks => {
            let mut ops = vec![delete];
            for link in inbound {
                let source = link.path;
                let body = concept_body(bundle, &source)?;
                let new_body =
                    rewrite_body_links(&body, source.as_str(), path, &LinkAction::Remove);
                ops.push(NormalizedOperation::ReplaceBody {
                    path: source,
                    body: new_body,
                });
            }
            Ok(ops)
        }
        // Sin criterio en E12-H06: mínimo defensible, solo el borrado (ver doc).
        InboundLinksPolicy::Retarget | InboundLinksPolicy::CreateStub => Ok(vec![delete]),
    }
}

// ---------------------------------------------------------------------------
// Normalización de operaciones SEMÁNTICAS (E12-H07): relaciones tipadas, ciclo de vida y fixes.
//
// A diferencia de las de estructura, resuelven SIEMPRE a un único `PatchFrontmatter` sobre el
// concepto afectado — la única verdad la dan `schema` (RelationDef/allowedStatuses, invariante #3)
// y el frontmatter del propio concepto. Todo PURO: sin I/O, sin reloj. `apply_fix` recompone el
// mismo universo de diagnósticos que `lodestar check` para re-localizar el fix por su id estable.
// ---------------------------------------------------------------------------

/// `type` declarado en el frontmatter del concepto en `path`, o `None` si el fichero no existe,
/// no tiene frontmatter, o no declara `type`. Reusa `model::parse_file` (la misma verdad del core).
fn concept_type(bundle: &Bundle, path: &RelPath) -> Option<String> {
    bundle
        .files()
        .get(path)
        .and_then(|raw| model::parse_file(path.as_str(), raw).frontmatter)
        .and_then(|fm| fm.get_text("type"))
}

/// Targets actuales del campo de relación `relation` en el frontmatter de `source` (secuencia YAML
/// o `String` único, vía `schema::relation_targets`). Vector vacío si el concepto no existe, no
/// tiene frontmatter, o el campo no está presente.
fn current_targets(bundle: &Bundle, source: &RelPath, relation: &str) -> Vec<String> {
    bundle
        .files()
        .get(source)
        .and_then(|raw| model::parse_file(source.as_str(), raw).frontmatter)
        .and_then(|fm| relation_targets(&fm, relation))
        .unwrap_or_default()
}

/// Construye un [`FrontmatterPatch`] que fija el campo `relation` a la secuencia YAML de `targets`
/// (lista de paths). Una lista vacía deja el campo como `[]` (presente pero sin targets): el
/// concepto deja de referenciar, sin borrar la declaración del campo.
fn relation_field_patch(relation: &str, targets: &[String]) -> FrontmatterPatch {
    let seq = serde_yaml::Value::Sequence(
        targets
            .iter()
            .map(|t| serde_yaml::Value::String(t.clone()))
            .collect(),
    );
    let mut map = BTreeMap::new();
    map.insert(relation.to_string(), Some(seq));
    FrontmatterPatch(map)
}

/// Normaliza un `add_relation`: añade `target` al campo de relación `relation` del frontmatter de
/// `source`, validando antes contra la [`crate::schema::RelationDef`] del `DocType` de `source` —
/// E12-H07.
///
/// Validaciones (solo si el `DocType` de `source` declara la relación `relation`; sin `RelationDef`
/// no hay restricción que imponer):
/// - **Tipo del target**: si `target_types` no está vacío y el `type` de `target` es conocido y no
///   figura en la lista → [`CoreError::RelationConstraintViolation`].
/// - **Cardinalidad**: si `cardinality == "one"` y añadir `target` dejaría el campo con más de un
///   target → [`CoreError::RelationConstraintViolation`].
///
/// Si es válida, devuelve un [`NormalizedOperation::PatchFrontmatter`] que fija el campo `relation`
/// a los targets actuales MÁS `target` (idempotente: no duplica si ya estaba). **Puro.**
pub fn normalize_add_relation(
    bundle: &Bundle,
    schema: &Schema,
    source: &RelPath,
    relation: &str,
    target: &RelPath,
) -> Result<NormalizedOperation, CoreError> {
    let mut new_targets = current_targets(bundle, source, relation);
    let already = new_targets.iter().any(|t| t == target.as_str());
    if !already {
        new_targets.push(target.as_str().to_string());
    }

    if let Some(reldef) = concept_type(bundle, source)
        .as_deref()
        .and_then(|tipo| schema.types.get(tipo))
        .and_then(|dt| dt.relations.get(relation))
    {
        if !reldef.target_types.is_empty() {
            if let Some(target_type) = target_type_of(bundle, target) {
                if !reldef.target_types.iter().any(|t| t == &target_type) {
                    return Err(CoreError::RelationConstraintViolation(format!(
                        "la relación «{relation}» de «{}» no admite un target de tipo «{target_type}» \
                         (admite: {}); target «{}».",
                        source.as_str(),
                        reldef.target_types.join(", "),
                        target.as_str(),
                    )));
                }
            }
        }

        if reldef.cardinality == "one" && new_targets.len() > 1 {
            return Err(CoreError::RelationConstraintViolation(format!(
                "la relación «{relation}» de «{}» admite como máximo un target (cardinalidad \
                 «one») pero quedaría con {}.",
                source.as_str(),
                new_targets.len(),
            )));
        }
    }

    Ok(NormalizedOperation::PatchFrontmatter {
        path: source.clone(),
        patch: relation_field_patch(relation, &new_targets),
    })
}

/// Normaliza un `remove_relation`: quita `target` del campo de relación `relation` del frontmatter
/// de `source` — E12-H07.
///
/// Devuelve un [`NormalizedOperation::PatchFrontmatter`] que fija el campo `relation` a los targets
/// actuales SIN `target` (idempotente: si no estaba, el campo queda igual). No valida contra la
/// `RelationDef` — quitar una relación nunca puede violar una restricción de tipo/cardinalidad.
/// **Puro.**
pub fn normalize_remove_relation(
    _bundle: &Bundle,
    _schema: &Schema,
    source: &RelPath,
    relation: &str,
    target: &RelPath,
) -> Result<NormalizedOperation, CoreError> {
    let remaining: Vec<String> = current_targets(_bundle, source, relation)
        .into_iter()
        .filter(|t| t != target.as_str())
        .collect();

    Ok(NormalizedOperation::PatchFrontmatter {
        path: source.clone(),
        patch: relation_field_patch(relation, &remaining),
    })
}

/// Normaliza un `transition_status`: valida `to` contra los `allowed_statuses` del `DocType` de
/// `reference` y produce el patch de `status` — E12-H07.
///
/// Si el `DocType` de `reference` declara `allowed_statuses` no vacío y `to` no está en la lista →
/// [`CoreError::InvalidStatusTransition`] (mismo criterio que `SCHEMA-STATUS` en `validate_schema`:
/// una lista vacía, o un tipo sin `DocType`, no impone restricción). Si es válida, devuelve un
/// [`NormalizedOperation::PatchFrontmatter`] que fija `status: to`. **Puro.**
pub fn normalize_transition_status(
    bundle: &Bundle,
    schema: &Schema,
    reference: &RelPath,
    to: &str,
) -> Result<NormalizedOperation, CoreError> {
    if let Some(doctype) = concept_type(bundle, reference)
        .as_deref()
        .and_then(|tipo| schema.types.get(tipo))
    {
        if !doctype.allowed_statuses.is_empty() && !doctype.allowed_statuses.iter().any(|s| s == to)
        {
            return Err(CoreError::InvalidStatusTransition(format!(
                "«{to}» no es un estado permitido para «{}» (permite: {}).",
                reference.as_str(),
                doctype.allowed_statuses.join(", "),
            )));
        }
    }

    let mut map = BTreeMap::new();
    map.insert(
        "status".to_string(),
        Some(serde_yaml::Value::String(to.to_string())),
    );
    Ok(NormalizedOperation::PatchFrontmatter {
        path: reference.clone(),
        patch: FrontmatterPatch(map),
    })
}

/// Normaliza un `apply_fix`: materializa el `Fix` `safe` cuyo `fix_id` casa con `fix_id`, entre los
/// diagnósticos recomputados del bundle — E12-H07.
///
/// Recompone el MISMO universo de diagnósticos que `lodestar check` (`all_checks`:
/// `analyze().per_file` + `validate_schema` + `validate_relations`) y comprueba que exista un
/// [`crate::types::Fix`] con `fix_id == fix_id` y `safe == true`; si no, falla con
/// [`CoreError::FixNotFound`]. Localizado el fix, deriva su arreglo a partir de
/// `schema::rel_target_repairs` (la contraparte estructurada de los fixes de relación rota,
/// con el mismo `fix_id` estable): el único arreglo soportado en esta historia es el de una relación
/// tipada ROTA (`REL-TARGET`), que se materializa como un [`NormalizedOperation::PatchFrontmatter`]
/// sobre el concepto origen QUITANDO el target roto de su campo de relación. Un `fix_id` sin repair
/// asociado (fix de otra familia aún no soportada) también da [`CoreError::FixNotFound`]. **Puro.**
pub fn normalize_apply_fix(
    bundle: &Bundle,
    schema: &Schema,
    fix_id: &str,
) -> Result<NormalizedOperation, CoreError> {
    let safe_fix_presente = all_checks(bundle, schema)
        .iter()
        .flat_map(|c| &c.fixes)
        .any(|f| f.fix_id == fix_id && f.safe);
    if !safe_fix_presente {
        return Err(CoreError::FixNotFound(fix_id.to_string()));
    }

    let repair = rel_target_repairs(bundle, schema)
        .into_iter()
        .find(|r| r.fix_id == fix_id)
        .ok_or_else(|| CoreError::FixNotFound(fix_id.to_string()))?;

    let remaining: Vec<String> = current_targets(bundle, &repair.source, &repair.rel_name)
        .into_iter()
        .filter(|t| t != &repair.target)
        .collect();

    Ok(NormalizedOperation::PatchFrontmatter {
        path: repair.source.clone(),
        patch: relation_field_patch(&repair.rel_name, &remaining),
    })
}

// ---------------------------------------------------------------------------
// Aplicación en memoria de un plan (E12-H08): construir el `FileMap` hipotético.
//
// `apply_normalized_ops` toma el `FileMap` actual y una lista de operaciones YA normalizadas a
// forma terminal (las que producen los normalizadores de E12-H05/H06/H07:
// `Create`/`PatchFrontmatter`/`ReplaceBody`/`Move`/`Delete`) y devuelve el `FileMap` resultante de
// aplicarlas EN ORDEN, sin tocar disco (invariante #1 de `CLAUDE.md`: la escritura real es E13).
// Es la simulación que alimenta `SemanticDiff`/`RiskAssessment`/`ValidationReport` del `change_plan`.
// ---------------------------------------------------------------------------

/// Aplica `ops` (ya normalizadas) sobre una **copia** de `files` y devuelve el `FileMap`
/// hipotético — E12-H08. **Puro**: no toca disco ni reloj; el `files` de entrada queda intacto.
///
/// Cada operación se materializa reusando la única canonicalización del core
/// (`model::build_raw`/`model::parse_file` y `bundle::apply_patch`, invariante #3 de `CLAUDE.md`):
/// - [`NormalizedOperation::Create`]: escribe el `.md` nuevo con el frontmatter del patch aplicado
///   sobre uno vacío y el cuerpo dado (o un heading por defecto `# {título}` si `body` es `None`).
/// - [`NormalizedOperation::PatchFrontmatter`]: aplica el merge-patch al frontmatter existente,
///   conservando el cuerpo (mismo camino que `Bundle::merge_frontmatter`).
/// - [`NormalizedOperation::ReplaceBody`]: conserva el frontmatter y sustituye el cuerpo.
/// - [`NormalizedOperation::Move`]: renombra la clave (mismo contenido); la reescritura de enlaces
///   entrantes llega como `ReplaceBody` aparte dentro del mismo plan (E12-H06).
/// - [`NormalizedOperation::Delete`]: elimina la clave.
///
/// Las ops se aplican secuencialmente, así que una op posterior ve el efecto de las anteriores.
///
/// # Errores
/// [`CoreError::OperationNotApplicable`] si llega una variante NO terminal (semántica/de contenido
/// que los normalizadores ya resuelven a las cinco de arriba) — es una violación de invariante
/// interno del pipeline, nunca una entrada del agente.
pub fn apply_normalized_ops(
    files: &FileMap,
    ops: &[NormalizedOperation],
) -> Result<FileMap, CoreError> {
    let mut out = files.clone();
    for op in ops {
        apply_one(&mut out, op)?;
    }
    Ok(out)
}

/// Frontmatter (o el vacío por defecto) y cuerpo actuales del `.md` en `path` dentro de `files`.
fn parsed_of(files: &FileMap, path: &RelPath) -> (serde_yaml::Mapping, String) {
    match files.get(path) {
        Some(raw) => {
            let parsed = model::parse_file(path.as_str(), raw);
            let map = parsed
                .frontmatter
                .as_ref()
                .map(|fm| fm.mapping().clone())
                .unwrap_or_default();
            (map, parsed.body)
        }
        None => (serde_yaml::Mapping::new(), String::new()),
    }
}

/// Aplica una única operación normalizada sobre `files` (mutación in situ). Ver
/// [`apply_normalized_ops`].
fn apply_one(files: &mut FileMap, op: &NormalizedOperation) -> Result<(), CoreError> {
    match op {
        NormalizedOperation::Create {
            path,
            frontmatter,
            body,
        } => {
            let mut map = serde_yaml::Mapping::new();
            crate::bundle::apply_patch(&mut map, frontmatter.clone());
            let fm = ParsedFrontmatter::from_mapping(map);
            let body = body.clone().unwrap_or_else(|| {
                // Sin cuerpo todavía: la cadena de `derived_title` se resuelve por el `title` del
                // frontmatter o, en su defecto, por el nombre del fichero.
                let title = model::derived_title(Some(&fm), "", path);
                format!("# {title}\n")
            });
            files.insert(path.clone(), model::build_raw(Some(&fm), &body));
        }
        NormalizedOperation::PatchFrontmatter { path, patch } => {
            let (mut map, body) = parsed_of(files, path);
            crate::bundle::apply_patch(&mut map, patch.clone());
            let fm = ParsedFrontmatter::from_mapping(map);
            files.insert(path.clone(), model::build_raw(Some(&fm), &body));
        }
        NormalizedOperation::ReplaceBody { path, body } => {
            let (map, _) = parsed_of(files, path);
            let fm = ParsedFrontmatter::from_mapping(map);
            files.insert(path.clone(), model::build_raw(Some(&fm), body));
        }
        NormalizedOperation::Move { from, to, .. } => {
            if let Some(raw) = files.remove(from) {
                files.insert(to.clone(), raw);
            }
        }
        NormalizedOperation::Delete { path, .. } => {
            files.remove(path);
        }
        // Variantes NO terminales: los normalizadores (E12-H05/H06/H07) siempre las resuelven a
        // las cinco de arriba antes de llegar aquí. Si aparecen, es un bug del pipeline.
        other => {
            return Err(CoreError::OperationNotApplicable(op_variant_name(other)));
        }
    }
    Ok(())
}

/// Nombre de la variante (para el mensaje de [`CoreError::OperationNotApplicable`]).
fn op_variant_name(op: &NormalizedOperation) -> String {
    match op {
        NormalizedOperation::Create { .. } => "create",
        NormalizedOperation::PatchFrontmatter { .. } => "patch_frontmatter",
        NormalizedOperation::ReplaceBody { .. } => "replace_body",
        NormalizedOperation::EditSection { .. } => "edit_section",
        NormalizedOperation::ReplaceText { .. } => "replace_text",
        NormalizedOperation::Move { .. } => "move",
        NormalizedOperation::Delete { .. } => "delete",
        NormalizedOperation::AddRelation { .. } => "add_relation",
        NormalizedOperation::RemoveRelation { .. } => "remove_relation",
        NormalizedOperation::TransitionStatus { .. } => "transition_status",
        NormalizedOperation::ApplyFix { .. } => "apply_fix",
    }
    .to_string()
}
