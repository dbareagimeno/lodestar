//! Staging transaccional (E13-H01, `ARCHITECTURE.md §19.5`, `REFACTOR §5.2`): materializa el
//! resultado hipotético de un [`ChangeSet`] en un directorio desechable bajo
//! `.lodestar/runtime/staging/` y lo valida contra la política de conformidad **antes** de
//! publicarlo, sin tocar jamás el conocimiento `.md` canónico.
//!
//! Es el primer eslabón de la publicación recuperable: la escritura real del canónico por el
//! único escritor (temp+rename) llega en E13-H05; aquí solo se prepara y se valida el resultado.
//!
//! Runtime, no canónico: el árbol de staging vive bajo `.lodestar/runtime/`, que el walker de
//! conocimiento (`discovery::discover`) y el watcher ya excluyen (E9-H06) y `WorkspaceRevision`
//! ignora (E10-H03). Por eso se escribe con `std::fs::write` normal — el protocolo atómico del
//! único-escritor (`io::write_atomic`) protege los `.md` canónicos, no este scratch desechable.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use lodestar_core::plan;
use lodestar_core::types::{ChangeSet, ChangeSetId, CheckCode, FileMap, RelPath, Severity};
use lodestar_core::DocumentSet;

use crate::config::ValidationSection;
use crate::error::WorkspaceError;
use crate::Workspace;

/// Directorio de staging materializado: contiene el árbol `.md` resultante de aplicar un
/// [`ChangeSet`] sobre el canónico, bajo `.lodestar/runtime/staging/<changeSetId saneado>/`.
///
/// La limpieza NO es automática: [`Workspace::validate_staging`] borra el directorio cuando el
/// resultado no es conforme; el flujo de publicación (E13-H05) lo consumirá y limpiará al
/// terminar. Mientras tanto persiste en disco (es el propósito del staging).
pub struct StagingDir {
    path: PathBuf,
}

impl StagingDir {
    /// El directorio raíz del árbol de staging materializado (bajo `.lodestar/runtime/staging/`).
    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// Nombre de directorio saneado para el staging de un `changeSetId` (E13-H01), siguiendo el mismo
/// criterio que E12-H09 con los planes: se descarta el prefijo `changeset:` (su `:` es hostil a
/// nombres de fichero en Windows) y se neutraliza cualquier carácter de path residual. El
/// resultado es determinista y basta para la trazabilidad del directorio.
fn staging_dir_name(id: &ChangeSetId) -> String {
    let stripped = id.0.strip_prefix("changeset:").unwrap_or(&id.0);
    stripped
        .chars()
        .map(|c| match c {
            ':' | '/' | '\\' => '_',
            other => other,
        })
        .collect()
}

/// Lee todos los `.md` bajo `root` a un [`FileMap`] con claves relativas a `root`.
///
/// Recorrido propio (no `discovery::discover`) a propósito: el árbol de staging vive dentro de
/// `.lodestar/`, que las reglas de `.gitignore` del workspace marcan como ignorado — un walker que
/// respete git ignoraría el árbol entero. Aquí solo interesa el contenido literal del staging.
fn read_tree(root: &Path) -> Result<FileMap, WorkspaceError> {
    fn walk(dir: &Path, root: &Path, files: &mut FileMap) -> Result<(), WorkspaceError> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                walk(&path, root, files)?;
            } else if path.extension().is_some_and(|e| e == "md") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Ok(rp) = RelPath::new(&rel) {
                    files.insert(rp, std::fs::read_to_string(&path)?);
                }
            }
        }
        Ok(())
    }
    let mut files = FileMap::new();
    walk(root, root, &mut files)?;
    Ok(files)
}

