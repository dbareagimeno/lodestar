//! `lodestar-store` — cache **derivada y desechable** (SQLite/FTS5) + watcher (`ARCHITECTURE.md §5`).
//!
//! Dueño único del DDL en `<workspace>/.lodestar/index.db` (WAL, gitignored, siempre reconstruible).
//! `rusqlite`/`notify`/`crossbeam` viven **solo aquí**. El core sigue siendo la autoridad: cuando
//! SQL y core podrían discrepar, **gana el core** (lo verifica el test de paridad). Materializa lo
//! barato (`files`/`links`/`tags`/`diagnostics` + FTS5) y **sintetiza on-demand** lo que invalidaría
//! en cascada (backlinks/aislados/dangling/blast-radius).

#![doc(html_no_source)]

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock, Weak};
use std::time::Instant;

use rusqlite::hooks::{AuthAction, Authorization};
use rusqlite::Connection;

use lodestar_core::types::{Inventory, RelPath};
use lodestar_core::{DocumentSet, DocumentStore};

mod error;
mod event;
mod index;
mod schema;
mod synth;
mod watch;

pub use error::StoreError;
pub use event::IndexEvent;
pub use watch::Watcher;

use crossbeam_channel::Receiver;
use event::Bus;

/// Subdirectorio (relativo al workspace) donde vive la cache. Gitignored.
pub const CACHE_DIR: &str = ".lodestar";
/// Nombre del fichero de base de datos de la cache.
pub const DB_FILE: &str = "index.db";

/// Un enlace saliente **clasificado** tal cual lo materializa la tabla `links` del store v2:
/// `(raw_href, target_kind, target_path, fragment)` (`§20.12`, E18-H04). `target_kind` es el
/// discriminante serde de `LinkTarget`; `target_path` y `fragment` son `None` para los destinos sin
/// path (externo, anchor propio, escape). Es la proyección a tuplas de `Analysis::outgoing` con la
/// que el test de paridad compara la clasificación del core (invariante #3).
pub type OutgoingLink = (String, String, Option<String>, Option<String>);

/// La cache de un workspace: base SQLite + bus de eventos. Compuesta por la workspace (E5).
pub struct Store {
    root: PathBuf,
    state: Arc<RootState>,
}

/// Estado compartido por todos los handles que apuntan al mismo workspace.  Compartir también
/// la conexión es importante: un `rename(index.db.next, index.db)` invalida la inode que una
/// segunda conexión ya tenía abierta; una conexión única garantiza que todos los handles ven la
/// generación publicada y que el cierre/reapertura se hace una sola vez.
struct RootState {
    conn: Mutex<Connection>,
    db_identity: Mutex<DbIdentity>,
    writer: Mutex<()>,
    bus: Bus,
}

/// Writer gate held by one store operation. The mutex serializes handles in this process and the
/// advisory file lock serializes real processes. The lock file is persistent: ownership belongs to
/// the open file description and the OS releases it on close or process termination, so there is
/// no stale marker, PID reuse or timeout reclamation race.
struct WriterGuard<'a> {
    _local: std::sync::MutexGuard<'a, ()>,
    file: std::fs::File,
}

impl Drop for WriterGuard<'_> {
    fn drop(&mut self) {
        let _ = unlock_file(&self.file);
    }
}

fn root_states() -> &'static Mutex<HashMap<PathBuf, Weak<RootState>>> {
    static STATES: OnceLock<Mutex<HashMap<PathBuf, Weak<RootState>>>> = OnceLock::new();
    STATES.get_or_init(|| Mutex::new(HashMap::new()))
}

fn shared_state(root: &Path, db: &Path) -> Result<Arc<RootState>, StoreError> {
    let key = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let mut states = root_states()
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    if let Some(state) = states.get(&key).and_then(Weak::upgrade) {
        return Ok(state);
    }
    let conn = open_and_migrate(db).or_else(|_| {
        remove_cache_files(db)?;
        open_and_migrate(db)
    })?;
    let db_identity = db_identity(db).map_err(|error| StoreError::Io(error.to_string()))?;
    let state = Arc::new(RootState {
        conn: Mutex::new(conn),
        db_identity: Mutex::new(db_identity),
        writer: Mutex::new(()),
        bus: Bus::default(),
    });
    states.insert(key, Arc::downgrade(&state));
    Ok(state)
}

