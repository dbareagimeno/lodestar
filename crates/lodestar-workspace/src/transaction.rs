//! Mecánica transaccional de publicación (E13-H08, `ARCHITECTURE.md §19.5/§19.6`, `REFACTOR §11.2`).
//!
//! [`Workspace::apply_transaction`] es el **orquestador** que compone las primitivas de E13-H01…H07
//! en una única transacción atómica y recuperable, ejecutada por el **único escritor** (invariante
//! #5) sobre el conocimiento `.md` canónico (invariante #1). Es la pieza que `App::change_apply`
//! (fachada `lodestar-app`) invoca DESPUÉS de haber validado el plan (caducidad, `planHash`); aquí
//! solo vive la mecánica de disco, no la lógica de plan.
//!
//! Orden EXACTO de la transacción (cada paso antes del siguiente; los renames del canónico llegan
//! solo al final, tras dejar todo lo necesario para recuperar/revertir):
//! 1. `acquire_lock()` — un solo publicador a la vez (RAII, liberado al final por `Drop`, E13-H02).
//! 2. `recover()` si hay una recuperación pendiente — nunca se publica sobre un estado a medio
//!    recuperar (E13-H06).
//! 3. `previous = workspace_revision()` — la revisión sobre la que se publica (`previousRevision`).
//! 4. **Resultado aumentado** = resultado de `apply_normalized_ops` + auto-regeneración de
//!    `index`/`tags` (`augment_with_regenerated`, E13-H11/D6a) → conjunto **afectado real** = paths
//!    creados/modificados/borrados por ESE resultado aumentado vs el canónico (no solo las ops
//!    crudas; contrato de E13-H06: journal y backup cubren ESE conjunto, para que un `Move` o una
//!    regeneración no rompan la recuperación).
//! 5. `assert_writable(path)` para **cada** afectado — si alguno cae fuera de `writableRoots` (o bajo
//!    `referenceRoots`), `Err(PermissionDenied)` ANTES de tocar el canónico (E11-H04).
//! 6. `materialize_staging` + `validate_staging` — resultado hipotético validado sin tocar el
//!    canónico (E13-H01).
//! 7. `reverify_base_revision` — control optimista bajo el lock (la base no cambió, E13-H02).
//! 8. `backup_originals` — copias de recuperación **antes** de publicar (E13-H04).
//! 9. `create_journal` — write-ahead journal `prepared`, fsynced antes del primer rename (E13-H03).
//! 10. `publish` — renames atómicos por el único escritor + journal `applied` (E13-H05).
//! 11. **Sellar**: limpiar staging + journal; **conservar** las copias de recuperación (el receipt y
//!     el `change_revert` de E13-H09 las necesitan).
//! 12. `result = workspace_revision()` — la revisión resultante (`resultRevision`).

use std::collections::BTreeSet;

use lodestar_core::plan;
use lodestar_core::types::{ChangeSet, ChangeSetId, FileMap, Mutation, RelPath, WorkspaceRevision};
use lodestar_core::Bundle;

use crate::config::WorkspaceConfig;
use crate::{io, Workspace, WorkspaceError};

/// Deriva el identificador de transacción de un [`ChangeSetId`]: el hash **desnudo** (sin el prefijo
/// `changeset:`). Ese mismo id nombra —tras el saneado común de `:`/`/`/`\`— el write-ahead journal
/// (`<id>.json`), el staging (`staging/<id>/`), las copias de recuperación (`recovery/<id>/`) y el
/// receipt (`receipts/<id>.json`), de modo que la recuperación (E13-H06) y el GC de recibos
/// (E13-H07) localizan las cuatro cosas por el mismo id. Como el hash es hexadecimal (sin caracteres
/// hostiles), el id derivado coincide ya con su forma saneada.
pub fn transaction_id(change_set_id: &ChangeSetId) -> String {
    change_set_id
        .0
        .strip_prefix("changeset:")
        .unwrap_or(&change_set_id.0)
        .to_string()
}

/// Conjunto de paths **afectados** por llevar `canonical` al estado `result`, en orden determinista
/// por [`RelPath`] (misma lógica que [`Workspace::publish`]): creados/modificados (el resultado deja
/// un contenido que difiere del canónico) + borrados (el canónico los tenía y el resultado ya no).
fn affected_paths(canonical: &FileMap, result: &FileMap) -> Vec<RelPath> {
    let mut set: BTreeSet<RelPath> = BTreeSet::new();
    for (rel, content) in result {
        if canonical.get(rel) != Some(content) {
            set.insert(rel.clone());
        }
    }
    for rel in canonical.keys() {
        if !result.contains_key(rel) {
            set.insert(rel.clone());
        }
    }
    set.into_iter().collect()
}

