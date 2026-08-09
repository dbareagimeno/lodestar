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
//! canónico ya es el resultado final; se limpia el staging, se promueve el registro durable del recibo
//! —E25-H04— y se sella la transacción **conservando** sus copias, que es lo que la deja reversible);
//! `prepared`/`applying` (renames parciales) → **RESTAURAR** el estado anterior desde las copias de
//! H04 (deshacer los renames hechos + borrar los creados que marca `.absent`). Toda escritura del
//! canónico durante la restauración va por el **único escritor** (`io::write_atomic`/`io::delete`,
//! invariante #5), que nunca deja un `.md` parcial. Mientras exista un journal no-`done`,
//! [`Workspace::recovery_pending`] devuelve `true` y las escrituras del canónico se rechazan con
//! `WORKSPACE_RECOVERY_REQUIRED`.
//!
//! **E25-H02** — las copias dejan de ser el eslabón débil de esa promesa. Se escriben con el mismo
//! protocolo durable que el único escritor (contenido volcado con `sync_all`, fsync del árbol una vez
//! al terminar, **antes** de que la transacción avance al journal), se registra la **huella** de cada
//! original (tamaño + revisión blake3) en el sidecar `recovery/<txnId>.digests.json`, y
//! [`Workspace::recover`] **verifica antes de escribir**: una copia que no casa no se restaura. Un
//! journal que no se puede recuperar ya no cierra el workspace para siempre — su material se mueve
//! íntegro a `journal/quarantine/<txnId>/`, la recuperación sigue con los demás y el fallo se reporta
//! una vez con `RECOVERY_FAILED`. Y el aborto de la ventana de publicación (E25-H01) **sella su
//! propia transacción** ([`Workspace::seal_window_abort`]), para que la recuperación de la siguiente
//! operación no deshaga lo que ese aborto acababa de proteger.
//!
//! Promesa declarada tras E25-H02: «el canónico converge a uno de los dos bordes» es incondicional
//! **mientras las copias verifiquen**. Con copias corruptas lo garantizado es (a) nada se escribe a
//! partir de una copia que no verifica, (b) el material se preserva en cuarentena, (c) el fallo lleva
//! código propio y (d) el workspace vuelve a ser escribible.
//!
//! Runtime, no canónico: el árbol de recuperación vive bajo `.lodestar/runtime/`, que el walker de
//! conocimiento (`discovery::discover`) y el watcher excluyen (E9-H06) y `WorkspaceRevision` ignora
//! (E10-H03), por lo que no viola «los `.md` son la única fuente de verdad» (invariante #1).
//! Copiar el original solo **lee** el canónico: nunca lo modifica.

use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use lodestar_core::types::{
    workspace_revision, ChangeReceipt, ChangeSetId, FileMap, ReceiptId, RelPath, SemanticDiff,
    WorkspaceRevision,
};

use crate::error::WorkspaceError;
#[cfg(feature = "test-failpoints")]
use crate::failpoints::FailPoint;
use crate::journal::JournalState;
use crate::{io, Workspace};

/// Nombre del manifiesto que registra, una línea por path relativo, los ficheros afectados que
/// **no existían** en el canónico al preparar las copias (se van a crear). Vive dentro del
/// directorio de recuperación de la transacción y permite reconstruir el conjunto "no existía" al
/// reabrir (E13-H06/H09) sin depender solo de la memoria.
const ABSENT_MANIFEST: &str = ".absent";

/// Sufijo del **sidecar de huellas** de una transacción (E25-H02):
/// `.lodestar/runtime/recovery/<txnId>.digests.json`, HERMANO del árbol de copias y nunca dentro de
/// él.
///
/// Vive fuera del árbol a propósito: el contrato «lo publicado es lo respaldado» (E25-H01) se mide
/// tratando **cada** fichero de `recovery/<txnId>/` como la copia de un path respaldado, y
/// `restore_backups_legacy`/`collect_backups` restauran con ese mismo criterio. Un fichero de
/// metadatos dentro del árbol se restauraría como si fuera un documento del usuario.
const DIGESTS_SUFFIX: &str = ".digests.json";

/// Subdirectorio de **cuarentena** del plano de control (E25-H02):
/// `.lodestar/runtime/journal/quarantine/<txnId>/`, donde acaba el material de una transacción cuya
/// recuperación falló. Cuelga de `journal/` y no interfiere con el gate de recuperación:
/// [`Workspace::pending_journals`] solo mira ficheros con extensión `.json` y el GC del plano de
/// control (E24-H06) solo barre `staging/` y `recovery/`.
const QUARANTINE_DIR: &str = "quarantine";

/// Huella durable de una copia de recuperación: el tamaño en bytes del original y su revisión de
/// contenido blake3 (`revision`), tal y como estaban al respaldarlo.
///
/// La revisión se calcula con la **única** función de identidad de contenido del core
/// ([`workspace_revision`] sobre un solo par path→contenido, invariante #3): no hay una segunda
/// forma de hashear un documento en el árbol. Es `None` solo si el original no era UTF-8 válido y
/// por tanto no se pudo interpretar como documento — un caso en el que la restauración fallaría de
/// todos modos, porque el único escritor escribe texto.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupDigest {
    /// Tamaño en bytes de la copia tal y como se escribió.
    size: u64,
    /// Revisión de contenido (`"blake3:<hex>"`) del original respaldado, si era texto.
    revision: Option<String>,
}

/// Manifiesto durable de las copias de recuperación de una transacción (E25-H02), persistido en el
/// sidecar `recovery/<txnId>.digests.json` **antes** de que la transacción avance al journal.
///
/// Es lo que convierte la restauración en verificable: describe, para el lote afectado completo, qué
/// paths tenían original (con la huella de su copia) y qué paths **no existían**. Al restaurar se
/// compara copia a copia contra estas huellas y se decide con este `absent` —no con el manifiesto
/// `.absent` del árbol—, de modo que perder ese manifiesto ya no deja el canónico a medio camino
/// entre los dos bordes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct BackupManifest {
    /// Identificador de la transacción a la que pertenecen las copias.
    txn_id: String,
    /// Paths afectados que tenían original → huella de su copia byte-a-byte.
    copies: BTreeMap<String, BackupDigest>,
    /// Paths afectados que **no existían** en el canónico (se iban a crear).
    absent: Vec<String>,
}

/// Huella de contenido de un documento respaldado: `(tamaño en bytes, revisión blake3)`, con la
/// revisión a `None` si el contenido no es UTF-8 válido (no es un documento que el único escritor
/// pueda republicar).
fn huella(rel: &RelPath, bytes: &[u8]) -> BackupDigest {
    let revision = std::str::from_utf8(bytes).ok().map(|texto| {
        let mut uno = FileMap::new();
        uno.insert(rel.clone(), texto.to_string());
        workspace_revision(&uno, &[]).0
    });
    BackupDigest {
        size: bytes.len() as u64,
        revision,
    }
}

