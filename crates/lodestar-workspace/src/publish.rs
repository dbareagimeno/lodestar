//! Aplicación atómica por lote (E13-H05, `ARCHITECTURE.md §19.5`, `REFACTOR §5.2` paso 11): publica
//! el resultado de un [`ChangeSet`] sobre el conocimiento `.md` canónico sustituyendo cada fichero
//! por un rename atómico (temp+fsync+rename), uno a uno, por el **único escritor**.
//!
//! Es el eslabón que materializa de verdad la transacción: E13-H01 prepara y valida el resultado en
//! staging sin tocar el canónico, E13-H03 registra la intención en el write-ahead journal, y aquí
//! (E13-H05) se sustituye el canónico y se marca el journal a medida que cada rename se completa.
//! La recuperación tras una caída a mitad es E13-H06; el receipt de cierre, E13-H07.
//!
//! Lo publicado es lo respaldado (E25-H01): el lote que se sustituye es **exactamente** el que pasó
//! por `assert_writable`, por las copias de recuperación y por el journal. Si el canónico cambió
//! entre el cálculo del lote y el primer rename —otro proceso, el usuario, un `.md` aparecido bajo
//! un `referenceRoot`—, la publicación aborta con `WriteConflict` sin escribir nada, en lugar de
//! recomputar la diferencia contra un estado que nadie respaldó.
//!
//! Único escritor (invariante #5): la publicación **solo** escribe el canónico a través de
//! `io::write_atomic` (creados/modificados) e `io::delete` (borrados); no hay ningún otro
//! camino de escritura del `.md` en este flujo. Si el watcher está activo, absorbe el lote
//! auto-originado por el gate de hash blake3 (descarta echoes/no-ops).

use std::collections::BTreeSet;

use lodestar_core::plan;
use lodestar_core::types::{ChangeSet, FileMap, RelPath, WorkspaceRevision};

use crate::error::WorkspaceError;
#[cfg(feature = "test-failpoints")]
use crate::failpoints::FailPoint;
use crate::journal::Journal;
use crate::{io, Workspace};

/// Describe, para el mensaje de error de un conflicto de ventana (E25-H01), los paths en los que
/// dos estados del canónico difieren: creados, modificados o desaparecidos. Se citan como mucho
/// [`MAX_PATHS_EN_CONFLICTO`] (el resto se resume como «y N más»), para que el mensaje siga siendo
/// legible en un workspace grande.
fn paths_divergentes(antes: &FileMap, ahora: &FileMap) -> String {
    let mut divergentes: BTreeSet<&RelPath> = BTreeSet::new();
    for (rel, content) in ahora {
        if antes.get(rel) != Some(content) {
            divergentes.insert(rel);
        }
    }
    for rel in antes.keys() {
        if !ahora.contains_key(rel) {
            divergentes.insert(rel);
        }
    }

    let total = divergentes.len();
    let citados: Vec<&str> = divergentes
        .into_iter()
        .take(MAX_PATHS_EN_CONFLICTO)
        .map(RelPath::as_str)
        .collect();
    let lista = citados.join(", ");
    if total > citados.len() {
        format!("{lista} (y {} más)", total - citados.len())
    } else {
        lista
    }
}

