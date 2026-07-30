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
//!
//! **El barrido corre bajo el lock de publicación** (E25-H03). El criterio de «transacción viva» del
//! plano de control (`gc_runtime_huerfanos`) es *presencia en `journal/` ∪ `receipts/`*,
//! y hay una ventana en la que una transacción **en curso** no está en ninguno de los dos: entre
//! `backup_originals` y `create_journal` (`crate::transaction`, pasos 8–9) tiene copias de
//! recuperación y todavía no tiene ni journal ni recibo. Con un solo proceso eso es inocuo —quien
//! barre es quien acaba de publicar—, pero con dos el GC del proceso B borraba el plano de
//! recuperación del proceso A: A publicaba **sin copias** y, si caía, `restore_from_recovery` no
//! encontraba directorio, devolvía `Ok(())` y la recuperación sellaba un estado parcial en silencio.
//! Tomar el mismo lock que la publicación cierra la ventana sin inventar una segunda señal de vida:
//! el criterio de propiedad (¿el dueño está vivo?, ¿la marca está rancia?) sigue viviendo en **un
//! solo sitio**, `crate::lock::reclamar_si_huerfano` (invariante #3).
//!
//! **E25-H04 — el recibo se persiste ANTES del punto de no retorno.** Escribir el recibo al final
//! («tras sellar») dejaba un agujero por el que una transacción **publicada** se volvía irreversible
//! para siempre: cualquier fallo posterior al primer rename —el sellado, `write_receipt`, el GC— salía
//! por `?` y el agente recibía un error sin recibo, con el disco ya cambiado (`change_revert` →
//! `PLAN_EXPIRED`; un segundo apply del mismo plan → `PLAN_STALE`). Degradar esos fallos a *warning* no
//! cubre el `SIGKILL`, que es el caso que de verdad ocurre. Lo que cubre los dos es escribir el
//! **registro durable del recibo** con el journal, en `receipts/pending/<txnId>.json`
//! (`write_pending_receipt`) — las dos revisiones que lo componen ya se conocen ahí—, y
//! **promoverlo** a recibo (`receipts/<txnId>.json`) desde los dos únicos sitios que saben que la
//! transacción publicó: el sellado, bajo el lock y antes de borrar el journal, y la vía COMPLETAR de la
//! recuperación (`promote_pending_receipt`).
//!
//! El registro **no es** un recibo mientras no lo sea: no lo listan `list_receipts`/`load_receipt` salvo
//! que sea **efectivo** (journal en `applied` = todos los renames hechos), y el aborto de ventana y la
//! vía RESTAURAR lo retiran (`discard_pending_receipt`). Así, «publicar implica recibo» y
//! «no publicar implica no recibo» son la misma regla leída en los dos sentidos.
//!
//! Ese «bajo el lock» es una exigencia **de tipos**, no de documentación: el barrido vive en
//! [`Workspace::gc_receipts_con_el_lock_tomado`], que pide un `&`[`WorkspaceLock`] como testigo, y
//! [`Workspace::gc_receipts`] es la única puerta que lo adquiere por su cuenta. Barrer sin el lock no
//! compila.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use lodestar_core::types::{ChangeReceipt, ReceiptId};

use crate::journal::JournalState;
use crate::{Workspace, WorkspaceError, WorkspaceLock};

/// Subdirectorio de los **registros durables de recibo** de transacciones en vuelo (E25-H04):
/// `.lodestar/runtime/receipts/pending/<txnId>.json`.
///
/// Cuelga de `receipts/` —misma convención de nombre, misma vecindad— pero es un **directorio**, así
/// que todo lo que itera `receipts/` filtrando por extensión `.json`
/// ([`Workspace::list_receipts`], la retención de [`Workspace::gc_receipts`], el criterio de vivos
/// de `gc_runtime_huerfanos`) lo salta sin cambio alguno. Que un registro pendiente **no** se cuele
/// por sí solo en esos listados es justamente lo que hace que una transacción que nunca publicó no
/// deje recibo.
const PENDING_DIR: &str = "pending";

