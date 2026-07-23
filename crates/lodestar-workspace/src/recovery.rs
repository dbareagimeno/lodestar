//! Copias de recuperación (E13-H04) y **crash-recovery determinista** (E13-H06,
//! `ARCHITECTURE.md §19.5`, `REFACTOR §5.2`, `§17`).
//!
//! **H04** — antes de sustituir el conocimiento `.md` canónico, [`Workspace::backup_originals`]
//! guarda el contenido previo de cada fichero afectado bajo `.lodestar/runtime/recovery/<txnId>/`
//! para poder restaurarlo si la publicación falla. Es el eslabón que hace la publicación
//! **recuperable**: con las copias listas, un fallo entre renames puede deshacerse restaurando los
//! originales; los paths que no existían quedan marcados "no existía" (`.absent`) para poder
//! borrarlos al restaurar/revertir.
//!
//! **H06** — al reabrir el workspace, [`Workspace::recover`] escanea los write-ahead journals
//! no-`done` (E13-H03/H05) y, **por el estado global durable del journal**, decide de forma
//! determinista: `applied` (todos los renames hechos, solo falta sellar) → **COMPLETAR** (el
//! canónico ya es el resultado final; solo se limpia staging/recovery y se sella la transacción);
//! `prepared`/`applying` (renames parciales) → **RESTAURAR** el estado anterior desde las copias de
//! H04 (deshacer los renames hechos + borrar los creados que marca `.absent`). Toda escritura del
//! canónico durante la restauración va por el **único escritor** (`io::write_atomic`/`io::delete`,
//! invariante #5), que nunca deja un `.md` parcial. Mientras exista un journal no-`done`,
//! [`Workspace::recovery_pending`] devuelve `true` y las escrituras del canónico se rechazan con
//! `WORKSPACE_RECOVERY_REQUIRED`.
//!
//! Runtime, no canónico: el árbol de recuperación vive bajo `.lodestar/runtime/`, que el walker de
//! conocimiento (`discovery::discover`) y el watcher excluyen (E9-H06) y `WorkspaceRevision` ignora
//! (E10-H03), por lo que no viola «los `.md` son la única fuente de verdad» (invariante #1).
//! Copiar el original solo **lee** el canónico: nunca lo modifica.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::Deserialize;

use lodestar_core::types::{workspace_revision, RelPath, WorkspaceRevision};

use crate::config::WorkspaceConfig;
use crate::error::WorkspaceError;
use crate::journal::JournalState;
use crate::{io, Workspace};

/// Nombre del manifiesto que registra, una línea por path relativo, los ficheros afectados que
/// **no existían** en el canónico al preparar las copias (se van a crear). Vive dentro del
/// directorio de recuperación de la transacción y permite reconstruir el conjunto "no existía" al
/// reabrir (E13-H06/H09) sin depender solo de la memoria.
const ABSENT_MANIFEST: &str = ".absent";

/// Directorio de recuperación de una transacción: contiene una copia **byte-a-byte** del original
/// de cada path afectado que existía en el canónico, bajo `.lodestar/runtime/recovery/<txnId>/`,
/// espejando su ruta relativa; y conoce el conjunto de paths que **no existían** (marcados en el
/// manifiesto `.absent`) para poder borrarlos al revertir.
///
/// La limpieza NO es automática: el flujo de publicación (E13-H05) y la recuperación tras fallo
/// (E13-H06) consumirán estas copias y las retirarán al terminar. Mientras tanto persisten en
/// disco (es su propósito: sobrevivir a un cierre a mitad de publicación).
pub struct RecoveryDir {
    /// Raíz `.lodestar/runtime/recovery/<txnId saneado>/`.
    path: PathBuf,
    /// Paths afectados que no tenían original que copiar (se van a crear).
    absent: BTreeSet<RelPath>,
}

impl RecoveryDir {
    /// El directorio raíz de las copias de recuperación de la transacción (bajo
    /// `.lodestar/runtime/recovery/`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// La ruta donde vive (o viviría) la copia de recuperación de `path` bajo el directorio de la
    /// transacción, espejando su ruta relativa. Existe en disco solo si `path` tenía original que
    /// copiar (véase [`RecoveryDir::was_absent`]).
    pub fn backup_path(&self, path: &RelPath) -> PathBuf {
        self.path.join(path.as_str())
    }

