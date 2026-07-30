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
//! Ese «bajo el lock» es una exigencia **de tipos**, no de documentación: el barrido vive en
//! [`Workspace::gc_receipts_con_el_lock_tomado`], que pide un `&`[`WorkspaceLock`] como testigo, y
//! [`Workspace::gc_receipts`] es la única puerta que lo adquiere por su cuenta. Barrer sin el lock no
//! compila.

use std::collections::BTreeSet;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use lodestar_core::types::{ChangeReceipt, ReceiptId};

use crate::{Workspace, WorkspaceError, WorkspaceLock};

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
    pub fn list_receipts(&self) -> Vec<ChangeReceipt> {
        let Ok(read_dir) = std::fs::read_dir(self.receipts_dir()) else {
            return Vec::new();
        };

        let mut entries: Vec<(SystemTime, ChangeReceipt)> = Vec::new();
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
            entries.push((mtime, receipt));
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
    /// ficheros afectados, o ya haberse limpiado).
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
    /// - [`WorkspaceError::Io`] si falla el borrado de un `.json` purgado o de su copia de
    ///   recuperación. **Nunca** falla por no haber conseguido el lock.
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
            std::fs::remove_file(path)?;
            let recovery = self.receipt_recovery_dir(stem);
            if recovery.exists() {
                std::fs::remove_dir_all(&recovery)?;
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

        // Temporales de la escritura atómica abandonados por un crash entre el `File::create` y el
        // `rename` (`io::write_bytes_atomic`, `journal::write_journal`,
        // `receipts::write_runtime_atomic`). No rompen nada —`pending_journals` filtra por extensión
        // `.json` y el descubrimiento por `.md`—, pero se acumulan sin límite. `recovery/` entró en la
        // lista con E25-H02: su sidecar de huellas se escribe con el mismo protocolo durable.
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

        Ok(())
    }
}