impl Store {
    /// Abre (o crea) la cache en `<root>/.lodestar/index.db`. Aplica el DDL; si `user_version`
    /// no coincide, hace un rebuild limpio del esquema. **No** indexa: llama a [`Store::rebuild`].
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        let dir = root.join(CACHE_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| StoreError::Io(e.to_string()))?;
        let db = dir.join(DB_FILE);
        // Opening can create or replace a derived database while applying the schema. Serialize
        // that write with rebuild/upsert too; otherwise two fresh processes could race before
        // either of them reaches `writer_guard`.
        let _open_gate = acquire_process_writer_lock(&dir.join("h03-writer.lock"))?;
        // La cache es DESECHABLE: si el fichero está corrupto o el esquema viejo no migra
        // (p. ej. un índice nuevo sobre una columna renombrada), se borra y se recrea limpio
        // en vez de dejar `open()` fallando para siempre.
        let state = shared_state(root, &db)?;
        let store = Store {
            root: root.to_path_buf(),
            state,
        };
        store.refresh_external_database_under_gate(&db)?;
        unlock_file(&_open_gate).map_err(|error| {
            StoreError::Io(format!(
                "unlock writer gate {}: {error}",
                dir.join("h03-writer.lock").display()
            ))
        })?;
        Ok(store)
    }

    /// Abre la cache y la reconstruye desde disco en una sola operación (lo habitual al arrancar).
    pub fn open_and_build(root: &Path) -> Result<Self, StoreError> {
        let store = Store::open(root)?;
        store.rebuild()?;
        Ok(store)
    }

    /// El root del workspace.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Suscribe un receptor de [`IndexEvent`] (broadcast). Sin suscriptores el productor no bloquea.
    pub fn subscribe(&self) -> Receiver<IndexEvent> {
        self.state.bus.subscribe()
    }

    // --- indexación -------------------------------------------------------

    /// Cold rebuild: inventario canónico → `core::parse_file` → upsert en **una** transacción.
    /// Reemplaza todo el contenido de la cache. Emite un `IndexEvent` con todos los paths.
    pub fn rebuild(&self) -> Result<serde_json::Value, StoreError> {
        let _writer = self.writer_guard()?;
        let inventory_window = RssWindow::new()?;
        let discovered = self.walk_inventory()?;
        verify_canonical_discovery_snapshot(&self.root, &discovered)?;
        self.rebuild_from_inventory_with_duration(
            &discovered.documents,
            &discovered.other_files,
            &discovered.directories,
            inventory_window,
            Some(&discovered),
        )
    }

    /// Rebuild from the compact inventory supplied by the canonical workspace discovery. Bodies
    /// are validated one at a time before the disposable generation is built; a TOCTOU mismatch
    /// aborts the generation instead of leaving a placeholder document in SQLite.
    pub fn rebuild_from_inventory(
        &self,
        paths: &[RelPath],
        others: &std::collections::BTreeSet<RelPath>,
    ) -> Result<serde_json::Value, StoreError> {
        let _writer = self.writer_guard()?;
        let inventory_window = RssWindow::new()?;
        let directories = inventory_ancestor_directories(paths, others);
        self.rebuild_from_inventory_with_duration(
            paths,
            others,
            &directories,
            inventory_window,
            None,
        )
    }

    /// Rebuilds from the complete canonical discovery snapshot. Besides documents and assets, the
    /// snapshot retains every traversed directory so additions to a previously empty directory are
    /// detectable without another walk between the inventory and payload passes.
    pub fn rebuild_from_discovered_inventory(
        &self,
        discovered: &lodestar_discovery::DiscoveredInventory,
    ) -> Result<serde_json::Value, StoreError> {
        let _writer = self.writer_guard()?;
        verify_canonical_discovery_snapshot(&self.root, discovered)?;
        let inventory_window = RssWindow::new()?;
        self.rebuild_from_inventory_with_duration(
            &discovered.documents,
            &discovered.other_files,
            &discovered.directories,
            inventory_window,
            Some(discovered),
        )
    }

    fn rebuild_from_inventory_with_duration(
        &self,
        paths: &[RelPath],
        others: &std::collections::BTreeSet<RelPath>,
        directories: &[RelPath],
        inventory_window: RssWindow,
        discovered: Option<&lodestar_discovery::DiscoveredInventory>,
    ) -> Result<serde_json::Value, StoreError> {
        let inventory = Inventory::new(
            std::iter::empty::<RelPath>(),
            paths.iter().chain(others.iter()).cloned(),
        );
        let root = self.root.clone();
        let snapshot = match discovered {
            Some(discovered) => canonical_rebuild_snapshot(&root, discovered),
            None => capture_rebuild_snapshot(&root, paths, others, directories)?,
        };
        pause_after_snapshot_before_read(&root)?;
        verify_rebuild_snapshot(&snapshot)?;
        let inventory_finished_at = monotonic_ns();
        let inventory_rss = inventory_window.finish(inventory_finished_at)?;
        let inventory_ns = inventory_finished_at
            .saturating_sub(inventory_rss.window_started_at)
            .max(1);
        let docs_root = root.clone();
        let docs_snapshot = snapshot.entries.clone();
        let docs = paths.iter().cloned().map(move |path| {
            let full = docs_root.join(path.as_str());
            let expected = docs_snapshot.get(&path).ok_or_else(|| {
                StoreError::Io(format!("rebuild inventory missing path: {}", path.as_str()))
            })?;
            let before = fs_fingerprint(&full).map_err(|error| {
                StoreError::Io(format!("rebuild before read {}: {error}", path.as_str()))
            })?;
            if before != *expected {
                return Err(StoreError::Io(format!(
                    "rebuild snapshot changed before indexing: {}",
                    path.as_str()
                )));
            }
            let content = read_payload(&docs_root, &full).map_err(|error| {
                StoreError::Io(format!("rebuild read {}: {error}", path.as_str()))
            })?;
            let after = fs_fingerprint(&full).map_err(|error| {
                StoreError::Io(format!("rebuild after read {}: {error}", path.as_str()))
            })?;
            if before.kind != 1 || before != after {
                return Err(StoreError::Io(format!(
                    "rebuild snapshot changed while indexing: {}",
                    path.as_str()
                )));
            }
            Ok((
                path,
                String::from_utf8(content).ok(),
                (after.mtime_ns / 1_000_000_000) as i64,
                after.size,
            ))
        });
        self.rebuild_iter(
            docs,
            paths.to_vec(),
            inventory,
            others.clone(),
            inventory_ns,
            inventory_rss,
            inventory_finished_at,
            snapshot,
        )
    }

    #[allow(clippy::too_many_arguments)]
    fn rebuild_iter<I>(
        &self,
        docs: I,
        _docs_by_path: Vec<RelPath>,
        mut inventory: Inventory,
        others: std::collections::BTreeSet<RelPath>,
        inventory_ns: u64,
        inventory_rss: RssMeasurement,
        inventory_finished_at: u64,
        snapshot: RebuildSnapshot,
    ) -> Result<serde_json::Value, StoreError>
    where
        I: IntoIterator<Item = Result<(RelPath, Option<String>, i64, i64), StoreError>>,
    {
        let started = Instant::now();
        let next = self.root.join(CACHE_DIR).join("index.db.next");
        remove_cache_files(&next)?;
        let mut trace = SqlTrace::new(&next)?;
        let mut next_conn = schema::open_build_connection(&next)?;
        let sql_audit = SqlAudit::new();
        next_conn.authorizer(Some(sql_audit.authorizer()));
        let index_started = Instant::now();
        let index_started_at = inventory_finished_at;
        let index_rss_window = RssWindow::new()?;
        let tx = next_conn.transaction()?;
        if failpoint_for(&self.root, "untraced_delete_during_build") {
            tx.execute("DELETE FROM documents WHERE 0", [])?;
        }
        let mark = sql_audit.begin_prepare();
        let mut inserts_other = tx.prepare("INSERT OR IGNORE INTO other_files(path) VALUES(?1)")?;
        sql_audit.finish_prepare(
            mark,
            "other_files",
            "INSERT OR IGNORE INTO other_files(path) VALUES(?1)",
        )?;
        trace.prepare(
            "other_files",
            "INSERT OR IGNORE INTO other_files(path) VALUES(?1)",
        );
        let mut rows_written = 0_u64;
        for path in &others {
            inserts_other.execute([path.as_str()])?;
            rows_written += 1;
            trace.execute(
                "other_files",
                "INSERT OR IGNORE INTO other_files(path) VALUES(?1)",
            );
        }
        let mark = sql_audit.begin_prepare();
        let mut seed = tx.prepare("INSERT INTO documents(path,title,body,frontmatter_json,frontmatter_text,content_hash,mtime,size) VALUES(?1,'','','{}','',zeroblob(0),0,0)")?;
        sql_audit.finish_prepare(mark, "documents", "INSERT INTO documents(path,...)")?;
        trace.prepare("documents", "INSERT INTO documents(path,title,body,frontmatter_json,frontmatter_text,content_hash,mtime,size)");
        let begin_audit = sql_audit.clone();
        let finish_audit = sql_audit.clone();
        let mut projection = index::StreamingProjection::prepare(
            &tx,
            move || begin_audit.begin_prepare(),
            move |mark, table, sql| finish_audit.finish_prepare(mark, table, sql),
            |table, sql| trace.prepare(table, sql),
        )?;
        if failpoint_for(&self.root, "untraced_prepare_during_build") {
            let _untraced = tx.prepare("SELECT 1")?;
        }
        let mut documents_read = 0_u64;
        let mut max_live_body_bytes = 0_u64;
        let mut relational_inserts = rows_written;
        let mut fts_inserts = 0_u64;
        let mut indexed_paths = Vec::new();
        for item in docs {
            let (path, raw, mtime, size) = item?;
            documents_read += 1;
            let Some(raw) = raw else {
                inserts_other.execute([path.as_str()])?;
                rows_written += 1;
                trace.execute(
                    "other_files",
                    "INSERT OR IGNORE INTO other_files(path) VALUES(?1)",
                );
                continue;
            };
            max_live_body_bytes = max_live_body_bytes.max(raw.len() as u64);
            let parsed = lodestar_core::model::parse_file(path.as_str(), &raw);
            seed.execute([path.as_str()])?;
            let doc_id = tx.last_insert_rowid();
            rows_written += 1;
            relational_inserts += 1;
            trace.execute("documents", "INSERT INTO documents(path,...) VALUES(...)");
            inventory.promote_document(path.clone());
            let mut callback = |table: &str| {
                if table == "documents_fts" {
                    fts_inserts += 1;
                    trace.execute(table, "INSERT INTO documents_fts(rowid,...) VALUES(...)");
                } else {
                    relational_inserts += 1;
                    trace.execute(table, "INSERT INTO ");
                }
            };
            projection.insert(
                index::ProjectionDocument {
                    path: &path,
                    raw: &raw,
                    parsed: &parsed,
                    doc_id,
                    mtime,
                    size,
                },
                &inventory,
                &sql_audit,
                &mut callback,
            )?;
            indexed_paths.push(path);
        }
        drop(seed);
        drop(inserts_other);
        drop(projection);
        // FTS5 may prepare segment-merge statements lazily while SQLite commits the transaction.
        // Keep the same narrow shadow-table allowance used by the audited FTS INSERT itself.
        let fts_commit = sql_audit.fts_execution();
        tx.commit()?;
        drop(fts_commit);
        sql_audit.assert_balanced()?;
        let index_ns = index_started.elapsed().as_nanos() as u64;
        let index_finished_at = monotonic_ns();
        let index_rss = index_rss_window.finish(index_finished_at)?;
        trace.footer(
            false,
            documents_read,
            rows_written,
            relational_inserts,
            fts_inserts,
        );
        drop(next_conn);

        // Revalidate the complete canonical snapshot after the streaming pass and before any
        // integrity check/swap. This closes the window where an admitted or non-document entry
        // changes while another body is being projected.
        verify_rebuild_snapshot(&snapshot)?;

        if failpoint_for(&self.root, "corrupt_next_before_integrity") {
            corrupt_sqlite_file(&next)?;
        }
        sync_generation(&next)?;
        let validate_start = Instant::now();
        let validate_started_at = index_finished_at;
        let validate_rss_window = RssWindow::new()?;
        let check = schema::validate_database(&next);
        match check {
            Ok(()) => trace.lifecycle("integrity_check", "ok"),
            Err(error) => {
                trace.lifecycle("integrity_check", "error");
                trace.footer(
                    true,
                    documents_read,
                    rows_written,
                    relational_inserts,
                    fts_inserts,
                );
                return Err(error);
            }
        }
        let validate_ns = validate_start.elapsed().as_nanos() as u64;
        let validate_finished_at = monotonic_ns();
        let validate_rss = validate_rss_window.finish(validate_finished_at)?;
        if failpoint_for(&self.root, "pause_before_swap") {
            let cache_dir = self.root.join(CACHE_DIR);
            let pause = cache_dir.join("h03-pause-before-swap");
            let release = cache_dir.join("h03-release-before-swap");
            std::fs::write(&pause, b"paused\n")
                .map_err(|error| StoreError::Io(error.to_string()))?;
            while !release.exists() {
                std::thread::sleep(std::time::Duration::from_millis(5));
            }
            let _ = std::fs::remove_file(&pause);
            let _ = std::fs::remove_file(&release);
        }
        // The pause seam allows an external writer to race the last validation deliberately. Keep
        // the canonical snapshot authoritative right at the publication boundary as well.
        verify_rebuild_snapshot(&snapshot)?;
        if failpoint_for(&self.root, "before_swap") {
            trace.lifecycle("swap", "blocked");
            trace.footer(
                true,
                documents_read,
                rows_written,
                relational_inserts,
                fts_inserts,
            );
            return Err(StoreError::Io("H03 failpoint before_swap".into()));
        }
        let swap_started = Instant::now();
        let swap_started_at = validate_finished_at;
        let swap_rss_window = RssWindow::new()?;
        self.swap_active(&next)?;
        let swap_ns = swap_started.elapsed().as_nanos() as u64;
        let swap_finished_at = monotonic_ns();
        let swap_rss = swap_rss_window.finish(swap_finished_at)?;
        trace.lifecycle("swap", "ok");
        let duration_ns = started.elapsed().as_nanos() as u64;
        let peak_rss = inventory_rss
            .peak_rss_bytes
            .max(index_rss.peak_rss_bytes)
            .max(validate_rss.peak_rss_bytes)
            .max(swap_rss.peak_rss_bytes);
        trace.footer(
            true,
            documents_read,
            rows_written,
            relational_inserts,
            fts_inserts,
        );
        let _ = trace.finish();
        self.state.bus.emit(IndexEvent {
            changed: indexed_paths,
            removed: Vec::new(),
        });
        Ok(serde_json::json!({
            "phases": [
                phase_json("inventory", inventory_ns.max(1), 0, 0, 0, inventory_rss.peak_rss_bytes, trace.prepares, inventory_rss.window_started_at, inventory_rss.window_started_at, inventory_rss.window_finished_at, inventory_finished_at, inventory_rss.sample_count),
                phase_json("index", index_ns.max(1), documents_read, relational_inserts, fts_inserts, index_rss.peak_rss_bytes, trace.prepares, index_started_at, index_rss.window_started_at, index_rss.window_finished_at, index_finished_at, index_rss.sample_count),
                phase_json("validate", validate_ns.max(1), 0, 0, 0, validate_rss.peak_rss_bytes, trace.prepares, validate_started_at, validate_rss.window_started_at, validate_rss.window_finished_at, validate_finished_at, validate_rss.sample_count),
                phase_json("swap", swap_ns.max(1), 0, 0, 0, swap_rss.peak_rss_bytes, trace.prepares, swap_started_at, swap_rss.window_started_at, swap_rss.window_finished_at, swap_finished_at, swap_rss.sample_count)
            ],
            "documents_read": documents_read,
            "rows_written": relational_inserts + fts_inserts,
            "relational_inserts": relational_inserts,
            "fts_inserts": fts_inserts,
            "prepared_statement_count": trace.prepares,
            "delete_statements": 0,
            "max_live_body_bytes": max_live_body_bytes,
            "peak_rss_bytes": peak_rss,
            "integrity_checked_before_swap": true,
            "duration_ns": duration_ns,
            "build_id": trace.build_id,
        }))
    }

    fn swap_active(&self, next: &Path) -> Result<(), StoreError> {
        let active = self.root.join(CACHE_DIR).join(DB_FILE);
        let placeholder = Connection::open_in_memory()?;
        let mut guard = self.state.conn.lock().unwrap();
        let old = std::mem::replace(&mut *guard, placeholder);
        old.close()
            .map_err(|(_, error)| StoreError::Sqlite(error))?;
        remove_cache_sidecars(&active)?;
        if let Err(error) = replace_durable(next, &active) {
            let reopened = open_and_migrate(&active)?;
            *guard = reopened;
            return Err(StoreError::Io(error.to_string()));
        }
        *guard = open_and_migrate(&active)?;
        let identity = db_identity(&active).map_err(|error| StoreError::Io(error.to_string()))?;
        *self
            .state
            .db_identity
            .lock()
            .unwrap_or_else(|poison| poison.into_inner()) = identity;
        sync_directory(active.parent().expect("cache directory"))?;
        Ok(())
    }

    /// Reabre la conexión compartida si otro proceso publicó una nueva inode de `index.db`.
    ///
    /// El caller conserva el lock interproceso; después se toma `conn` y finalmente la identidad,
    /// el mismo orden que usa `swap_active`. La segunda consulta de metadata ocurre ya bajo el
    /// writer para no decidir con una identidad observada antes de una publicación.
    fn refresh_external_database_under_gate(&self, db: &Path) -> Result<(), StoreError> {
        let current = db_identity(db).map_err(|error| StoreError::Io(error.to_string()))?;
        let mut conn = self
            .state
            .conn
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let mut known = self
            .state
            .db_identity
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if *known == current {
            return Ok(());
        }

        let replacement = open_and_migrate(db)?;
        let old = std::mem::replace(&mut *conn, replacement);
        old.close()
            .map_err(|(_, error)| StoreError::Sqlite(error))?;
        *known = db_identity(db).map_err(|error| StoreError::Io(error.to_string()))?;
        Ok(())
    }

    fn writer_guard(&self) -> Result<WriterGuard<'_>, StoreError> {
        let waiting = self.root.join(CACHE_DIR).join("h03-writer-waiting");
        if failpoint_for(&self.root, "pause_before_swap")
            || self
                .root
                .join(CACHE_DIR)
                .join("h03-pause-before-swap")
                .exists()
        {
            let _ = std::fs::write(&waiting, b"waiting\n");
        }
        let local = self
            .state
            .writer
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let process_lock = self.root.join(CACHE_DIR).join("h03-writer.lock");
        let file = acquire_process_writer_lock(&process_lock)?;
        let _ = std::fs::remove_file(waiting);
        if let Err(error) =
            self.refresh_external_database_under_gate(&self.root.join(CACHE_DIR).join(DB_FILE))
        {
            let _ = unlock_file(&file);
            return Err(error);
        }
        Ok(WriterGuard {
            _local: local,
            file,
        })
    }

    /// Upsert incremental de un path con contenido ya en memoria. **Gate por hash blake3**:
    /// si el contenido coincide con el de la cache (no-op/echo), no toca nada ni emite evento.
    /// Devuelve `true` si hubo cambio efectivo.
    pub fn upsert(
        &self,
        path: &RelPath,
        content: &str,
        mtime: i64,
        size: i64,
    ) -> Result<bool, StoreError> {
        let _writer = self.writer_guard()?;
        let new_hash = blake3::hash(content.as_bytes());
        let changed = {
            let mut conn = self.state.conn.lock().unwrap();
            if current_hash(&conn, path)?.as_deref() == Some(new_hash.as_bytes().as_slice()) {
                false
            } else {
                // El inventario se reconstruye de la cache (documentos + `other_files` conocidos, más
                // el propio `path`, que quizá aún no esté indexado). Un fichero de proyecto añadido
                // sin rebuild deja el inventario retrasado: es la misma limitación de cascada que
                // aceptan los diagnósticos de enlace (la paridad plena es E18-H04) — la clasificación
                // de grafo (`is_edge`/`target_path`) no depende del inventario, solo el `target_kind`
                // cosmético de un `workspaceFile`.
                let inventory = build_inventory_from_db(&conn, path)?;
                let tx = conn.transaction()?;
                index::upsert_file(&tx, path, content, mtime, size, &inventory)?;
                tx.commit()?;
                true
            }
        };
        if changed {
            self.state.bus.emit(IndexEvent {
                changed: vec![path.clone()],
                removed: Vec::new(),
            });
        }
        Ok(changed)
    }

    /// Elimina un path de la cache. Devuelve `true` si existía.
    pub fn remove(&self, path: &RelPath) -> Result<bool, StoreError> {
        let _writer = self.writer_guard()?;
        let removed = {
            let mut conn = self.state.conn.lock().unwrap();
            if current_hash(&conn, path)?.is_none() {
                false
            } else {
                let tx = conn.transaction()?;
                index::delete_file(&tx, path)?;
                tx.commit()?;
                true
            }
        };
        if removed {
            self.state.bus.emit(IndexEvent {
                changed: Vec::new(),
                removed: vec![path.clone()],
            });
        }
        Ok(removed)
    }

    /// Reconcilia la cache con el disco: upsert de lo cambiado (gate por hash) y borrado de lo
    /// que ya no existe. Repara drift tras tormentas de eventos. Emite un `IndexEvent` con el delta.
    pub fn reconcile_all(&self) -> Result<IndexEvent, StoreError> {
        let _writer = self.writer_guard()?;
        let (disk, others) = self.walk_disk()?;
        // Inventario fresco del disco (documentos + `other_files`): los documentos que se re-upserten
        // se clasifican contra el workspace completo actual.
        let inventory = Inventory::new(
            disk.iter().map(|(p, _, _, _)| p.clone()),
            others.iter().cloned(),
        );
        let event = {
            let mut conn = self.state.conn.lock().unwrap();
            let cached = cached_paths(&conn)?;
            let cached_others = cached_other_files(&conn)?;
            let other_files_changed = cached_others != others;
            let disk_set: std::collections::BTreeSet<RelPath> =
                disk.iter().map(|(p, _, _, _)| p.clone()).collect();

            let tx = conn.transaction()?;
            // Reserva los IDs de todos los documentos del snapshot antes de proyectar cualquier
            // upsert. Así un enlace entre dos ficheros que aparecen en la misma tanda ve también
            // el destino futuro, igual que en el rebuild en frío, sin alterar los hashes ni el
            // delta de eventos de los documentos que no cambiaron.
            let paths: Vec<RelPath> = disk.iter().map(|(path, _, _, _)| path.clone()).collect();
            index::seed_document_ids(&tx, &paths)?;
            write_other_files(&tx, &others)?;
            let mut changed = Vec::new();
            for (path, content, mtime, size) in &disk {
                let new_hash = blake3::hash(content.as_bytes());
                let hash_changed =
                    current_hash_tx(&tx, path)?.as_deref() != Some(new_hash.as_bytes().as_slice());
                if other_files_changed || hash_changed {
                    // TOCTOU: el walk se hizo ANTES de tomar el lock; una escritura del único
                    // escritor pudo colarse en medio (su upsert optimista ya está en la cache).
                    // Se relee el fichero con el lock tomado para no pisar contenido nuevo con
                    // el snapshot viejo del walk (flip-flop visible en la UI).
                    let fresh = std::fs::read_to_string(self.root.join(path.as_str()));
                    let (content, mtime, size) = match &fresh {
                        Ok(c) => {
                            let (m, s) = fs_meta(&self.root.join(path.as_str()));
                            (c.as_str(), m, s)
                        }
                        Err(_) => (content.as_str(), *mtime, *size),
                    };
                    let fresh_hash = blake3::hash(content.as_bytes());
                    if !other_files_changed
                        && current_hash_tx(&tx, path)?.as_deref()
                            == Some(fresh_hash.as_bytes().as_slice())
                    {
                        continue;
                    }
                    index::upsert_file(&tx, path, content, mtime, size, &inventory)?;
                    changed.push(path.clone());
                }
            }
            let mut removed = Vec::new();
            for path in cached.difference(&disk_set) {
                index::delete_file(&tx, path)?;
                removed.push(path.clone());
            }
            tx.commit()?;
            IndexEvent { changed, removed }
        };
        self.state.bus.emit(event.clone());
        Ok(event)
    }

    /// Materializa los cuerpos del inventario canónico para reconciliar. La admisión, ignores y
    /// clasificación de documentos pertenecen a `lodestar-discovery`; aquí solo se leen, una vez,
    /// los paths admitidos y se conserva su metadata para el upsert. Esto mantiene `reconcile_all`
    /// alineado con el rebuild y evita un segundo walker con una policy implícita.
    #[allow(clippy::type_complexity)]
    fn walk_disk(
        &self,
    ) -> Result<
        (
            Vec<(RelPath, String, i64, i64)>,
            std::collections::BTreeSet<RelPath>,
        ),
        StoreError,
    > {
        let policy = lodestar_discovery::load_policy(&self.root)
            .map_err(|error| StoreError::Io(error.to_string()))?;
        let discovered = lodestar_discovery::discover_inventory(&self.root, &policy)
            .map_err(|error| StoreError::Io(error.to_string()))?;
        let mut docs = Vec::with_capacity(discovered.documents.len());
        for rp in discovered.documents {
            let path = self.root.join(rp.as_str());
            let content = match std::fs::read_to_string(&path) {
                Ok(c) => c,
                Err(e) => {
                    eprintln!(
                        "lodestar-store: aviso: se salta {} (no UTF-8 o ilegible): {e}",
                        path.display()
                    );
                    continue;
                }
            };
            let (mtime, size) = fs_meta(&path);
            docs.push((rp, content, mtime, size));
        }
        Ok((docs, discovered.other_files))
    }

    /// First pass of the cold builder: only paths and the non-document inventory are retained.
    fn walk_inventory(&self) -> Result<lodestar_discovery::DiscoveredInventory, StoreError> {
        let policy = lodestar_discovery::load_policy(&self.root)
            .map_err(|error| StoreError::Io(error.to_string()))?;
        lodestar_discovery::discover_inventory(&self.root, &policy)
            .map_err(|error| StoreError::Io(error.to_string()))
    }

    // --- síntesis / agregados (SQL == core, verificado por paridad) -------

    /// `hard_fail`/`warn_count` derivados de la tabla `diagnostics`.
    pub fn validation_counts(&self) -> Result<(usize, usize), StoreError> {
        let conn = self.state.conn.lock().unwrap();
        Ok((synth::hard_fail(&conn)?, synth::warn_count(&conn)?))
    }

    /// Todos los documentos del workspace, en orden estable.
    pub fn documents(&self) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.state.conn.lock().unwrap();
        synth::documents(&conn)
    }

    /// Backlinks entrantes de un documento (sintetizados sobre `links.dst`).
    pub fn backlinks(&self, path: &RelPath) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.state.conn.lock().unwrap();
        synth::backlinks(&conn, path)
    }

    /// Documentos **aislados** sintetizados (`Analysis::isolated`): sin entrantes ni salientes.
    /// Sustituye a `orphans()`, retirado con su definición en E16-H02.
    pub fn isolated(&self) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.state.conn.lock().unwrap();
        synth::isolated(&conn)
    }

    /// Destinos colgantes sintetizados (los fantasmas del grafo: `LinkTarget::Missing`).
    pub fn dangling(&self) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.state.conn.lock().unwrap();
        synth::dangling(&conn)
    }

    /// Blast-radius direccional (`Direction::In`): CTE recursivo sobre aristas inversas.
    pub fn blast_radius(&self, root: &RelPath, depth: u32) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.state.conn.lock().unwrap();
        synth::blast_radius(&conn, root, depth)
    }

    /// Candidatos FTS5 (acelerador, con escapado de la expresión de usuario).
    pub fn fts_candidates(&self, needle: &str) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.state.conn.lock().unwrap();
        synth::fts_candidates(&conn, needle)
    }

    /// Búsqueda de subcadena (semántica del core; FTS solo acelera).
    pub fn search(&self, needle: &str) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.state.conn.lock().unwrap();
        synth::search_substring(&conn, needle)
    }

    /// Enlaces salientes materializados de un documento **con su clasificación** (`§20.12`,
    /// E18-H04): una tupla `(raw_href, target_kind, target_path, fragment)` por enlace del cuerpo,
    /// leída de la tabla `links`. Es la superficie pública por la que el test de paridad compara la
    /// clasificación de `Analysis::outgoing` (invariante #3: cuando core y cache podrían discrepar,
    /// gana el core).
    pub fn outgoing_links(&self, source: &RelPath) -> Result<Vec<OutgoingLink>, StoreError> {
        let conn = self.state.conn.lock().unwrap();
        synth::outgoing_links(&conn, source)
    }

    /// Un `DocumentSet` del core servido desde la cache (vía el trait [`DocumentStore`]).
    /// Su análisis es idéntico al de `DocumentSet::from_files` sobre el mismo corpus.
    pub fn document_set(&self) -> DocumentSet {
        DocumentSet::from_store(self)
    }

    /// Desglose persistente de SQLite mediante `dbstat`. La conexión rusqlite permanece encapsulada
    /// en `lodestar-store`; consumidores como `lodestar-bench` reciben solo JSON estructurado.
    pub fn dbstat_report(&self) -> Result<serde_json::Value, StoreError> {
        let conn = self.state.conn.lock().unwrap();
        conn.execute_batch(
            "CREATE VIRTUAL TABLE IF NOT EXISTS temp.lodestar_dbstat USING dbstat(main)",
        )?;
        let page_count: u64 =
            conn.query_row("PRAGMA page_count", [], |r| r.get::<_, i64>(0))? as u64;
        let page_size: u64 = conn.query_row("PRAGMA page_size", [], |r| r.get::<_, i64>(0))? as u64;
        let main_bytes = page_count.saturating_mul(page_size);
        let mut stmt = conn.prepare(
            "WITH dbstat_sizes AS (
                 SELECT name, SUM(pgsize) AS bytes
                 FROM temp.lodestar_dbstat
                 GROUP BY name
             ), names AS (
                 SELECT name FROM main.sqlite_schema
                 UNION
                 SELECT name FROM dbstat_sizes
             )
             SELECT names.name, sqlite_schema.type, COALESCE(dbstat_sizes.bytes,0)
             FROM names
             LEFT JOIN main.sqlite_schema ON sqlite_schema.name=names.name
             LEFT JOIN dbstat_sizes ON dbstat_sizes.name=names.name
             ORDER BY names.name",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok((
                r.get::<_, String>(0)?,
                r.get::<_, Option<String>>(1)?,
                r.get::<_, i64>(2)? as u64,
            ))
        })?;
        let mut objects = Vec::new();
        let mut attributed = 0_u64;
        for row in rows {
            let (name, schema_type, bytes) = row?;
            let kind = if name == "documents_fts" {
                "fts"
            } else if name.starts_with("documents_fts_") {
                "fts_shadow"
            } else if schema_type.as_deref() == Some("index") {
                "index"
            } else {
                "table"
            };
            attributed = attributed.saturating_add(bytes);
            objects.push(serde_json::json!({"name": name, "kind": kind, "bytes": bytes}));
        }
        let unattributed = main_bytes.saturating_sub(attributed);
        Ok(serde_json::json!({
            "main_bytes": main_bytes,
            "page_count": page_count,
            "page_size": page_size,
            "objects": objects,
            "unattributed_bytes": unattributed,
        }))
    }
}