/// Aplica una [`Mutation`] (plan de un generador) sobre un [`FileMap`] en memoria: inserta cada
/// `write` y elimina cada `delete`. No toca disco — es la fusión pura de la regeneración dentro del
/// resultado hipotético de la transacción.
fn apply_mutation(files: &mut FileMap, mutation: &Mutation) {
    for (path, content) in &mutation.writes {
        files.insert(path.clone(), content.clone());
    }
    for path in &mutation.deletes {
        files.remove(path);
    }
}

/// **Auto-regeneración de los generados dentro de la transacción** (E13-H11, decisión **D6a**,
/// `ARCHITECTURE.md §19.6`): toma el resultado hipotético del plan (`result`) y le fusiona lo que
/// producirían `lodestar index` y `lodestar tags`, de modo que los `index.md` de directorio y el
/// árbol de índices de `tags/` queden coherentes con la estructura nueva **en el mismo lote**.
///
/// - **`index.md` de directorio**: se regeneran los que YA existían en el resultado (misma política
///   que la CLI, que regenera índices existentes; no se inventan índices de directorios nuevos). Se
///   **excluye** el árbol `tags/`, cuyos `index.md` son propiedad de [`Bundle::gen_tag_indexes`], no
///   índices de directorio.
/// - **Índices de tags**: [`Bundle::gen_tag_indexes`] escribe los vigentes y **purga** (borra) los
///   obsoletos (un tag que se quedó sin conceptos) — reproduciendo `lodestar tags`.
///
/// La regeneración se ejecuta **siempre** por simplicidad, y es **idempotente**: si el change set no
/// altera conceptos/tags, los generadores producen el mismo contenido que ya hay → sin diferencia
/// contra el canónico → sin paths extra en el lote (el conjunto afectado se computa por diferencia,
/// [`affected_paths`], que descarta los no-cambios). El [`Bundle`] se construye una sola vez desde
/// `result`: `index.md`/`tags/*` son reservados y no influyen en el análisis de conceptos que
/// alimenta a los generadores, así que ver el resultado del plan (con los generados aún stale)
/// basta para reproducir su salida canónica.
fn augment_with_regenerated(result: &FileMap) -> FileMap {
    let bundle = Bundle::from_files(result.clone());
    let mut augmented = result.clone();

    // (a) `index.md` de directorio existentes, excluyendo el árbol de índices de tags.
    let dirs: BTreeSet<String> = result
        .keys()
        .filter(|p| p.basename() == "index.md")
        .map(|p| p.dir())
        .filter(|dir| dir != "tags/" && !dir.starts_with("tags/"))
        .collect();
    for dir in &dirs {
        apply_mutation(&mut augmented, &bundle.gen_index(dir));
    }

    // (b) Índices de tags: escribe los vigentes y purga los obsoletos.
    apply_mutation(&mut augmented, &bundle.gen_tag_indexes());

    augmented
}

