//! Write-ahead journal transaccional (E13-H03, `ARCHITECTURE.md §19.5`, `REFACTOR §5.2`): registra
//! la **intención completa** de la publicación —qué operaciones va a sustituir y entre qué
//! [`WorkspaceRevision`] base y resultado— en `.lodestar/runtime/journal/<txnId>.json`, **fsynced a
//! disco antes de la primera sustitución del canónico**, y va marcando cada rename a medida que se
//! completa. Es el registro que E13-H06 releerá para recuperar una publicación interrumpida a
//! mitad: por eso el JSON es la fuente de verdad y los nombres de campo/estado son parte del
//! contrato de recuperación.
//!
//! Runtime, no canónico: el journal vive bajo `.lodestar/runtime/`, que el walker de conocimiento
//! (`discovery::discover`) y el watcher ya excluyen (E9-H06) y `WorkspaceRevision` ignora (E10-H03), así
//! que no viola el invariante #1 («los `.md` en disco son la única fuente de verdad»).
//!
//! Durabilidad write-ahead: el journal se persiste con `write` + [`std::fs::File::sync_all`] (fsync
//! del fichero) tanto al crearlo como tras cada `mark_applied`. El fsync garantiza que el registro
//! ya está en disco antes de que se toque el canónico; sin él, una caída de energía podría dejar el
//! canónico modificado sin rastro de la transacción que lo modificó, y la recuperación no tendría
//! qué releer.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use lodestar_core::types::{RelPath, WorkspaceRevision};

use crate::error::WorkspaceError;
use crate::Workspace;

/// Estado global del write-ahead journal a lo largo de la publicación.
///
/// Progresión monótona `prepared → applying → applied → done`: `prepared` en cuanto se registra la
/// intención (antes de tocar el canónico), `applying` con la primera sustitución completada,
/// `applied` cuando todas lo están (E13-H05) y `done` tras el sellado final (E13-H07). Se serializa
/// en minúsculas (`prepared`, `applying`, …) porque es la etiqueta que la recuperación (E13-H06)
/// lee del JSON.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JournalState {
    /// Intención registrada y fsynced; el canónico aún intacto.
    Prepared,
    /// Al menos una sustitución completada; la publicación está en curso.
    Applying,
    /// Todas las operaciones aplicadas (E13-H05).
    Applied,
    /// Transacción sellada y cerrada (E13-H07).
    Done,
}

/// Estado de una operación individual del journal.
///
/// `pending` mientras el rename atómico no se ha completado, `applied` una vez el canónico refleja
/// la sustitución. Se serializa en minúsculas por el mismo contrato de recuperación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OpState {
    /// El rename de esta operación aún no se ha completado.
    Pending,
    /// El rename atómico se completó: el canónico ya refleja la sustitución.
    Applied,
}

/// Una operación registrada en el journal: la ruta relativa del `.md` que la transacción sustituye
/// y el estado de su rename.
#[derive(Debug, Clone, Serialize, Deserialize)]
struct JournalOp {
    /// Ruta relativa (POSIX) del `.md` canónico que esta operación sustituye.
    path: String,
    /// Estado del rename de esta operación.
    state: OpState,
}

/// Cuerpo serializable del journal — el JSON que se materializa en disco y que E13-H06 releerá para
/// recuperar. Las claves van en `camelCase` (`txnId`, `baseWorkspaceRevision`, …) como fija el
/// contrato de recuperación.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct JournalData {
    /// Identificador de la transacción (da nombre al fichero `<txnId>.json`).
    txn_id: String,
    /// Estado global de la transacción.
    state: JournalState,
    /// [`WorkspaceRevision`] esperada del conocimiento escribible **antes** de publicar.
    base_workspace_revision: String,
    /// [`WorkspaceRevision`] que la publicación debe dejar al terminar.
    result_workspace_revision: String,
    /// Las N operaciones que la transacción va a sustituir, en orden.
    operations: Vec<JournalOp>,
}

/// Handle vivo del write-ahead journal de una transacción (E13-H03).
///
/// Se obtiene con [`Workspace::create_journal`] (que ya lo deja `prepared` y fsynced en disco) y
/// expone [`Journal::mark_applied`] para registrar cada rename a medida que se completa. Cada
/// mutación re-persiste el JSON con fsync, de modo que el fichero en disco es siempre el reflejo
/// durable del progreso — la fuente de verdad que la recuperación releerá.
pub struct Journal {
    /// Ruta del fichero `<txnId>.json` bajo `.lodestar/runtime/journal/`.
    path: PathBuf,
    /// Estado en memoria, espejo de lo último persistido a disco.
    data: JournalData,
}

impl Journal {
    /// Ruta del fichero de journal materializado (bajo `.lodestar/runtime/journal/`).
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Estado global actual del journal (espejo de lo persistido en disco).
    pub fn state(&self) -> JournalState {
        self.data.state
    }

