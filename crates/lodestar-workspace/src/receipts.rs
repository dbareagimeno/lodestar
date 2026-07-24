//! Persistencia y retención de recibos transaccionales (E13-H07, `ARCHITECTURE.md §19.3`,
//! `REFACTOR §6.5, §11.3`).
//!
//! Tras sellar una transacción (`done`, E13-H08), el `ChangeReceipt` resultante se persiste como
//! `.lodestar/runtime/receipts/<receiptId saneado>.json` para poder revertir (E13-H09) y auditar.
//! Runtime desechable (invariante #1: los `.md` canónicos son la única fuente de verdad; un
//! recibo perdido no compromete el conocimiento, solo la capacidad de revertir/inspeccionar).
//!
//! **Convención de vínculo con la copia de recuperación**: el directorio
//! `.lodestar/runtime/recovery/<id>/` de una transacción se nombra con el `txnId` (E13-H04,
//! [`crate::Workspace::backup_originals`]). E13-H08 (`change_apply`, fuera de alcance aquí) reutiliza
//! ese mismo `txnId` como `receiptId` al sellar la transacción, así que el saneado del `receiptId`
//! (idéntico al de `recovery_dir_name`/`staging_dir_name`: neutraliza `:`/`/`/`\`) localiza tanto el
//! recibo como su copia de recuperación con el mismo nombre. El GC de este módulo se apoya en esa
//! convención para borrar ambos juntos.
//!
//! **Retención**: [`Workspace::gc_receipts`] purga por dos criterios independientes leídos de
//! `WorkspaceConfig::transactions` (E9-H05, default `retainReceiptsFor: "24h"` /
//! `maximumReceipts: 20`) — excedentes (los más antiguos por encima del límite de cantidad) y
//! caducados (más viejos que la retención por edad) —, decidiendo "más antiguo" por el **mtime**
//! del fichero `<receiptId>.json`: `ChangeReceipt` no lleva timestamp propio (es runtime
//! desechable) y el mtime es el mismo reloj que gobierna la retención por edad.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use lodestar_core::types::{ChangeReceipt, ReceiptId};

use crate::{Workspace, WorkspaceError};

/// Nombre de fichero saneado para un `ReceiptId` (E13-H07), mismo criterio que staging (E13-H01,
/// [`crate::staging`]) y recovery (E13-H04, [`crate::recovery`]): neutraliza `:`/`/`/`\`
/// (hostiles a nombres de fichero en Windows y a la estructura de directorios) por `_`. El
/// resultado es determinista y permite recuperar/listar el recibo por su id.
fn receipt_file_stem(id: &ReceiptId) -> String {
    id.0.chars()
        .map(|c| match c {
            ':' | '/' | '\\' => '_',
            other => other,
        })
        .collect()
}

/// Interpreta la unidad de `transactions.retainReceiptsFor` (p. ej. `"24h"`): un número entero
/// seguido opcionalmente de un sufijo `s`/`m`/`h`/`d` (segundos/minutos/horas/días; sin sufijo se
/// interpreta como segundos). No es un parser de duraciones completo (no admite combinaciones como
/// `"1h30m"`) — cubre el caso de uso de esta config (`ARCHITECTURE.md §19.4`). Una entrada vacía o
/// no reconocida devuelve `None` ("sin caducidad por edad"): ante un valor malformado, el GC no
/// purga agresivamente por edad (el límite de `maximumReceipts` sigue aplicando igual).
fn parse_retention(spec: &str) -> Option<Duration> {
    let s = spec.trim();
    if s.is_empty() {
        return None;
    }
    let (num_part, unit) = match s.chars().last() {
        Some(c) if c.is_ascii_alphabetic() => {
            (&s[..s.len() - c.len_utf8()], c.to_ascii_lowercase())
        }
        _ => (s, 's'),
    };
    let n: u64 = num_part.trim().parse().ok()?;
    let secs = match unit {
        's' => n,
        'm' => n.checked_mul(60)?,
        'h' => n.checked_mul(3600)?,
        'd' => n.checked_mul(86400)?,
        _ => return None,
    };
    Some(Duration::from_secs(secs))
}