    /// `true` si `path` se marcó "no existía" (no había original que copiar; se creará y habrá que
    /// borrarlo al revertir); `false` si tenía original y se copió byte-a-byte.
    pub fn was_absent(&self, path: &RelPath) -> bool {
        self.absent.contains(path)
    }
}

/// Nombre de directorio saneado para la recuperación de un `txnId` (E13-H04), siguiendo el mismo
/// criterio que el staging (E13-H01) y los planes (E12-H09): se neutraliza cualquier `:`/`/`/`\`
/// (hostil a nombres de fichero en Windows y a la estructura de directorios) por `_`. El resultado
/// es determinista y basta para la trazabilidad del directorio.
fn recovery_dir_name(txn_id: &str) -> String {
    txn_id
        .chars()
        .map(|c| match c {
            ':' | '/' | '\\' => '_',
            other => other,
        })
        .collect()
}

impl Workspace {
    /// Prepara las copias de recuperación de una transacción **antes** de sustituir el canónico
    /// (E13-H04). Para cada `RelPath` de `affected`, si el `.md` existe en el canónico copia su
    /// contenido **byte-a-byte** a `.lodestar/runtime/recovery/<txnId>/<path>` (creando los
    /// subdirectorios del path relativo); si NO existe, lo registra en el manifiesto "no existía"
    /// (`.absent`) sin crear copia, para poder borrarlo al revertir. Devuelve el
    /// [`RecoveryDir`] que referenciará el journal (E13-H03).
    ///
    /// Solo **lee** el canónico: copiar el original nunca modifica los `.md` (invariante #1). La
    /// copia se hace con [`std::fs::copy`] (preserva los bytes exactos, incluido UTF-8 multibyte);
    /// el árbol de recuperación es scratch runtime, así que no necesita el protocolo atómico del
    /// único-escritor que protege los `.md` canónicos.
    ///
    /// Si ya existía una recuperación con el mismo `txnId` (reintento), se limpia antes de
    /// reescribir para que el árbol refleje exactamente el estado actual de los afectados.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla la creación del directorio runtime, la copia de un
    ///   original o la escritura del manifiesto.
    pub fn backup_originals(
        &self,
        txn_id: &str,
        affected: &[RelPath],
    ) -> Result<RecoveryDir, WorkspaceError> {
        let dir = self
            .root
            .join(".lodestar")
            .join("runtime")
            .join("recovery")
            .join(recovery_dir_name(txn_id));

        // Reintento idempotente: parte de un directorio limpio.
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;

        let mut absent = BTreeSet::new();
        for path in affected {
            let original = self.root.join(path.as_str());
            if original.is_file() {
                // Existe original: copia byte-a-byte, espejando la ruta relativa.
                let target = dir.join(path.as_str());
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::copy(&original, &target)?;
            } else {
                // No existía: se marca "no existía" (se creará y habrá que borrarlo al revertir).
                absent.insert(path.clone());
            }
        }

        // Persiste el conjunto "no existía" en el manifiesto (una línea por path), para poder
        // reconstruirlo al reabrir tras un fallo (E13-H06/H09) sin depender solo de la memoria.
        let manifest: String = absent.iter().map(|p| format!("{}\n", p.as_str())).collect();
        std::fs::write(dir.join(ABSENT_MANIFEST), manifest)?;

        Ok(RecoveryDir { path: dir, absent })
    }
}

/// Cabecera mínima del write-ahead journal que la recuperación (E13-H06) necesita leer del JSON en
/// disco: el estado global y el `txnId`. Los demás campos (`operations`, revisiones) se ignoran a
/// propósito — la restauración deriva el conjunto de paths afectados del **árbol de recuperación**
/// de H04, no de la lista de operaciones del journal, de modo que converge igual aunque el journal
/// esté torn (los renames del canónico solo ocurren TRAS crear esas copias).
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalHeader {
    txn_id: String,
    state: JournalState,
}

/// Reconstruye el conjunto "no existía" desde el manifiesto `.absent` de un directorio de
/// recuperación (una línea por path relativo). Un directorio o manifiesto ausente/ilegible produce
/// un conjunto vacío (no había nada que crear que borrar).
fn read_absent_manifest(recovery_root: &Path) -> Vec<RelPath> {
    let Ok(raw) = std::fs::read_to_string(recovery_root.join(ABSENT_MANIFEST)) else {
        return Vec::new();
    };
    raw.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter_map(|l| RelPath::new(l).ok())
        .collect()
}