impl Workspace {
    /// Materializa en staging el resultado hipotético de aplicar `change_set` sobre el canónico
    /// (E13-H01). Carga el `FileMap` canónico, computa el resultado con
    /// [`plan::apply_normalized_ops`] (reutilizando la única canonicalización del core) y escribe
    /// **todos** los ficheros resultantes bajo `.lodestar/runtime/staging/<changeSetId saneado>/`,
    /// espejando su ruta relativa. Nunca toca los `.md` canónicos.
    ///
    /// Si ya existía un staging con el mismo id (reintento), se limpia antes de reescribir para
    /// que el árbol refleje exactamente el resultado actual.
    ///
    /// # Errores
    /// - [`WorkspaceError::Core`] si `change_set` trae una operación no terminal (violación del
    ///   pipeline de normalización; nunca una entrada de agente).
    /// - [`WorkspaceError::Io`] si falla la lectura del canónico o la escritura del staging.
    pub fn materialize_staging(
        &self,
        change_set: &ChangeSet,
    ) -> Result<StagingDir, WorkspaceError> {
        let canonical = self.discover_files()?;
        let result = plan::apply_normalized_ops(&canonical, &change_set.operations)?;
        self.materialize_staging_result(&change_set.id, &result)
    }

    /// Materializa en staging un `FileMap` resultado **ya computado**, bajo
    /// `.lodestar/runtime/staging/<changeSetId saneado>/`. Es el núcleo de
    /// [`Workspace::materialize_staging`] extraído para que la transacción (E13-H08) materialice el
    /// resultado ya computado en lugar de recomputarlo desde las ops — así el árbol de staging (y
    /// por tanto lo que se valida y se publica) refleja exactamente el mismo mapa. Nunca toca los
    /// `.md` canónicos.
    ///
    /// Nota histórica: la escisión nació en E13-H11 para el plan *aumentado* con la auto-regeneración
    /// de `index`/`tags` (D6a), retirada en E15-H02; la escisión sobrevive por la propiedad de
    /// arriba.
    ///
    /// Si ya existía un staging con el mismo id (reintento), se limpia antes de reescribir.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla la creación de directorios o la escritura del staging.
    pub(crate) fn materialize_staging_result(
        &self,
        id: &ChangeSetId,
        result: &FileMap,
    ) -> Result<StagingDir, WorkspaceError> {
        let dir = self
            .root
            .join(".lodestar")
            .join("runtime")
            .join("staging")
            .join(staging_dir_name(id));

        // Reintento idempotente: parte de un directorio limpio.
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;

        for (rel, content) in result {
            let target = dir.join(rel.as_str());
            if let Some(parent) = target.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&target, content)?;
        }

        Ok(StagingDir { path: dir })
    }

    /// Valida un staging materializado contra la **política de cambios** antes de publicar (E13-H01,
    /// gate diferencial de E20-H04). Compara el conjunto de **errores** del resultado (el árbol de
    /// staging) contra el del canónico **actual** y decide con `transactions.rejectNewErrors`/
    /// `allowExistingErrors` (`ARCHITECTURE.md §20.9`, `REFACTOR_PHASE_2 §Fase 10`):
    ///
    /// - `rejectNewErrors: true` (default) — el cambio **no** puede **introducir** errores nuevos:
    ///   se rechaza si el resultado tiene algún error que el canónico no tenía.
    /// - `allowExistingErrors: true` (default) — se **permite** aplicar sobre un workspace que ya
    ///   tiene errores; una reparación parcial que no añade errores pasa aunque queden otros sin
    ///   arreglar. `allowExistingErrors: false` reinstaura el gate estricto («cualquier error en el
    ///   resultado rechaza»).
    ///
    /// Al rechazar, **aborta sin tocar el canónico** y limpia el directorio de staging, devolviendo
    /// [`WorkspaceError::NonconformantResult`]. Si pasa, devuelve `Ok(())` y el staging queda listo
    /// para publicarse.
    ///
    /// **Severidad configurable**: qué diagnóstico cuenta como error lo decide
    /// [`ValidationSection::effective_severity`] (misma reclasificación por familia que
    /// `App::knowledge_check`, invariante #3), no la severidad hardcodeada — así un
    /// `validation.danglingDocumentLinks: ignore` no bloquea ni cuenta.
    ///
    /// **Identidad de diagnóstico** (para distinguir «error nuevo» de «error preexistente que sigue»):
    /// `(código, targets, related)` — código + documento culpable + destino relacionado. No depende
    /// del `id` efímero: un `LINK-TARGET-MISSING` de `roto.md` que existía antes y sigue después tiene
    /// la misma clave (no es nuevo); uno de `nuevo.md` que no existía sí lo es.
    ///
    /// El **«antes»** se computa aquí dentro, vía `self` (que ya tiene acceso al canónico), para no
    /// cambiar la firma del método (E20-H04).
    pub fn validate_staging(&self, staging: &StagingDir) -> Result<(), WorkspaceError> {
        let after_files = read_tree(staging.path())?;

        // «Antes»: el canónico actual con su inventario completo (los `other_files` para clasificar
        // enlaces a ficheros del proyecto igual que el resto del motor). El «después» reusa esos
        // mismos `other_files`: una transacción solo escribe `.md`, así que los ficheros no-documento
        // del proyecto son idénticos en ambos árboles, y resolver los enlaces con el mismo inventario
        // evita que un enlace a código pase de `WorkspaceFile` a `Missing` y parezca un error nuevo.
        let canonical = crate::discovery::discover(&self.root, &self.discovery_policy())?;
        let before =
            DocumentSet::with_other_files(canonical.files.clone(), canonical.other_files.clone());
        let after = DocumentSet::with_other_files(after_files, canonical.other_files);

        let cfg = self.config();
        let before_errors = error_keys(&before, &cfg.validation);
        let after_errors = error_keys(&after, &cfg.validation);

        let introduces_new = after_errors.iter().any(|k| !before_errors.contains(k));
        let reject = (cfg.transactions.reject_new_errors && introduces_new)
            || (!cfg.transactions.allow_existing_errors && !after_errors.is_empty());

        if reject {
            // Aborta: limpia el staging (best-effort) y no toca el canónico.
            let _ = std::fs::remove_dir_all(staging.path());
            let nuevos = after_errors.difference(&before_errors).count();
            return Err(WorkspaceError::NonconformantResult(format!(
                "el resultado del plan no pasa la política de cambios: {nuevos} error(es) nuevo(s), \
                 {} error(es) en total (rejectNewErrors={}, allowExistingErrors={})",
                after_errors.len(),
                cfg.transactions.reject_new_errors,
                cfg.transactions.allow_existing_errors
            )));
        }
        Ok(())
    }
}

