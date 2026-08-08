//! Análisis de un plan (`ChangeSet`): riesgo (E12-H02) y diff semántico (E12-H03).
//!
//! Función pura [`assess_risk`]: dado un conjunto de [`NormalizedOperation`] ya resueltas y el
//! `DocumentSet` anterior al cambio, deriva un [`RiskAssessment`] con razones en español. No hace I/O;
//! toda la verdad la da el core (`DocumentSet::backlinks`, invariante #3 de `CLAUDE.md`).
//!
//! Función pura [`semantic_diff`]: dado el `DocumentSet` antes/después de un plan, deriva un
//! [`SemanticDiff`] reusando [`crate::diff::diff_snap`] y el conjunto de diagnósticos de
//! `DocumentSet::analyze` — ver la doc de la función para el detalle.
//!
//! Función pura [`validate_result`]: dado el `DocumentSet` hipotético resultante de un `ChangeSet`,
//! deriva un [`ValidationReport`] reusando el mismo universo de diagnósticos que `all_checks`
//! (`analyze().diagnostics`). Junto con [`PlanPolicy`] y [`can_apply`] (E12-H04) decide si el plan
//! es aplicable.
//!
//! ## Heurística (documentada, no normativa — H02 solo fija el orden de magnitud)
//!
//! Para cada operación que **encoge** el grafo de documentos — deprecar (un `PatchFrontmatter` cuyo
//! patch pone `status: deprecated`), borrar (`Delete`) o mover (`Move`) — se mide su *blast radius*:
//! los backlinks entrantes
//! (`DocumentSet::backlinks(&path).inbound`) que el documento afectado tenía en el workspace **antes**
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
//! documento sin backlinks) no generan factor de riesgo — de ahí que un cambio aislado sea
//! siempre `Low`.
//!
//! `workspace_after` se recibe por coherencia de firma con el resto del pipeline de plan (E12-H03
//! `SemanticDiff`, E12-H04 `ValidationReport` sí lo necesitan); esta heurística de riesgo solo
//! necesita el workspace *antes* del cambio para medir backlinks del documento afectado.

use std::collections::BTreeSet;

use crate::diff::{diff_snap, BodyHunk, ChangeKind};
use crate::error::CoreError;
use crate::model;
use once_cell::sync::Lazy;
use regex::{Captures, Regex};
use serde::{Deserialize, Serialize};

use crate::links;
use crate::types::{
    Check, CheckCode, EditSectionMode, FileMap, FrontmatterPatch, InboundLinksPolicy, Inventory,
    LinkKind, LinkTarget, NormalizedOperation, ParsedFrontmatter, RawLink, RelPath, RiskAssessment,
    RiskLevel, SemanticDiff, Severity, ValidationReport, ValidationSummary,
};
use crate::DocumentSet;

/// A partir de este número de backlinks entrantes, el factor de riesgo es `High` (por debajo,
/// `Medium`, siempre que haya al menos uno).
const HIGH_BACKLINKS_THRESHOLD: usize = 5;

/// Evalúa el riesgo de aplicar `ops` sobre `workspace_before`. Ver heurística en el módulo.
pub fn assess_risk(
    ops: &[NormalizedOperation],
    workspace_before: &DocumentSet,
    _workspace_after: &DocumentSet,
) -> RiskAssessment {
    let mut level = RiskLevel::Low;
    let mut reasons = Vec::new();

    for op in ops {
        let Some(path) = shrinking_path(op) else {
            continue;
        };
        let backlinks = workspace_before.backlinks(&path).inbound.len();
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
            "{} «{}» afecta a {} documento{} que lo referencian.",
            accion(op),
            path.as_str(),
            backlinks,
            if backlinks == 1 { "" } else { "s" },
        ));
    }

    RiskAssessment { level, reasons }
}