/// El store sirve al core el snapshot Markdown exacto conservado en SQLite. El snapshot es derivado
/// del disco por el único escritor durante rebuild/upsert; el disco sigue siendo la fuente canónica.
impl DocumentStore for Store {
    fn paths(&self) -> Vec<RelPath> {
        let conn = self.state.conn.lock().unwrap();
        let mut stmt = match conn.prepare("SELECT path FROM documents ORDER BY path") {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let rows = stmt.query_map([], |r| r.get::<_, String>(0));
        match rows {
            Ok(iter) => iter
                .filter_map(|s| s.ok())
                .filter_map(|s| RelPath::new(&s).ok())
                .collect(),
            Err(_) => Vec::new(),
        }
    }

    fn raw(&self, path: &RelPath) -> Option<String> {
        let conn = self.state.conn.lock().unwrap();
        conn.query_row(
            "SELECT body FROM documents WHERE path=?1",
            [path.as_str()],
            |r| r.get::<_, String>(0),
        )
        .ok()
    }

    /// Los ficheros del proyecto que **no** son documentos (código, imágenes…), materializados en la
    /// tabla `other_files`. Es lo que permite que `DocumentSet::from_store` clasifique un enlace a
    /// código como `WorkspaceFile` y no como `Missing` (E18-H04), con el mismo inventario que ve el
    /// core al analizar el disco.
    fn other_files(&self) -> Vec<RelPath> {
        let conn = self.state.conn.lock().unwrap();
        synth::read_other_files(&conn).unwrap_or_default()
    }
}

/// Reescribe la tabla `other_files` con el conjunto dado (los ficheros de proyecto no-`.md`). Se
/// vuelca entera en cada walk completo: es pequeña y así el inventario no arrastra ficheros ya
/// borrados.
fn write_other_files(
    tx: &rusqlite::Transaction,
    others: &std::collections::BTreeSet<RelPath>,
) -> Result<(), StoreError> {
    tx.execute("DELETE FROM other_files", [])?;
    for p in others {
        tx.execute(
            "INSERT OR IGNORE INTO other_files (path) VALUES (?1)",
            [p.as_str()],
        )?;
    }
    Ok(())
}

/// Reconstruye el [`Inventory`] desde la cache: los documentos indexados + los `other_files`
/// conocidos, más `current` (el documento que se está upserteando, que quizá aún no esté en la
/// tabla). Es la vista del workspace que usa un upsert incremental para clasificar sus enlaces.
fn build_inventory_from_db(conn: &Connection, current: &RelPath) -> Result<Inventory, StoreError> {
    let mut documents: std::collections::BTreeSet<RelPath> = std::collections::BTreeSet::new();
    {
        let mut stmt = conn.prepare("SELECT path FROM documents")?;
        let iter = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for s in iter {
            if let Ok(rp) = RelPath::new(&s?) {
                documents.insert(rp);
            }
        }
    }
    documents.insert(current.clone());
    let mut others: std::collections::BTreeSet<RelPath> = std::collections::BTreeSet::new();
    {
        let mut stmt = conn.prepare("SELECT path FROM other_files")?;
        let iter = stmt.query_map([], |r| r.get::<_, String>(0))?;
        for s in iter {
            if let Ok(rp) = RelPath::new(&s?) {
                others.insert(rp);
            }
        }
    }
    Ok(Inventory::new(documents, others))
}

/// Abre la conexión y migra el esquema. El check de `user_version` va ANTES de crear el DDL
/// nuevo: aplicarlo sobre un esquema viejo puede fallar (índice sobre columna inexistente).
fn open_and_migrate(db: &Path) -> Result<Connection, StoreError> {
    let conn = Connection::open(db)?;
    schema::apply_pragmas(&conn)?;
    if schema::read_user_version(&conn)? != schema::USER_VERSION
        || !schema::schema_is_current(&conn)?
    {
        // La cache es desechable: cerrar y eliminar el fichero completo evita que objetos SQLite
        // ajenos (triggers, vistas, tablas o índices) sobrevivan a un rebuild basado en una lista
        // de drops. El workspace Markdown nunca entra en este alcance.
        drop(conn);
        remove_cache_files(db)?;
        return create_fresh_cache(db);
    } else {
        // Misma versión y esquema vigente: el DDL idempotente completa cualquier detalle declarado
        // por el constructor sin reabrir una migración ni cambiar la cache validada.
        schema::create_schema(&conn)?;
    }
    Ok(conn)
}

fn create_fresh_cache(db: &Path) -> Result<Connection, StoreError> {
    let conn = Connection::open(db)?;
    schema::apply_pragmas(&conn)?;
    schema::create_schema(&conn)?;
    schema::set_user_version(&conn)?;
    Ok(conn)
}

fn remove_cache_files(db: &Path) -> Result<(), StoreError> {
    for suffix in ["", "-wal", "-shm"] {
        let mut os_path = db.as_os_str().to_os_string();
        os_path.push(suffix);
        let path = PathBuf::from(os_path);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(StoreError::Io(format!("{}: {error}", path.display()))),
        }
    }
    Ok(())
}

