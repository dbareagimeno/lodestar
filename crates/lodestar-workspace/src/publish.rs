//! Aplicación atómica por lote (E13-H05, `ARCHITECTURE.md §19.5`, `REFACTOR §5.2` paso 11): publica
//! el resultado de un [`ChangeSet`] sobre el conocimiento `.md` canónico sustituyendo cada fichero
//! por un rename atómico (temp+fsync+rename), uno a uno, por el **único escritor**.
//!
//! Es el eslabón que materializa de verdad la transacción: E13-H01 prepara y valida el resultado en
//! staging sin tocar el canónico, E13-H03 registra la intención en el write-ahead journal, y aquí
//! (E13-H05) se sustituye el canónico y se marca el journal a medida que cada rename se completa.
//! La recuperación tras una caída a mitad es E13-H06; el receipt de cierre, E13-H07.
//!
//! Único escritor (invariante #5): la publicación **solo** escribe el canónico a través de
//! `io::write_atomic` (creados/modificados) e `io::delete` (borrados); no hay ningún otro
//! camino de escritura del `.md` en este flujo. Si el watcher está activo, absorbe el lote
//! auto-originado por el gate de hash blake3 (descarta echoes/no-ops).

use std::collections::BTreeSet;

use lodestar_core::plan;
use lodestar_core::types::{ChangeSet, FileMap, RelPath, WorkspaceRevision};

use crate::error::WorkspaceError;
use crate::journal::Journal;
use crate::{io, Workspace};

impl Workspace {
    /// Publica el resultado de `change_set` sobre el conocimiento canónico por el **único escritor**
    /// (E13-H05), actualizando `journal` a medida que cada operación se completa.
    ///
    /// Carga el `FileMap` canónico actual, computa el resultado del plan con
    /// [`plan::apply_normalized_ops`] (la única canonicalización del core, la misma que usó
    /// `materialize_staging`) y determina el conjunto de cambios: paths **creados/modificados**
    /// (los que el resultado deja con un contenido que difiere del canónico) y **borrados** (los que
    /// el canónico tenía y el resultado ya no contiene). En **orden determinista por [`RelPath`]**
    /// aplica cada cambio con `io::write_atomic` (temp+fsync+rename) o `io::delete`, y tras cada
    /// sustitución llama a [`Journal::mark_applied`] (que re-persiste el journal con fsync). Al
    /// terminar todas, transiciona el journal a `applied` con [`Journal::mark_all_applied`].
    ///
    /// Devuelve la `resultWorkspaceRevision` recalculada del canónico ya publicado
    /// ([`Workspace::workspace_revision`]); si el plan es correcto, coincide con la `result_rev` con
    /// la que se creó el journal (E13-H03).
    ///
    /// Único escritor (invariante #5): esta función es el único camino que escribe el canónico
    /// durante la transacción y lo hace exclusivamente por `write_atomic`/`delete`; el watcher, si
    /// está activo, absorbe el lote auto-originado por el gate blake3.
    ///
    /// # Errores
    /// - [`WorkspaceError::Core`] si `change_set` trae una operación no terminal (violación del
    ///   pipeline de normalización).
    /// - [`WorkspaceError::Io`] si falla la lectura del canónico, alguna escritura/borrado atómico o
    ///   la re-persistencia del journal.
    pub fn publish(
        &self,
        change_set: &ChangeSet,
        journal: &mut Journal,
    ) -> Result<WorkspaceRevision, WorkspaceError> {
        // Estado de partida y resultado previsto por el plan (misma lógica que el staging).
        let canonical = self.discover_files()?;
        let result = plan::apply_normalized_ops(&canonical, &change_set.operations)?;
        self.publish_result(&result, journal)
    }

