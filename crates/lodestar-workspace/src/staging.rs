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

use std::path::{Path, PathBuf};

use lodestar_core::plan;
use lodestar_core::types::{ChangeSet, ChangeSetId, FileMap, RelPath};
use lodestar_core::DocumentSet;

use crate::error::WorkspaceError;
use crate::{Workspace, WorkspaceSchema};

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

    /// Valida un staging materializado contra la política de conformidad **completa** antes de
    /// publicar (E13-H01; conformidad schema-driven añadida en E14-H04). Construye un [`DocumentSet`]
    /// desde el árbol de staging y evalúa el **mismo universo de diagnósticos** que
    /// `App::knowledge_check`/`lodestar check`: los diagnósticos de [`DocumentSet::analyze`] (`§20.9`) MÁS la
    /// validación schema-driven (`SCHEMA-*`/`REL-*`, `core::schema::validate_schema` +
    /// `validate_relations`) contra el esquema del workspace. Si el resultado deja **cualquier**
    /// diagnóstico de nivel `err`, **aborta sin tocar el canónico** y limpia el directorio de
    /// staging, devolviendo [`WorkspaceError::NonconformantResult`]. Si es conforme, devuelve
    /// `Ok(())` y el staging queda listo para publicarse.
    ///
    /// **Invariante #3 (una sola verdad computada)**: el veredicto del gate debe coincidir
    /// EXACTAMENTE con el de `App::knowledge_check` scope workspace sobre el mismo resultado. Para
    /// no duplicar la composición se reutiliza [`plan::validate_result`] — la MISMA función que
    /// `change_plan` usa para computar `canApply`/`diagnosticsAfter`, que a su vez compone
    /// `analyze().diagnostics` + `validate_schema` + `validate_relations` (`plan::all_checks`) y
    /// declara `conformant == (errors == 0)`. Antes (E13-H01) el gate medía solo
    /// `analyze().hard_fail()` (los del documento) y NO ejecutaba la validación schema-driven, así que un
    /// `SCHEMA-REQFIELD`/`REL-*` (level `err`) pasaba el gate y `change_apply` publicaba un
    /// resultado que `knowledge_check` declararía no conforme: gate transaccional y motor de
    /// conformidad DIVERGÍAN. Reusar `validate_result` cierra esa divergencia por construcción.
    ///
    /// El esquema se carga con [`WorkspaceSchema::load`] (I/O de `workspace`, nunca del core:
    /// invariante #2 — el core es puro y recibe el `Schema` ya deserializado); un workspace sin
    /// `.lodestar/schema.yaml` produce `Schema::default()` (vacío/permisivo) y la validación
    /// schema-driven devuelve cero checks, así que el veredicto de un workspace sin esquema no cambia.
    ///
    /// El gate es estricto por diseño: cuenta solo `err` (no `warn`), nunca se publica un resultado
    /// con errores duros, con independencia de que la config bloquee o no los avisos.
    pub fn validate_staging(&self, staging: &StagingDir) -> Result<(), WorkspaceError> {
        let files = read_tree(staging.path())?;
        let doc_set = DocumentSet::from_files(files);
        // Esquema del workspace (el mismo que carga `App::knowledge_check`); su ausencia no es error.
        let schema = WorkspaceSchema::load(&self.root).map_err(WorkspaceError::Io)?;
        // Conformidad COMPLETA (documento + SCHEMA-* + REL-*): misma composición y mismo criterio
        // (`conformant == errors == 0`) que `change_plan`/`knowledge_check` (invariante #3).
        let report = plan::validate_result(&doc_set, &schema);
        if !report.conformant {
            // Aborta: limpia el staging (best-effort) y no toca el canónico.
            let _ = std::fs::remove_dir_all(staging.path());
            let errors = report.summary.errors;
            return Err(WorkspaceError::NonconformantResult(format!(
                "el resultado del plan deja {errors} fallo(s) duro(s) de conformidad"
            )));
        }
        Ok(())
    }
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