fn current_hash(conn: &Connection, path: &RelPath) -> Result<Option<Vec<u8>>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT content_hash FROM documents WHERE path = ?1",
            [path.as_str()],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .ok())
}

fn cached_other_files(
    conn: &Connection,
) -> Result<std::collections::BTreeSet<RelPath>, StoreError> {
    Ok(synth::read_other_files(conn)?.into_iter().collect())
}

fn current_hash_tx(
    tx: &rusqlite::Transaction,
    path: &RelPath,
) -> Result<Option<Vec<u8>>, StoreError> {
    Ok(tx
        .query_row(
            "SELECT content_hash FROM documents WHERE path = ?1",
            [path.as_str()],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .ok())
}

fn cached_paths(conn: &Connection) -> Result<std::collections::BTreeSet<RelPath>, StoreError> {
    let mut stmt = conn.prepare("SELECT path FROM documents")?;
    let iter = stmt.query_map([], |r| r.get::<_, String>(0))?;
    let mut out = std::collections::BTreeSet::new();
    for s in iter {
        out.insert(RelPath::new(&s?)?);
    }
    Ok(out)
}

fn fs_meta(path: &Path) -> (i64, i64) {
    match std::fs::metadata(path) {
        Ok(m) => {
            let size = m.len() as i64;
            let mtime = m
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_secs() as i64)
                .unwrap_or(0);
            (mtime, size)
        }
        Err(_) => (0, 0),
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct DbIdentity {
    identity: u64,
    size: u64,
    mtime_ns: i128,
    ctime_ns: i128,
}

/// Identity compacta de la base activa. En Unix dev+inode detecta el `rename` atómico que publica
/// una generación; tamaño/mtime/ctime cubren también reemplazos que no cambian la inode y ofrecen
/// una huella portable en plataformas sin esos campos.
fn db_identity(path: &Path) -> Result<DbIdentity, std::io::Error> {
    let fingerprint = lodestar_discovery::filesystem_fingerprint(path, true)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(DbIdentity {
        identity: fingerprint.identity,
        size: u64::try_from(fingerprint.size).map_err(|_| {
            std::io::Error::other(format!("{}: tamaño de DB inválido", path.display()))
        })?,
        mtime_ns: fingerprint.mtime_ns,
        ctime_ns: fingerprint.ctime_ns,
    })
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct FileFingerprint {
    kind: u8,
    size: i64,
    mtime_ns: i128,
    identity: u64,
    ctime_ns: i128,
}

/// Huellas compactas que gobiernan las dos pasadas del rebuild. En el camino canónico se
/// construye exclusivamente desde `DiscoveredInventory`; el store no vuelve a capturar la
/// referencia de ningún path admitido.
#[derive(Clone)]
struct RebuildSnapshot {
    root: PathBuf,
    entries: BTreeMap<RelPath, FileFingerprint>,
    directories: BTreeMap<PathBuf, FileFingerprint>,
    root_target: FileFingerprint,
}

fn fs_fingerprint(path: &Path) -> Result<FileFingerprint, std::io::Error> {
    let fingerprint = lodestar_discovery::filesystem_fingerprint(path, false)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(FileFingerprint {
        kind: fingerprint.kind,
        size: fingerprint.size,
        mtime_ns: fingerprint.mtime_ns,
        identity: fingerprint.identity,
        ctime_ns: fingerprint.ctime_ns,
    })
}

fn fs_target_fingerprint(path: &Path) -> Result<FileFingerprint, std::io::Error> {
    let fingerprint = lodestar_discovery::filesystem_fingerprint(path, true)
        .map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(FileFingerprint {
        kind: fingerprint.kind,
        size: fingerprint.size,
        mtime_ns: fingerprint.mtime_ns,
        identity: fingerprint.identity,
        ctime_ns: fingerprint.ctime_ns,
    })
}

fn discovery_fingerprint(fingerprint: lodestar_discovery::DiscoveryFingerprint) -> FileFingerprint {
    FileFingerprint {
        kind: fingerprint.kind,
        size: fingerprint.size,
        mtime_ns: fingerprint.mtime_ns,
        identity: fingerprint.identity,
        ctime_ns: fingerprint.ctime_ns,
    }
}

fn capture_rebuild_snapshot(
    root: &Path,
    paths: &[RelPath],
    others: &BTreeSet<RelPath>,
    directories: &[RelPath],
) -> Result<RebuildSnapshot, StoreError> {
    prepare_payload_audit()?;
    let mut entries = BTreeMap::new();
    for path in paths.iter().chain(others.iter()) {
        let full = root.join(path.as_str());
        let fingerprint = fs_fingerprint(&full).map_err(|error| {
            StoreError::Io(format!("rebuild inventory {}: {error}", path.as_str()))
        })?;
        entries.insert(path.clone(), fingerprint);
    }
    Ok(RebuildSnapshot {
        root: root.to_path_buf(),
        entries,
        directories: snapshot_directories(root, directories)?,
        root_target: fs_target_fingerprint(root)
            .map_err(|error| StoreError::Io(format!("rebuild inventory root target: {error}")))?,
    })
}

/// El seam de auditoría de lectura escribe un sidecar bajo el root. Créalo antes de capturar la
/// huella de la frontera para que su primera escritura de contenido no parezca una mutación del
/// workspace; las altas reales siguen cambiando la huella del directorio y se rechazan.
fn prepare_payload_audit() -> Result<(), StoreError> {
    let Some(audit) = std::env::var_os("LODESTAR_H03_TEST_READ_AUDIT") else {
        return Ok(());
    };
    std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit)
        .map(|_| ())
        .map_err(|error| StoreError::Io(format!("rebuild read audit: {error}")))
}

fn canonical_rebuild_snapshot(
    root: &Path,
    discovered: &lodestar_discovery::DiscoveredInventory,
) -> RebuildSnapshot {
    let entries = discovered
        .entry_fingerprints
        .iter()
        .map(|(path, fingerprint)| (path.clone(), discovery_fingerprint(*fingerprint)))
        .collect();
    let mut directories = discovered
        .directory_fingerprints
        .iter()
        .map(|(path, fingerprint)| {
            (
                root.join(path.as_str()),
                discovery_fingerprint(*fingerprint),
            )
        })
        .collect::<BTreeMap<_, _>>();
    directories.insert(
        root.to_path_buf(),
        discovery_fingerprint(discovered.root_fingerprint),
    );
    RebuildSnapshot {
        root: root.to_path_buf(),
        entries,
        directories,
        root_target: discovery_fingerprint(discovered.root_target_fingerprint),
    }
}

/// Derives the directory boundary available to callers that supply only documents and
/// `other_files`. Canonical discovery uses its complete traversed-directory inventory instead, so
/// it also covers additions inside directories that were empty during the first pass.
fn inventory_ancestor_directories(paths: &[RelPath], others: &BTreeSet<RelPath>) -> Vec<RelPath> {
    let mut directories = BTreeSet::new();
    for path in paths.iter().chain(others.iter()) {
        let mut parent = Path::new(path.as_str()).parent();
        while let Some(directory) = parent {
            if directory.as_os_str().is_empty() {
                break;
            }
            // A parent of a valid RelPath is itself representable and cannot escape the root.
            if let Some(text) = directory.to_str() {
                if let Ok(relative) = RelPath::new(text) {
                    directories.insert(relative);
                }
            }
            parent = directory.parent();
        }
    }
    directories.into_iter().collect()
}

fn snapshot_directories(
    root: &Path,
    directories: &[RelPath],
) -> Result<BTreeMap<PathBuf, FileFingerprint>, StoreError> {
    let mut paths = BTreeSet::from([root.to_path_buf()]);
    paths.extend(
        directories
            .iter()
            .map(|directory| root.join(directory.as_str())),
    );
    paths
        .into_iter()
        .map(|path| {
            let fingerprint = fs_fingerprint(&path).map_err(|error| {
                StoreError::Io(format!(
                    "rebuild inventory directory {}: {error}",
                    path.display()
                ))
            })?;
            Ok((path, fingerprint))
        })
        .collect()
}

fn verify_rebuild_snapshot(snapshot: &RebuildSnapshot) -> Result<(), StoreError> {
    for (path, expected) in &snapshot.directories {
        let current = fs_fingerprint(path).map_err(|error| {
            StoreError::Io(format!(
                "rebuild inventory changed at directory {}: {error}",
                path.display()
            ))
        })?;
        if current != *expected {
            return Err(StoreError::Io(format!(
                "rebuild inventory changed at directory {}",
                path.display()
            )));
        }
    }
    let current_target = fs_target_fingerprint(&snapshot.root).map_err(|error| {
        StoreError::Io(format!(
            "rebuild inventory changed at workspace root target: {error}"
        ))
    })?;
    if current_target != snapshot.root_target {
        return Err(StoreError::Io(
            "rebuild inventory changed at workspace root target".into(),
        ));
    }
    for (entry, expected) in &snapshot.entries {
        let path = snapshot.root.join(entry.as_str());
        let current = fs_fingerprint(&path).map_err(|error| {
            StoreError::Io(format!(
                "rebuild inventory changed at entry {}: {error}",
                entry.as_str()
            ))
        })?;
        if current != *expected {
            return Err(StoreError::Io(format!(
                "rebuild inventory changed at entry {}",
                entry.as_str()
            )));
        }
    }
    Ok(())
}

fn verify_canonical_discovery_snapshot(
    root: &Path,
    discovered: &lodestar_discovery::DiscoveredInventory,
) -> Result<(), StoreError> {
    let current_root = fs_fingerprint(root).map_err(|error| {
        StoreError::Io(format!(
            "rebuild inventory changed at workspace root: {error}"
        ))
    })?;
    if !matches_discovery_fingerprint(current_root, discovered.root_fingerprint) {
        return Err(StoreError::Io(
            "rebuild inventory changed at workspace root".into(),
        ));
    }
    let current_root_target = fs_target_fingerprint(root).map_err(|error| {
        StoreError::Io(format!(
            "rebuild inventory changed at workspace root target: {error}"
        ))
    })?;
    if !matches_discovery_fingerprint(current_root_target, discovered.root_target_fingerprint) {
        return Err(StoreError::Io(
            "rebuild inventory changed at workspace root target".into(),
        ));
    }
    for (directory, expected) in &discovered.directory_fingerprints {
        let path = root.join(directory.as_str());
        let current = fs_fingerprint(&path).map_err(|error| {
            StoreError::Io(format!(
                "rebuild inventory changed at directory {}: {error}",
                directory.as_str()
            ))
        })?;
        if !matches_discovery_fingerprint(current, *expected) {
            return Err(StoreError::Io(format!(
                "rebuild inventory changed at directory {}",
                directory.as_str()
            )));
        }
    }
    for (entry, expected) in &discovered.entry_fingerprints {
        let path = root.join(entry.as_str());
        let current = fs_fingerprint(&path).map_err(|error| {
            StoreError::Io(format!(
                "rebuild inventory changed at entry {}: {error}",
                entry.as_str()
            ))
        })?;
        if !matches_discovery_fingerprint(current, *expected) {
            return Err(StoreError::Io(format!(
                "rebuild inventory changed at entry {}",
                entry.as_str()
            )));
        }
    }
    Ok(())
}

fn matches_discovery_fingerprint(
    current: FileFingerprint,
    expected: lodestar_discovery::DiscoveryFingerprint,
) -> bool {
    current.kind == expected.kind
        && current.size == expected.size
        && current.mtime_ns == expected.mtime_ns
        && current.identity == expected.identity
        && current.ctime_ns == expected.ctime_ns
}

fn remove_cache_sidecars(db: &Path) -> Result<(), StoreError> {
    for suffix in ["-wal", "-shm"] {
        let mut os_path = db.as_os_str().to_os_string();
        os_path.push(suffix);
        match std::fs::remove_file(PathBuf::from(os_path)) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(StoreError::Io(error.to_string())),
        }
    }
    Ok(())
}

fn sync_generation(path: &Path) -> Result<(), StoreError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| StoreError::Io(error.to_string()))?;
    file.sync_all()
        .map_err(|error| StoreError::Io(error.to_string()))
}