impl Workspace {
    /// El directorio de write-ahead journals de la transacción (`.lodestar/runtime/journal/`).
    fn journal_dir(&self) -> PathBuf {
        self.root.join(".lodestar").join("runtime").join("journal")
    }

    /// Rutas de los write-ahead journals **pendientes de recuperar** bajo
    /// `.lodestar/runtime/journal/`: todo `<txnId>.json` cuyo estado global no sea `done` —o cuyo
    /// JSON sea ilegible/torn, que también exige recuperación conservadora—. Con `exclude =
    /// Some(path)` se omite el journal de ese nombre de fichero: lo usa [`Workspace::publish`] para
    /// no confundir el registro write-ahead de la transacción en curso (recién creado en
    /// `prepared`) con una recuperación pendiente de una transacción anterior.
    ///
    /// Comprobación perezosa por disco (sin estado en el handle): el JSON del journal es la fuente
    /// de verdad recuperable, así que reabrir el workspace y consultar esto refleja siempre lo que
    /// hay durable en disco.
    pub(crate) fn pending_journals(&self, exclude: Option<&Path>) -> Vec<PathBuf> {
        let exclude_name = exclude.and_then(|p| p.file_name());
        let mut out = Vec::new();
        let Ok(entries) = std::fs::read_dir(self.journal_dir()) else {
            return out;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            if exclude_name.is_some() && path.file_name() == exclude_name {
                continue;
            }
            // `done` está sellado (nada que recuperar); cualquier otro estado —o un JSON
            // ilegible/torn— cuenta como recuperación pendiente.
            let done = std::fs::read_to_string(&path)
                .ok()
                .and_then(|raw| serde_json::from_str::<JournalHeader>(&raw).ok())
                .is_some_and(|h| h.state == JournalState::Done);
            if !done {
                out.push(path);
            }
        }
        out
    }

    /// `true` si hay una recuperación de publicación **pendiente** (E13-H06): existe algún
    /// write-ahead journal no-`done` (o torn) bajo `.lodestar/runtime/journal/`. Mientras lo haya,
    /// las escrituras del canónico se rechazan con `WORKSPACE_RECOVERY_REQUIRED` (gate interno)
    /// hasta que [`Workspace::recover`] complete/restaure la transacción interrumpida.
    pub fn recovery_pending(&self) -> bool {
        !self.pending_journals(None).is_empty()
    }

    /// Ejecuta la **recuperación determinista** de toda transacción de publicación interrumpida cuyo
    /// write-ahead journal quedó no-`done` (E13-H03/H05). Explícita (no un efecto colateral de
    /// `open`): la fachada la invoca al detectar una recuperación pendiente.
    ///
    /// Por cada journal pendiente, decide **por su estado global durable** (la única fuente de
    /// verdad recuperable):
    /// - `applied` → **COMPLETAR**: todos los renames se hicieron antes de caer; el canónico ya es
    ///   el resultado final, así que solo se limpia el staging/recovery y se sella la transacción.
    /// - `prepared`/`applying` → **RESTAURAR**: se deshace la transacción devolviendo el canónico a
    ///   su estado anterior desde las copias de H04 (restaurar cada original respaldado y borrar los
    ///   paths que `.absent` marcó "no existía"), y luego se limpia y sella.
    ///
    /// Convergencia sin parciales: la decisión depende SOLO del estado durable del journal (nunca de
    /// cuántos renames se llegaron a ver en disco) y toda escritura del canónico va por el único
    /// escritor (`io::write_atomic`, temp+fsync+rename / `io::delete`), que jamás deja un `.md` a
    /// medias. Por eso el conocimiento converge determinista a UNO de los dos bordes de la
    /// transacción —todo el original íntegro o todo el resultado íntegro—, para cualquier punto de
    /// caída.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla la restauración/limpieza sobre el runtime o el canónico.
    pub fn recover(&self) -> Result<(), WorkspaceError> {
        for journal_path in self.pending_journals(None) {
            let header = std::fs::read_to_string(&journal_path)
                .ok()
                .and_then(|raw| serde_json::from_str::<JournalHeader>(&raw).ok());

            match header {
                // COMPLETAR: el canónico ya es el resultado (todos los renames hechos).
                Some(h) if h.state == JournalState::Applied => {
                    self.finish_recovery(&h.txn_id, &journal_path)?;
                }
                // RESTAURAR el estado anterior (renames parciales).
                Some(h) => {
                    self.restore_from_recovery(&h.txn_id)?;
                    self.finish_recovery(&h.txn_id, &journal_path)?;
                }
                // Journal torn (JSON ilegible/truncado): política defensiva. `write_journal`
                // persiste atómico (temp+rename), así que un torn es rarísimo; aun así NO se
                // paniquea. Como los renames del canónico solo ocurren TRAS crear las copias de
                // recuperación (H04), restaurar desde el árbol de recuperación (si existe) deshace
                // cualquier rename parcial; si no existe, la caída fue antes de tocar el canónico y
                // no hay nada que restaurar. En ambos casos se converge al estado ANTERIOR (opción
                // conservadora: ante la duda, no dar por buena una transacción cuyo registro no se
                // puede leer). El `txnId` se toma del nombre del fichero `<txnId>.json`.
                None => {
                    let txn_id = journal_path
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or_default()
                        .to_string();
                    eprintln!(
                        "lodestar: aviso: journal de recuperación ilegible {}: se restaura \
                         conservadoramente al estado anterior desde las copias de recuperación",
                        journal_path.display()
                    );
                    self.restore_from_recovery(&txn_id)?;
                    self.finish_recovery(&txn_id, &journal_path)?;
                }
            }
        }
        Ok(())
    }