impl Workspace {
    /// Ejecuta la **transacción de publicación** de `change_set` sobre el conocimiento canónico
    /// (E13-H08), componiendo las primitivas de E13-H01…H07 en el orden documentado a nivel de
    /// módulo. Devuelve `(previousRevision, resultRevision, changedPaths)`: la
    /// [`WorkspaceRevision`] antes y después de la transacción y el conjunto de paths que sustituyó
    /// (creados/modificados/borrados), en orden determinista.
    ///
    /// Es el **único escritor** (invariante #5): toda escritura del canónico pasa por
    /// [`Workspace::publish`] (`io::write_atomic`/`io::delete`). La transacción es **recuperable**
    /// (invariante «nunca un estado parcial silencioso»): si el proceso muere en cualquier punto, al
    /// reabrir el workspace [`Workspace::recover`] completa o restaura de forma determinista desde el
    /// write-ahead journal (E13-H03) y las copias de recuperación (E13-H04), que se preparan **antes**
    /// del primer rename.
    ///
    /// El lock de publicación (E13-H02) se adquiere al inicio y se libera al final por `Drop` del
    /// guard —incluido durante el desenrollado de un `panic`—, garantizando un solo publicador a la
    /// vez. Las copias de recuperación **se conservan** al sellar (a diferencia de la limpieza de
    /// `recover`): el receipt (E13-H07) y `change_revert` (E13-H09) las necesitan.
    ///
    /// # Errores
    /// - [`WorkspaceError::WriteConflict`] si el lock ya está tomado (otro publicador) o si la base
    ///   del plan cambió entre plan y apply (E13-H02).
    /// - [`WorkspaceError::PermissionDenied`] si algún path afectado cae fuera de `writableRoots` o
    ///   bajo un `referenceRoot` (E11-H04) — comprobado **antes** de tocar el canónico.
    /// - [`WorkspaceError::NonconformantResult`] si el resultado hipotético no es conforme (E13-H01).
    /// - [`WorkspaceError::WorkspaceRecoveryRequired`] si una recuperación pendiente no se pudo
    ///   completar antes de publicar.
    /// - [`WorkspaceError::Core`]/[`WorkspaceError::Io`] ante un fallo de normalización o de IO.
    pub fn apply_transaction(
        &self,
        change_set: &ChangeSet,
    ) -> Result<(WorkspaceRevision, WorkspaceRevision, Vec<RelPath>), WorkspaceError> {
        // (1) Lock exclusivo de publicación (RAII: liberado al final por `Drop`, incluso en panic).
        let _lock = self.acquire_lock()?;

        // (2) Recuperación pendiente primero: si una transacción anterior quedó a medias, complétala
        //     o restáurala antes de publicar sobre un estado a medio recuperar (E13-H06). Bajo el
        //     lock, así dos publicadores no recuperan a la vez.
        if self.recovery_pending() {
            self.recover()?;
        }

        // (3) Revisión base actual: será la `previousRevision` del receipt.
        let previous = self.workspace_revision()?;

        // (4) Resultado del plan y su AUMENTO con la auto-regeneración de index/tags (E13-H11, D6a):
        //     el objetivo real de la transacción es `result_augmented` = resultado del plan + lo que
        //     producirían `lodestar index`/`tags` sobre él, para que los generados queden coherentes
        //     EN EL MISMO LOTE (mismo staging/journal/backup/publish/receipt). El conjunto AFECTADO
        //     se computa contra `result_augmented`, no contra las ops crudas: así el journal y las
        //     copias cubren tanto el `.md` del plan como los `index.md`/`tags/*` regenerados/purgados
        //     (contrato H06: un `Move` o una regeneración no rompen la recuperación).
        let canonical = io::load_bundle(&self.root)?;
        let result_files = plan::apply_normalized_ops(&canonical, &change_set.operations)?;
        let result_augmented = augment_with_regenerated(&result_files);
        let affected = affected_paths(&canonical, &result_augmented);

        // (5) Guard del único escritor (E11-H04): si algún path afectado no es escribible, se rechaza
        //     ANTES de tocar el canónico (ni staging del canónico, ni backup, ni rename).
        for path in &affected {
            self.assert_writable(path)?;
        }

        // (6) Staging: materializa y valida el resultado hipotético AUMENTADO sin tocar el canónico
        //     (E13-H01). Se materializa `result_augmented` (no las ops crudas) para que la validación
        //     de conformidad cubra también los generados regenerados y para que el lote publicado
        //     coincida exactamente con lo materializado.
        let staging = self.materialize_staging_result(&change_set.id, &result_augmented)?;
        let staging_path = staging.path().to_path_buf();
        self.validate_staging(&staging)?;

        // (7) Control optimista bajo el lock: la base del plan sigue siendo la revisión actual.
        self.reverify_base_revision(&change_set.base_revision)?;

        // Id de transacción: nombra journal, staging, recuperación y receipt (misma convención).
        let txn_id = transaction_id(&change_set.id);
        let writable = WorkspaceConfig::load(&self.root)
            .unwrap_or_default()
            .workspace
            .writable_roots;
        let result_rev = lodestar_core::types::workspace_revision(&result_augmented, &writable);

        // (8) Copias de recuperación de los originales afectados, ANTES de publicar (E13-H04). Se
        //     conservan al sellar (para el receipt y el revert de H09).
        self.backup_originals(&txn_id, &affected)?;

        // (9) Write-ahead journal `prepared`, fsynced antes del primer rename (E13-H03).
        let mut journal = self.create_journal(&txn_id, &affected, &previous, &result_rev)?;

        // (10) Publica el resultado AUMENTADO por el único escritor (renames atómicos + journal
        //      `applied`, E13-H05): el mismo `result_augmented` que se materializó y validó en
        //      staging, de modo que index/tags se publican en el MISMO lote que el `.md` del plan.
        let result = self.publish_result(&result_augmented, &mut journal)?;

        // (11) Sella la transacción: limpia el staging y el journal (levanta el gate de recuperación)
        //      pero CONSERVA las copias de recuperación (el receipt y `change_revert` las usan). El
        //      journal ya está `applied` en disco; borrarlo es el sellado `done` efectivo (tras esto
        //      `recovery_pending()` vuelve a `false`).
        let journal_path = journal.path().to_path_buf();
        if staging_path.exists() {
            std::fs::remove_dir_all(&staging_path)?;
        }
        if journal_path.exists() {
            std::fs::remove_file(&journal_path)?;
        }

        // (12) Revisión resultante + conjunto de paths cambiados.
        Ok((previous, result, affected))
    }
}