/// Replaces the active generation atomically on the same volume and makes the publication
/// durable before returning. Windows needs the write-through flag because `rename` has no
/// equivalent durability guarantee for the directory entry.
fn replace_durable(next: &Path, active: &Path) -> Result<(), StoreError> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::Storage::FileSystem::{
            MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
        };

        let next_wide: Vec<u16> = next
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let active_wide: Vec<u16> = active
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let result = unsafe {
            MoveFileExW(
                next_wide.as_ptr(),
                active_wide.as_ptr(),
                MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
            )
        };
        if result == 0 {
            return Err(StoreError::Io(std::io::Error::last_os_error().to_string()));
        }
        Ok(())
    }
    #[cfg(not(windows))]
    {
        std::fs::rename(next, active).map_err(|error| StoreError::Io(error.to_string()))
    }
}

fn sync_directory(path: &Path) -> Result<(), StoreError> {
    #[cfg(unix)]
    {
        let directory =
            std::fs::File::open(path).map_err(|error| StoreError::Io(error.to_string()))?;
        directory
            .sync_all()
            .map_err(|error| StoreError::Io(error.to_string()))?;
    }
    #[cfg(not(unix))]
    {
        #[cfg(windows)]
        {
            // MoveFileExW with MOVEFILE_WRITE_THROUGH in replace_durable already supplies the
            // Windows directory-entry durability barrier; validate that this call has a real
            // cache directory rather than silently accepting an empty path.
            if path.as_os_str().is_empty() {
                return Err(StoreError::Io(
                    "cannot validate an empty cache directory path".into(),
                ));
            }
        }
        #[cfg(not(windows))]
        {
            return Err(StoreError::Io(
                "directory synchronization is unsupported on this platform".into(),
            ));
        }
    }
    Ok(())
}

