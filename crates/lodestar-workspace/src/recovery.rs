//! Copias de recuperación (E13-H04, `ARCHITECTURE.md §19.5`, `REFACTOR §5.2`): antes de sustituir
//! el conocimiento `.md` canónico, guarda el contenido previo de cada fichero afectado bajo
//! `.lodestar/runtime/recovery/<txnId>/` para poder restaurarlo si la publicación falla.
//!
//! Es el eslabón que hace la publicación **recuperable**: con las copias listas, un fallo entre
//! renames (E13-H06) puede deshacerse restaurando los originales; los paths que no existían quedan
//! marcados "no existía" para poder borrarlos al revertir (E13-H09).
//!
//! Runtime, no canónico: el árbol de recuperación vive bajo `.lodestar/runtime/`, que el walker de
//! conocimiento (`io::load_bundle`) y el watcher excluyen (E9-H06) y `WorkspaceRevision` ignora
//! (E10-H03), por lo que no viola «los `.md` son la única fuente de verdad» (invariante #1).
//! Copiar el original solo **lee** el canónico: nunca lo modifica.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use lodestar_core::types::RelPath;

use crate::error::WorkspaceError;
use crate::Workspace;

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