/// Clave de identidad de un diagnóstico independiente de su `id` efímero: código + documento(s)
/// culpable(s) + destino(s) relacionado(s). Es lo que distingue un error **nuevo** de uno
/// preexistente que sobrevive al cambio (E20-H04).
type DiagKey = (CheckCode, Vec<RelPath>, Vec<RelPath>);

/// El conjunto de claves de los diagnósticos de **error** de `doc_set` bajo la política `validation`
/// (severidad efectiva, no la hardcodeada). Solo entran los que quedan en [`Severity::Err`] tras
/// aplicar [`ValidationSection::effective_severity`] — los reclasificados a `Warn`/`ignore` no son
/// errores y no cuentan para el gate diferencial.
fn error_keys(doc_set: &DocumentSet, validation: &ValidationSection) -> BTreeSet<DiagKey> {
    let mut set: BTreeSet<DiagKey> = BTreeSet::new();
    for checks in doc_set.analyze().diagnostics.values() {
        for check in checks {
            if validation.effective_severity(check) == Some(Severity::Err) {
                set.insert((check.code, check.targets.clone(), check.related.clone()));
            }
        }
    }
    set
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pin del contrato de saneado de `staging_dir_name` (chokepoint del `changeSetId` hacia el
    /// nombre de directorio bajo `.lodestar/runtime/staging/`): descarta el prefijo `changeset:` y
    /// neutraliza cualquier carácter de path (`:`/`/`/`\`) → `_`, de modo que un id hostil no puede
    /// escapar de la raíz de staging. Sin este test, mutar la función a una constante sobrevive
    /// (el directorio es efímero y su nombre no entra en las aserciones de las transacciones).
    #[test]
    fn staging_dir_name_sanea_prefijo_y_separadores() {
        // Prefijo descartado + los tres separadores de path neutralizados.
        let id = ChangeSetId("changeset:a/b:c\\d".into());
        assert_eq!(staging_dir_name(&id), "a_b_c_d");

        // Sin prefijo `changeset:`, el id se conserva (solo se sanean separadores).
        let sin_prefijo = ChangeSetId("a/b".into());
        assert_eq!(staging_dir_name(&sin_prefijo), "a_b");

        // Un id ya limpio pasa intacto (determinista y distinto por id).
        let limpio = ChangeSetId("changeset:abc123".into());
        assert_eq!(staging_dir_name(&limpio), "abc123");
    }
}