fn failpoint_for(root: &Path, phase: &str) -> bool {
    let Some(value) = std::env::var_os("LODESTAR_H03_FAILPOINT") else {
        return false;
    };
    let value = value.to_string_lossy();
    value == format!("{}:{phase}", root.display())
}

/// Acquires the cross-process writer gate. The file stays on disk and only its OS lock state
/// changes; closing it releases ownership even when a process exits abnormally.
fn acquire_process_writer_lock(path: &Path) -> Result<std::fs::File, StoreError> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| StoreError::Io(format!("open writer gate {}: {error}", path.display())))?;
    lock_file_exclusive(&file)
        .map_err(|error| StoreError::Io(format!("lock writer gate {}: {error}", path.display())))?;
    Ok(file)
}

#[cfg(unix)]
fn lock_file_exclusive(file: &std::fs::File) -> Result<(), std::io::Error> {
    use std::os::fd::AsRawFd;
    loop {
        let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX) };
        if result == 0 {
            return Ok(());
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

#[cfg(unix)]
fn unlock_file(file: &std::fs::File) -> Result<(), std::io::Error> {
    use std::os::fd::AsRawFd;
    let result = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_UN) };
    if result == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn lock_file_exclusive(file: &std::fs::File) -> Result<(), std::io::Error> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::{LockFileEx, LOCKFILE_EXCLUSIVE_LOCK};
    use windows_sys::Win32::System::IO::OVERLAPPED;
    let mut overlapped = OVERLAPPED::default();
    let result = unsafe {
        LockFileEx(
            file.as_raw_handle() as _,
            LOCKFILE_EXCLUSIVE_LOCK,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if result != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn unlock_file(file: &std::fs::File) -> Result<(), std::io::Error> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Storage::FileSystem::UnlockFileEx;
    use windows_sys::Win32::System::IO::OVERLAPPED;
    let mut overlapped = OVERLAPPED::default();
    let result = unsafe {
        UnlockFileEx(
            file.as_raw_handle() as _,
            0,
            u32::MAX,
            u32::MAX,
            &mut overlapped,
        )
    };
    if result != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(not(any(unix, windows)))]
fn lock_file_exclusive(_file: &std::fs::File) -> Result<(), std::io::Error> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "cross-process writer locks are unsupported on this platform",
    ))
}