    /// **RESTAURAR** (E13-H06): devuelve el conocimiento canónico a su estado anterior a la
    /// transacción `txn_id` usando exclusivamente las copias de recuperación de H04
    /// (`.lodestar/runtime/recovery/<txnId>/`). Deriva el conjunto afectado del propio árbol de
    /// recuperación (robusto ante un journal torn):
    /// 1. cada fichero del árbol (salvo el manifiesto `.absent`) es la copia byte-a-byte de un
    ///    original que se sobrescribió → se devuelve a su sitio con `io::write_atomic`;
    /// 2. cada path del manifiesto `.absent` no existía antes → si la transacción lo creó, se borra
    ///    con `io::delete` (idempotente si no llegó a crearse).
    ///
    /// Si el directorio de recuperación no existe, la caída fue antes del backup de H04 (que precede
    /// a todo rename), así que el canónico está intacto y no hay nada que restaurar.
    fn restore_from_recovery(&self, txn_id: &str) -> Result<(), WorkspaceError> {
        let recovery_root = self
            .root
            .join(".lodestar")
            .join("runtime")
            .join("recovery")
            .join(recovery_dir_name(txn_id));
        if !recovery_root.exists() {
            return Ok(());
        }

        // (1) Restaurar cada original respaldado (deshace los renames que sí ocurrieron).
        self.restore_backups(&recovery_root, &recovery_root)?;

        // (2) Borrar los paths marcados "no existía" que la transacción pudo crear.
        for rel in read_absent_manifest(&recovery_root) {
            io::delete(&self.root, &rel)?;
        }
        Ok(())
    }

