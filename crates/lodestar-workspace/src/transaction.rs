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
//!    Desde E25-H03 ese mismo lock es lo que protege el material de la transacción del GC del plano
//!    de control mientras dura la ventana `[backup, journal)`, en la que la transacción está viva y
//!    todavía no aparece ni en `journal/` ni en `receipts/` (ver [`crate::Workspace::gc_receipts`]).
//! 2. `recover()` si hay una recuperación pendiente — nunca se publica sobre un estado a medio
//!    recuperar (E13-H06).
//! 3. `previous = workspace_revision()` — la revisión sobre la que se publica (`previousRevision`).
//! 4. **Resultado** = `apply_normalized_ops` sobre el canónico → conjunto **afectado real** = paths
//!    creados/modificados/borrados por ESE resultado vs el canónico (contrato de E13-H06: journal y
//!    backup cubren ESE conjunto, para que un `Move` no rompa la recuperación). Desde E15-H02 el
//!    resultado es **exactamente** lo que pide el change set: no hay auto-regeneración de
//!    `index`/`tags` que lo aumente — ningún fichero tiene semántica de catálogo.
//! 5. `assert_writable(path)` para **cada** afectado — si alguno cae fuera de `writableRoots` (o bajo
//!    `referenceRoots`), `Err(PermissionDenied)` ANTES de tocar el canónico (E11-H04).
//! 6. `materialize_staging` + `validate_staging` — resultado hipotético validado sin tocar el
//!    canónico (E13-H01); el gate diferencial de E20-H04 (`rejectNewErrors`/`allowExistingErrors`)
//!    rechaza solo si el resultado **introduce** errores que el canónico no tenía.
//! 7. `reverify_base_revision` — control optimista bajo el lock (la base no cambió, E13-H02).
//! 8. `backup_originals` — copias de recuperación **antes** de publicar (E13-H04).
//! 9. `create_journal` — write-ahead journal `prepared`, fsynced antes del primer rename (E13-H03), y
//!    con él el **registro durable del recibo** (E25-H04): las dos revisiones que lo componen ya se
//!    conocen aquí, y este es el último instante en que escribirlo aún sirve para poder deshacer.
//! 10. `publish` — renames atómicos por el único escritor + journal `applied` (E13-H05), sobre el
//!     MISMO canónico del paso (4): si cambió por debajo mientras la transacción se preparaba,
//!     `WriteConflict` antes del primer rename (E25-H01) — lo publicado es siempre lo respaldado. Ese
//!     aborto **sella su propia transacción** bajo el mismo lock (journal primero, copias después,
//!     E25-H02): con cero renames no hay nada que restaurar, así que dejar el journal en disco solo
//!     serviría para que la siguiente operación pisara la edición externa al «recuperar».
//! 11. **Sellar**: promover el registro del recibo a recibo definitivo (antes de borrar el journal, para
//!     que el GC del plano de control nunca vea la transacción sin respaldo), limpiar staging + journal
//!     y **conservar** las copias de recuperación (el receipt y el `change_revert` de E13-H09 las
//!     necesitan). Todo el paso es **best-effort** (E25-H04): está al otro lado del punto de no retorno,
//!     así que un fallo suyo avisa por stderr y no convierte la publicación en un error. Desde E28-H01
//!     esa coreografía vive en **un solo sitio**, [`Workspace::seal_published_transaction`], que
//!     comparte con la reversión de `crate::recovery` (`decisiones §16(i)`): antes estaba escrita dos
//!     veces a mano, con la única diferencia de que el apply también limpia `staging/`.
//! 12. `result = workspace_revision()` — la revisión resultante (`resultRevision`).

use std::collections::BTreeSet;
use std::path::Path;

use lodestar_core::plan;
use lodestar_core::types::{
    ChangeReceipt, ChangeSet, ChangeSetId, FileMap, ReceiptId, RelPath, SemanticDiff,
    WorkspaceRevision,
};