/// Nombre de fichero saneado (E13-H07), mismo criterio que staging (E13-H01, [`crate::staging`]) y
/// recovery (E13-H04, [`crate::recovery`]): neutraliza `:`/`/`/`\` (hostiles a nombres de fichero en
/// Windows y a la estructura de directorios) por `_`.
///
/// Es la **única** implementación de ese saneado en el crate (E25-H04): la usan los cuatro nombres que
/// una transacción deriva de su `txnId` —`journal/`, `staging/`, `recovery/` y `receipts/` (pendiente
/// incluido)—, que es lo que permite localizarlos entre sí. Cuando había dos copias del criterio, el
/// journal se nombraba con el id **crudo** y el registro del recibo con el saneado: con un `txnId`
/// exótico el pendiente no habría encontrado nunca su journal y no habría llegado a ser efectivo, en
/// silencio.
///
/// **Idempotente** (`_` se mapea a sí mismo), así que da igual si el id que llega es crudo o ya
/// saneado: los llamantes no tienen que saberlo.
pub(crate) fn sanear_nombre(id: &str) -> String {
    id.chars()
        .map(|c| match c {
            ':' | '/' | '\\' => '_',
            other => other,
        })
        .collect()
}

/// Nombre de fichero saneado para un `ReceiptId` (E13-H07). El resultado es determinista y permite
/// recuperar/listar el recibo por su id.
fn receipt_file_stem(id: &ReceiptId) -> String {
    sanear_nombre(&id.0)
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

    /// El directorio de **registros durables de recibo** de transacciones en vuelo
    /// (`.lodestar/runtime/receipts/pending/`), exista o no (E25-H04).
    fn pending_receipts_dir(&self) -> PathBuf {
        self.receipts_dir().join(PENDING_DIR)
    }

    /// La ruta del registro durable de recibo de `txn_id` (mismo saneado que su `.json` definitivo).
    fn pending_receipt_path(&self, txn_id: &str) -> PathBuf {
        self.pending_receipts_dir()
            .join(format!("{}.json", sanear_nombre(txn_id)))
    }

    /// Persiste el **registro durable del recibo** de una transacción que aún no ha publicado
    /// (E25-H04), en `.lodestar/runtime/receipts/pending/<txnId>.json`, con el mismo protocolo
    /// atómico y fsynced que el recibo definitivo.
    ///
    /// Lo escribe [`Workspace::apply_transaction`] **después del journal y antes del primer rename**,
    /// que es el único sitio donde puede cerrar el agujero de E25-H04: tras el primer rename el disco
    /// canónico ya cambió, así que cualquier registro escrito *después* se pierde con el proceso —y
    /// con él la única forma de deshacer la publicación (`change_revert` responde `PLAN_EXPIRED` para
    /// siempre y un segundo `change_apply` del mismo plan, `PLAN_STALE`).
    ///
    /// **No es todavía un recibo**: mientras vive aquí no lo lista `list_receipts` ni lo encuentra
    /// `load_receipt`, salvo que sea **efectivo** (ver `pending_receipt_efectivo`). Se
    /// convierte en recibo de verdad al sellar la transacción o al completarla la recuperación
    /// (`promote_pending_receipt`), y muere sin dejar rastro si la transacción aborta.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla la creación del directorio, la serialización o la escritura.
    pub(crate) fn write_pending_receipt(
        &self,
        receipt: &ChangeReceipt,
    ) -> Result<(), WorkspaceError> {
        let dir = self.pending_receipts_dir();
        std::fs::create_dir_all(&dir)?;
        let json = serde_json::to_vec_pretty(receipt).map_err(|e| {
            WorkspaceError::Io(format!(
                "no se pudo serializar el registro del receipt: {e}"
            ))
        })?;
        write_runtime_atomic(&self.pending_receipt_path(&receipt.id.0), &json)
    }

    /// El registro durable de recibo de `txn_id` tal y como está en disco, sin juzgar si la
    /// transacción publicó. `None` si no hay registro o no es JSON válido de `ChangeReceipt`.
    fn read_pending_receipt(&self, txn_id: &str) -> Option<ChangeReceipt> {
        let raw = std::fs::read_to_string(self.pending_receipt_path(txn_id)).ok()?;
        serde_json::from_str(&raw).ok()
    }

    /// El registro durable de recibo de `txn_id` **si la transacción llegó a publicar**, o `None`.
    ///
    /// El criterio es el mismo estado durable con el que la recuperación decide COMPLETAR frente a
    /// RESTAURAR (invariante #3: una sola verdad, no un segundo juicio): el journal en `applied`
    /// significa que **todos** los renames se hicieron y solo falta sellar, así que el canónico ya es
    /// el resultado y su recibo es utilizable. Con `prepared`/`applying` —renames parciales o
    /// ninguno— la recuperación va a deshacer la transacción, y un recibo suyo restauraría copias de
    /// un estado que nunca se sustituyó.
    ///
    /// Sin journal el registro no es efectivo: el sellado promueve el registro a recibo **antes** de
    /// borrar el fichero de journal, de modo que un pendiente sin journal solo puede ser un resto de
    /// una promoción ya hecha (o de un aborto que lo retiró), nunca la única prueba de una
    /// publicación. El GC lo recoge como tal.
    fn pending_receipt_efectivo(&self, txn_id: &str) -> Option<ChangeReceipt> {
        match self.journal_state_of(txn_id) {
            Some(JournalState::Applied) => self.read_pending_receipt(txn_id),
            _ => None,
        }
    }

    /// **Promueve** el registro durable de recibo de `txn_id` a recibo definitivo (E25-H04): lo
    /// escribe en `receipts/<txnId>.json` y retira el pendiente.
    ///
    /// Se invoca desde los dos únicos sitios que **saben** que la transacción publicó: el sellado de
    /// [`Workspace::apply_transaction`] (paso 11, bajo el lock y **antes** de borrar el journal, para
    /// que el hueco `[sellado, recibo)` no exista y el GC del plano de control nunca vea la
    /// transacción como basura) y la vía COMPLETAR de [`Workspace::recover`], que es la que da por
    /// bueno el registro cuando el proceso murió antes de sellar.
    ///
    /// Sin registro pendiente es un **no-op** (`Ok(())`): las transacciones que no piden recibo
    /// —[`Workspace::apply_transaction`] sin `semantic_diff`— no dejan nada que promover.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si falla la escritura del recibo definitivo. La retirada del
    ///   pendiente, en cambio, es best-effort: un pendiente que sobreviva a su promoción es
    ///   redundante (el recibo ya está) y lo recoge el GC.
    pub(crate) fn promote_pending_receipt(&self, txn_id: &str) -> Result<(), WorkspaceError> {
        let Some(receipt) = self.read_pending_receipt(txn_id) else {
            return Ok(());
        };
        self.write_receipt(&receipt)?;
        let pending = self.pending_receipt_path(txn_id);
        if let Err(e) = std::fs::remove_file(&pending) {
            eprintln!(
                "lodestar: aviso: no se pudo retirar el registro de recibo ya promovido {}: {e} \
                 (redundante: el recibo definitivo ya está persistido; el GC lo recogerá)",
                pending.display()
            );
        }
        Ok(())
    }

    /// Retira el registro durable de recibo de `txn_id` **sin promoverlo** (E25-H04): la transacción
    /// no publicó, así que su recibo no puede sobrevivirla.
    ///
    /// Lo usan el sellado del aborto de ventana ([`Workspace::seal_window_abort`], donde va **antes**
    /// de borrar el journal para que no quede ni un instante en el que el pendiente podría pasar por
    /// efectivo) y la vía RESTAURAR de la recuperación. Sin ello, `change_revert` podría escribir las
    /// copias de T1 encima de la edición externa que el aborto existe para no pisar.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si el fichero existe y no se puede borrar.
    pub(crate) fn discard_pending_receipt(&self, txn_id: &str) -> Result<(), WorkspaceError> {
        let pending = self.pending_receipt_path(txn_id);
        if pending.exists() {
            std::fs::remove_file(&pending)?;
        }
        Ok(())
    }

    /// Carga un [`ChangeReceipt`] persistido por su id (E13-H07).
    ///
    /// Desde E25-H04 cae, si no hay recibo definitivo, al **registro durable efectivo** de la
    /// transacción (ver `pending_receipt_efectivo`): es la ventana en la que el proceso
    /// publicó y murió antes de sellar, y sin esta caída `change_revert` respondería `PLAN_EXPIRED`
    /// sobre algo que sí se aplicó.
    ///
    /// # Errores
    /// - [`WorkspaceError::Io`] si no hay recibo ni registro efectivo, o si el fichero no es legible
    ///   o no es JSON válido de `ChangeReceipt`.
    pub fn load_receipt(&self, id: &ReceiptId) -> Result<ChangeReceipt, WorkspaceError> {
        let path = self
            .receipts_dir()
            .join(format!("{}.json", receipt_file_stem(id)));
        let raw = match std::fs::read_to_string(&path) {
            Ok(raw) => raw,
            Err(e) => {
                return self.pending_receipt_efectivo(&id.0).ok_or_else(|| {
                    WorkspaceError::Io(format!("receipt ilegible {}: {e}", path.display()))
                })
            }
        };
        serde_json::from_str(&raw)
            .map_err(|e| WorkspaceError::Io(format!("receipt corrupto {}: {e}", path.display())))
    }

    /// Lista los recibos persistidos, **del más reciente al más antiguo** (E23-H11).
    ///
    /// Sin esto, un agente que pierde el `receiptId` que devolvió `change_apply` no puede revertir
    /// aunque el recibo siga en disco: el undo era inalcanzable por accidente de memoria del
    /// llamador. Lo consume `workspace_status`, que ya es donde vive `recovery.pendingTransaction`.
    ///
    /// El orden es por **mtime descendente** — el mismo criterio de antigüedad que usa
    /// [`Workspace::gc_receipts`], por la misma razón: `ChangeReceipt` no lleva timestamp propio (es
    /// runtime desechable) y el mtime del `.json` es el único reloj disponible. Descendente porque
    /// el recibo que se quiere revertir es casi siempre el último.
    ///
    /// **Tolerante como el resto del runtime**: un directorio de recibos ausente es una lista
    /// **vacía**, no un error (mismo patrón que `gc_receipts` y `pending_journals`), y un `.json`
    /// ilegible o corrupto se salta en vez de tumbar la llamada — es una tool de estado, y un
    /// recibo roto no puede impedir que el agente vea los sanos. La lista está acotada de fábrica
    /// por `transactions.maximumReceipts` (default 20), que el GC hace cumplir.
    ///
    /// **E25-H04** — al listado se suman los **registros durables efectivos**: transacciones que
    /// publicaron (journal `applied`) y cuyo proceso murió antes de sellar, así que su recibo aún no
    /// está en `receipts/`. Sin ellos, un agente que mira `workspace_status.receipts` justo después
    /// de un crash no vería la transacción que sí cambió su conocimiento —y no podría revertirla— hasta
    /// que la recuperación corriese. Un pendiente cuya transacción **no** publicó no aparece aquí
    /// nunca (ver `pending_receipt_efectivo`).
    pub fn list_receipts(&self) -> Vec<ChangeReceipt> {
        let mut entries: Vec<(SystemTime, ChangeReceipt)> = Vec::new();
        let mut vistos: BTreeSet<String> = BTreeSet::new();

        if let Ok(read_dir) = std::fs::read_dir(self.receipts_dir()) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if path.extension().and_then(|x| x.to_str()) != Some("json") {
                    continue;
                }
                let Ok(mtime) = entry.metadata().and_then(|m| m.modified()) else {
                    continue;
                };
                let Ok(raw) = std::fs::read_to_string(&path) else {
                    continue;
                };
                let Ok(receipt) = serde_json::from_str::<ChangeReceipt>(&raw) else {
                    continue;
                };
                vistos.insert(receipt.id.0.clone());
                entries.push((mtime, receipt));
            }
        }

        // Registros durables efectivos (E25-H04): el recibo definitivo, cuando existe, manda — es el
        // mismo contenido, y así el pendiente redundante de una promoción a medio limpiar no duplica
        // la entrada.
        if let Ok(read_dir) = std::fs::read_dir(self.pending_receipts_dir()) {
            for entry in read_dir.flatten() {
                let path = entry.path();
                if path.extension().and_then(|x| x.to_str()) != Some("json") {
                    continue;
                }
                let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                let Some(receipt) = self.pending_receipt_efectivo(stem) else {
                    continue;
                };
                if !vistos.insert(receipt.id.0.clone()) {
                    continue;
                }
                let mtime = entry
                    .metadata()
                    .and_then(|m| m.modified())
                    .unwrap_or_else(|_| SystemTime::now());
                entries.push((mtime, receipt));
            }
        }

        // Más reciente primero; a igualdad de mtime (relojes de baja resolución), el id desempata
        // para que el orden sea total y reproducible.
        entries.sort_by(|(ma, ra), (mb, rb)| mb.cmp(ma).then_with(|| ra.id.0.cmp(&rb.id.0)));
        entries.into_iter().map(|(_, receipt)| receipt).collect()
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
    /// ficheros afectados, o ya haberse limpiado) — y desde E25-H04 tampoco lo es que **no se pueda
    /// borrar**: el barrido avisa por stderr y lo reintenta en la siguiente pasada, porque corre
    /// después de que la transacción llamante haya publicado y un `Err` suyo la convertiría en un
    /// fallo (ver `el_cierre_no_convierte_un_apply_publicado_en_error`).
    ///
    /// **Corre bajo el lock de publicación** (E25-H03) y es **fail-fast y silencioso**: si el lock
    /// está tomado —o su fichero es ilegible, o no se puede crear el runtime— **no barre** y devuelve
    /// `Ok(())`. Las dos mitades de esa frase son requisitos:
    /// - *bajo el lock*, porque el criterio de «vivo» del plano de control no ve a una transacción de
    ///   **otro proceso** detenida en la ventana `[backup, journal)` y barrerla le quitaría las copias
    ///   con las que se recupera (ver la documentación de módulo);
    /// - *fail-fast y silencioso*, porque el GC se invoca **después** de que la transacción llamante
    ///   haya publicado: un `Err` suyo convertiría un apply ya publicado en un fallo sin recibo. No
    ///   espera al lock (el modelo de `ARCHITECTURE.md §19.5` es no bloqueante) y no propaga el
    ///   `WriteConflict` de [`Workspace::acquire_lock`]; como mucho pospone la retención al siguiente
    ///   barrido, que es una degradación aceptable de algo que es best-effort por definición.
    ///
    /// Quien ya posee el lock —la recuperación de `App::recover_if_pending`— no puede pasar por aquí
    /// (se auto-bloquearía y el GC quedaría en un no-op permanente): usa
    /// [`Workspace::gc_receipts_con_el_lock_tomado`], que **exige el guard como argumento**.
    ///
    /// # Errores
    /// En la práctica, ninguno: desde E25-H04 el barrido entero es best-effort (avisa por stderr y
    /// sigue). Conserva el `Result` porque es el contrato de sus llamantes y porque el criterio de
    /// «no tumbar a quien me llamó» tiene que poder comprobarse (`el_gc_nunca_tumba_a_quien_lo_llama`).
    /// **Nunca** falla por no haber conseguido el lock.
    pub fn gc_receipts(&self) -> Result<(), WorkspaceError> {
        // Fail-fast: sin lock no se barre, y no barrer no es un error (ver doc de la función).
        let Ok(lock) = self.acquire_lock() else {
            return Ok(());
        };
        self.gc_receipts_con_el_lock_tomado(&lock)
    }

    /// [`Workspace::gc_receipts`] para quien **ya posee** el lock de publicación (E25-H03).
    ///
    /// Mismo barrido y mismos errores; lo único que no hace es adquirir el lock, porque el lock de
    /// este workspace es **no reentrante** (`O_CREAT | O_EXCL` sobre un fichero: el propio poseedor se
    /// bloquea a sí mismo). El único llamante es la recuperación de la fachada
    /// (`App::recover_if_pending`), que recupera y barre bajo un mismo lock — justo después de un
    /// crash es cuando hay basura en el plano de control, así que ese camino no puede quedarse sin GC.
    ///
    /// # El lock se pide por TIPOS, no por documentación
    ///
    /// `lock` es un **testigo**: no se lee para barrer, se exige para poder llamar. Barrer el plano de
    /// control sin el lock reabre el defecto que E25-H03 cerró —el barrido se llevaría las copias de
    /// recuperación de una transacción en curso en **otro** proceso—, y un contrato así no puede
    /// quedarse en un párrafo de rustdoc que el siguiente call-site no leerá. Como [`WorkspaceLock`]
    /// solo se obtiene de [`Workspace::acquire_lock`] y libera el lock en su `Drop`, tener una
    /// referencia viva a uno **es** la prueba de posesión: llamar a esta función sin poseer el lock no
    /// compila, y la referencia mantiene el guard vivo durante todo el barrido (no puede dropearse a
    /// mitad). Mismo patrón de chokepoint que [`lodestar_core::types::RelPath`] para el
    /// path-traversal (invariante #6): lo que no debe poder expresarse, que no compile.
    ///
    /// # Errores
    /// Los mismos que [`Workspace::gc_receipts`].
    ///
    /// # Pánicos
    /// En compilaciones de debug, si `lock` no es el lock de **este** workspace (un `debug_assert`:
    /// el testigo prueba posesión, y esto comprueba además identidad — un mismatch solo puede venir
    /// de un llamante que barre un workspace mientras sostiene el lock de otro).
    pub fn gc_receipts_con_el_lock_tomado(
        &self,
        lock: &WorkspaceLock,
    ) -> Result<(), WorkspaceError> {
        debug_assert_eq!(
            lock.path(),
            self.lock_path(),
            "el testigo tiene que ser el lock de ESTE workspace: barrer el plano de control de uno \
             sosteniendo el lock de otro no protege nada"
        );
        let ttl = parse_retention(&self.config().transactions.retain_receipts_for);

        let dir = self.receipts_dir();
        let Ok(read_dir) = std::fs::read_dir(&dir) else {
            // Sin recibos no hay nada que purgar por retención, pero SÍ puede haber huérfanos:
            // una transacción abortada deja staging sin llegar nunca a producir recibo, que es
            // justo el caso que E24-H06 recoge. Salir aquí sin barrer dejaría el arreglo sin
            // efecto en el escenario que lo motivó.
            return self.gc_runtime_huerfanos();
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
            // Best-effort a propósito (E25-H04): el barrido corre DESPUÉS de que la transacción
            // llamante haya publicado, así que un `Err` aquí convertiría un apply ya publicado en un
            // fallo. La retención se pospone al siguiente barrido, que es una degradación aceptable de
            // algo que es best-effort por definición.
            if let Err(e) = std::fs::remove_file(path) {
                eprintln!(
                    "lodestar: aviso: no se pudo purgar el recibo caducado {}: {e} (la retención se \
                     reintentará en el siguiente barrido)",
                    path.display()
                );
                continue;
            }
            let recovery = self.receipt_recovery_dir(stem);
            if recovery.exists() {
                if let Err(e) = std::fs::remove_dir_all(&recovery) {
                    eprintln!(
                        "lodestar: aviso: no se pudieron purgar las copias de recuperación {}: {e}",
                        recovery.display()
                    );
                }
            }
        }

        self.gc_runtime_huerfanos()?;

        Ok(())
    }

    /// Barre el plano de control: directorios de `staging/` y `recovery/` que ya no pertenecen a
    /// ninguna transacción **viva ni recordada**, y temporales `*.lodestar-tmp` abandonados
    /// (E24-H06).
    ///
    /// El GC de recibos itera **solo** `receipts/`, así que un `staging/<txn>/` cuya transacción
    /// nunca llegó a producir recibo le es invisible: no hay entrada con ese stem. Este barrido va
    /// al revés —recorre `staging/` y `recovery/` y purga lo que no tiene respaldo— que es la única
    /// forma de recoger lo que dejaban las transacciones abortadas.
    ///
    /// **Qué se considera vivo, y por qué no se toca**:
    /// - una transacción con **journal** presente está a medio publicar o a medio recuperar: su
    ///   staging y sus copias son justamente lo que la recuperación necesita;
    /// - una transacción con **recibo** vigente puede revertirse, y `change_revert` restaura desde
    ///   `recovery/<txn>/`.
    ///
    /// Todo lo demás es basura: la convención de nombre única de este módulo (mismo `txnId` saneado
    /// para `staging/`, `recovery/`, `journal/` y `receipts/`) es lo que permite decidirlo.
    ///
    /// **Ese criterio solo es correcto con el lock de publicación en la mano** (E25-H03). Entre
    /// `backup_originals` y `create_journal` una transacción en curso no aparece ni en `journal/` ni en
    /// `receipts/`, así que «no tiene respaldo» y «no existe» son indistinguibles desde aquí: es el
    /// llamante quien garantiza que nadie está publicando, sosteniendo el lock. No hay una tercera
    /// lista de «vivos» a propósito — la vida de una transacción se pregunta donde ya se preguntaba,
    /// al lock (`crate::lock::reclamar_si_huerfano`), y no en un formato durable paralelo que habría
    /// que caducar por separado.
    ///
    /// **El registro durable del recibo (E25-H04) no añade una cuarta lista, y no le hace falta**: vive
    /// bajo el **journal**, en el sentido literal de que solo existe mientras el journal existe. Se
    /// escribe después de `create_journal` y el sellado lo **promueve a recibo antes** de borrar el
    /// fichero de journal, así que el par `journal/ ∪ receipts/` cubre sin discontinuidad la vida entera
    /// de la transacción, incluido el hueco `[sellado, recibo)` que dejaba el recibo escrito por la
    /// fachada. Lo único que este barrido añade es recoger un pendiente **sin journal**: solo puede ser
    /// el resto de una promoción ya hecha o de un aborto que lo retiró, y sin barrerlo se acumularía
    /// para siempre.
    fn gc_runtime_huerfanos(&self) -> Result<(), WorkspaceError> {
        let runtime = self.root.join(".lodestar").join("runtime");

        // Stems con respaldo: journal vivo (transacción en curso o pendiente de recuperar) o
        // recibo persistido (revertible).
        let mut vivos: BTreeSet<String> = BTreeSet::new();
        for (sub, ext) in [("journal", "json"), ("receipts", "json")] {
            if let Ok(rd) = std::fs::read_dir(runtime.join(sub)) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.extension().and_then(|x| x.to_str()) == Some(ext) {
                        if let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
                            vivos.insert(stem.to_string());
                        }
                    }
                }
            }
        }

        for sub in ["staging", "recovery"] {
            let Ok(rd) = std::fs::read_dir(runtime.join(sub)) else {
                continue;
            };
            for e in rd.flatten() {
                let p = e.path();
                let Some(nombre) = p.file_name().and_then(|s| s.to_str()) else {
                    continue;
                };
                if p.is_dir() {
                    if !vivos.contains(nombre) {
                        // Best-effort: que un huérfano no se pueda borrar no puede tumbar la
                        // transacción que acaba de publicarse con éxito.
                        let _ = std::fs::remove_dir_all(&p);
                    }
                    continue;
                }
                // El sidecar de huellas de las copias (`recovery/<txnId>.digests.json`, E25-H02) es un
                // FICHERO hermano del árbol de su transacción: vive y muere con él, así que se juzga
                // con el mismo criterio de propiedad. Sin esto, el barrido —que solo miraba
                // directorios— dejaría un sidecar huérfano por cada transacción abortada.
                if let Some(stem) = nombre.strip_suffix(".digests.json") {
                    if !vivos.contains(stem) {
                        let _ = std::fs::remove_file(&p);
                    }
                }
            }
        }

        // Registros durables de recibo (E25-H04) sin journal: o su transacción ya los promovió a
        // recibo, o un aborto los retiró. En ninguno de los dos casos vuelven a servir para nada, y sin
        // este barrido se acumularían sin límite (mismo criterio best-effort que los huérfanos de
        // arriba: un pendiente que no se pueda borrar no puede tumbar a quien acaba de publicar).
        if let Ok(rd) = std::fs::read_dir(self.pending_receipts_dir()) {
            for e in rd.flatten() {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("json") {
                    continue;
                }
                let Some(stem) = p.file_stem().and_then(|s| s.to_str()) else {
                    continue;
                };
                if self.journal_state_of(stem).is_none() {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }

        // Temporales de la escritura atómica abandonados por un crash entre el `File::create` y el
        // `rename` (`io::write_bytes_atomic`, `journal::write_journal`,
        // `receipts::write_runtime_atomic`). No rompen nada —`pending_journals` filtra por extensión
        // `.json` y el descubrimiento por `.md`—, pero se acumulan sin límite. `recovery/` entró en la
        // lista con E25-H02: su sidecar de huellas se escribe con el mismo protocolo durable, y
        // `receipts/pending/` con E25-H04 (mismo `write_runtime_atomic`).
        for sub in ["journal", "receipts", "plans", "recovery"] {
            if let Ok(rd) = std::fs::read_dir(runtime.join(sub)) {
                for e in rd.flatten() {
                    let p = e.path();
                    if p.to_string_lossy().ends_with(".lodestar-tmp") {
                        let _ = std::fs::remove_file(&p);
                    }
                }
            }
        }
        if let Ok(rd) = std::fs::read_dir(self.pending_receipts_dir()) {
            for e in rd.flatten() {
                let p = e.path();
                if p.to_string_lossy().ends_with(".lodestar-tmp") {
                    let _ = std::fs::remove_file(&p);
                }
            }
        }

        Ok(())
    }
}