    /// Recorre el árbol de recuperación restaurando por el único escritor cada copia de un original
    /// a su ruta canónica (espejando la ruta relativa bajo `recovery_root`). Salta el manifiesto
    /// `.absent` (no es una copia). Auxiliar recursivo de [`Workspace::restore_from_recovery`].
    fn restore_backups(&self, dir: &Path, recovery_root: &Path) -> Result<(), WorkspaceError> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                self.restore_backups(&path, recovery_root)?;
                continue;
            }
            if path.file_name().and_then(|n| n.to_str()) == Some(ABSENT_MANIFEST) {
                continue;
            }
            let rel = path
                .strip_prefix(recovery_root)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            let Ok(rp) = RelPath::new(&rel) else {
                continue;
            };
            let content = std::fs::read_to_string(&path).map_err(|e| {
                WorkspaceError::Io(format!(
                    "copia de recuperación ilegible {}: {e}",
                    path.display()
                ))
            })?;
            io::write_atomic(&self.root, &rp, &content)?;
        }
        Ok(())
    }

    /// Sella una transacción recuperada (E13-H06): limpia el staging (`.lodestar/runtime/staging/
    /// <txnId>/`) y las copias de recuperación (`.lodestar/runtime/recovery/<txnId>/`), y **borra el
    /// fichero de journal** para levantar el gate (tras esto ya no queda ningún journal no-`done`,
    /// de modo que [`Workspace::recovery_pending`] vuelve a `false` y las escrituras se permiten).
    ///
    /// El `txnId` (sin prefijo `changeset:`) nombra por igual el staging, la recuperación y el
    /// journal (convención de E13-H06), así que un mismo nombre saneado localiza los tres.
    fn finish_recovery(&self, txn_id: &str, journal_path: &Path) -> Result<(), WorkspaceError> {
        let name = recovery_dir_name(txn_id);
        let runtime = self.root.join(".lodestar").join("runtime");

        let staging = runtime.join("staging").join(&name);
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        let recovery = runtime.join("recovery").join(&name);
        if recovery.exists() {
            std::fs::remove_dir_all(&recovery)?;
        }
        if journal_path.exists() {
            std::fs::remove_file(journal_path)?;
        }
        Ok(())
    }

    /// El directorio raíz de las copias de recuperación de una transacción
    /// (`.lodestar/runtime/recovery/<txnId saneado>/`), exista o no.
    fn recovery_root(&self, txn_id: &str) -> PathBuf {
        self.root
            .join(".lodestar")
            .join("runtime")
            .join("recovery")
            .join(recovery_dir_name(txn_id))
    }

    /// Recoge, en orden determinista por [`RelPath`], las copias byte-a-byte del árbol de
    /// recuperación de `recovery_root` (cada fichero salvo el manifiesto `.absent`), como pares
    /// `(rutaCanónica, contenido)`. Auxiliar de [`Workspace::revert_transaction`] (no toca disco:
    /// solo lee las copias).
    fn collect_backups(
        &self,
        recovery_root: &Path,
    ) -> Result<Vec<(RelPath, String)>, WorkspaceError> {
        let mut out = std::collections::BTreeMap::new();
        collect_backups_into(recovery_root, recovery_root, &mut out)?;
        Ok(out.into_iter().collect())
    }

    /// Revierte la transacción `orig_txn_id` como una **nueva transacción inversa recuperable**
    /// (E13-H09, `ARCHITECTURE.md §19.5/§19.6`, `REFACTOR §11.3`), devolviendo el conocimiento
    /// canónico al estado ANTERIOR a `orig_txn_id` desde sus copias de recuperación (E13-H04).
    ///
    /// Toda escritura del canónico va por el **único escritor** (invariante #5): las copias
    /// respaldadas se restauran con `io::write_atomic` y los paths que se habían creado (marcados
    /// `.absent`) se borran con `io::delete`. La reversión es ella misma **recuperable**: bajo el
    /// lock de publicación (E13-H02) respalda el estado ACTUAL de los afectados en su propio árbol
    /// de recuperación (`new_txn_id`) y registra su intención en un write-ahead journal propio
    /// (E13-H03) **antes** del primer rename, de modo que una caída a mitad converge determinista al
    /// reabrir (E13-H06). Devuelve `(previousRevision, resultRevision, changedPaths)`: la
    /// [`WorkspaceRevision`] antes (== `resultRevision` del apply original) y después (==
    /// `previousRevision` del apply original) de la reversión, y los paths que restauró.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si faltan las copias de recuperación de `orig_txn_id` (no se puede
    ///   revertir: transacción no disponible), o ante un fallo de IO de la restauración.
    /// - [`WorkspaceError::WriteConflict`] si el lock ya está tomado (otro publicador).
    /// - [`WorkspaceError::PermissionDenied`] si algún path afectado ya no es escribible.
    pub fn revert_transaction(
        &self,
        orig_txn_id: &str,
        new_txn_id: &str,
    ) -> Result<(WorkspaceRevision, WorkspaceRevision, Vec<RelPath>), WorkspaceError> {
        // (1) Lock exclusivo de publicación (RAII: liberado al final por `Drop`, incluso en panic).
        let _lock = self.acquire_lock()?;

        // (2) Recuperación pendiente primero: nunca se revierte sobre un estado a medio recuperar.
        if self.recovery_pending() {
            self.recover()?;
        }

        // (3) Localizar el árbol de recuperación de la transacción a revertir. Si no está, la
        //     transacción ya no es reversible (copias purgadas por el GC, E13-H07).
        let recovery_root = self.recovery_root(orig_txn_id);
        if !recovery_root.exists() {
            return Err(WorkspaceError::Io(format!(
                "no hay copias de recuperación para la transacción {orig_txn_id}: no se puede \
                 revertir"
            )));
        }

        // (4) Conjunto afectado = originales respaldados (a restaurar) + paths creados (a borrar).
        let backups = self.collect_backups(&recovery_root)?;
        let absent = read_absent_manifest(&recovery_root);
        let mut affected_set: BTreeSet<RelPath> = BTreeSet::new();
        for (rel, _) in &backups {
            affected_set.insert(rel.clone());
        }
        for rel in &absent {
            affected_set.insert(rel.clone());
        }
        let affected: Vec<RelPath> = affected_set.into_iter().collect();

        // (5) Guard del único escritor (E11-H04): los afectados deben seguir siendo escribibles.
        for path in &affected {
            self.assert_writable(path)?;
        }

        // (6) Revisión actual (== `resultRevision` del apply, ya re-verificado por la fachada) y
        //     resultado hipotético de la reversión (canónico con backups restaurados / creados
        //     borrados) para estampar la `resultRevision` en el journal ANTES de tocar el canónico.
        let previous = self.workspace_revision()?;
        let canonical = self.discover_files()?;
        let mut result_files = canonical.clone();
        for (rel, content) in &backups {
            result_files.insert(rel.clone(), content.clone());
        }
        for rel in &absent {
            result_files.remove(rel);
        }
        let writable = WorkspaceConfig::load(&self.root)
            .unwrap_or_default()
            .workspace
            .writable_roots;
        let result_rev = workspace_revision(&result_files, &writable);

        // (7) Copias de recuperación de la INVERSA (respalda el estado actual) → la reversión es
        //     recuperable (E13-H04): si cae a mitad, `recover` restaura desde `recovery/<new>/`.
        self.backup_originals(new_txn_id, &affected)?;

        // (8) Write-ahead journal `prepared` de la inversa, fsynced antes del primer rename (H03).
        let mut journal = self.create_journal(new_txn_id, &affected, &previous, &result_rev)?;

        // (9) Restaura por el único escritor: escribe cada original respaldado; borra los creados.
        for (rel, content) in &backups {
            io::write_atomic(&self.root, rel, content)?;
            journal.mark_applied(rel)?;
        }
        for rel in &absent {
            io::delete(&self.root, rel)?;
            journal.mark_applied(rel)?;
        }
        journal.mark_all_applied()?;

        // (10) Sella: borra el fichero de journal (levanta el gate de recuperación). Conserva las
        //      copias de recuperación de la inversa (el receipt de la reversión las referencia; el
        //      GC de E13-H07 las purgará con su recibo).
        let journal_path = journal.path().to_path_buf();
        if journal_path.exists() {
            std::fs::remove_file(&journal_path)?;
        }

        // (11) Revisión resultante (== `previousRevision` del apply original) + paths restaurados.
        let result = self.workspace_revision()?;
        Ok((previous, result, affected))
    }
}

/// Recorre el árbol de recuperación bajo `dir` acumulando en `out` cada copia de un original
/// (`RelPath` espejado bajo `recovery_root` → contenido byte-a-byte), saltando el manifiesto
/// `.absent`. Auxiliar recursivo de [`Workspace::collect_backups`].
fn collect_backups_into(
    dir: &Path,
    recovery_root: &Path,
    out: &mut std::collections::BTreeMap<RelPath, String>,
) -> Result<(), WorkspaceError> {
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        if path.is_dir() {
            collect_backups_into(&path, recovery_root, out)?;
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some(ABSENT_MANIFEST) {
            continue;
        }
        let rel = path
            .strip_prefix(recovery_root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        let Ok(rp) = RelPath::new(&rel) else {
            continue;
        };
        let content = std::fs::read_to_string(&path).map_err(|e| {
            WorkspaceError::Io(format!(
                "copia de recuperación ilegible {}: {e}",
                path.display()
            ))
        })?;
        out.insert(rp, content);
    }
    Ok(())
}