/// Cuántos paths en conflicto se citan como mucho en el mensaje de [`WorkspaceError::WriteConflict`].
const MAX_PATHS_EN_CONFLICTO: usize = 5;

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
    /// - [`WorkspaceError::WriteConflict`] si el canónico cambió entre la lectura de partida y el
    ///   primer rename (E25-H01).
    /// - [`WorkspaceError::Io`] si falla la lectura del canónico, alguna escritura/borrado atómico o
    ///   la re-persistencia del journal.
    ///
    /// **No es la pieza que usa producción** (E29-H10, `decisiones §16(g)`): el orquestador
    /// transaccional real (`apply_transaction`, en `transaction.rs`) llama directamente a
    /// [`Workspace::publish_result`] con el `FileMap` que ya materializó y validó en staging, para
    /// publicar exactamente lo que se validó — nunca a esta función, que recomputa el resultado
    /// desde `change_set` sin pasar por staging. Por eso se repliega a `pub(crate)`: es una
    /// composición de conveniencia (`discover_files` + `apply_normalized_ops` + `publish_result`)
    /// sin lock ni backup propios, y ningún consumidor legítimo fuera de este crate debería
    /// llamarla en vez del camino transaccional completo. Se conserva (no se retira) como
    /// primitiva de test bajo `--features test-support`.
    ///
    /// `#[allow(dead_code)]`: sin llamante dentro del crate en una build normal; deliberado.
    #[cfg(not(feature = "test-support"))]
    #[allow(dead_code)]
    pub(crate) fn publish(
        &self,
        change_set: &ChangeSet,
        journal: &mut Journal,
    ) -> Result<WorkspaceRevision, WorkspaceError> {
        self.publish_impl(change_set, journal)
    }

    /// Igual que la versión `pub(crate)` de arriba, pero pública **solo** bajo
    /// `--features test-support` (E29-H10).
    #[cfg(feature = "test-support")]
    pub fn publish(
        &self,
        change_set: &ChangeSet,
        journal: &mut Journal,
    ) -> Result<WorkspaceRevision, WorkspaceError> {
        self.publish_impl(change_set, journal)
    }

    fn publish_impl(
        &self,
        change_set: &ChangeSet,
        journal: &mut Journal,
    ) -> Result<WorkspaceRevision, WorkspaceError> {
        // Estado de partida y resultado previsto por el plan (misma lógica que el staging). El
        // canónico se lee **una sola vez** y se le pasa a `publish_result`, que lo usa como el
        // estado de partida sobre el que se computó el resultado (E25-H01).
        let canonical = self.discover_files()?;
        let result = plan::apply_normalized_ops(&canonical, &change_set.operations)?;
        self.publish_result(&canonical, &result, journal)
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
    /// `canonical` es el estado de partida **con el que se computó `result`** (el `FileMap` leído en
    /// T1 por el orquestador, el mismo sobre el que se ejerció `assert_writable`, el backup y el
    /// journal). Antes del primer rename se relee el canónico y se **comparan ambos**: si difieren
    /// en cualquier path —una edición externa, un `.md` nuevo, incluso bajo un `referenceRoot`—, la
    /// publicación aborta con [`WorkspaceError::WriteConflict`] sin haber tocado nada (E25-H01). El
    /// conjunto respaldado y anotado en el journal es la **única** escritura legítima: antes, el
    /// conjunto afectado se recomputaba contra el canónico de T3, de modo que cualquier fichero
    /// aparecido en la ventana se borraba sin guard, sin copia de recuperación y sin entrada de
    /// journal.
    ///
    /// El conjunto de cambios se deriva de `canonical` (el de T1), no del relectura: paths
    /// **creados/modificados** (el resultado deja un contenido que difiere del canónico) y
    /// **borrados** (el canónico los tenía y el resultado ya no) — exactamente el conjunto que el
    /// journal declara, porque el orquestador lo computó del mismo par. En **orden determinista por
    /// [`RelPath`]** aplica cada cambio con `io::write_atomic` (temp+fsync+rename) o `io::delete`,
    /// marcando el journal tras cada sustitución; al terminar transiciona el journal a `applied`.
    /// Devuelve la `resultWorkspaceRevision` recalculada del canónico ya publicado.
    ///
    /// # Errores
    /// - [`WorkspaceError::WorkspaceRecoveryRequired`] si existe un journal no-`done` de OTRA
    ///   transacción sin recuperar (no se publica sobre un estado a medio recuperar).
    /// - [`WorkspaceError::WriteConflict`] si el canónico cambió entre T1 y el primer rename
    ///   (E25-H01). Es **terminal** para esa transacción: el modelo es fail-fast (`§19.5`), no hay
    ///   reintento; el agente replanifica.
    /// - [`WorkspaceError::Io`] si falla la lectura del canónico, alguna escritura/borrado atómico o
    ///   la re-persistencia del journal.
    pub(crate) fn publish_result(
        &self,
        canonical: &FileMap,
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

        // Control de la ventana `[T1, T3)` (E25-H01): el canónico que se sustituye tiene que ser el
        // mismo con el que se computaron el resultado, el conjunto afectado, las copias de
        // recuperación y el journal. Si algo cambió en esa ventana —otro proceso, el usuario, un
        // `.md` nuevo bajo un `referenceRoot` que el control optimista no puede ver porque
        // `workspace_revision` excluye lo que queda fuera de `writableRoots`—, se aborta ANTES del
        // primer rename: publicar sobre un canónico distinto escribiría fuera de lo respaldado.
        let ahora = self.discover_files()?;
        if ahora != *canonical {
            let divergentes = paths_divergentes(canonical, &ahora);
            return Err(WorkspaceError::WriteConflict(format!(
                "el conocimiento canónico cambió mientras la transacción se preparaba \
                 (entre el cálculo del lote y su publicación): {divergentes}. No se publica nada: \
                 el conjunto respaldado y anotado en el journal ya no describe el estado real; \
                 vuelve a planificar sobre el conocimiento actual"
            )));
        }

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
            failpoint!(FailPoint::EntreRenames);
            #[cfg(feature = "test-failpoints")]
            crate::failpoints::ejecutar_gancho(
                crate::failpoints::PuntoDeGancho::DespuesDelPrimerRename,
            );
        }

        // Todas las operaciones aplicadas: el journal pasa a `applied` (E13-H05).
        journal.mark_all_applied()?;

        // `resultWorkspaceRevision` calculada del canónico ya publicado.
        self.workspace_revision()
    }
}