/// Serializa `bytes` en `path` de forma **atómica y durable** (temp+fsync+rename), mismo patrón
/// que el write-ahead journal (E13-H03, `write_journal` en [`crate::journal`]). Los recibos son
/// runtime desechable, pero fsyncarlos evita que una caída justo tras `done` deje un `.json` a
/// medias que confundiría un `load_receipt`/GC posterior.
fn write_runtime_atomic(path: &Path, bytes: &[u8]) -> Result<(), WorkspaceError> {
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
        f.write_all(bytes).map_err(io_err)?;
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

impl Workspace {
    /// El directorio de recibos persistidos (`.lodestar/runtime/receipts/`), exista o no.
    fn receipts_dir(&self) -> PathBuf {
        self.root.join(".lodestar").join("runtime").join("receipts")
    }

    /// El directorio de recuperación asociado a un recibo por convención de nombre (mismo
    /// `<id saneado>` que su `.json`, ver documentación de módulo).
    fn receipt_recovery_dir(&self, stem: &str) -> PathBuf {
        self.root
            .join(".lodestar")
            .join("runtime")
            .join("recovery")
            .join(stem)
    }

    /// Persiste un [`ChangeReceipt`] de una aplicación completada como
    /// `.lodestar/runtime/receipts/<receiptId>.json` (E13-H07). Crea el directorio de recibos si
    /// falta. Escritura atómica y fsynced (temp+fsync+rename, ver `write_runtime_atomic`).
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla la creación del directorio, la serialización o la
    ///   escritura del fichero.
    pub fn write_receipt(&self, receipt: &ChangeReceipt) -> Result<(), WorkspaceError> {
        let dir = self.receipts_dir();
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_vec_pretty(receipt)
            .map_err(|e| WorkspaceError::Io(format!("no se pudo serializar el receipt: {e}")))?;
        let path = dir.join(format!("{}.json", receipt_file_stem(&receipt.id)));
        write_runtime_atomic(&path, &json)
    }

    /// Carga un [`ChangeReceipt`] persistido por su id (E13-H07).
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si el fichero no existe, no es legible o no es JSON válido de
    ///   `ChangeReceipt`.
    pub fn load_receipt(&self, id: &ReceiptId) -> Result<ChangeReceipt, WorkspaceError> {
        let path = self
            .receipts_dir()
            .join(format!("{}.json", receipt_file_stem(id)));
        let raw = std::fs::read_to_string(&path)
            .map_err(|e| WorkspaceError::Io(format!("receipt ilegible {}: {e}", path.display())))?;
        serde_json::from_str(&raw)
            .map_err(|e| WorkspaceError::Io(format!("receipt corrupto {}: {e}", path.display())))
    }

    /// Recolecta los recibos caducados (`transactions.retainReceiptsFor`) o excedentes
    /// (`transactions.maximumReceipts`) según la config del workspace (E9-H05, default `24h`/`20`),
    /// borrando además las copias de recuperación asociadas
    /// (`.lodestar/runtime/recovery/<receiptId>/`, ver convención en la documentación de módulo).
    ///
    /// Ordena los recibos por **mtime** del `.json` (más antiguo primero, ver documentación de
    /// módulo) y purga la unión de:
    /// - los excedentes: los más antiguos por encima de `maximumReceipts`;
    /// - los caducados: cuyo mtime es más viejo que `retainReceiptsFor` (si el valor no se puede
    ///   interpretar —`parse_retention` devuelve `None`—, este criterio no purga nada; el de
    ///   cantidad sigue aplicando igual).
    ///
    /// Ausencia del directorio de recibos = nada que recolectar (`Ok(())`). Best-effort por
    /// recibo: si falta la copia de recuperación de uno purgado, no es un error (pudo no haber
    /// ficheros afectados, o ya haberse limpiado).
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla el borrado de un `.json` purgado o de su copia de
    ///   recuperación.
    pub fn gc_receipts(&self) -> Result<(), WorkspaceError> {
        let ttl = parse_retention(&self.config().transactions.retain_receipts_for);

        let dir = self.receipts_dir();
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            return Ok(());
        };

        let mut entries: Vec<(PathBuf, SystemTime, String)> = Vec::new();
        for entry in read_dir.flatten() {
            let path = entry.path();
            if path.extension().and_then(|x| x.to_str()) != Some("json") {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let Ok(mtime) = meta.modified() else {
                continue;
            };
            let stem = path
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or_default()
                .to_string();
            entries.push((path, mtime, stem));
        }
        // Más antiguo primero: es el orden que gobierna tanto el corte por cantidad como la
        // inspección por edad.
        entries.sort_by_key(|(_, mtime, _)| *mtime);

        let mut purge: BTreeSet<String> = BTreeSet::new();

        // (a) Excedentes: los más antiguos por encima de `maximumReceipts`.
        let max = self.config().transactions.maximum_receipts;
        if entries.len() > max {
            let excess = entries.len() - max;
            for (_, _, stem) in entries.iter().take(excess) {
                purge.insert(stem.clone());
            }
        }

        // (b) Caducados por `retainReceiptsFor`.
        if let Some(ttl) = ttl {
            let now = SystemTime::now();
            for (_, mtime, stem) in &entries {
                let age = now.duration_since(*mtime).unwrap_or_default();
                if age > ttl {
                    purge.insert(stem.clone());
                }
            }
        }

        for (path, _, stem) in &entries {
            if !purge.contains(stem) {
                continue;
            }
            std::fs::remove_file(path)?;
            let recovery = self.receipt_recovery_dir(stem);
            if recovery.exists() {
                std::fs::remove_dir_all(&recovery)?;
            }
        }

        Ok(())
    }
}