    /// Marca la operación de `path` como aplicada (rename completado) y **re-persiste** el journal a
    /// disco con fsync (E13-H03). La primera marca transiciona el estado global de `prepared` a
    /// `applying`; las siguientes lo dejan en `applying` (el salto a `applied` es de E13-H05).
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si `path` no figura entre las operaciones registradas (registrar un
    ///   rename que el journal no previó es una incoherencia del plan), o si falla la re-escritura.
    pub fn mark_applied(&mut self, path: &RelPath) -> Result<(), WorkspaceError> {
        let target = path.as_str();
        let op = self
            .data
            .operations
            .iter_mut()
            .find(|op| op.path == target)
            .ok_or_else(|| {
                WorkspaceError::Io(format!(
                    "el journal no registra la operación {target}: no puede marcarse aplicada"
                ))
            })?;
        op.state = OpState::Applied;

        // La primera sustitución completada saca la transacción de `prepared`: ya se tocó el
        // canónico, así que a partir de aquí una caída deja trabajo a medias que recuperar.
        if self.data.state == JournalState::Prepared {
            self.data.state = JournalState::Applying;
        }

        write_journal(&self.path, &self.data)
    }

    /// Transiciona el journal a estado global `applied` (E13-H05): todas las operaciones de la
    /// transacción ya se sustituyeron en el canónico. Marca también cada operación como `applied`
    /// (deja el registro internamente coherente: sin `pending` bajo un estado `applied`) y
    /// **re-persiste** el journal a disco con fsync.
    ///
    /// Se llama una sola vez, al final de [`Workspace::publish`], después de que el último rename
    /// se haya completado. `applied` es lo que E13-H06 leerá para decidir **completar** una
    /// publicación interrumpida (todo renombrado, solo falta sellar), frente a `applying`/`prepared`
    /// (renames parciales que hay que **restaurar**).
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla la re-escritura fsynced del journal.
    pub fn mark_all_applied(&mut self) -> Result<(), WorkspaceError> {
        for op in &mut self.data.operations {
            op.state = OpState::Applied;
        }
        self.data.state = JournalState::Applied;
        write_journal(&self.path, &self.data)
    }
}

impl Workspace {
    /// Prepara el write-ahead journal de una transacción y lo persiste **fsynced antes de la primera
    /// sustitución del canónico** (E13-H03). Crea `.lodestar/runtime/journal/` si falta, construye
    /// el registro en estado `prepared` con una operación `pending` por cada `RelPath` de `ops` (en
    /// orden), la `base_rev` y la `result_rev` esperadas, y lo escribe con fsync a
    /// `.lodestar/runtime/journal/<txn_id>.json`. Devuelve el [`Journal`] vivo para marcar los
    /// renames a medida que se completen.
    ///
    /// El fsync es lo que hace el journal *write-ahead*: garantiza que la intención completa está
    /// durable en disco antes de tocar el conocimiento canónico, de modo que una publicación
    /// interrumpida siempre deja rastro recuperable (E13-H06).
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla la creación del directorio runtime o la escritura fsynced
    ///   del journal.
    pub fn create_journal(
        &self,
        txn_id: &str,
        ops: &[RelPath],
        base_rev: &WorkspaceRevision,
        result_rev: &WorkspaceRevision,
    ) -> Result<Journal, WorkspaceError> {
        let dir = self.root.join(".lodestar").join("runtime").join("journal");
        std::fs::create_dir_all(&dir)?;

        let data = JournalData {
            txn_id: txn_id.to_string(),
            state: JournalState::Prepared,
            base_workspace_revision: base_rev.0.clone(),
            result_workspace_revision: result_rev.0.clone(),
            operations: ops
                .iter()
                .map(|p| JournalOp {
                    path: p.as_str().to_string(),
                    state: OpState::Pending,
                })
                .collect(),
        };

        let path = dir.join(format!("{txn_id}.json"));
        write_journal(&path, &data)?;

        Ok(Journal { path, data })
    }
}

/// Serializa `data` a JSON y lo persiste en `path` de forma **atómica y durable**
/// (temp+fsync+rename).
///
/// El journal es el registro que E13-H06 releerá para recuperar una publicación interrumpida, así
/// que una re-escritura no debe poder dejarlo *torn* (JSON a medias) ni siquiera si el proceso cae
/// justo mientras lo actualiza. Por eso se escribe a un temporal hermano, se hace `sync_all` del
/// temporal (durabilidad: los datos están en el medio físico) y se hace `rename` sobre el fichero
/// definitivo (atomicidad: el lector ve el JSON viejo íntegro o el nuevo íntegro, nunca uno a
/// medias). Endurecido en E13-H05 (cierra la reserva de E13-H03): antes se escribía in situ, lo que
/// bastaba para la durabilidad pero no descartaba un fichero torn si la caída ocurría a mitad de la
/// escritura.
fn write_journal(path: &Path, data: &JournalData) -> Result<(), WorkspaceError> {
    let json = serde_json::to_vec_pretty(data)
        .map_err(|e| WorkspaceError::Io(format!("no se pudo serializar el journal: {e}")))?;
    let io_err = |e: std::io::Error| WorkspaceError::Io(e.to_string());

    // Temporal hermano único por proceso+secuencia (evita que dos escrituras se pisen el temp).
    let tmp = {
        use std::sync::atomic::{AtomicU64, Ordering};
        static SEQ: AtomicU64 = AtomicU64::new(0);
        let mut name = path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(
            ".{}-{}.lodestar-tmp",
            std::process::id(),
            SEQ.fetch_add(1, Ordering::Relaxed)
        ));
        path.with_file_name(name)
    };

    {
        let mut f = std::fs::File::create(&tmp).map_err(io_err)?;
        f.write_all(&json).map_err(io_err)?;
        f.sync_all().map_err(io_err)?;
    }
    if let Err(e) = std::fs::rename(&tmp, path) {
        let _ = std::fs::remove_file(&tmp);
        return Err(io_err(e));
    }
    // Persiste la entrada del directorio (best-effort en Unix), como en `io::write_atomic`.
    #[cfg(unix)]
    if let Some(parent) = path.parent() {
        if let Ok(dir) = std::fs::File::open(parent) {
            let _ = dir.sync_all();
        }
    }
    Ok(())
}