#[cfg(not(any(unix, windows)))]
fn unlock_file(_file: &std::fs::File) -> Result<(), std::io::Error> {
    Ok(())
}

fn pause_after_snapshot_before_read(root: &Path) -> Result<(), StoreError> {
    if !failpoint_for(root, "after_snapshot_before_read") {
        return Ok(());
    }
    let cache_dir = root.join(CACHE_DIR);
    let pause = cache_dir.join("h03-pause-after-snapshot-before-read");
    let release = cache_dir.join("h03-release-after-snapshot-before-read");
    std::fs::write(&pause, b"paused\n").map_err(|error| StoreError::Io(error.to_string()))?;
    while !release.exists() {
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let _ = std::fs::remove_file(pause);
    let _ = std::fs::remove_file(release);
    Ok(())
}

fn corrupt_sqlite_file(path: &Path) -> Result<(), StoreError> {
    use std::io::{Seek, SeekFrom};
    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|error| StoreError::Io(error.to_string()))?;
    file.seek(SeekFrom::Start(100))
        .map_err(|error| StoreError::Io(error.to_string()))?;
    file.write_all(b"H03-CORRUPTED")
        .map_err(|error| StoreError::Io(error.to_string()))?;
    file.sync_all()
        .map_err(|error| StoreError::Io(error.to_string()))
}

fn process_rss_bytes() -> Result<u64, StoreError> {
    #[cfg(target_os = "linux")]
    {
        let statm = std::fs::read_to_string("/proc/self/statm")
            .map_err(|error| StoreError::Io(format!("leer RSS de /proc/self/statm: {error}")))?;
        let pages = statm
            .split_whitespace()
            .nth(1)
            .and_then(|value| value.parse::<u64>().ok())
            .filter(|pages| *pages > 0)
            .ok_or_else(|| StoreError::Io("/proc/self/statm no devolvió RSS residente".into()))?;
        Ok(pages.saturating_mul(4096))
    }
    #[cfg(windows)]
    {
        use windows_sys::Win32::System::ProcessStatus::{
            GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS,
        };
        use windows_sys::Win32::System::Threading::GetCurrentProcess;
        let mut counters = std::mem::MaybeUninit::<PROCESS_MEMORY_COUNTERS>::zeroed();
        let ok = unsafe {
            GetProcessMemoryInfo(
                GetCurrentProcess(),
                counters.as_mut_ptr(),
                std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32,
            )
        };
        if ok == 0 {
            return Err(StoreError::Io(format!(
                "GetProcessMemoryInfo: {}",
                std::io::Error::last_os_error()
            )));
        }
        let counters = unsafe { counters.assume_init() };
        let bytes = counters.WorkingSetSize as u64;
        if bytes == 0 {
            return Err(StoreError::Io(
                "GetProcessMemoryInfo devolvió WorkingSetSize=0".into(),
            ));
        }
        Ok(bytes)
    }
    #[cfg(target_os = "macos")]
    {
        // `ru_maxrss` es un high-water mark acumulado y no sirve para atribuir una muestra a la
        // ventana actual. `mach_task_basic_info` expone el resident_size actual, análogo al
        // resident set de `/proc/self/statm` en Linux.
        if let Some(rss) = macos_resident_size_bytes() {
            if rss > 0 {
                return Ok(rss);
            }
        }
        Err(StoreError::Io(
            "mach_task_basic_info devolvió resident_size=0 o falló".into(),
        ))
    }
    #[cfg(all(unix, not(any(target_os = "macos", target_os = "linux"))))]
    {
        let mut usage = std::mem::MaybeUninit::<libc::rusage>::zeroed();
        if unsafe { libc::getrusage(libc::RUSAGE_SELF, usage.as_mut_ptr()) } == 0 {
            let bytes = (unsafe { usage.assume_init().ru_maxrss as u64 }).saturating_mul(1024);
            if bytes > 0 {
                return Ok(bytes);
            }
        }
        Err(StoreError::Io(
            "getrusage(RUSAGE_SELF) no devolvió ru_maxrss positivo".into(),
        ))
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(StoreError::Io(
            "RSS residente no está soportado en esta plataforma".into(),
        ))
    }
}

#[cfg(target_os = "macos")]
#[allow(deprecated)]
fn macos_resident_size_bytes() -> Option<u64> {
    let mut info = std::mem::MaybeUninit::<libc::mach_task_basic_info_data_t>::zeroed();
    let mut count = libc::MACH_TASK_BASIC_INFO_COUNT;
    let result = unsafe {
        libc::task_info(
            libc::mach_task_self(),
            libc::MACH_TASK_BASIC_INFO,
            info.as_mut_ptr() as libc::task_info_t,
            &mut count,
        )
    };
    (result == 0).then(|| unsafe { info.assume_init().resident_size })
}

#[derive(Debug, Clone, Copy)]
struct RssMeasurement {
    peak_rss_bytes: u64,
    sample_count: u64,
    window_started_at: u64,
    window_finished_at: u64,
}

/// Muestrea RSS dentro de una fase.  Es diagnóstico del proceso (no el presupuesto de memoria
/// controlable) y usa la misma lectura portable de Linux/macOS que el informe global.
struct RssWindow {
    started_at: u64,
    samples: Arc<Mutex<Vec<(u64, u64)>>>,
    stop: Arc<AtomicBool>,
    sampler: Option<std::thread::JoinHandle<()>>,
}