/// Si `op` "encoge" el grafo de documentos (deprecar/borrar/mover), devuelve el path del documento
/// cuyos backlinks entrantes hay que medir. `None` para el resto de operaciones (no generan
/// factor de riesgo bajo esta heurística).
fn shrinking_path(op: &NormalizedOperation) -> Option<RelPath> {
    match op {
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
/// cercana a "cambió una relación" sin reimplementar una resolución de relaciones tipadas (que ya
/// no existe: `§20.10`, una relación es un enlace Markdown).
///
/// `diagnostics_introduced`/`diagnostics_resolved` comparan el conjunto COMPLETO de diagnósticos
/// de cada workspace: los de `DocumentSet::analyze().diagnostics` (`§20.9`) — el mismo universo que
/// ve `lodestar check`. La identidad de un check para este diff es la tupla `(targets, code, msg)`:
/// dos checks son "el mismo problema" si coinciden en los paths afectados, el código y el
/// mensaje; se ignoran `id`/`range`/`related`/`fixes` (metadatos aditivos sin relevancia para
/// "¿sigue el mismo diagnóstico?"). `diagnostics_introduced` = checks de `after` cuya clave no
/// está en `before`; `diagnostics_resolved` = checks de `before` cuya clave no está en `after`.
/// Se descartan los `Severity::Pass` (no son diagnósticos, son la ausencia de uno).
pub fn semantic_diff(before: &DocumentSet, after: &DocumentSet) -> SemanticDiff {
    let snap = diff_snap(before.files(), after.files());

    let mut created = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    let mut frontmatter_changes = Vec::new();
    let mut body_changes = Vec::new();
    let mut relation_changes = Vec::new();

    for fd in &snap.files {
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

    let before_checks = all_checks(before);
    let after_checks = all_checks(after);
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

/// Conjunto completo de diagnósticos de `doc_set`: los de `DocumentSet::analyze` (`§20.9`) — el
/// mismo universo que ve `lodestar check`. Tras el retiro de `core::schema` (E20-H03) ya no hay
/// extensiones de esquema que añadir; queda como el punto único donde el pipeline de plan reúne los
/// diagnósticos, por si futuras historias vuelven a componer más fuentes. Descarta `Severity::Pass`:
/// no es un diagnóstico, es la ausencia de uno.
fn all_checks(doc_set: &DocumentSet) -> Vec<Check> {
    let mut out: Vec<Check> = doc_set
        .analyze()
        .diagnostics
        .values()
        .flatten()
        .cloned()
        .collect();
    out.retain(|c| c.level != Severity::Pass);
    out
}

/// Clave de identidad de un check para `semantic_diff`: `(targets, code, msg)`.
fn check_key(c: &Check) -> (Vec<RelPath>, CheckCode, String) {
    (c.targets.clone(), c.code, c.msg.clone())
}

// --- E12-H04: `ValidationReport` + `PlanPolicy` -------------------------------------------

/// Valida el `DocumentSet` hipotético resultante de un `ChangeSet` — E12-H04.
///
/// Reusa `all_checks` (el mismo universo de diagnósticos que `semantic_diff` y `lodestar
/// check`: los diagnósticos de `DocumentSet::analyze` (`§20.9`), sin `Severity::Pass`). `summary`
/// cuenta por severidad; `valid` se computa explícitamente como `summary.errors == 0` (no se
/// reusa `ValidationSummary::default().valid`, que sería `false` por defecto — aquí "conforme"
/// significa "sin errores duros", con o sin warnings).
pub fn validate_result(doc_set: &DocumentSet) -> ValidationReport {
    let diagnostics = all_checks(doc_set);

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
        valid: summary.errors == 0,
        summary,
        diagnostics,
    }
}

/// Política de aplicación de un plan — E12-H04. Wire camelCase
/// (`requireValidResult`/`allowWarnings`) por coherencia con el resto del contrato de plan.
///
/// `#[serde(default)]` a nivel de struct (E29-H02, `decisiones §19(b)`): un objeto `policy`
/// PARCIAL —con solo uno de los dos campos, o incluso `{}`— deserializa completando cada campo
/// omitido con el valor de [`PlanPolicy::default`], que ya es la única fuente de esos defaults
/// (invariante #4: no se duplican los literales en el `inputSchema`/`contracts/mcp.yml`, que solo
/// los *documentan*). Antes de este arreglo, omitir una sola clave fallaba con
/// «missing field …» pese a que ambas ya se declaraban opcionales por contrato.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", default)]
pub struct PlanPolicy {
    /// Si `true`, un [`ValidationReport`] no conforme (`valid == false`) bloquea `can_apply`.
    pub require_valid_result: bool,
    /// Si `false`, cualquier warning (`summary.warnings > 0`) bloquea `can_apply`, incluso con
    /// resultado conforme.
    pub allow_warnings: bool,
}

impl Default for PlanPolicy {
    /// Default razonable: exige conformidad y permite warnings (el prototipo no bloquea por
    /// avisos, solo por errores duros).
    fn default() -> Self {
        Self {
            require_valid_result: true,
            allow_warnings: true,
        }
    }
}

/// Decide si un plan cuyo resultado hipotético dio `report` es aplicable bajo `policy` —
/// E12-H04. `false` si `policy.require_valid_result` y `!report.valid`; `false` si
/// `!policy.allow_warnings` y `report.summary.warnings > 0`; `true` en cualquier otro caso.
pub fn can_apply(report: &ValidationReport, policy: &PlanPolicy) -> bool {
    if policy.require_valid_result && !report.valid {
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
// Cada `normalize_*` toma el `DocumentSet` (fuente de verdad, invariante #1/#3) y una operación de alto
// nivel, y devuelve la [`NormalizedOperation`] YA RESUELTA a path(s) + contenido concreto, lista
// para que la workspace (único escritor) la aplique. Todo es PURO: no toca disco ni reloj.
// Estructura (move/delete) y semántica (relaciones/status) quedan para E12-H06/H07.
// ---------------------------------------------------------------------------

/// Estado de **ocupación de paths** con el que se juzgan las colisiones de existencia — E28-H04.
///
/// Es la **única verdad** sobre «¿este path ya está ocupado?» del pipeline de plan (invariante #3 de
/// `CLAUDE.md`): la consultan tanto los guards de una sola operación de E28-H02
/// ([`normalize_create`], [`normalize_move`]) como el recorrido acumulado de
/// [`assert_sin_colisiones_intra_plan`]. Sin ella habría dos criterios de colisión —uno contra disco
/// y otro intra-plan— que podrían divergir.
///
/// Arranca del `FileMap` del workspace (lo que hay en disco) y va registrando el **delta** que las
/// operaciones ya aceptadas del plan producen: cada `create`/`move.to` **ocupa** su path, cada
/// `delete`/`move.from` lo **libera**. Solo guarda el delta, así que construirla y clonarla es
/// barato aunque el workspace tenga decenas de miles de documentos: no copia ni reparsea el mapa.
///
/// La comparación de paths es **byte a byte** (la clave del `FileMap`), sin normalizar caja ni forma
/// Unicode — fuera de alcance por decisión explícita, ver `decisiones/24-equivalencia-caja-unicode.md`.
#[derive(Debug, Clone)]
pub struct EstadoOcupacion<'a> {
    /// Lo que hay en disco al empezar a planificar (la fuente de verdad, invariante #1).
    base: &'a FileMap,
    /// Paths que el plan ha ocupado y que no estaban ocupados en `base`.
    ocupados: BTreeSet<RelPath>,
    /// Paths que el plan ha liberado (y que por tanto vuelven a poder reocuparse).
    liberados: BTreeSet<RelPath>,
}

impl<'a> EstadoOcupacion<'a> {
    /// Estado inicial: exactamente lo que hay en `files`, sin ninguna operación aplicada.
    pub fn nueva(files: &'a FileMap) -> Self {
        Self {
            base: files,
            ocupados: BTreeSet::new(),
            liberados: BTreeSet::new(),
        }
    }

    /// ¿`path` tiene documento en este estado (disco + lo que el plan lleva hecho)?
    pub fn esta_ocupado(&self, path: &RelPath) -> bool {
        if self.ocupados.contains(path) {
            return true;
        }
        self.base.contains_key(path) && !self.liberados.contains(path)
    }

    /// Marca `path` como ocupado (lo hace un `create` o el destino de un `move` ya aceptados).
    pub fn ocupar(&mut self, path: &RelPath) {
        self.liberados.remove(path);
        self.ocupados.insert(path.clone());
    }

    /// Marca `path` como libre (lo hace un `delete` o el origen de un `move` ya aceptados).
    pub fn liberar(&mut self, path: &RelPath) {
        self.ocupados.remove(path);
        self.liberados.insert(path.clone());
    }

    /// Registra el efecto de una operación **ya aceptada** sobre la ocupación.
    ///
    /// Un `move` libera su origen **antes** de ocupar su destino, para que el no-op `from == to`
    /// (E28-H02) deje el path ocupado, que es lo que de verdad queda en disco. Las operaciones que
    /// solo tocan contenido (`patch_frontmatter`, `replace_body`, `replace_text`, `edit_section`) no
    /// mueven la ocupación de ningún path.
    pub fn aplicar(&mut self, op: &NormalizedOperation) {
        match op {
            NormalizedOperation::Create { path, .. } => self.ocupar(path),
            NormalizedOperation::Move { from, to, .. } => {
                self.liberar(from);
                self.ocupar(to);
            }
            NormalizedOperation::Delete { path, .. } => self.liberar(path),
            NormalizedOperation::PatchFrontmatter { .. }
            | NormalizedOperation::ReplaceBody { .. }
            | NormalizedOperation::ReplaceText { .. }
            | NormalizedOperation::EditSection { .. } => {}
        }
    }

    /// Veredicto de colisión de un `create` sobre `path`: `Err` si el path ya está ocupado.
    ///
    /// # Errores
    /// [`CoreError::DocumentAlreadyExists`] con el path colisionado.
    pub fn asevera_libre(&self, path: &RelPath) -> Result<(), CoreError> {
        if self.esta_ocupado(path) {
            return Err(CoreError::DocumentAlreadyExists(path.clone()));
        }
        Ok(())
    }

    /// Veredicto de colisión del destino de un `move`: `Err` si `to` está ocupado por **otro**
    /// documento. `from == to` no es colisión (el destino coincide consigo mismo, E28-H02).
    ///
    /// # Errores
    /// [`CoreError::DocumentAlreadyExists`] con el destino colisionado.
    pub fn asevera_destino_libre(&self, from: &RelPath, to: &RelPath) -> Result<(), CoreError> {
        if from == to {
            return Ok(());
        }
        self.asevera_libre(to)
    }
}

/// Normaliza un `create`: el documento nuevo es **exactamente** lo que pidió el llamador.
///
/// - `frontmatter`: YAML **arbitrario y opcional**. `None` crea el `.md` **sin bloque de
///   frontmatter** (no con uno vacío); `Some(patch)` escribe exactamente esas claves, con sus tipos
///   YAML reales.
/// - `body`: si es `Some`, se usa tal cual; si es `None`, [`apply_normalized_ops`] genera un heading
///   por defecto a partir del título derivado (`§20.4`).
///
/// # Por qué la firma no tiene `doctype` ni `title` (E23-H02)
///
/// Hasta E23-H02 esta función tomaba `doctype: &str` y `title: Option<&str>` y los insertaba
/// **incondicionalmente** en el frontmatter, así que un `create` sin `type` producía en disco un
/// `type: ''` —una clave OKF vacía— y un `title` que nadie había pedido. Eso contradice de frente el
/// invariante 3 de `§20.2`: *el frontmatter **nunca** es obligatorio y sus claves **no** tienen
/// semántica impuesta*. `type` dejó de ser un campo privilegiado cuando `core::schema` murió en
/// E20-H03; el escritor era el último sitio donde seguía vivo.
///
/// El título **se deriva** (`frontmatter.title` → primer H1 → nombre del fichero, `model::derived_title`),
/// no se materializa como metadata. Quien quiera un `title:` explícito lo pasa en `frontmatter`.
///
/// # Errores
/// [`CoreError::DocumentAlreadyExists`] si `path` **ya tiene fichero** en el workspace (E28-H02).
///
/// Hasta E28-H02 esta función descartaba el `DocumentSet` (lo recibía como `_workspace`) y emitía
/// el `Create` sin condición alguna, así que un `create` sobre un documento existente producía un
/// plan aplicable que lo **pisaba sin un solo diagnóstico** (defecto A-05 del testbench homelab,
/// `decisiones/23-hallazgos-testbench-homelab.md`). El guard vive aquí, en la normalización pura:
/// es donde está la fuente de verdad (el `DocumentSet`) y donde el plan aún no existe.
///
/// Juzga contra el estado de partida del workspace. Cuando la operación va **dentro de un plan con
/// más operaciones**, el llamador usa [`normalize_create_en`] con el estado acumulado (E28-H04): el
/// criterio de colisión es el mismo ([`EstadoOcupacion`]), lo que cambia es contra qué estado se
/// aplica.
pub fn normalize_create(
    workspace: &DocumentSet,
    path: &RelPath,
    frontmatter: Option<FrontmatterPatch>,
    body: Option<String>,
) -> Result<NormalizedOperation, CoreError> {
    normalize_create_en(
        &EstadoOcupacion::nueva(workspace.files()),
        path,
        frontmatter,
        body,
    )
}

/// Como [`normalize_create`], pero juzgando la colisión contra el **estado de ocupación acumulado**
/// del change set en curso, no contra el workspace de partida — E28-H04.
///
/// Es la variante que usa el bucle de normalización de un plan con varias operaciones: así un
/// `create` ve el path que un `delete`/`move` anterior del mismo plan **liberó** (idioma legítimo) y,
/// al revés, ve ocupado el que otro `create`/`move` anterior ya reclamó (colisión intra-plan).
///
/// # Errores
/// [`CoreError::DocumentAlreadyExists`] si `path` ya está ocupado en `estado`.
pub fn normalize_create_en(
    estado: &EstadoOcupacion<'_>,
    path: &RelPath,
    frontmatter: Option<FrontmatterPatch>,
    body: Option<String>,
) -> Result<NormalizedOperation, CoreError> {
    estado.asevera_libre(path)?;
    Ok(NormalizedOperation::Create {
        path: path.clone(),
        frontmatter,
        body,
    })
}

/// Cuerpo (tras el frontmatter) del documento en `path`, o `Err(NormalizeTargetNotFound)` si el
/// path no tiene fichero en el workspace. Reusa `model::parse_file` (la misma verdad del core).
fn document_body(doc_set: &DocumentSet, path: &RelPath) -> Result<String, CoreError> {
    let raw = doc_set
        .files()
        .get(path)
        .ok_or_else(|| CoreError::NormalizeTargetNotFound(path.as_str().to_string()))?;
    Ok(model::parse_file(path.as_str(), raw).body)
}

/// Normaliza un `replace_text`: sustituye todas las ocurrencias literales de `find` por `replace`
/// en el CUERPO del documento (tras el frontmatter).
///
/// Si `expected_occurrences` es `Some(n)`, cuenta las coincidencias de `find` en el cuerpo y falla
/// con [`CoreError::ReplaceTextMismatch`] cuando el número real no es `n` (guarda contra ediciones
/// ciegas que tocan más —o menos— de lo previsto). Con `None` no se comprueba el conteo.
///
/// Devuelve [`NormalizedOperation::ReplaceBody`] con el cuerpo entero ya reescrito.
///
/// # Errores
/// - [`CoreError::NormalizeTargetNotFound`] si `path` no existe en el workspace.
/// - [`CoreError::ReplaceTextMismatch`] si el conteo no casa con `expected_occurrences`.
pub fn normalize_replace_text(
    doc_set: &DocumentSet,
    path: &RelPath,
    find: &str,
    replace: &str,
    expected_occurrences: Option<usize>,
) -> Result<NormalizedOperation, CoreError> {
    let body = document_body(doc_set, path)?;

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
    doc_set: &DocumentSet,
    path: &RelPath,
    heading_path: &[String],
    mode: EditSectionMode,
    content: &str,
) -> Result<NormalizedOperation, CoreError> {
    let body = document_body(doc_set, path)?;
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

/// `Err(NormalizeTargetNotFound)` si `path` no tiene fichero en el workspace. Punto único de
/// verificación de existencia para los normalizadores que solo necesitan saber que el documento
/// objetivo existe (sin leer su cuerpo).
fn ensure_exists(doc_set: &DocumentSet, path: &RelPath) -> Result<(), CoreError> {
    if doc_set.files().contains_key(path) {
        Ok(())
    } else {
        Err(CoreError::NormalizeTargetNotFound(
            path.as_str().to_string(),
        ))
    }
}

/// Normaliza un `patch_frontmatter`: aplica un merge-patch RFC 7386 al frontmatter de un documento
/// existente (E12-H05, reserva completada en E12-H08).
///
/// El `patch` (un [`FrontmatterPatch`]) ya es la forma normalizada del merge-patch — `Some(v)`
/// escribe/reemplaza, `None` **borra** la clave (`null` en el wire), clave ausente = no se toca —,
/// así que esta normalización se limita a **validar que el documento objetivo existe** y envolverlo
/// en [`NormalizedOperation::PatchFrontmatter`]. El merge real sobre el `Frontmatter` lo materializa
/// el aplicador ([`apply_normalized_ops`]) reusando la misma lógica que `DocumentSet::merge_frontmatter`
/// (un solo camino de merge-patch en el core, invariante #3). **Puro.**
///
/// # Errores
/// [`CoreError::NormalizeTargetNotFound`] si `path` no existe en el workspace.
pub fn normalize_patch_frontmatter(
    doc_set: &DocumentSet,
    path: &RelPath,
    patch: FrontmatterPatch,
) -> Result<NormalizedOperation, CoreError> {
    ensure_exists(doc_set, path)?;
    Ok(NormalizedOperation::PatchFrontmatter {
        path: path.clone(),
        patch,
    })
}

/// Normaliza un `replace_body`: reemplaza el cuerpo completo (tras el frontmatter) de un documento
/// existente por `body` (E12-H05, reserva completada en E12-H08).
///
/// Valida que el documento objetivo existe y devuelve [`NormalizedOperation::ReplaceBody`] con el
/// cuerpo nuevo tal cual; el frontmatter se conserva al aplicar. **Puro.**
///
/// # Errores
/// [`CoreError::NormalizeTargetNotFound`] si `path` no existe en el workspace.
pub fn normalize_replace_body(
    doc_set: &DocumentSet,
    path: &RelPath,
    body: String,
) -> Result<NormalizedOperation, CoreError> {
    ensure_exists(doc_set, path)?;
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
// Toda la verdad la da el core (`DocumentSet::backlinks`, `links::resolve`); nada de I/O.
// ---------------------------------------------------------------------------

/// Léxico **textual** del enlace inline `[texto](href "title")`, con el TEXTO en el grupo 1 y el
/// href en el grupo 2: es lo que permite "desenlazar" un enlace inline a texto plano.
///
/// No decide **a dónde apunta** un enlace —eso lo hace [`links::resolve`], la única verdad de
/// resolución del core (E17-H02)—, solo dónde está escrito en el cuerpo para poder sustituirlo.
/// Es una limitación conocida y acotada: solo alcanza a los enlaces inline, no a las definiciones
/// de un enlace de referencia. Desde E21-H03 la reescritura de `move_document` NO la usa —va por el
/// `span` de bytes de cada [`crate::types::ResolvedLink`] ([`retarget_body_links`]), que sí cubre las
/// definiciones `[id]: destino`—; esta regex queda solo para el "desenlazado" de `delete remove_links`
/// ([`remove_inline_links`]), cuya política los tests fijan sobre enlaces inline.
static LINK_REWRITE_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"\[([^\]]*)\]\(([^)\s]+)(?:\s+"[^"]*")?\)"#).unwrap());

/// Qué hacer con un enlace entrante cuyo destino resuelto es el documento afectado.
enum LinkAction<'a> {
    /// Reescribir el href para que apunte a `to` (usado por `move` con `rewriteInboundLinks`).
    Retarget(&'a RelPath),
    /// Quitar el enlace dejando solo su texto (usado por `delete` con `remove_links`).
    Remove,
}

/// Divide un href crudo en `(base, sufijo)`, donde el sufijo empieza en el primer `#` (fragmento) o
/// `?` (query). [`links::resolve`] ignora ese sufijo al resolver el path; al reescribir lo
/// conservamos.
fn split_href_suffix(href: &str) -> (&str, &str) {
    match href.find(['#', '?']) {
        Some(i) => (&href[..i], &href[i..]),
        None => (href, ""),
    }
}

/// Construye un href relativo desde el directorio de `source_path` hasta `target` (ambos paths de
/// workspace normalizados), reusando el álgebra de directorios del core (`model::dir_of`). Es puro
/// cálculo de rutas: sin `..` cuando comparten prefijo, con `./` cuando el destino queda en el mismo
/// directorio (para que se lea inequívocamente como relativo, como hacía el prototipo).
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

/// Reescribe el CUERPO de un documento entrante aplicando `action` SOLO a los enlaces markdown cuyo
/// href resuelve al documento `target`. El resto de enlaces y de texto queda intacto — nunca se
/// toca un enlace que apunte a otro sitio.
///
/// La **decisión** («¿este href apunta al documento afectado?») la toma [`links::resolve`] contra
/// el `inventory` del workspace, la única verdad de resolución del core (invariante #3): así un
/// href externo, un anchor o un destino que escapa del workspace nunca se confunden con el target,
/// y un enlace al documento afectado se reconoce escrito como se escriba (`./x.md`, `../a/x.md`,
/// `/x.md`, con `%20`, con fragmento…).
///
/// Dos caminos según la acción:
/// - [`LinkAction::Retarget`] (`move`): reescribe el destino **por el `span` de bytes** de cada
///   enlace (E21-H03) — así alcanza también las **definiciones** de un enlace de referencia
///   (`[id]: destino`), no solo los inline.
/// - [`LinkAction::Remove`] (`delete remove_links`): "desenlaza" a texto plano; la política se fija
///   sobre enlaces **inline**, así que se resuelve con la sustitución textual de [`LINK_REWRITE_RE`].
fn rewrite_body_links(
    body: &str,
    source_path: &RelPath,
    target: &RelPath,
    action: &LinkAction,
    inventory: &Inventory,
) -> String {
    match action {
        LinkAction::Retarget(to) => retarget_body_links(body, source_path, target, to, inventory),
        LinkAction::Remove => remove_inline_links(body, source_path, target, inventory),
    }
}

/// Reescribe, **por el `span` de bytes del destino**, cada enlace del cuerpo que resuelve a `target`
/// para que apunte a `to`, conservando label y fragmento (E21-H03, `§20.11`).
///
/// A diferencia de la sustitución por regex, el `span` de [`links::extract_links`]/[`links::resolve`]
/// (E17-H01) apunta al destino **tal como está escrito** —en un enlace de referencia, dentro de su
/// definición `[id]: destino`—, así que la reescritura alcanza también los enlaces de referencia, no
/// solo los inline. El nuevo destino lo calcula [`retarget_href`] a partir del texto del span
/// (`body[span]`, que incluye su `#fragmento`/`?query`), preservando estilo absoluto/relativo y sufijo.
///
/// Los spans se aplican de **mayor a menor offset** para que cada sustitución no invalide los de los
/// siguientes, y se **deduplican**: varios usos de la misma definición de referencia comparten el
/// mismo span y deben reescribirse una sola vez.
fn retarget_body_links(
    body: &str,
    source_path: &RelPath,
    target: &RelPath,
    to: &RelPath,
    inventory: &Inventory,
) -> String {
    let mut spans: Vec<std::ops::Range<usize>> = links::extract_links(body)
        .iter()
        .map(|raw| links::resolve(raw, source_path, inventory))
        .filter(|link| link.target.internal_path() == Some(target))
        .map(|link| link.span.clone())
        .collect();
    spans.sort_by_key(|span| std::cmp::Reverse(span.start));
    spans.dedup();

    let mut out = body.to_string();
    for span in spans {
        let nuevo = retarget_href(&out[span.clone()], source_path.as_str(), to);
        out.replace_range(span, &nuevo);
    }
    out
}

/// Recalcula, **por el `span` de bytes del destino**, los enlaces del cuerpo de un documento que se
/// mueve de `from` a `to`, para que sigan apuntando a los mismos ficheros desde la ubicación nueva
/// (E23-H03, `§20.11`).
///
/// A diferencia de [`retarget_body_links`] —que cambia **el destino** de los enlaces que apuntan a
/// un documento movido—, esta cambia **el origen**: el destino de cada enlace no se mueve, se mueve
/// quien lo escribe, y un href relativo se interpreta desde el directorio de su documento.
///
/// Qué se toca y qué no, decidido por [`links::resolve`] contra el `inventory` (invariante #3, la
/// única verdad de resolución del core):
/// - **Se recalcula** todo enlace con path interno: [`LinkTarget::Document`],
///   [`LinkTarget::WorkspaceFile`] (un enlace a código: `../src/token.rs` también se rompe al mover)
///   y [`LinkTarget::Missing`] (un enlace ya roto se mantiene roto **apuntando al mismo sitio**, en
///   vez de pasar a señalar un fichero distinto — no se inventa destino, se preserva la intención).
/// - **No se tocan**: [`LinkTarget::ExternalUri`] (`https://…`), [`LinkTarget::SelfAnchor`]
///   (`#seccion`, que sigue siendo del propio documento) ni [`LinkTarget::EscapesWorkspace`] (no hay
///   path que recalcular).
/// - Un href **raíz-absoluto** (`/docs/x.md`) no depende del origen: [`retarget_href`] conserva ese
///   estilo y produce el mismo texto, así que queda intacto de hecho.
///
/// Nota sobre `internal_path()`: **no** sirve aquí, porque excluye `WorkspaceFile` y los `Missing`
/// no-markdown. Usar ese helper dejaría los enlaces a código del documento movido rotos, que es el
/// caso que más duele en un repo de software.
fn rebase_body_links(body: &str, from: &RelPath, to: &RelPath, inventory: &Inventory) -> String {
    // (span, destino) de cada enlace recalculable, de mayor a menor offset para que una sustitución
    // no invalide los spans siguientes; deduplicado porque varios usos de una misma definición de
    // referencia comparten span.
    //
    // Las definiciones de referencia entran por su propia vía (`links::reference_definitions`)
    // porque una definición SIN usar no genera evento en el parser y `extract_links` —que emite por
    // uso— no la vería. No es una arista del grafo, pero sí es un path relativo escrito por el
    // usuario: dejarla sin recalcular la haría apuntar a otro sitio en silencio. Los spans de las
    // usadas coinciden con los de su definición, así que el `dedup_by` de abajo los funde.
    let mut objetivos: Vec<(std::ops::Range<usize>, Destino)> = links::extract_links(body)
        .into_iter()
        .chain(links::reference_definitions(body))
        .map(|raw| links::resolve(&raw, from, inventory))
        .filter_map(|link| {
            let destino = match &link.target {
                LinkTarget::Document(p) | LinkTarget::WorkspaceFile(p) | LinkTarget::Missing(p) => {
                    Destino::Fichero(p.clone())
                }
                // Un directorio (`[volver](../)`) también depende del origen: si el documento
                // cambia de profundidad, ese href pasa a señalar otro directorio en silencio.
                LinkTarget::WorkspaceDirectory(d) => Destino::Directorio(d.clone()),
                LinkTarget::ExternalUri(_)
                | LinkTarget::SelfAnchor(_)
                | LinkTarget::EscapesWorkspace => return None,
            };
            Some((link.span.clone(), destino))
        })
        .collect();
    objetivos.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
    objetivos.dedup_by(|a, b| a.0 == b.0);

    let mut out = body.to_string();
    for (span, destino) in objetivos {
        // El href se recalcula como si lo escribiera el documento YA en `to`: mismo destino, origen
        // nuevo. Se conserva el sufijo `#fragmento`/`?query` y el estilo absoluto.
        let nuevo = match &destino {
            Destino::Fichero(p) => retarget_href(&out[span.clone()], to.as_str(), p),
            Destino::Directorio(d) => {
                redirigir_href_a_directorio(&out[span.clone()], to, d.as_ref())
            }
        };
        out.replace_range(span, &nuevo);
    }
    out
}

/// A dónde apunta un enlace que hay que recalcular al mover su documento — E23-H11.
enum Destino {
    /// Un fichero del workspace (documento, código, o un destino ausente).
    Fichero(RelPath),
    /// Un directorio; `None` es la raíz del workspace.
    Directorio(Option<RelPath>),
}

/// Recalcula un href que apunta a un **directorio** para que siga apuntando al mismo directorio
/// desde `to` — E23-H11. Conserva el sufijo `#fragmento`/`?query` y el estilo raíz-absoluto.
///
/// Es el gemelo de [`retarget_href`] para destinos sin fichero: [`relative_href`] no sirve porque
/// trata el último segmento como nombre de fichero, y aquí **todos** los segmentos son directorios.
/// El resultado termina en `/` (o es `./`) para que siga leyéndose como un directorio.
fn redirigir_href_a_directorio(old_href: &str, to: &RelPath, destino: Option<&RelPath>) -> String {
    let (base, suffix) = split_href_suffix(old_href);
    let destino_str = destino.map(RelPath::as_str).unwrap_or("");
    let new_base = if base.starts_with('/') {
        format!("/{destino_str}")
    } else {
        relative_dir_href(to.as_str(), destino_str)
    };
    format!("{new_base}{suffix}")
}

/// El href relativo desde el directorio de `source_path` hasta el **directorio** `target_dir`
/// (cadena vacía = la raíz del workspace) — E23-H11.
///
/// Termina siempre en `/`, y es `./` cuando origen y destino son el mismo directorio: así el href
/// sigue siendo inequívocamente una navegación a directorio y no se confunde con un fichero.
fn relative_dir_href(source_path: &str, target_dir: &str) -> String {
    let from_dir = model::dir_of(source_path);
    let from_parts: Vec<&str> = from_dir.split('/').filter(|s| !s.is_empty()).collect();
    let to_parts: Vec<&str> = target_dir.split('/').filter(|s| !s.is_empty()).collect();

    let mut common = 0;
    while common < from_parts.len()
        && common < to_parts.len()
        && from_parts[common] == to_parts[common]
    {
        common += 1;
    }

    let subidas = from_parts.len() - common;
    let bajadas = &to_parts[common..];
    if subidas == 0 && bajadas.is_empty() {
        return "./".to_string();
    }
    let mut segs: Vec<&str> = vec![".."; subidas];
    segs.extend_from_slice(bajadas);
    format!("{}/", segs.join("/"))
}

/// "Desenlaza" a texto plano los enlaces **inline** cuyo href resuelve a `target`, dejando su texto
/// visible (usado por `delete remove_links`). Un enlace a otro destino queda intacto; la decisión la
/// da [`links::resolve`] (invariante #3), no el patrón textual.
fn remove_inline_links(
    body: &str,
    source_path: &RelPath,
    target: &RelPath,
    inventory: &Inventory,
) -> String {
    LINK_REWRITE_RE
        .replace_all(body, |caps: &Captures| {
            let full = caps.get(0).map_or("", |m| m.as_str());
            let text = &caps[1];
            let href = &caps[2];
            let crudo = RawLink {
                href: href.to_string(),
                text: text.to_string(),
                span: 0..0,
                kind: LinkKind::Inline,
            };
            let resuelto = links::resolve(&crudo, source_path, inventory);
            if resuelto.target.internal_path() != Some(target) {
                return full.to_string();
            }
            text.to_string()
        })
        .into_owned()
}

/// Normaliza un `move` (rename de un documento) — E12-H06, E23-H03.
///
/// Produce siempre un [`NormalizedOperation::Move`] `{ from, to }`, seguido de un
/// [`NormalizedOperation::ReplaceBody`] sobre el **propio documento movido** con sus enlaces
/// salientes recalculados desde la ubicación nueva (ver `rebase_body_links`). Si
/// `rewrite_inbound_links`, añade además, por cada documento que enlaza a `from` (los entrantes de
/// `DocumentSet::backlinks(from)`, invariante #3 de `CLAUDE.md`), otro `ReplaceBody` con su enlace
/// apuntando a `to` (ver `rewrite_body_links` / `retarget_href`).
///
/// # Por qué hay que recalcular los salientes (E23-H03)
///
/// Un enlace relativo se interpreta **desde el directorio del documento que lo contiene**, así que
/// mover el documento cambia a dónde apunta cada uno de sus hrefs relativos sin tocar ni un byte de
/// su texto. Hasta E23-H03 solo se reescribían los entrantes, y el resultado era que mover una nota
/// que enlazaba a sus vecinas la dejaba con todos los enlaces rotos — lo que el gate diferencial de
/// E20-H04 detectaba como «errores nuevos», haciendo el `move` **imposible** (`canApply: false`).
/// Mover una nota que enlaza a sus vecinas es el caso normal en una base de conocimiento.
///
/// El `ReplaceBody` va **después** del `Move` y sobre `to`: cuando se aplica, el documento ya está
/// en su ubicación nueva. Se emite solo si el recálculo cambia algo, para no meter una op no-op (ni
/// un path afectado de más) en los planes donde el documento no tiene salientes relativos.
///
/// # Errores
/// - [`CoreError::DocumentAlreadyExists`] si el destino `to` **ya tiene fichero** en el workspace
///   (E28-H02): el rename publicaría encima del documento existente. `from == to` **no** es una
///   colisión —el destino coincide consigo mismo, no con un documento distinto— y sigue siendo el
///   no-op válido de siempre.
/// - [`CoreError::NormalizeTargetNotFound`] si `from` (o algún documento entrante) no tiene fichero
///   en el workspace. La dirección contraria conserva su código: un origen inexistente es
///   `DOCUMENT_NOT_FOUND`, no una colisión de destino.
///
/// Igual que [`normalize_create`], juzga el destino contra el estado de partida; dentro de un plan
/// con más operaciones el llamador usa [`normalize_move_en`] con el estado acumulado (E28-H04).
pub fn normalize_move(
    doc_set: &DocumentSet,
    from: &RelPath,
    to: &RelPath,
    rewrite_inbound_links: bool,
) -> Result<Vec<NormalizedOperation>, CoreError> {
    normalize_move_en(
        &EstadoOcupacion::nueva(doc_set.files()),
        doc_set,
        from,
        to,
        rewrite_inbound_links,
    )
}

/// Como [`normalize_move`], pero juzgando la ocupación del destino contra el **estado acumulado**
/// del change set en curso — E28-H04.
///
/// El `doc_set` se sigue necesitando aparte: de ahí salen el cuerpo del documento movido y sus
/// enlaces entrantes (contenido), que [`EstadoOcupacion`] no guarda —solo lleva qué paths están
/// ocupados—.
///
/// # Errores
/// Los mismos que [`normalize_move`], con la colisión de destino juzgada contra `estado`.
pub fn normalize_move_en(
    estado: &EstadoOcupacion<'_>,
    doc_set: &DocumentSet,
    from: &RelPath,
    to: &RelPath,
    rewrite_inbound_links: bool,
) -> Result<Vec<NormalizedOperation>, CoreError> {
    // E28-H02/H04: el destino no puede estar ocupado por OTRO documento —ni en disco, ni por una
    // operación anterior del mismo plan—. Mover un documento sobre sí mismo (`from == to`) es un
    // no-op, no una colisión.
    estado.asevera_destino_libre(from, to)?;
    let mut ops = vec![NormalizedOperation::Move {
        from: from.clone(),
        to: to.clone(),
        rewrite_inbound_links,
    }];

    // Los salientes del documento movido, recalculados desde `to`.
    let cuerpo = document_body(doc_set, from)?;
    let rebasado = rebase_body_links(&cuerpo, from, to, doc_set.inventory());
    if rebasado != cuerpo {
        ops.push(NormalizedOperation::ReplaceBody {
            path: to.clone(),
            body: rebasado,
        });
    }

    if rewrite_inbound_links {
        // Un origen que enlaza VARIAS veces al documento movido aparece una vez por enlace en
        // `inbound` (E17-H04): se reescribe su cuerpo UNA sola vez —`rewrite_body_links` ya
        // sustituye todos sus enlaces al target— y no se emiten dos `ReplaceBody` del mismo path.
        let mut vistos: BTreeSet<RelPath> = BTreeSet::new();
        for link in doc_set.backlinks(from).inbound {
            let source = link.from;
            if !vistos.insert(source.clone()) {
                continue;
            }
            let body = document_body(doc_set, &source)?;
            let new_body = rewrite_body_links(
                &body,
                &source,
                from,
                &LinkAction::Retarget(to),
                doc_set.inventory(),
            );
            ops.push(NormalizedOperation::ReplaceBody {
                path: source,
                body: new_body,
            });
        }
    }

    Ok(ops)
}

/// Normaliza un `delete` (borrado de un documento) según la política ante enlaces entrantes — E12-H06.
///
/// - [`InboundLinksPolicy::Reject`] (el default): si el documento tiene entrantes
///   (`DocumentSet::backlinks(path).inbound`), falla con [`CoreError::InboundLinksExist`] (wire
///   `"INBOUND_LINKS_EXIST"`); sin entrantes, devuelve solo el [`NormalizedOperation::Delete`].
/// - [`InboundLinksPolicy::RemoveLinks`]: devuelve el `Delete` MÁS, por cada entrante, un
///   [`NormalizedOperation::ReplaceBody`] que quita el enlace al documento borrado dejando su texto
///   plano (ver `rewrite_body_links`).
///
/// Ya no hay tercer camino: `Retarget`/`CreateStub` se **retiraron** en E23-H05 (ver
/// [`InboundLinksPolicy`]), donde llevaban desde E12-H06 aceptándose sin ejecutarse.
pub fn normalize_delete(
    doc_set: &DocumentSet,
    path: &RelPath,
    policy: InboundLinksPolicy,
) -> Result<Vec<NormalizedOperation>, CoreError> {
    let inbound = doc_set.backlinks(path).inbound;
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
            let mut vistos: BTreeSet<RelPath> = BTreeSet::new();
            for link in inbound {
                let source = link.from;
                if !vistos.insert(source.clone()) {
                    continue;
                }
                let body = document_body(doc_set, &source)?;
                let new_body = rewrite_body_links(
                    &body,
                    &source,
                    path,
                    &LinkAction::Remove,
                    doc_set.inventory(),
                );
                ops.push(NormalizedOperation::ReplaceBody {
                    path: source,
                    body: new_body,
                });
            }
            Ok(ops)
        }
    }
}

// ---------------------------------------------------------------------------
// NOTA E23-H11 — `normalize_apply_fix` (E12-H07) se RETIRÓ junto con la operación `apply_fix`.
//
// E21-H01 ya había retirado las operaciones SEMÁNTICAS (`add_relation`/`remove_relation`/
// `transition_status`) y sus helpers de relación: el modelo es universal (`§20.11`), una relación es
// un enlace Markdown y un estado es una propiedad arbitraria del frontmatter, así que ambas se
// expresan con las operaciones universales (una transición es un `PatchFrontmatter{status}`).
// Quedaba `apply_fix`, que desde que E20-H03 retiró `core::schema` —y con él el único productor de
// `Fix`— devolvía SIEMPRE `CoreError::FixNotFound`: una operación anunciada en el enum del schema
// que no podía tener éxito nunca. El lado de LECTURA de los fixes (`Fix`, `Check.fixes`,
// `includeSuggestedFixes`) sigue intacto. Análisis en `docs/history/PROPUESTA_FIXES.md`.
// ---------------------------------------------------------------------------

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
/// (`model::build_raw`/`model::parse_file` y `document_set::apply_patch`, invariante #3 de `CLAUDE.md`):
/// - [`NormalizedOperation::Create`]: escribe el `.md` nuevo con el frontmatter del patch aplicado
///   sobre uno vacío y el cuerpo dado (o un heading por defecto `# {título}` si `body` es `None`).
/// - [`NormalizedOperation::PatchFrontmatter`]: aplica el merge-patch al frontmatter existente,
///   conservando el cuerpo (mismo camino que `DocumentSet::merge_frontmatter`).
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

/// Comprueba que la secuencia `ops` no contiene ninguna **colisión intra-plan** de existencia —
/// E28-H04.
///
/// Los guards de E28-H02 (`normalize_create`/`normalize_move`) juzgan **una** operación contra el
/// `DocumentSet` con el que empezó a planificar. Cuando el plan tiene varias operaciones que tocan
/// paths relacionados, ese `DocumentSet` deja de ser cierto a partir de la segunda: ninguna op ve el
/// efecto de las que la preceden. Esta función cierra el hueco recorriendo `ops` **en orden** sobre
/// un estado de ocupación acumulado que arranca de `files`: cada `Create`/`Move.to` aceptado
/// **ocupa** su path; cada `Delete`/`Move.from` aceptado lo **libera**.
///
/// Por eso `[delete X, create X]` y `[move A→B, create A]` son legítimos (liberar y reocupar), y
/// `[move a→final, move b→final]`, `[create X, move b→X]` y `[create X, create X]` no lo son: el
/// segundo miembro de cada par publicaría encima de lo que dejó el primero.
///
/// El criterio de colisión es **el mismo** que el de los guards de una sola operación: los tres
/// caminos consultan [`EstadoOcupacion`] (invariante #3 de `CLAUDE.md`, una sola verdad computada).
/// Lo único que cambia es contra qué estado se pregunta — el de partida o el acumulado.
///
/// # Errores
/// [`CoreError::DocumentAlreadyExists`] con el path colisionado, en cuanto la primera operación
/// intenta ocupar un path que el estado acumulado ya tiene ocupado.
pub fn assert_sin_colisiones_intra_plan(
    files: &FileMap,
    ops: &[NormalizedOperation],
) -> Result<(), CoreError> {
    let mut estado = EstadoOcupacion::nueva(files);
    for op in ops {
        match op {
            NormalizedOperation::Create { path, .. } => estado.asevera_libre(path)?,
            NormalizedOperation::Move { from, to, .. } => estado.asevera_destino_libre(from, to)?,
            // Sin comodín a propósito: las operaciones de contenido no ocupan ningún path, y si
            // algún día entra una variante nueva que sí lo haga, el compilador obliga a decidir aquí
            // si colisiona — en vez de dejarla pasar en silencio hasta el disco.
            NormalizedOperation::Delete { .. }
            | NormalizedOperation::PatchFrontmatter { .. }
            | NormalizedOperation::ReplaceBody { .. }
            | NormalizedOperation::ReplaceText { .. }
            | NormalizedOperation::EditSection { .. } => {}
        }
        estado.aplicar(op);
    }
    Ok(())
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
            // E23-H02: `None` ⇒ documento SIN bloque de frontmatter. `build_raw(None, …)` devuelve
            // el cuerpo verbatim, así que el `.md` es exactamente lo que pidió el llamador — nada de
            // un `---\n---` vacío, y desde luego nada de `type`/`title` inyectados.
            let fm = frontmatter.as_ref().map(|patch| {
                let mut map = serde_yaml::Mapping::new();
                crate::document_set::apply_patch(&mut map, patch.clone());
                ParsedFrontmatter::from_mapping(map)
            });
            let body = body.clone().unwrap_or_else(|| {
                // Sin cuerpo: la cadena de `derived_title` se resuelve por el `title` del
                // frontmatter (si el llamador lo pidió) o, en su defecto, por el nombre del fichero.
                let title = model::derived_title(fm.as_ref(), "", path);
                format!("# {title}\n")
            });
            files.insert(path.clone(), model::build_raw(fm.as_ref(), &body));
        }
        NormalizedOperation::PatchFrontmatter { path, patch } => {
            // Por el patch QUIRÚRGICO (E16-H04, invariante #3: una sola verdad de patcheo): las
            // líneas que el patch no toca sobreviven byte a byte, y un frontmatter ilegible hace
            // fallar la operación en vez de reconstruirse encima.
            let raw = files.get(path).cloned().unwrap_or_default();
            let patched = model::patch_frontmatter(&raw, patch)?;
            files.insert(path.clone(), patched.raw);
        }
        NormalizedOperation::ReplaceBody { path, body } => {
            // Patch QUIRÚRGICO del cuerpo (E31-H02, `decisiones §26`), simétrico al del brazo
            // `PatchFrontmatter` de arriba: se sustituye el cuerpo y la CABECERA sobrevive byte a
            // byte, sin pasar por ningún serializador. Eso conserva de una vez —y por
            // construcción, no por ramas que se coordinan— las tres cosas que la reconstrucción
            // anterior perdía: el estilo del YAML (`tags: [a, b]` no se vuelve lista en bloque),
            // el separador exacto, y el bloque ILEGIBLE, cuyo borrado era pérdida de datos.
            //
            // Lo que ya cumplía la reconstrucción y sigue cumpliéndose, ahora gratis: la
            // **ausencia** de frontmatter se conserva (E23-H03 — un `README.md` sin bloque no
            // recibe un `---\n{}\n---`, que era lo que corrompía a los enlazantes en cada `move`
            // con `rewriteInboundLinks`) y el BOM UTF-8 tampoco se lo come nadie (E24-H01),
            // porque queda a la izquierda del corte.
            let raw = files.get(path).cloned().unwrap_or_default();
            files.insert(
                path.clone(),
                model::replace_body_preservando_cabecera(&raw, body),
            );
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

/// El documento que una operación normalizada **nombra**: su objetivo, y para un `move` su
/// **destino** (que es donde queda el contenido tras aplicarla).
///
/// Lo consume el `noOpOperations` del plan (E31-H02, `decisiones §26`) para decir de QUÉ documento
/// habla una operación sin efecto. Match exhaustivo **sin comodín**, con el mismo criterio que
/// [`assert_sin_colisiones_intra_plan`]: si algún día entra una variante nueva, el compilador obliga
/// a decidir aquí cuál es su documento en vez de dejarla caer en un `_` que devolvería el path
/// equivocado en silencio.
pub fn op_target_path(op: &NormalizedOperation) -> &RelPath {
    match op {
        NormalizedOperation::Create { path, .. }
        | NormalizedOperation::PatchFrontmatter { path, .. }
        | NormalizedOperation::ReplaceBody { path, .. }
        | NormalizedOperation::ReplaceText { path, .. }
        | NormalizedOperation::EditSection { path, .. }
        | NormalizedOperation::Delete { path, .. } => path,
        NormalizedOperation::Move { to, .. } => to,
    }
}

/// Nombre de la variante en el vocabulario del wire (snake_case), tal como lo nombra el
/// `inputSchema` de `change_plan`.
///
/// Lo consumen el mensaje de [`CoreError::OperationNotApplicable`] y, desde E31-H02, el
/// `noOpOperations` del plan: **una sola verdad** para los nombres de operación (invariante #4), de
/// modo que añadir una variante no pueda dejar dos catálogos discrepando.
pub fn op_variant_name(op: &NormalizedOperation) -> String {
    match op {
        NormalizedOperation::Create { .. } => "create",
        NormalizedOperation::PatchFrontmatter { .. } => "patch_frontmatter",
        NormalizedOperation::ReplaceBody { .. } => "replace_body",
        NormalizedOperation::EditSection { .. } => "edit_section",
        NormalizedOperation::ReplaceText { .. } => "replace_text",
        NormalizedOperation::Move { .. } => "move",
        NormalizedOperation::Delete { .. } => "delete",
    }
    .to_string()
}