    /// Publica un `FileMap` resultado **ya computado** sobre el canónico por el **único escritor**.
    /// Es el núcleo de [`Workspace::publish`] extraído para que la transacción (E13-H08) publique
    /// **el mismo `FileMap` que se materializó y validó en staging**, en lugar de recomputarlo desde
    /// las ops: lo que se valida es exactamente lo que se publica, bajo el mismo journal.
    ///
    /// Nota histórica: la escisión nació en E13-H11 para publicar el plan *aumentado* con la
    /// auto-regeneración de `index`/`tags` (D6a). Esa auto-regeneración se **retiró** en E15-H02
    /// (`ARCHITECTURE.md §20.13`), pero la escisión se conserva porque la propiedad de arriba
    /// —validar y publicar el mismo mapa— vale por sí sola.
    ///
    /// Determina el conjunto de cambios contra el canónico: paths **creados/modificados** (el
    /// resultado deja un contenido que difiere del canónico) y **borrados** (el canónico los tenía y
    /// el resultado ya no). En **orden determinista por [`RelPath`]** aplica cada cambio con
    /// `io::write_atomic` (temp+fsync+rename) o `io::delete`, marcando el journal tras cada
    /// sustitución; al terminar transiciona el journal a `applied`. Devuelve la
    /// `resultWorkspaceRevision` recalculada del canónico ya publicado.
    ///
    /// # Errores
    /// - [`WorkspaceError::WorkspaceRecoveryRequired`] si existe un journal no-`done` de OTRA
    ///   transacción sin recuperar (no se publica sobre un estado a medio recuperar).
    /// - [`WorkspaceError::Io`] si falla la lectura del canónico, alguna escritura/borrado atómico o
    ///   la re-persistencia del journal.
    pub(crate) fn publish_result(
        &self,
        result: &FileMap,
        journal: &mut Journal,
    ) -> Result<WorkspaceRevision, WorkspaceError> {
        // Gate de recuperación (E13-H06): si existe un journal no-`done` de OTRA transacción
        // (una publicación anterior interrumpida que aún no se ha recuperado), no se publica sobre
        // un estado a medio recuperar. Se excluye el journal de ESTA transacción —recién creado en
        // `prepared` por `create_journal` (E13-H03)—, que no es una recuperación pendiente sino el
        // registro write-ahead del lote en curso.
        if !self.pending_journals(Some(journal.path())).is_empty() {
            return Err(WorkspaceError::WorkspaceRecoveryRequired(
                "hay un journal de publicación anterior sin completar bajo \
                 .lodestar/runtime/journal/: ejecuta la recuperación (Workspace::recover) antes \
                 de publicar una nueva transacción"
                    .to_string(),
            ));
        }

        let canonical = self.discover_files()?;

        // Conjunto de paths afectados, en orden determinista por `RelPath` (BTreeSet).
        //
        // - Creado/modificado: el resultado deja `rel` con un contenido que difiere del canónico
        //   (incluye los `.md` que el canónico no tenía).
        // - Borrado: el canónico tenía `rel` y el resultado ya no lo contiene.
        //
        // Un `rel` cuyo contenido no cambia no se toca (no es una sustitución): no hay rename inútil
        // ni echo espurio para el watcher.
        let mut affected: BTreeSet<&RelPath> = BTreeSet::new();
        for (rel, content) in result {
            if canonical.get(rel) != Some(content) {
                affected.insert(rel);
            }
        }
        for rel in canonical.keys() {
            if !result.contains_key(rel) {
                affected.insert(rel);
            }
        }

        // Aplica cada cambio por el ÚNICO escritor, marcando el journal tras cada sustitución.
        for rel in affected {
            match result.get(rel) {
                Some(content) => io::write_atomic(&self.root, rel, content)?,
                None => io::delete(&self.root, rel)?,
            }
            journal.mark_applied(rel)?;
        }

        // Todas las operaciones aplicadas: el journal pasa a `applied` (E13-H05).
        journal.mark_all_applied()?;

        // `resultWorkspaceRevision` calculada del canónico ya publicado.
        self.workspace_revision()
    }
}