impl RssWindow {
    fn new() -> Result<Self, StoreError> {
        let now = monotonic_ns();
        let initial = process_rss_bytes()?;
        let samples = Arc::new(Mutex::new(vec![(now, initial)]));
        let stop = Arc::new(AtomicBool::new(false));
        let thread_samples = Arc::clone(&samples);
        let thread_stop = Arc::clone(&stop);
        let sampler = std::thread::spawn(move || {
            while !thread_stop.load(Ordering::Relaxed) {
                let timestamp = monotonic_ns();
                if let Ok(rss) = process_rss_bytes() {
                    thread_samples
                        .lock()
                        .unwrap_or_else(|poison| poison.into_inner())
                        .push((timestamp, rss));
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        });
        Ok(Self {
            started_at: now,
            samples,
            stop,
            sampler: Some(sampler),
        })
    }

    fn finish(mut self, phase_finished_at: u64) -> Result<RssMeasurement, StoreError> {
        // Endpoint sampling is synchronous; periodic samples are retained only when their
        // timestamps fall inside the phase, so a racing sampler cannot extend its window.
        let final_rss = process_rss_bytes()?;
        self.samples
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .push((phase_finished_at, final_rss));
        self.stop.store(true, Ordering::Relaxed);
        if let Some(sampler) = self.sampler.take() {
            let _ = sampler.join();
        }
        let samples = self
            .samples
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let in_window: Vec<_> = samples
            .iter()
            .copied()
            .filter(|(timestamp, _)| {
                *timestamp >= self.started_at && *timestamp <= phase_finished_at
            })
            .collect();
        let peak_rss_bytes = in_window
            .iter()
            .map(|(_, rss)| *rss)
            .max()
            .ok_or_else(|| StoreError::Io("RSS window no contiene muestras".into()))?;
        Ok(RssMeasurement {
            peak_rss_bytes,
            sample_count: in_window.len() as u64,
            window_started_at: in_window
                .first()
                .map(|(timestamp, _)| *timestamp)
                .unwrap_or(self.started_at),
            window_finished_at: in_window
                .last()
                .map(|(timestamp, _)| *timestamp)
                .unwrap_or(phase_finished_at),
        })
    }
}

impl Drop for RssWindow {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(sampler) = self.sampler.take() {
            let _ = sampler.join();
        }
    }
}

fn read_payload(root: &Path, path: &Path) -> Result<Vec<u8>, std::io::Error> {
    let content = std::fs::read(path)?;
    let Some(audit) = std::env::var_os("LODESTAR_H03_TEST_READ_AUDIT") else {
        return Ok(content);
    };
    let audit = PathBuf::from(audit);
    let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
    let audit_parent = audit.parent().unwrap_or(Path::new("."));
    let audit_parent =
        std::fs::canonicalize(audit_parent).unwrap_or_else(|_| audit_parent.to_path_buf());
    if !audit_parent.starts_with(&root) {
        return Ok(content);
    }
    static AUDIT_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    let _guard = AUDIT_LOCK
        .get_or_init(|| Mutex::new(()))
        .lock()
        .unwrap_or_else(|poison| poison.into_inner());
    let canonical_path = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let relative = canonical_path
        .strip_prefix(&root)
        .unwrap_or(&canonical_path)
        .to_string_lossy()
        .replace('\\', "/");
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(audit)?;
    writeln!(
        file,
        "{}",
        serde_json::json!({
            "event": "payload_read",
            "path": relative,
            "open_count": 1,
            "read_count": 1,
            "bytes": content.len(),
        })
    )?;
    Ok(content)
}

#[allow(clippy::too_many_arguments)]
fn phase_json(
    name: &str,
    duration_ns: u64,
    documents_read: u64,
    relational_inserts: u64,
    fts_inserts: u64,
    peak_rss_bytes: u64,
    prepared_statement_count: u64,
    phase_started_monotonic_ns: u64,
    sample_window_started_monotonic_ns: u64,
    sample_window_finished_monotonic_ns: u64,
    phase_finished_monotonic_ns: u64,
    sample_count: u64,
) -> serde_json::Value {
    serde_json::json!({
        "name": name,
        "duration_ns": duration_ns,
        "peak_rss_bytes": peak_rss_bytes,
        "phase_started_monotonic_ns": phase_started_monotonic_ns,
        "sample_window_started_monotonic_ns": sample_window_started_monotonic_ns,
        "sample_window_finished_monotonic_ns": sample_window_finished_monotonic_ns,
        // Compatibilidad con consumidores H03 anteriores: representa una muestra real dentro de
        // la ventana (la última), no una lectura global posterior al swap.
        "rss_sample_monotonic_ns": sample_window_finished_monotonic_ns,
        "sample_count": sample_count,
        "phase_finished_monotonic_ns": phase_finished_monotonic_ns,
        "counters": {
            "documents_read": documents_read,
            "relational_inserts": relational_inserts,
            "fts_inserts": fts_inserts,
            "delete_statements": 0,
            "prepared_statement_count": prepared_statement_count
        }
    })
}

fn monotonic_ns() -> u64 {
    static ORIGIN: OnceLock<Instant> = OnceLock::new();
    ORIGIN
        .get_or_init(Instant::now)
        .elapsed()
        .as_nanos()
        .min(u64::MAX as u128) as u64
}

#[derive(Clone, Default)]
pub(crate) struct SqlAudit {
    state: Arc<Mutex<SqlAuditState>>,
}

#[derive(Default)]
struct SqlAuditState {
    authorizer_events: u64,
    reconciled_events: u64,
    fts_execution_depth: u32,
}

impl SqlAudit {
    fn new() -> Self {
        Self::default()
    }

    fn authorizer(
        &self,
    ) -> impl for<'r> FnMut(rusqlite::hooks::AuthContext<'r>) -> Authorization + Send + 'static
    {
        let state = Arc::clone(&self.state);
        move |context| {
            let fts_shadow_action = matches!(
                context.action,
                AuthAction::Insert { table_name }
                    | AuthAction::Delete { table_name }
                    | AuthAction::Read { table_name, .. }
                    | AuthAction::Update { table_name, .. }
                    if table_name.starts_with("documents_fts_")
            );
            let mut state = state.lock().unwrap_or_else(|poison| poison.into_inner());
            if fts_shadow_action && state.fts_execution_depth > 0 {
                return Authorization::Allow;
            }
            if matches!(
                context.action,
                AuthAction::Transaction { .. } | AuthAction::Pragma { .. }
            ) {
                return Authorization::Allow;
            }
            state.authorizer_events += 1;
            match context.action {
                // Deletes in FTS5 shadow tables are allowed only while executing the one audited
                // logical INSERT into `documents_fts`; a direct shadow-table DELETE remains denied.
                AuthAction::Delete { .. } => Authorization::Deny,
                _ => Authorization::Allow,
            }
        }
    }

    pub(crate) fn fts_execution(&self) -> FtsExecutionGuard {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.fts_execution_depth = state.fts_execution_depth.saturating_add(1);
        FtsExecutionGuard {
            audit: self.clone(),
        }
    }

    fn begin_prepare(&self) -> u64 {
        self.state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .authorizer_events
    }

    fn finish_prepare(&self, start: u64, table: &str, sql: &str) -> Result<(), StoreError> {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let delta = state.authorizer_events.saturating_sub(start);
        if delta == 0 {
            return Err(StoreError::Io(format!(
                "prepare audit divergence: {table} generated no authorizer event ({sql})"
            )));
        }
        state.reconciled_events = state.reconciled_events.saturating_add(delta);
        Ok(())
    }

    fn assert_balanced(&self) -> Result<(), StoreError> {
        let state = self
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if state.authorizer_events != state.reconciled_events {
            return Err(StoreError::Io(format!(
                "prepare audit divergence: authorizer observed {} events, reconciled {}",
                state.authorizer_events, state.reconciled_events
            )));
        }
        Ok(())
    }
}

pub(crate) struct FtsExecutionGuard {
    audit: SqlAudit,
}

impl Drop for FtsExecutionGuard {
    fn drop(&mut self) {
        let mut state = self
            .audit
            .state
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        state.fts_execution_depth = state.fts_execution_depth.saturating_sub(1);
    }
}

struct SqlTrace {
    file: Option<std::fs::File>,
    seq: u64,
    build_id: String,
    prepares: u64,
    executes: u64,
    deletes: u64,
}

impl SqlTrace {
    fn new(next: &Path) -> Result<Self, StoreError> {
        let timestamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        let build_id = format!("{}-{timestamp}", std::process::id());
        let file = match sql_trace_path_for(next) {
            Some(path) => {
                if let Some(parent) = path.parent() {
                    std::fs::create_dir_all(parent)
                        .map_err(|error| StoreError::Io(error.to_string()))?;
                }
                Some(
                    std::fs::File::create(path)
                        .map_err(|error| StoreError::Io(error.to_string()))?,
                )
            }
            None => None,
        };
        let mut trace = Self {
            file,
            seq: 0,
            build_id,
            prepares: 0,
            executes: 0,
            deletes: 0,
        };
        trace.emit(serde_json::json!({"event":"header","seq":0,"build_id":trace.build_id}))?;
        trace.seq = 1;
        Ok(trace)
    }

    fn emit(&mut self, value: serde_json::Value) -> Result<(), StoreError> {
        if let Some(file) = &mut self.file {
            writeln!(
                file,
                "{}",
                serde_json::to_string(&value).map_err(|e| StoreError::Io(e.to_string()))?
            )
            .map_err(|error| StoreError::Io(error.to_string()))?;
        }
        Ok(())
    }

    fn prepare(&mut self, table: &str, sql: &str) {
        self.prepares += 1;
        let _ = self.emit(serde_json::json!({"event":"prepare","seq":self.seq,"build_id":self.build_id,"sql":sql,"table":table}));
        self.seq += 1;
    }

    fn execute(&mut self, table: &str, sql: &str) {
        self.executes += 1;
        if sql.trim_start().to_ascii_uppercase().starts_with("DELETE") {
            self.deletes += 1;
        }
        let _ = self.emit(serde_json::json!({"event":"execute","seq":self.seq,"build_id":self.build_id,"sql":sql,"table":table}));
        self.seq += 1;
    }

    fn lifecycle(&mut self, event: &str, result: &str) {
        let _ = self.emit(serde_json::json!({"event":event,"seq":self.seq,"build_id":self.build_id,"result":result}));
        self.seq += 1;
    }

    fn footer(&mut self, complete: bool, documents: u64, rows: u64, relational: u64, fts: u64) {
        let _ = self.emit(serde_json::json!({"event":"footer","seq":self.seq,"build_id":self.build_id,"complete":complete,"counts":{"prepare":self.prepares,"execute":self.executes,"delete":self.deletes,"documents_read":documents,"rows_written":rows,"relational_inserts":relational,"fts_inserts":fts}}));
        self.seq += 1;
    }

    fn finish(&mut self) -> Result<(), StoreError> {
        if let Some(file) = &mut self.file {
            file.flush()
                .map_err(|error| StoreError::Io(error.to_string()))?;
        }
        Ok(())
    }
}

fn sql_trace_path_for(next: &Path) -> Option<PathBuf> {
    let path = PathBuf::from(std::env::var_os("LODESTAR_H03_SQL_TRACE")?);
    let trace_name = path.file_name()?.to_string_lossy();
    let next_name = next.file_name()?.to_string_lossy();

    // A process-wide diagnostic seam can briefly be visible to a rebuild for another root when
    // tests or callers rebuild independent workspaces concurrently. If the requested trace names
    // this concrete next generation, only its sibling database may claim it. Arbitrarily named
    // external collectors (for example the benchmark report) remain supported.
    if trace_name.starts_with(&format!("{next_name}.")) && path.parent() != next.parent() {
        return None;
    }
    Some(path)
}