#[cfg(feature = "test-failpoints")]
use crate::failpoints::FailPoint;
use crate::{Workspace, WorkspaceError};

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

/// Sufijo con el que una transacción **inversa** nombra su material (E13-H09).
const REVERT_SUFFIX: &str = "-revert";

/// Deriva el `txnId` de la transacción que **deshace** a `txn_id`, con identidad propia y componible
/// sin límite (E28-H01).
///
/// El id de una reversión se deriva del `txnId` de la transacción **que efectivamente deshace** — que
/// es el `receiptId` de su recibo, porque `change_apply`/`change_revert` nombran cada recibo con el
/// `txnId` de su transacción—, y NO del `changeSetId` que ese recibo lleva dentro. Ese era el defecto
/// M-01 del testbench homelab: un recibo `X-revert` **hereda** el `changeSetId` de la transacción
/// original, así que derivar de él recalculaba `X-revert` una segunda vez y la nueva transacción
/// colisionaba consigo misma, sobrescribiendo el `recovery/` que guardaba el estado *redo*.
///
/// La regla es apilar un contador en el sufijo, no repetir el sufijo:
///
/// | `txn_id` | reversión |
/// |---|---|
/// | `abc123` | `abc123-revert` |
/// | `abc123-revert` | `abc123-revert-2` |
/// | `abc123-revert-2` | `abc123-revert-3` |
///
/// El primer escalón conserva **exactamente** la convención `<txnId>-revert` de E13-H09 (que fijan
/// `crash_senal.rs` y `escritura.rs`), y a partir de ahí el contador crece de uno en uno: cada
/// reversión de la cadena obtiene un nombre que ninguna anterior usó, así que `staging/`, `recovery/`,
/// `journal/` y `receipts/` siguen atados por un mismo `txnId` (la convención de `crate::receipts`) sin
/// que dos transacciones lo compartan jamás.
///
/// La derivación es **determinista** —no lleva reloj ni contador global—, que es lo que hace que un
/// reintento tras un crash vuelva a producir el mismo id y reutilice el material que la recuperación
/// ya limpió, en vez de sembrar un huérfano nuevo en cada intento. Y como solo añade `-` y dígitos, no
/// introduce caracteres que el saneado de nombres (`crate::receipts::sanear_nombre`) tenga que
/// neutralizar: el id derivado ya coincide con su forma saneada si lo estaba el de partida.
pub fn revert_transaction_id(txn_id: &str) -> String {
    // `<base>-revert-<n>` → `<base>-revert-<n+1>`, con `n` numérico (el único formato que produce
    // esta función; cualquier otra cosa cae al caso general de abajo).
    if let Some((cabeza, cola)) = txn_id.rsplit_once('-') {
        if cabeza.ends_with(REVERT_SUFFIX) {
            if let Ok(n) = cola.parse::<u64>() {
                return format!("{cabeza}-{}", n.saturating_add(1));
            }
        }
    }
    // `<base>-revert` → `<base>-revert-2` (el segundo escalón de la cadena).
    if txn_id.ends_with(REVERT_SUFFIX) {
        return format!("{txn_id}-2");
    }
    // Primer escalón: la convención histórica de E13-H09, intacta.
    format!("{txn_id}{REVERT_SUFFIX}")
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

impl Workspace {
    /// **Coreografía de sellado** de una transacción ya publicada, compartida por el apply y la
    /// reversión (E28-H01, `decisiones §16(i)`): promueve el recibo pendiente → limpia el staging (si
    /// la transacción tenía) → borra el fichero de journal.
    ///
    /// Hasta E28-H01 esta secuencia estaba escrita **dos veces a mano**
    /// —[`Workspace::apply_transaction_con_recibo`] pasos (11a)/(11b)/(11c) y
    /// `Workspace::revert_transaction_con_recibo` pasos (10a)/(10b)—, con la única diferencia de que el
    /// apply también limpia `staging/`. Dos copias de un orden cuyo sentido está en el orden mismo son
    /// exactamente la clase de divergencia que nadie ve hasta que una de las dos se queda atrás; aquí
    /// vive una sola vez y las dos la llaman.
    ///
    /// # El orden es el contrato
    ///
    /// 1. **El recibo primero**, antes de tocar el journal: así la transacción pasa de estar respaldada
    ///    por `journal/` a estarlo por `receipts/` sin un solo instante en el que el GC del plano de
    ///    control (cuyo criterio de vivos es `journal/ ∪ receipts/`) la vea como basura y le purgue las
    ///    copias con las que se revierte.
    /// 2. **El staging** —`Some(path)` solo para el apply; la reversión no materializa staging—, que
    ///    sobra desde el instante en que la transacción publicó.
    /// 3. **El journal, y solo si el recibo quedó a salvo**: si la promoción falló, dejarlo en disco es
    ///    lo que hace que la vía COMPLETAR de [`Workspace::recover`] reintente la promoción en la
    ///    siguiente operación, en vez de perder el recibo de algo ya publicado.
    ///
    /// # Todo es best-effort, a propósito
    ///
    /// Corre al otro lado del punto de no retorno: el canónico ya cambió, así que un `Err` aquí
    /// devolvería un error por algo que SÍ se publicó y el agente actuaría sobre la premisa falsa de
    /// que su operación no ocurrió (regla de E25-H04, heredada por E25-H05). Cada fallo avisa por
    /// stderr y el sellado continúa; por eso la función **no devuelve `Result`**.
    ///
    /// `que` nombra a la transacción en los avisos (`"transacción"` / `"reversión"`), que es la única
    /// diferencia que un operador necesita leer en stderr entre los dos caminos.
    pub(crate) fn seal_published_transaction(
        &self,
        txn_id: &str,
        journal_path: &Path,
        staging_path: Option<&Path>,
        que: &str,
    ) {
        // (1) El recibo, ANTES de borrar el journal.
        let recibo_a_salvo = match self.promote_pending_receipt(txn_id) {
            Ok(()) => true,
            Err(e) => {
                eprintln!(
                    "lodestar: aviso: no se pudo promover el recibo de la {que} {txn_id} ({e}): se \
                     conserva su journal para que la recuperación lo reintente"
                );
                false
            }
        };

        // (2) El staging (solo el apply materializa uno).
        if let Some(staging_path) = staging_path {
            if staging_path.exists() {
                if let Err(e) = std::fs::remove_dir_all(staging_path) {
                    eprintln!(
                        "lodestar: aviso: no se pudo limpiar el staging {} de la {que} {txn_id} \
                         ({e}): el cambio está publicado; el GC del plano de control lo recogerá",
                        staging_path.display()
                    );
                }
            }
        }

        // (3) El journal, y solo si el recibo quedó a salvo.
        if recibo_a_salvo && journal_path.exists() {
            if let Err(e) = std::fs::remove_file(journal_path) {
                eprintln!(
                    "lodestar: aviso: no se pudo borrar el journal {} de la {que} {txn_id} ({e}): el \
                     conocimiento ya está publicado y la recuperación completará el sellado al \
                     reabrir",
                    journal_path.display()
                );
            }
        }
    }

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
    /// - [`WorkspaceError::WriteConflict`] si el lock ya está tomado (otro publicador), si la base
    ///   del plan cambió entre plan y apply (E13-H02) o si el canónico cambió entre el cálculo del
    ///   lote y su publicación (E25-H01) — en los tres casos **antes** del primer rename.
    /// - [`WorkspaceError::PermissionDenied`] si algún path afectado cae fuera de `writableRoots` o
    ///   bajo un `referenceRoot` (E11-H04) — comprobado **antes** de tocar el canónico.
    /// - [`WorkspaceError::InvalidResult`] si el resultado hipotético no es conforme (E13-H01).
    /// - [`WorkspaceError::WorkspaceRecoveryRequired`] si una recuperación pendiente no se pudo
    ///   completar antes de publicar.
    /// - [`WorkspaceError::Core`]/[`WorkspaceError::Io`] ante un fallo de normalización o de IO.
    pub fn apply_transaction(
        &self,
        change_set: &ChangeSet,
    ) -> Result<(WorkspaceRevision, WorkspaceRevision, Vec<RelPath>), WorkspaceError> {
        self.apply_transaction_con_recibo(change_set, None)
    }

    /// [`Workspace::apply_transaction`] que además **registra durablemente el recibo antes del punto de
    /// no retorno** (E25-H04).
    ///
    /// Es la variante que usa la fachada (`App::change_apply`), y la única diferencia con la de arriba
    /// es `semantic_diff`: el diff del plan es la única pieza del [`ChangeReceipt`] que esta mecánica no
    /// puede conocer (nace en `change_plan`, no en el disco), así que quien lo tenga lo presta. Con
    /// `Some(..)` la transacción compone el recibo completo —`previousRevision` es la revisión del paso
    /// (3) y `resultRevision` la que estampa el journal, las dos conocidas **antes** del primer
    /// rename— y lo persiste con el journal (`crate::receipts`, `write_pending_receipt`), promoviéndolo a
    /// recibo definitivo al sellar. Con `None` no hay recibo: es el contrato de los llamantes que solo
    /// ejercitan la mecánica de disco.
    ///
    /// Por qué eso importa: sin el registro previo, **cualquier** fallo posterior al primer rename
    /// —incluido un `SIGKILL`— dejaba el canónico cambiado y sin recibo, y una transacción publicada sin
    /// recibo es irreversible **para siempre** (`change_revert` → `PLAN_EXPIRED`, y un segundo apply del
    /// mismo plan → `PLAN_STALE` porque la base ya cambió).
    ///
    /// # Errores
    /// Los mismos que [`Workspace::apply_transaction`], más un [`WorkspaceError::Io`] si el registro del
    /// recibo no se puede escribir — que ocurre **antes** del primer rename, así que no publica nada.
    /// A partir de la publicación ningún fallo se propaga: el cierre (promoción del recibo, limpieza del
    /// staging, borrado del journal) es best-effort con aviso por stderr, porque un `Err` ahí convertiría
    /// una publicación consumada en un fallo aparente.
    pub fn apply_transaction_con_recibo(
        &self,
        change_set: &ChangeSet,
        semantic_diff: Option<&SemanticDiff>,
    ) -> Result<(WorkspaceRevision, WorkspaceRevision, Vec<RelPath>), WorkspaceError> {
        // (1) Lock exclusivo de publicación (RAII: liberado al final por `Drop`, incluso en panic).
        let _lock = self.acquire_lock()?;

        failpoint!(FailPoint::AlEntrar);

        // (2) Recuperación pendiente primero: si una transacción anterior quedó a medias, complétala
        //     o restáurala antes de publicar sobre un estado a medio recuperar (E13-H06). Bajo el
        //     lock, así dos publicadores no recuperan a la vez.
        if self.recovery_pending() {
            self.recover()?;
        }

        // (3) Revisión base actual: será la `previousRevision` del receipt.
        let previous = self.workspace_revision()?;

        // (4) Resultado hipotético del plan: EXACTAMENTE lo que piden las ops normalizadas, sin
        //     aumento alguno (E15-H02 retiró la auto-regeneración de index/tags de E13-H11: un
        //     `index.md` del proyecto es un documento del usuario, no un catálogo que Lodestar
        //     reescriba por su cuenta). El conjunto AFECTADO se computa por diferencia contra el
        //     canónico, así que journal y copias cubren justo ese lote (contrato H06: un `Move`
        //     sigue cubierto).
        let canonical = self.discover_files()?;
        let result_files = plan::apply_normalized_ops(&canonical, &change_set.operations)?;
        let affected = affected_paths(&canonical, &result_files);

        // (5) Guard del único escritor (E11-H04): si algún path afectado no es escribible, se rechaza
        //     ANTES de tocar el canónico (ni staging del canónico, ni backup, ni rename).
        for path in &affected {
            self.assert_writable(path)?;
        }

        // (6) Staging: materializa y valida el resultado hipotético sin tocar el canónico (E13-H01),
        //     de modo que el lote publicado coincida exactamente con lo materializado y validado.
        // E24-H05: `StagingDir` es RAII. Si cualquiera de los pasos (7)–(10) sale por `?`, su
        // `Drop` limpia el árbol — antes se quedaba en disco para siempre, porque el
        // `remove_dir_all` del paso (11) quedaba por debajo del `?`.
        let mut staging = self.materialize_staging_result(&change_set.id, &result_files)?;
        let staging_path = staging.path().to_path_buf();
        self.validate_staging(&staging)?;

        // (7) Control optimista bajo el lock: la base del plan sigue siendo la revisión actual.
        self.reverify_base_revision(&change_set.base_revision)?;

        // Id de transacción: nombra journal, staging, recuperación y receipt (misma convención).
        let txn_id = transaction_id(&change_set.id);
        let writable = &self.config().workspace.writable_roots;
        let result_rev = lodestar_core::types::workspace_revision(&result_files, writable);

        // (8) Copias de recuperación de los originales afectados, ANTES de publicar (E13-H04). Se
        //     conservan al sellar (para el receipt y el revert de H09).
        //     Desde aquí y hasta (9) esta transacción tiene copias y NO tiene ni journal ni recibo,
        //     así que el criterio de «vivos» del GC del plano de control (`journal/` ∪ `receipts/`)
        //     no la ve. Lo que la protege es el lock que sostiene este `_lock`: `gc_receipts` lo
        //     adquiere también (E25-H03) y, al no conseguirlo, no barre.
        self.backup_originals(&txn_id, &affected)?;

        // Seam de test (E25-H03), dentro de la ventana `[backup, journal)`: la transacción tiene
        // copias y todavía no tiene ni journal ni recibo, así que el criterio de «vivos» del GC del
        // plano de control no la ve. Un gancho del test puede congelarla aquí —viva— mientras otro
        // handle barre, que es lo que `failpoint!` (que solo aborta) no sabe hacer. Sin
        // `--features test-failpoints` no genera ni una instrucción.
        #[cfg(feature = "test-failpoints")]
        crate::failpoints::ejecutar_gancho(crate::failpoints::PuntoDeGancho::TrasElBackup);

        failpoint!(FailPoint::TrasBackupSinJournal);

        // (9) Write-ahead journal `prepared`, fsynced antes del primer rename (E13-H03).
        let mut journal = self.create_journal(&txn_id, &affected, &previous, &result_rev)?;

        // (9b) Registro durable del RECIBO, con el journal y antes del primer rename (E25-H04). A
        //      partir del rename siguiente el disco canónico ya está cambiado, así que este es el
        //      último instante en el que escribir el recibo aún sirve de algo: un `SIGKILL` a mitad de
        //      la publicación dejaba, hasta aquí, un cambio consumado que NADIE podía deshacer.
        //      Va DESPUÉS del journal a propósito: el registro es efectivo solo mientras su journal
        //      declare `applied`, así que su vida está contenida en la del journal y ni el GC del plano
        //      de control ni la recuperación necesitan una tercera señal que caducar.
        if let Some(diff) = semantic_diff {
            self.write_pending_receipt(&ChangeReceipt {
                id: ReceiptId(txn_id.clone()),
                change_set_id: change_set.id.clone(),
                previous_revision: previous.clone(),
                result_revision: result_rev.clone(),
                changed_paths: affected.clone(),
                semantic_diff: diff.clone(),
            })?;
        }

        failpoint!(FailPoint::TrasJournalPrepared);

        // Seam de test (E25-H01), último instante de la ventana `[T1, T3)`: con las copias y el
        // journal ya en disco y sin un solo rename hecho, un gancho del test puede simular aquí la
        // edición externa de otro proceso y dejar que la transacción CONTINÚE — que es lo que
        // `failpoint!` no sabe hacer. Sin `--features test-failpoints` no genera ni una instrucción.
        #[cfg(feature = "test-failpoints")]
        crate::failpoints::ejecutar_gancho(crate::failpoints::PuntoDeGancho::AntesDePublicar);

        // (10) Publica el resultado por el único escritor (renames atómicos + journal `applied`,
        //      E13-H05): el mismo `result_files` que se materializó y validó en staging, sobre el
        //      MISMO canónico de T1 con el que se computaron el conjunto afectado, las copias y el
        //      journal. Si el canónico cambió por debajo, `publish_result` aborta con
        //      `WriteConflict` antes del primer rename (E25-H01).
        let result = match self.publish_result(&canonical, &result_files, &mut journal) {
            Ok(result) => result,
            // (10b) Sellado del aborto de ventana (E25-H02, enmienda del defecto 5). El ÚNICO
            //       `WriteConflict` que `publish_result` puede devolver es el de la divergencia de la
            //       ventana `[T1, T3)`, y lo devuelve **antes** de entrar en el bucle de renames: por
            //       control de flujo sabemos que el canónico no se ha movido ni un byte, así que no
            //       hay nada que restaurar y sellar aquí es exacto. Se sella bajo el MISMO lock, antes
            //       de devolver el error, porque si el journal `prepared` y sus copias de T1
            //       sobrevivieran, la recuperación de la siguiente operación las escribiría encima de
            //       la edición externa que este aborto existe para no pisar.
            //
            //       Deliberadamente NO se generaliza a «`recover()` no restaura un journal `prepared`
            //       con cero `applied`»: `mark_applied` re-persiste el journal DESPUÉS de cada rename,
            //       así que ese estado describe también la caída entre el primer rename y su
            //       anotación, y sellarlo daría por buena una publicación parcial. La decisión la toma
            //       el camino que SABE que no publicó.
            Err(WorkspaceError::WriteConflict(motivo)) => {
                let journal_path = journal.path().to_path_buf();
                self.seal_window_abort(&txn_id, &journal_path)?;
                return Err(WorkspaceError::WriteConflict(motivo));
            }
            Err(e) => return Err(e),
        };

        failpoint!(FailPoint::TrasPublicarSinSellar);

        // (11) Sella la transacción: promueve el recibo, limpia el staging y el journal (levanta el gate
        //      de recuperación) pero CONSERVA las copias de recuperación (el receipt y `change_revert`
        //      las usan). El journal ya está `applied` en disco; borrarlo es el sellado `done` efectivo
        //      (tras esto `recovery_pending()` vuelve a `false`).
        // A partir de aquí la limpieza del staging es de este camino, no del `Drop` (E24-H05): se
        // borra DESPUÉS de publicar y en el mismo orden que el sellado del journal.
        //
        // EL BLOQUE ENTERO ES BEST-EFFORT (E25-H04). Está al otro lado del punto de no retorno: el
        // canónico ya cambió, así que un `?` aquí devolvería un error por algo que SÍ se aplicó —el
        // agente concluiría que no se aplicó nada y actuaría sobre una premisa falsa—. Se avisa por
        // stderr y se sigue, con el mismo criterio que ya usaban el GC (`receipts.rs`) y el `.gitignore`
        // gestionado (`gitignore.rs`). Lo que NO se degrada son los `failpoint!`: modelan un proceso que
        // muere, y morirse sí aborta.
        staging.keep();
        let journal_path = journal.path().to_path_buf();
        failpoint!(FailPoint::AntesDeSellar);

        // (11a/b/c) La coreografía de sellado, en el ÚNICO sitio en el que vive (E28-H01): recibo →
        //           staging → journal, con el orden y las garantías best-effort documentados en
        //           [`Workspace::seal_published_transaction`]. La reversión llama a la misma función.
        self.seal_published_transaction(&txn_id, &journal_path, Some(&staging_path), "transacción");

        // (12) Revisión resultante + conjunto de paths cambiados.
        Ok((previous, result, affected))
    }
}