/// Directorio de recuperación de una transacción: contiene una copia **byte-a-byte** del original
/// de cada path afectado que existía en el canónico, bajo `.lodestar/runtime/recovery/<txnId>/`,
/// espejando su ruta relativa; y conoce el conjunto de paths que **no existían** (marcados en el
/// manifiesto `.absent`) para poder borrarlos al revertir.
///
/// Desde E25-H02 le acompaña, **fuera** del árbol, el sidecar de huellas
/// `recovery/<txnId>.digests.json`: los dos van siempre juntos (el sidecar sin árbol no describe nada;
/// el árbol sin sidecar no es verificable) y se descartan a la vez al sellar la recuperación.
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
///
/// Delega en `crate::receipts::sanear_nombre` (E25-H04): el saneado tiene **una** implementación en el
/// crate, porque de que los cuatro nombres derivados del `txnId` coincidan depende que se localicen
/// entre sí.
fn recovery_dir_name(txn_id: &str) -> String {
    crate::receipts::sanear_nombre(txn_id)
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
    /// copia preserva los bytes exactos (incluido UTF-8 multibyte) y se escribe con el **mismo
    /// protocolo durable que el único escritor** (`io::write_bytes_atomic`: temporal + `sync_all` +
    /// rename), con un fsync del árbol de recuperación **una sola vez** al terminar (E25-H02).
    ///
    /// Ese orden —copias durables → journal durable → renames— es lo que hace verdad la premisa de
    /// la recuperación. Hasta v0.3.1 las copias se hacían con [`std::fs::copy`] y el manifiesto con
    /// [`std::fs::write`], sin volcado alguno, mientras el journal SÍ se fsyncaba antes del primer
    /// rename: un corte de energía podía dejar un journal **durable** apuntando a una copia
    /// **truncada o ausente**, justo la combinación que la recuperación daba por buena.
    ///
    /// Además de las copias, registra su **huella** (tamaño + revisión blake3) en el sidecar
    /// `recovery/<txnId>.digests.json` —hermano del árbol, nunca dentro— para que
    /// [`Workspace::recover`] pueda verificar cada copia **antes** de escribirla sobre el canónico y
    /// para que el conjunto "no existía" sobreviva a la pérdida del manifiesto `.absent`.
    ///
    /// Si ya existía una recuperación con el mismo `txnId` (reintento), se limpia antes de
    /// reescribir para que el árbol refleje exactamente el estado actual de los afectados.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla la creación del directorio runtime, la copia de un
    ///   original o la escritura del manifiesto o del sidecar de huellas.
    pub fn backup_originals(
        &self,
        txn_id: &str,
        affected: &[RelPath],
    ) -> Result<RecoveryDir, WorkspaceError> {
        let dir = self.recovery_root(txn_id);

        // Reintento idempotente: parte de un directorio limpio (y de un sidecar limpio).
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        std::fs::create_dir_all(&dir)?;
        let digests = self.digests_path(txn_id);
        if digests.exists() {
            std::fs::remove_file(&digests)?;
        }

        let mut absent = BTreeSet::new();
        let mut copies: BTreeMap<String, BackupDigest> = BTreeMap::new();
        // Directorios cuyos renames hay que persistir al final (una sola pasada de fsync).
        let mut a_sincronizar: BTreeSet<PathBuf> = BTreeSet::new();
        a_sincronizar.insert(dir.clone());

        for path in affected {
            let original = self.root.join(path.as_str());
            if original.is_file() {
                // Existe original: copia byte-a-byte DURABLE, espejando la ruta relativa.
                let target = dir.join(path.as_str());
                if let Some(parent) = target.parent() {
                    std::fs::create_dir_all(parent)?;
                    a_sincronizar.insert(parent.to_path_buf());
                }
                let bytes = std::fs::read(&original)?;
                io::write_bytes_atomic(&target, &bytes)?;
                copies.insert(path.as_str().to_string(), huella(path, &bytes));
            } else {
                // No existía: se marca "no existía" (se creará y habrá que borrarlo al revertir).
                absent.insert(path.clone());
            }
        }

        // Persiste el conjunto "no existía" en el manifiesto (una línea por path), para poder
        // reconstruirlo al reabrir tras un fallo (E13-H06/H09) sin depender solo de la memoria.
        //
        // E24-H06: solo si hay algo que anotar. Escribirlo siempre dejaba, en toda transacción que
        // únicamente CREA ficheros, un `recovery/<txn>/` con un `.absent` vacío como único
        // contenido — un directorio que sobrevive al sellado (el paso (11) conserva las copias a
        // propósito) y que solo desaparecía si el GC llegaba a purgar su recibo homónimo.
        // `read_absent_manifest` ya trata la ausencia del fichero como conjunto vacío.
        if !absent.is_empty() {
            let manifest: String = absent.iter().map(|p| format!("{}\n", p.as_str())).collect();
            io::write_bytes_atomic(&dir.join(ABSENT_MANIFEST), manifest.as_bytes())?;
        }

        // Sidecar de huellas del lote completo (copias + "no existía"), hermano del árbol.
        let manifiesto = BackupManifest {
            txn_id: txn_id.to_string(),
            copies,
            absent: absent.iter().map(|p| p.as_str().to_string()).collect(),
        };
        let json = serde_json::to_vec_pretty(&manifiesto).map_err(|e| {
            WorkspaceError::Io(format!(
                "no se pudo serializar las huellas de las copias: {e}"
            ))
        })?;
        io::write_bytes_atomic(&digests, &json)?;

        // Fsync del árbol de recuperación UNA vez, al terminar: hasta aquí las copias tienen su
        // contenido volcado (`sync_all`), pero sus NOMBRES podían perderse en una caída. Se sincroniza
        // también el directorio padre, que es donde vive el sidecar. Después de este punto la
        // transacción puede avanzar al journal.
        //
        // E25-H05: su fallo se PROPAGA (antes se ignoraba). Estamos **antes** del primer rename, así
        // que abortar aquí no deja nada publicado; y seguir sería avanzar al journal declarando
        // durables unas copias que quizá no lo son — justo la premisa que la recuperación da por
        // buena para escribirlas encima del canónico.
        for d in &a_sincronizar {
            io::sync_dir(d)?;
        }
        if let Some(padre) = digests.parent() {
            io::sync_dir(padre)?;
        }

        Ok(RecoveryDir { path: dir, absent })
    }

    /// Ruta del **sidecar de huellas** de una transacción
    /// (`.lodestar/runtime/recovery/<txnId>.digests.json`), exista o no.
    fn digests_path(&self, txn_id: &str) -> PathBuf {
        self.root
            .join(".lodestar")
            .join("runtime")
            .join("recovery")
            .join(format!("{}{DIGESTS_SUFFIX}", recovery_dir_name(txn_id)))
    }

    /// Lee el manifiesto de huellas de `txn_id`: `Ok(None)` si el sidecar no existe (transacción
    /// respaldada por una versión anterior a E25-H02, o caída antes del backup), `Err` si existe pero
    /// no se puede leer o interpretar — en ese caso las copias eran verificables por contrato y ya no
    /// lo son, así que la restauración no puede darse por buena.
    fn read_backup_manifest(&self, txn_id: &str) -> Result<Option<BackupManifest>, WorkspaceError> {
        let path = self.digests_path(txn_id);
        if !path.is_file() {
            return Ok(None);
        }
        let raw = std::fs::read(&path).map_err(|e| {
            WorkspaceError::RecoveryFailed(format!(
                "las huellas de las copias de recuperación son ilegibles ({}): {e}",
                path.display()
            ))
        })?;
        let manifiesto = serde_json::from_slice::<BackupManifest>(&raw).map_err(|e| {
            WorkspaceError::RecoveryFailed(format!(
                "las huellas de las copias de recuperación no se pueden interpretar ({}): {e}",
                path.display()
            ))
        })?;
        Ok(Some(manifiesto))
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

    /// Estado global **durable** del write-ahead journal de `txn_id`, leído del disco, o `None` si no
    /// hay journal para esa transacción (o su JSON es ilegible/torn).
    ///
    /// Es la misma lectura con la que [`Workspace::recover`] decide COMPLETAR frente a RESTAURAR, y por
    /// tanto la **única** fuente de verdad sobre «esta transacción llegó a publicar» (invariante #3).
    /// La consulta el registro durable del recibo (E25-H04, `crate::receipts`) para decidir si es
    /// efectivo: sin ella habría que inventar un segundo juicio sobre lo mismo.
    ///
    /// Localiza el fichero por la convención de [`Workspace::create_journal`]
    /// (`journal/<txnId saneado>.json`), con el **mismo** saneado que nombra el registro durable del
    /// recibo: si los dos no coincidieran, un `txnId` exótico dejaría al pendiente sin poder encontrar
    /// su journal —y por tanto nunca efectivo— sin que nada lo dijera. Como el saneado es idempotente,
    /// da igual que `txn_id` llegue crudo (del cuerpo del journal) o ya saneado (del nombre de un
    /// fichero).
    pub(crate) fn journal_state_of(&self, txn_id: &str) -> Option<JournalState> {
        let path = self
            .journal_dir()
            .join(format!("{}.json", recovery_dir_name(txn_id)));
        let raw = std::fs::read_to_string(path).ok()?;
        serde_json::from_str::<JournalHeader>(&raw)
            .ok()
            .map(|h| h.state)
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
    ///   el resultado final, así que solo se limpia el staging, se **da por bueno el recibo** (E25-H04:
    ///   el registro durable escrito con el journal se promueve a recibo definitivo) y se sella la
    ///   transacción. Las copias de recuperación **se conservan**: la transacción publicó y tiene
    ///   recibo, así que sigue siendo reversible como cualquier otra que hubiera sellado sin morirse.
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
    /// Un journal cuya recuperación **falla** (copia ausente, ilegible o que no verifica) no encalla
    /// el workspace (E25-H02): su material se **mueve** a `journal/quarantine/<txnId>/` sin borrar
    /// nada, la recuperación **sigue** con los demás journals pendientes y el fallo se reporta una
    /// sola vez con [`WorkspaceError::RecoveryFailed`], nombrando la cuarentena. Al levantarse el
    /// gate, la siguiente operación ya no encuentra recuperación pendiente y procede.
    ///
    /// # Errores
    /// - [`WorkspaceError::RecoveryFailed`] si la recuperación de al menos un journal no se pudo
    ///   llevar a término (su material queda en cuarentena).
    /// - [`WorkspaceError::Io`] si falla la propia puesta en cuarentena.
    pub fn recover(&self) -> Result<(), WorkspaceError> {
        let mut fallos: Vec<String> = Vec::new();

        for journal_path in self.pending_journals(None) {
            let header = std::fs::read_to_string(&journal_path)
                .ok()
                .and_then(|raw| serde_json::from_str::<JournalHeader>(&raw).ok());

            let (txn_id, restaurar) = match header {
                // COMPLETAR: el canónico ya es el resultado (todos los renames hechos).
                Some(h) if h.state == JournalState::Applied => (h.txn_id, false),
                // RESTAURAR el estado anterior (renames parciales).
                Some(h) => (h.txn_id, true),
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
                    (txn_id, true)
                }
            };

            // Una transacción a la vez: su fallo no puede abortar el bucle (defecto 3 de E25-H02).
            // La vía COMPLETAR no puede fallar —y por eso no devuelve `Result`, E25-H04—: la
            // cuarentena se lleva el árbol de copias, y una transacción que SÍ publicó y ya tiene
            // recibo no puede acabar como material forense por un fallo al limpiar detrás de ella.
            let resultado = if restaurar {
                self.restore_from_recovery(&txn_id)
                    .and_then(|()| self.finish_recovery(&txn_id, &journal_path))
            } else {
                self.finish_recovery_completada(&txn_id, &journal_path);
                Ok(())
            };

            if let Err(causa) = resultado {
                let cuarentena = self.quarantine_transaction(&txn_id, &journal_path)?;
                let aviso = format!(
                    "la transacción {txn_id} no se pudo recuperar ({causa}): su journal y sus copias \
                     se han MOVIDO íntegros a {} (nada se ha borrado) y el workspace vuelve a ser \
                     escribible",
                    cuarentena.display()
                );
                eprintln!("lodestar: aviso: {aviso}");
                fallos.push(aviso);
            }
        }

        if !fallos.is_empty() {
            return Err(WorkspaceError::RecoveryFailed(fallos.join(" · ")));
        }
        Ok(())
    }

    /// Mueve a **cuarentena** (`.lodestar/runtime/journal/quarantine/<txnId>/`) el material de una
    /// transacción cuya recuperación falló (E25-H02) y devuelve la ruta del destino.
    ///
    /// Se **mueve**, no se depura: el journal (con su intención completa), el árbol de copias —
    /// incluida la que no verificó— y el sidecar de huellas quedan ahí byte a byte, como material
    /// forense. El journal va **primero**: es el fichero que sostiene el gate de
    /// [`Workspace::recovery_pending`], así que en cuanto sale, el workspace deja de estar cerrado a
    /// la escritura.
    ///
    /// Nunca sobrescribe una cuarentena previa: si el destino ya existe (misma transacción
    /// cuarentenada dos veces) se usa el primer nombre libre `<txnId>.2`, `<txnId>.3`…
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla la creación del destino o alguno de los movimientos.
    fn quarantine_transaction(
        &self,
        txn_id: &str,
        journal_path: &Path,
    ) -> Result<PathBuf, WorkspaceError> {
        let name = recovery_dir_name(txn_id);
        let base = self.journal_dir().join(QUARANTINE_DIR);
        std::fs::create_dir_all(&base)?;

        let mut destino = base.join(&name);
        let mut n = 2;
        while destino.exists() {
            destino = base.join(format!("{name}.{n}"));
            n += 1;
        }
        std::fs::create_dir_all(&destino)?;

        // (1) El journal primero: es lo que levanta el gate de recuperación. Y se sincroniza el
        //     directorio de ORIGEN, no solo el destino: lo que abre el workspace a la escritura es la
        //     desaparición de `journal/<txnId>.json`, así que esa entrada de directorio tiene que
        //     estar durable. Sin este fsync, una caída inmediatamente después podía resucitar el
        //     journal y repetir la cuarentena en `<txnId>.2` (mismo criterio que `seal_window_abort`).
        if journal_path.exists() {
            let nombre = journal_path
                .file_name()
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from(format!("{name}.json")));
            std::fs::rename(journal_path, destino.join(nombre))?;
            if let Some(origen) = journal_path.parent() {
                io::sync_dir(origen)?;
            }
        }
        // (2) El árbol de copias, íntegro.
        let recovery = self.recovery_root(txn_id);
        if recovery.exists() {
            std::fs::rename(&recovery, destino.join("recovery"))?;
        }
        // (3) Y las huellas con las que se iba a verificar.
        let digests = self.digests_path(txn_id);
        if digests.exists() {
            std::fs::rename(&digests, destino.join(format!("{name}{DIGESTS_SUFFIX}")))?;
        }
        io::sync_dir(&destino)?;

        Ok(destino)
    }

    /// **RESTAURAR** (E13-H06 + E25-H02): devuelve el conocimiento canónico a su estado anterior a la
    /// transacción `txn_id` usando exclusivamente las copias de recuperación de H04
    /// (`.lodestar/runtime/recovery/<txnId>/`), **verificándolas antes de escribir**.
    ///
    /// El lote se lee del **sidecar de huellas** (`recovery/<txnId>.digests.json`), no del journal —que
    /// puede estar torn— ni del recorrido del árbol:
    /// 1. cada `copies[path]` es la copia byte-a-byte de un original que se sobrescribió: se compara su
    ///    tamaño y su revisión blake3 con lo registrado y, solo si **todas** casan, se devuelven a su
    ///    sitio con `io::write_atomic` (restauración todo-o-nada: una copia rota no puede dejar el
    ///    canónico a medio restaurar);
    /// 2. cada `absent[i]` no existía antes: si la transacción lo creó, se borra con `io::delete`
    ///    (idempotente si no llegó a crearse).
    ///
    /// Una copia ausente, ilegible o que no verifica es un fallo de recuperación
    /// ([`WorkspaceError::RecoveryFailed`]) y **no** un `.md` que publicar: quien llama la manda a
    /// cuarentena. Que el conjunto "no existía" venga del sidecar y no del manifiesto `.absent` es lo
    /// que impide el híbrido «originales restaurados **más** ficheros creados» cuando ese manifiesto no
    /// llegó a disco. Los ficheros del árbol que el sidecar no declara se ignoran: el lote respaldado
    /// es el que el sidecar describe, no lo que alguien haya dejado caer dentro del directorio.
    ///
    /// Si no hay sidecar (transacción respaldada por una versión anterior a E25-H02), se restaura como
    /// en v0.3.1, sin verificar. Si además el directorio de recuperación no existe, la caída fue antes
    /// del backup de H04 (que precede a todo rename), así que el canónico está intacto y no hay nada
    /// que restaurar.
    fn restore_from_recovery(&self, txn_id: &str) -> Result<(), WorkspaceError> {
        let recovery_root = self.recovery_root(txn_id);
        let manifiesto = self.read_backup_manifest(txn_id)?;

        let Some(manifiesto) = manifiesto else {
            // Sin sidecar de huellas: transacción respaldada antes de E25-H02 (o caída antes del
            // backup). No hay con qué verificar, así que se restaura como en v0.3.1 — nunca peor que
            // el comportamiento que esta historia endurece.
            if !recovery_root.exists() {
                return Ok(());
            }
            eprintln!(
                "lodestar: aviso: las copias de {} no llevan huellas de verificación (transacción \
                 anterior a v0.4.0): se restauran sin verificar",
                recovery_root.display()
            );
            self.restore_backups_legacy(&recovery_root, &recovery_root)?;
            for rel in read_absent_manifest(&recovery_root) {
                io::delete(&self.root, &rel)?;
            }
            return Ok(());
        };

        // (1) Verificar TODAS las copias antes de escribir una sola: la restauración es
        //     todo-o-nada, así que una copia rota no deja el canónico a medio restaurar.
        let mut a_restaurar: Vec<(RelPath, String)> = Vec::new();
        for (path, esperada) in &manifiesto.copies {
            let rel = RelPath::new(path).map_err(|e| {
                WorkspaceError::RecoveryFailed(format!(
                    "las huellas registran un path inválido ({path}): {e}"
                ))
            })?;
            let copia = recovery_root.join(path);
            let bytes = std::fs::read(&copia).map_err(|e| {
                WorkspaceError::RecoveryFailed(format!(
                    "la copia de recuperación de {path} no se puede leer ({}): {e}",
                    copia.display()
                ))
            })?;
            let real = huella(&rel, &bytes);
            if real.size != esperada.size {
                return Err(WorkspaceError::RecoveryFailed(format!(
                    "la copia de recuperación de {path} no verifica: {} bytes en disco frente a los \
                     {} que se respaldaron ({})",
                    real.size,
                    esperada.size,
                    copia.display()
                )));
            }
            if real.revision != esperada.revision {
                return Err(WorkspaceError::RecoveryFailed(format!(
                    "la copia de recuperación de {path} no verifica: su contenido no casa con la \
                     revisión respaldada ({})",
                    copia.display()
                )));
            }
            let contenido = String::from_utf8(bytes).map_err(|_| {
                WorkspaceError::RecoveryFailed(format!(
                    "la copia de recuperación de {path} no es texto válido ({})",
                    copia.display()
                ))
            })?;
            a_restaurar.push((rel, contenido));
        }

        // (2) Restaurar cada original respaldado por el único escritor (deshace los renames hechos).
        for (rel, contenido) in &a_restaurar {
            io::write_atomic(&self.root, rel, contenido)?;
        }

        // (3) Borrar los paths que el sidecar marca "no existía" y que la transacción pudo crear.
        for path in &manifiesto.absent {
            let rel = RelPath::new(path).map_err(|e| {
                WorkspaceError::RecoveryFailed(format!(
                    "las huellas registran un path inválido ({path}): {e}"
                ))
            })?;
            io::delete(&self.root, &rel)?;
        }
        Ok(())
    }

    /// Restauración **sin verificación** desde el árbol de recuperación, para transacciones cuyas
    /// copias se prepararon antes de E25-H02 (sin sidecar de huellas): recorre el árbol y devuelve
    /// cada copia a su ruta canónica por el único escritor, saltando el manifiesto `.absent`. Es el
    /// comportamiento exacto de v0.3.1, conservado como camino de migración.
    fn restore_backups_legacy(
        &self,
        dir: &Path,
        recovery_root: &Path,
    ) -> Result<(), WorkspaceError> {
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            if path.is_dir() {
                self.restore_backups_legacy(&path, recovery_root)?;
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
                WorkspaceError::RecoveryFailed(format!(
                    "copia de recuperación ilegible {}: {e}",
                    path.display()
                ))
            })?;
            io::write_atomic(&self.root, &rp, &content)?;
        }
        Ok(())
    }

    /// Sella una transacción **deshecha** por la vía RESTAURAR (E13-H06): limpia el staging
    /// (`.lodestar/runtime/staging/<txnId>/`) y las copias de recuperación
    /// (`.lodestar/runtime/recovery/<txnId>/` + su sidecar de huellas), **retira el registro durable
    /// del recibo** (E25-H04: la transacción no publicó, así que su recibo no puede sobrevivirla) y
    /// **borra el fichero de journal** para levantar el gate (tras esto ya no queda ningún journal
    /// no-`done`, de modo que [`Workspace::recovery_pending`] vuelve a `false` y las escrituras se
    /// permiten).
    ///
    /// El `txnId` (sin prefijo `changeset:`) nombra por igual el staging, la recuperación, el registro
    /// del recibo y el journal (convención de E13-H06), así que un mismo nombre saneado los localiza
    /// todos.
    fn finish_recovery(&self, txn_id: &str, journal_path: &Path) -> Result<(), WorkspaceError> {
        let name = recovery_dir_name(txn_id);
        let runtime = self.root.join(".lodestar").join("runtime");

        let staging = runtime.join("staging").join(&name);
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        self.discard_pending_receipt(txn_id)?;
        self.discard_recovery_copies(txn_id)?;
        if journal_path.exists() {
            std::fs::remove_file(journal_path)?;
        }
        Ok(())
    }

    /// Sella una transacción **completada** por la vía COMPLETAR (E13-H06, endurecida por E25-H04): el
    /// journal declaraba `applied`, así que el canónico ya es el resultado final y lo único que faltaba
    /// era el cierre que el proceso muerto no llegó a hacer.
    ///
    /// Se diferencia de [`Workspace::finish_recovery`] en las dos cosas que distinguen «publicó» de «no
    /// publicó»:
    /// 1. **Da por bueno el recibo**: promueve el registro durable escrito con el journal a recibo
    ///    definitivo (`receipts/<txnId>.json`). Es lo que convierte una publicación huérfana en una
    ///    transacción reversible, en vez de un cambio del canónico que nadie puede deshacer.
    /// 2. **Conserva las copias de recuperación**: exactamente como el sellado del camino feliz
    ///    (`crate::transaction`, paso 11), porque `change_revert` las necesita. Borrarlas —lo que hacía
    ///    hasta E25-H04— dejaba el recibo apuntando a un plano de reversión que ya no existía: había
    ///    recibo y no había *undo*, que es la mitad inútil de la promesa.
    ///
    /// # Por qué NO devuelve `Result`
    ///
    /// Una transacción que se completa **no puede fallar al cerrarse**, y aquí eso es una exigencia de
    /// tipos y no un comentario. El camino de esta función es el mismo que el paso (11) del apply
    /// (`crate::transaction`) visto desde el otro lado de un crash, así que hereda su regla: pasado el
    /// punto de no retorno, nada convierte lo publicado en un fallo. Con `Result`, un `?` en la limpieza
    /// caía en el `quarantine_transaction` del bucle de [`Workspace::recover`] — que **mueve el árbol de
    /// copias** a `journal/quarantine/`— justo después de haber promovido el recibo: el recibo anunciaba
    /// una reversión cuyo material acababa de irse a material forense. Y una transacción COMPLETADA con
    /// éxito no es material forense por definición: la cuarentena existe para las que **no** se pudieron
    /// recuperar.
    ///
    /// Los tres pasos son por tanto best-effort con aviso por stderr:
    /// - **la promoción**: si falla, el conocimiento sigue en el borde correcto pero la transacción no
    ///   será reversible; se avisa y se sigue.
    /// - **el staging**: sobra desde el instante en que la transacción publicó; el GC lo recoge.
    /// - **el fichero de journal**: si no se puede borrar, el gate de [`Workspace::recovery_pending`]
    ///   sigue en pie y el siguiente `recover` vuelve a pasar por aquí. Es un reintento seguro porque la
    ///   promoción es **idempotente**: con el pendiente ya retirado no hace nada y el recibo definitivo
    ///   ya está escrito.
    ///
    /// # NO es `seal_published_transaction`, aunque se le parezca (registro de E28-H03)
    ///
    /// Comparte forma superficial con la coreografía compartida que E28-H01 extrajo para el apply y la
    /// reversión —promover recibo → limpiar staging → borrar journal— y aun así es deliberadamente
    /// **otra cosa**, verificada como tal por dos jueces ciegos de la ronda de E28. Las diferencias son
    /// exactamente las que separan «sellar lo que acabo de publicar» de «cerrar lo que otro proceso
    /// dejó publicado»:
    /// - **borra el journal incondicionalmente**, no solo si el recibo quedó a salvo. Aquí el journal
    ///   declaraba `applied`, así que su único trabajo pendiente era este cierre; en el camino feliz,
    ///   conservarlo es lo que hace que la recuperación reintente una promoción fallida.
    /// - **la promoción es idempotente por diseño** (E25-H04), porque esta vía puede ejecutarse más de
    ///   una vez sobre la misma transacción.
    /// - **corre fuera de la ventana de publicación normal**, desde el bucle de
    ///   [`Workspace::recover`] y no desde un camino que acaba de renombrar el canónico.
    ///
    /// Fusionarlas obligaría a condicionar el borrado del journal y perdería esas tres propiedades:
    /// si alguna vez parece código duplicado, esto es lo que hay que releer antes de unificarlo.
    fn finish_recovery_completada(&self, txn_id: &str, journal_path: &Path) {
        let name = recovery_dir_name(txn_id);
        let runtime = self.root.join(".lodestar").join("runtime");

        if let Err(e) = self.promote_pending_receipt(txn_id) {
            eprintln!(
                "lodestar: aviso: la transacción {txn_id} se completó pero su recibo no se pudo \
                 persistir ({e}): el conocimiento está en el estado correcto, pero la transacción no \
                 será reversible"
            );
        }

        let staging = runtime.join("staging").join(&name);
        if staging.exists() {
            if let Err(e) = std::fs::remove_dir_all(&staging) {
                eprintln!(
                    "lodestar: aviso: no se pudo limpiar el staging {} de la transacción completada \
                     {txn_id} ({e}): el GC del plano de control lo recogerá",
                    staging.display()
                );
            }
        }
        if journal_path.exists() {
            if let Err(e) = std::fs::remove_file(journal_path) {
                eprintln!(
                    "lodestar: aviso: no se pudo borrar el journal {} de la transacción completada \
                     {txn_id} ({e}): el conocimiento ya está en su borde final y su recibo persistido; \
                     la siguiente recuperación reintentará el sellado",
                    journal_path.display()
                );
            }
        }
    }

    /// Borra el árbol de copias de recuperación de una transacción **y su sidecar de huellas** (los
    /// dos van siempre juntos: el sidecar sin árbol no describe nada y el árbol sin sidecar no es
    /// verificable).
    fn discard_recovery_copies(&self, txn_id: &str) -> Result<(), WorkspaceError> {
        let recovery = self.recovery_root(txn_id);
        if recovery.exists() {
            std::fs::remove_dir_all(&recovery)?;
        }
        let digests = self.digests_path(txn_id);
        if digests.exists() {
            std::fs::remove_file(&digests)?;
        }
        Ok(())
    }

    /// **Sella el aborto de la ventana de publicación** (E25-H02, enmienda del defecto 5): cierra la
    /// transacción que acaba de abortar con `WRITE_CONFLICT` por divergencia del canónico en la
    /// ventana `[T1, T3)` (E25-H01), **bajo el mismo lock** y antes de devolver el error.
    ///
    /// Sellar es exacto, no una amnistía: quien llama lo sabe por control de flujo —el aborto ocurre
    /// **antes** del bucle de renames, así que el canónico no se movió ni un byte— y por tanto no hay
    /// nada que restaurar. Si el journal y sus copias sobrevivieran, la siguiente operación los
    /// clasificaría como «renames parciales» y escribiría las copias de T1 **encima** de la edición
    /// externa que este aborto existe para no pisar (y borraría, por el manifiesto `.absent`, el `.md`
    /// que el usuario creó dentro de la ventana).
    ///
    /// Orden **deliberado**: el fichero de journal primero —es lo que sostiene el gate de
    /// [`Workspace::recovery_pending`]— y las copias después. Si el proceso muere entre los dos,
    /// queda un árbol de recuperación **sin** journal: un huérfano legítimo que recoge el GC
    /// (E24-H06). Al revés quedaría un journal apuntando a copias que ya no están, y la recuperación
    /// sellaría un estado parcial en silencio.
    ///
    /// El **registro durable del recibo** (E25-H04) va antes que los dos, y por el mismo tipo de razón:
    /// es efectivo mientras su journal declare `applied` y aquí el journal declara `prepared`, así que
    /// ya no lo es —pero retirarlo primero cierra el orden por completo. Un recibo de esta transacción
    /// que sobreviviera al aborto haría que `change_revert` escribiera las copias de T1 **encima** de la
    /// edición externa que este aborto existe para no pisar.
    pub(crate) fn seal_window_abort(
        &self,
        txn_id: &str,
        journal_path: &Path,
    ) -> Result<(), WorkspaceError> {
        self.discard_pending_receipt(txn_id)?;
        if journal_path.exists() {
            std::fs::remove_file(journal_path)?;
        }
        if let Some(padre) = journal_path.parent() {
            io::sync_dir(padre)?;
        }

        failpoint!(FailPoint::EnMedioDelSelladoDelAborto);

        self.discard_recovery_copies(txn_id)
    }

    /// La señal que delata a `txn_id` como **tomado** por una transacción con material vigente
    /// (E28-H01), o `None` si está libre. Es el juicio que impide que publicar bajo un id destruya en
    /// silencio el plano de recuperación de otra transacción.
    ///
    /// El criterio de «vigente» es el mismo con el que el GC del plano de control decide qué está vivo
    /// (`journal/ ∪ receipts/`, ver la documentación de `crate::receipts`), y por la misma razón que
    /// allí es ese y no otro (invariante #3: una sola verdad, no un segundo juicio sobre lo mismo):
    ///
    /// - **journal presente** → la transacción está a medio publicar o pendiente de recuperar, y su
    ///   material es justamente lo que la recuperación necesita;
    /// - **recibo persistido** → la transacción es revertible, y `change_revert` restaura desde
    ///   `recovery/<txnId>/`.
    ///
    /// Un `recovery/<txnId>/` **sin** ninguna de las dos señales NO cuenta como vigente: es un huérfano
    /// que el GC recoge (E24-H06), y `backup_originals` ya lo reescribe de forma idempotente. Contarlo
    /// convertiría cualquier resto de una transacción abortada en un bloqueo permanente de un id que
    /// nadie reclama.
    ///
    /// Desde E28-H03 su **único** consumidor es `Workspace::resolve_free_txn_id`, que ya no rechaza
    /// sino que elige otro id: el guard de solo-rechazo que E28-H01 puso en el camino del `revert`
    /// resultó dejar sin salida una secuencia legítima, así que la decisión entera vive en un punto
    /// que los dos caminos de publicación comparten. El texto de la señal sobrevive como **motivo**
    /// del `WriteConflict` que se emite cuando ni siquiera queda una variante libre que probar.
    ///
    /// # El motivo lo lee un AGENTE (E28-H03, reserva)
    /// Por eso el texto nombra el `txnId` y lo que pasaría, pero **nunca** una ruta del plano de
    /// control (`.lodestar/runtime/receipts/…`, `recovery/…`) ni una ruta absoluta de esta máquina:
    /// acaba tal cual en el `message` del `WRITE_CONFLICT` que cruza la frontera MCP, y ahí una ruta
    /// interna no es accionable —el agente no puede tocarla— además de filtrar la disposición del
    /// disco del usuario. El diagnóstico de dónde vive el material es cosa de la recuperación, no
    /// del mensaje de error.
    fn senal_de_txn_id_tomado(&self, txn_id: &str) -> Option<String> {
        if self.journal_state_of(txn_id).is_some() {
            return Some(format!(
                "la transacción {txn_id} ya tiene un write-ahead journal en curso: publicar bajo ese \
                 identificador sobrescribiría su plano de recuperación y el estado que guarda se \
                 perdería"
            ));
        }
        let recibo = self
            .receipts_dir()
            .join(format!("{}.json", recovery_dir_name(txn_id)));
        if recibo.exists() {
            return Some(format!(
                "la transacción {txn_id} ya tiene un recibo persistido: publicar bajo ese \
                 identificador reescribiría ese recibo y sobrescribiría las copias de recuperación \
                 con las que se deshace"
            ));
        }
        None
    }

    /// **El punto de decisión de identidad de toda publicación** (E28-H03): dado el `txnId`
    /// `candidato`, devuelve el primero de su cadena determinista de variantes que **no** identifica
    /// ya a una transacción con material vigente.
    ///
    /// Lo consumen los **dos** caminos que escriben —[`Workspace::apply_transaction_con_recibo`] y
    /// [`Workspace::revert_transaction_con_recibo`]—, siempre bajo el lock de publicación y **después**
    /// de la recuperación pendiente, que es lo que hace fiable la lectura del disco: mientras dura el
    /// lock nadie más puede tomar un id.
    ///
    /// # Por qué existe
    ///
    /// El `changeSetId` es determinista (`blake3(baseRevision, normalizedOperations)`), así que
    /// replanificar el mismo cambio sobre la misma base produce el mismo `txnId`. Hasta E28-H03 eso
    /// dejaba a los dos caminos en el peor sitio posible: el apply **sobrescribía** en silencio
    /// `recovery/<txnId>/` y `receipts/<txnId>.json` de la transacción anterior —destruyendo las
    /// únicas copias con las que aquélla se deshacía— y el revert, protegido por
    /// [`Workspace::assert_txn_id_libre`] desde E28-H01, **fallaba sin salida**: el id derivado ya
    /// tenía recibo y no había ningún otro que probar, de modo que el re-apply quedaba
    /// permanentemente no revertible. Resolver la identidad en un solo sitio cierra las dos mitades a
    /// la vez: nunca se pisa material vigente y nunca se agota la salida.
    ///
    /// # Cómo elige, y por qué así
    ///
    /// Recorre `candidato`, [`crate::transaction::siguiente_variante_de_txn_id`]`(candidato)`, … hasta que
    /// [`Workspace::senal_de_txn_id_tomado`] no delate a ninguna señal viva. La cadena comparte
    /// familia de sufijo con [`crate::revert_transaction_id`] a propósito (`X` → `X-2` → `X-3`;
    /// `X-revert` → `X-revert-2` → …), así que resolver una colisión recorre exactamente los mismos
    /// escalones que habría recorrido una cadena de reversiones y ninguno puede tapar al otro.
    ///
    /// Es **determinista**: no lleva reloj, ni aleatoriedad, ni contador global. Solo depende del
    /// candidato y del material que hay en disco, de modo que un reintento tras un crash converge al
    /// mismo id —la vía RESTAURAR ya retiró journal, pendiente y copias del intento muerto, así que el
    /// candidato vuelve a estar libre— en vez de sembrar un huérfano nuevo en cada intento.
    ///
    /// # Errores
    /// - [`WorkspaceError::WriteConflict`] si la cadena **no puede avanzar**: la variante siguiente
    ///   coincide con la actual, que es el punto fijo declarado de `u64::MAX`
    ///   (ver [`crate::revert_transaction_id`]). Es ruidoso y anterior a la primera escritura, que es
    ///   justo lo contrario de sobrescribir en silencio.
    pub(crate) fn resolve_free_txn_id(&self, candidato: &str) -> Result<String, WorkspaceError> {
        let mut id = candidato.to_string();
        loop {
            let Some(motivo) = self.senal_de_txn_id_tomado(&id) else {
                return Ok(id);
            };
            let siguiente = crate::transaction::siguiente_variante_de_txn_id(&id);
            if siguiente == id {
                return Err(WorkspaceError::WriteConflict(format!(
                    "{motivo}. No queda ninguna variante libre del identificador que probar (la \
                     cadena de variantes agotó su contador): replanifica el cambio sobre el estado \
                     actual"
                )));
            }
            id = siguiente;
        }
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
    /// `(rutaCanónica, contenido)`. Auxiliar de [`Workspace::revert_transaction_con_recibo`] (no
    /// toca disco: solo lee las copias).
    fn collect_backups(
        &self,
        recovery_root: &Path,
    ) -> Result<Vec<(RelPath, String)>, WorkspaceError> {
        let mut out = std::collections::BTreeMap::new();
        collect_backups_into(recovery_root, recovery_root, &mut out)?;
        Ok(out.into_iter().collect())
    }

    /// Revierte la transacción `orig_txn_id` como una **nueva transacción inversa recuperable**
    /// (E13-H09, `ARCHITECTURE.md §19.5/§19.6`), y además **registra durablemente el recibo de la
    /// inversa antes de su punto de no retorno** (E25-H05, defecto (c)).
    ///
    /// Es la **única** vía para deshacer una transacción. Hasta E31-H01 (`decisiones §25`) convivía
    /// con una `Workspace::revert_transaction` que delegaba aquí con `recibo: None` y aplanaba el
    /// resultado a tupla; se **retiró** porque no la llamaba nadie —ni fuera del crate ni dentro— y
    /// porque lo que ofrecía era una reversión **sin registro durable**, que puede quedarse sin
    /// vuelta atrás: sin recibo, el criterio de «vivo» del GC del plano de control (`journal/` ∪
    /// `receipts/`) no ve su árbol de recuperación y lo purga. La pasada de `/mutantes` de `§16(l)`
    /// la había encontrado sustituyendo su cuerpo por `unreachable!()` sin que ninguno de los 52
    /// binarios de test del workspace se pusiera rojo; es el mismo desenlace que `§16(b)`.
    ///
    /// Toda escritura del canónico va por el **único escritor** (invariante #5): las copias
    /// respaldadas se restauran con `io::write_atomic` y los paths que se habían creado (marcados
    /// `.absent`) se borran con `io::delete`. La reversión es ella misma **recuperable**: bajo el
    /// lock de publicación (E13-H02) respalda el estado ACTUAL de los afectados en su propio árbol
    /// de recuperación (`new_txn_id`) y registra su intención en un write-ahead journal propio
    /// (E13-H03) **antes** del primer rename, de modo que una caída a mitad converge determinista al
    /// reabrir (E13-H06).
    ///
    /// `observed` es la [`WorkspaceRevision`] que la fachada vio al decidir que la transacción era
    /// reversible, y se **re-verifica bajo el lock** antes de la primera escritura (E25-H05): la
    /// fachada mira sin el lock, y en esa ventana otro escritor puede tocar un `.md` afectado. Si ya
    /// no casa → [`WorkspaceError::WriteConflict`] y no se escribe nada, en vez de sobrescribir esa
    /// edición con la copia respaldada. Es la simetría que le faltaba al camino que deshace: el apply
    /// re-verifica su base bajo el lock desde E13-H02.
    ///
    /// **`new_txn_id` es un candidato, no un destino** (E28-H01, corregido por E28-H03): si ya
    /// identifica a una transacción con material vigente —journal presente o recibo persistido, el
    /// mismo criterio de «vivo» del GC del plano de control— la reversión **no** lo sobrescribe; se
    /// publica bajo la primera variante libre que devuelve `Workspace::resolve_free_txn_id`. Ese
    /// material guarda el estado con el que se deshace el propio *undo*, y pisarlo lo destruye para
    /// siempre; pero rechazar sin alternativa —lo que hacía E28-H01— dejaba sin salida una secuencia
    /// legítima: tras `apply → revert → re-apply`, el `X-revert` que deriva la fachada ya lo ocupa la
    /// primera reversión y el re-apply quedaba permanentemente no revertible.
    ///
    /// La única igualdad que **sí** se rechaza es `new_txn_id == orig_txn_id`: pedir que la inversa
    /// publique bajo la identidad de la transacción que deshace no es una colisión de nombres que se
    /// pueda resolver moviendo el id, es una contradicción del llamante —restauraría desde el mismo
    /// árbol que estaría reescribiendo— y así se reporta, ruidosamente y sin escribir nada.
    ///
    /// Es la variante que usa la fachada (`App::change_revert`), y el espejo exacto de
    /// [`Workspace::apply_transaction_con_recibo`]: `recibo` presta las dos piezas del
    /// [`ChangeReceipt`] que esta mecánica no puede conocer —el `changeSetId` de la transacción que se
    /// deshace y su `semanticDiff`, que nacieron en `change_plan` y no en el disco—. Con `Some(..)` la
    /// reversión compone su recibo completo (`previousRevision` = la revisión del paso (6),
    /// `resultRevision` = la que estampa su journal, las dos conocidas **antes** del primer rename) y
    /// lo persiste con el journal (`crate::receipts`, `write_pending_receipt`), promoviéndolo a recibo
    /// definitivo al sellar. Con `None` no hay recibo: es el contrato de quien solo ejercita la
    /// mecánica de disco.
    ///
    /// Por qué eso importa: hasta E25-H05 el recibo de la inversa lo escribía la fachada **después**
    /// de que el canónico ya hubiera vuelto atrás. Un `SIGKILL` o un `ENOSPC` en ese hueco devolvía
    /// `Err` sobre algo ya publicado y dejaba la reversión **sin recibo**; y como el recibo es el
    /// criterio de «vivo» del GC (`journal/` ∪ `receipts/`), el árbol `recovery/<txnId>-revert/` quedaba
    /// huérfano y se purgaba: deshacer el *undo* se volvía imposible para siempre. Persistido con el
    /// journal, la vía COMPLETAR de [`Workspace::recover`] lo promueve sin código nuevo — la
    /// convención que ata journal, copias, registro pendiente y recibo bajo un mismo `txnId` los
    /// localiza a los cuatro. Lo que E28-H01 cambió no es esa convención sino **cómo se deriva** ese
    /// id: ver [`crate::revert_transaction_id`]; y lo que E28-H03 cambió es que ese id derivado es un
    /// **candidato**, resuelto contra el material vigente por `Workspace::resolve_free_txn_id`.
    ///
    /// Devuelve una [`crate::PublishedTransaction`] con el `txnId` **efectivo** por delante: quien
    /// compone el recibo de la inversa (la fachada) ya no puede recalcularlo, porque la identidad se
    /// decide aquí dentro, bajo el lock.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si faltan las copias de recuperación de `orig_txn_id` (no se puede
    ///   revertir: transacción no disponible), ante un fallo de IO de la restauración, o si el
    ///   registro del recibo no se puede escribir — este último **antes** del primer rename, así que
    ///   no publica nada.
    /// - [`WorkspaceError::WriteConflict`] si el lock ya está tomado (otro publicador), si el
    ///   canónico cambió entre la comprobación de la fachada y la toma del lock (E25-H05) o si
    ///   `new_txn_id` coincide con `orig_txn_id` (E28-H01/E28-H03).
    /// - [`WorkspaceError::PermissionDenied`] si algún path afectado ya no es escribible.
    ///
    /// A partir de la publicación de la inversa el cierre (promoción del recibo, borrado del
    /// journal) es best-effort con aviso por stderr: un `Err` ahí convertiría una reversión consumada
    /// en un fallo aparente.
    pub fn revert_transaction_con_recibo(
        &self,
        orig_txn_id: &str,
        new_txn_id: &str,
        observed: &WorkspaceRevision,
        recibo: Option<(&ChangeSetId, &SemanticDiff)>,
    ) -> Result<crate::PublishedTransaction, WorkspaceError> {
        // (1) Lock exclusivo de publicación (RAII: liberado al final por `Drop`, incluso en panic).
        let _lock = self.acquire_lock()?;

        // (2) Recuperación pendiente primero: nunca se revierte sobre un estado a medio recuperar.
        if self.recovery_pending() {
            self.recover()?;
        }

        // Seam de test (E25-H05), dentro de la ventana `[comprobación de la fachada, primera
        // escritura)`: el gancho hace de «otro escritor» y la reversión CONTINÚA —lo que `failpoint!`,
        // que solo aborta, no sabe hacer—. Va **antes** de la re-verificación del paso (2b), que es
        // justamente lo que tiene que cazar esa edición. Sin `--features test-failpoints` no genera ni
        // una instrucción.
        #[cfg(feature = "test-failpoints")]
        crate::failpoints::ejecutar_gancho(crate::failpoints::PuntoDeGancho::AntesDeRestaurar);

        // (2b) Control optimista BAJO EL LOCK (E25-H05, defecto (b)): la revisión que la fachada
        //      observó al decidir que esta transacción era reversible sigue siendo la actual. La
        //      fachada mira **sin** el lock —lo toma este método—, así que en esa ventana otro escritor
        //      puede tocar un `.md` afectado; sin esta comprobación la reversión le escribía la copia
        //      respaldada encima, en silencio y sin respaldo posible de lo pisado.
        //
        //      Es el mismo `reverify_base_revision` que usa el apply (invariante #3: una sola verdad,
        //      no una segunda comprobación escrita a mano), y va **antes** de la primera escritura: al
        //      abortar aquí no hay journal, ni copias de la inversa, ni registro de recibo que sellar
        //      —a diferencia del aborto de ventana del apply (E25-H02), que sí tiene que sellarse
        //      porque ocurre con el journal ya en disco—. El canónico no se ha movido ni un byte.
        self.reverify_base_revision(observed)?;

        // (2c) IDENTIDAD EFECTIVA DE LA INVERSA (E28-H01, corregido por E28-H03), antes de la primera
        //      escritura y después de la recuperación del paso (2).
        //
        //      Publicar bajo un id ya usado no es una colisión de nombres cualquiera:
        //      `backup_originals` empieza por `remove_dir_all` del árbol previo y
        //      `write_pending_receipt`/`promote_pending_receipt` reescriben su recibo, así que el
        //      estado que ese material guardaba —el *redo* de la cadena de reversiones— desaparece
        //      para siempre (defecto M-01 del testbench). E28-H01 lo cerró rechazando; E28-H03
        //      descubrió que rechazar **sin alternativa** deja sin salida una secuencia legítima
        //      (`apply → revert → re-apply → revert`, donde el `X-revert` derivado ya lo ocupa la
        //      primera reversión), así que la decisión pasa por el punto único de identidad: se
        //      publica bajo la primera variante LIBRE, con el mismo criterio de «vivo» del GC del
        //      plano de control (`journal/ ∪ receipts/`).
        //
        //      Lo único que sigue siendo un `Err` es `new == orig`: eso no es una colisión que se
        //      resuelva moviendo el id, sino un llamante que pide restaurar desde el mismo árbol que
        //      estaría reescribiendo. Se rechaza aquí, ruidosamente y sin haber tocado nada.
        //
        //      Determinismo post-crash: un reintento vuelve a derivar el mismo candidato, y para
        //      entonces la vía RESTAURAR ya limpió journal, pendiente y copias de aquel intento, así
        //      que el candidato vuelve a estar libre y la resolución converge al mismo id.
        if new_txn_id == orig_txn_id {
            return Err(WorkspaceError::WriteConflict(format!(
                "la reversión de la transacción {orig_txn_id} no puede publicarse bajo esa misma \
                 identidad: restauraría desde el árbol de recuperación que estaría reescribiendo, y \
                 el estado que guarda se perdería"
            )));
        }
        let new_txn_id = &self.resolve_free_txn_id(new_txn_id)?;

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

        // (6) Revisión actual (== `resultRevision` del apply, re-verificada en (2b) bajo el lock) y
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
        let writable = &self.config().workspace.writable_roots;
        let result_rev = workspace_revision(&result_files, writable);

        // (7) Copias de recuperación de la INVERSA (respalda el estado actual) → la reversión es
        //     recuperable (E13-H04): si cae a mitad, `recover` restaura desde `recovery/<new>/`.
        self.backup_originals(new_txn_id, &affected)?;

        // (8) Write-ahead journal `prepared` de la inversa, fsynced antes del primer rename (H03).
        let mut journal = self.create_journal(new_txn_id, &affected, &previous, &result_rev)?;

        // (8b) Registro durable del RECIBO de la inversa, con su journal y antes del primer rename
        //      (E25-H05, misma mecánica y mismas garantías que E25-H04 dio al apply). Va DESPUÉS del
        //      journal a propósito: el registro es efectivo solo mientras su journal declare `applied`
        //      (`pending_receipt_efectivo` → `journal_state_of`), así que su vida queda contenida en la
        //      del journal y no hace falta una tercera señal que caducar. A partir del primer rename el
        //      canónico ya volvió atrás, así que este es el último instante en el que escribirlo aún
        //      sirve de algo.
        if let Some((change_set_id, diff)) = recibo {
            self.write_pending_receipt(&ChangeReceipt {
                id: ReceiptId(new_txn_id.to_string()),
                change_set_id: change_set_id.clone(),
                previous_revision: previous.clone(),
                result_revision: result_rev.clone(),
                changed_paths: affected.clone(),
                semantic_diff: diff.clone(),
            })?;
        }

        failpoint!(FailPoint::TrasJournalPrepared);

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

        // (10) Sella la inversa: promueve su recibo y borra el fichero de journal (levanta el gate de
        //      recuperación). Conserva las copias de recuperación de la inversa (el receipt de la
        //      reversión las referencia; el GC de E13-H07 las purgará con su recibo).
        //
        // EL BLOQUE ENTERO ES BEST-EFFORT (E25-H05, regla heredada de E25-H04). Está al otro lado del
        // punto de no retorno: el canónico ya volvió atrás, así que un `?` aquí devolvería un error por
        // algo que SÍ se deshizo, y el agente actuaría sobre la premisa falsa de que su reversión no
        // ocurrió. Se avisa por stderr y se sigue.
        let journal_path = journal.path().to_path_buf();

        // (10a/b) La coreografía de sellado, en el ÚNICO sitio en el que vive (E28-H01): recibo →
        //         staging (la inversa no materializa ninguno: `None`) → journal, con el orden y las
        //         garantías best-effort documentados en `Workspace::seal_published_transaction`. El
        //         apply llama a la misma función, así que las dos no pueden volver a divergir.
        self.seal_published_transaction(new_txn_id, &journal_path, None, "reversión");

        // (11) Revisión resultante (== `previousRevision` del apply original) + paths restaurados, con
        //      el `txnId` EFECTIVO por delante (E28-H03): es el `receiptId` de la inversa y la fachada
        //      ya no puede derivarlo.
        let result = self.workspace_revision()?;
        Ok(crate::PublishedTransaction {
            txn_id: new_txn_id.clone(),
            previous,
            result,
            changed_paths: affected,
        })
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
