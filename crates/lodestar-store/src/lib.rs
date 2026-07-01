//! `lodestar-store` — cache **derivada y desechable** (SQLite/FTS5) + watcher (`ARCHITECTURE.md §5`).
//!
//! Dueño único del DDL en `<bundle>/.lodestar/index.db` (WAL, gitignored, siempre reconstruible).
//! `rusqlite`/`notify`/`crossbeam` viven **solo aquí**. El core sigue siendo la autoridad: cuando
//! SQL y core podrían discrepar, **gana el core** (lo verifica el test de paridad). Materializa lo
//! barato (`files`/`links`/`tags`/`diagnostics` + FTS5) y **sintetiza on-demand** lo que invalidaría
//! en cascada (backlinks/orphans/dangling/blast-radius).

#![doc(html_no_source)]

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use rusqlite::Connection;

use lodestar_core::types::RelPath;
use lodestar_core::{Bundle, ConceptStore};

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

/// Subdirectorio (relativo al bundle) donde vive la cache. Gitignored.
pub const CACHE_DIR: &str = ".lodestar";
/// Nombre del fichero de base de datos de la cache.
pub const DB_FILE: &str = "index.db";

/// La cache de un bundle: base SQLite + bus de eventos. Compuesta por la workspace (E5).
pub struct Store {
    root: PathBuf,
    conn: Mutex<Connection>,
    bus: Bus,
}

impl Store {
    /// Abre (o crea) la cache en `<root>/.lodestar/index.db`. Aplica el DDL; si `user_version`
    /// no coincide, hace un rebuild limpio del esquema. **No** indexa: llama a [`Store::rebuild`].
    pub fn open(root: &Path) -> Result<Self, StoreError> {
        let dir = root.join(CACHE_DIR);
        std::fs::create_dir_all(&dir).map_err(|e| StoreError::Io(e.to_string()))?;
        let conn = Connection::open(dir.join(DB_FILE))?;
        schema::apply_pragmas(&conn)?;
        schema::create_schema(&conn)?;
        if schema::read_user_version(&conn)? != schema::USER_VERSION {
            schema::drop_schema(&conn)?;
            schema::create_schema(&conn)?;
            schema::set_user_version(&conn)?;
        }
        Ok(Store {
            root: root.to_path_buf(),
            conn: Mutex::new(conn),
            bus: Bus::default(),
        })
    }

    /// Abre la cache y la reconstruye desde disco en una sola operación (lo habitual al arrancar).
    pub fn open_and_build(root: &Path) -> Result<Self, StoreError> {
        let store = Store::open(root)?;
        store.rebuild()?;
        Ok(store)
    }

    /// El root del bundle.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Suscribe un receptor de [`IndexEvent`] (broadcast). Sin suscriptores el productor no bloquea.
    pub fn subscribe(&self) -> Receiver<IndexEvent> {
        self.bus.subscribe()
    }

    // --- indexación -------------------------------------------------------

    /// Cold rebuild: `WalkBuilder` → `core::parse_file` → upsert en **una** transacción.
    /// Reemplaza todo el contenido de la cache. Emite un `IndexEvent` con todos los paths.
    pub fn rebuild(&self) -> Result<(), StoreError> {
        let disk = self.walk_disk()?;
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        schema::truncate_all(&tx)?;
        let mut changed = Vec::new();
        for (path, content, mtime, size) in &disk {
            index::upsert_file(&tx, path, content, *mtime, *size)?;
            changed.push(path.clone());
        }
        tx.commit()?;
        drop(conn);
        self.bus.emit(IndexEvent {
            changed,
            removed: Vec::new(),
        });
        Ok(())
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
        let new_hash = blake3::hash(content.as_bytes());
        let changed = {
            let mut conn = self.conn.lock().unwrap();
            if current_hash(&conn, path)?.as_deref() == Some(new_hash.as_bytes().as_slice()) {
                false
            } else {
                let tx = conn.transaction()?;
                index::upsert_file(&tx, path, content, mtime, size)?;
                tx.commit()?;
                true
            }
        };
        if changed {
            self.bus.emit(IndexEvent {
                changed: vec![path.clone()],
                removed: Vec::new(),
            });
        }
        Ok(changed)
    }

    /// Elimina un path de la cache. Devuelve `true` si existía.
    pub fn remove(&self, path: &RelPath) -> Result<bool, StoreError> {
        let removed = {
            let mut conn = self.conn.lock().unwrap();
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
            self.bus.emit(IndexEvent {
                changed: Vec::new(),
                removed: vec![path.clone()],
            });
        }
        Ok(removed)
    }

    /// Reconcilia la cache con el disco: upsert de lo cambiado (gate por hash) y borrado de lo
    /// que ya no existe. Repara drift tras tormentas de eventos. Emite un `IndexEvent` con el delta.
    pub fn reconcile_all(&self) -> Result<IndexEvent, StoreError> {
        let disk = self.walk_disk()?;
        let event = {
            let mut conn = self.conn.lock().unwrap();
            let cached = cached_paths(&conn)?;
            let disk_set: std::collections::BTreeSet<RelPath> =
                disk.iter().map(|(p, _, _, _)| p.clone()).collect();

            let tx = conn.transaction()?;
            let mut changed = Vec::new();
            for (path, content, mtime, size) in &disk {
                let new_hash = blake3::hash(content.as_bytes());
                if current_hash_tx(&tx, path)?.as_deref() != Some(new_hash.as_bytes().as_slice()) {
                    index::upsert_file(&tx, path, content, *mtime, *size)?;
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
        self.bus.emit(event.clone());
        Ok(event)
    }

    fn walk_disk(&self) -> Result<Vec<(RelPath, String, i64, i64)>, StoreError> {
        let mut out = Vec::new();
        let walker = ignore::WalkBuilder::new(&self.root)
            .hidden(false)
            .git_ignore(true)
            .filter_entry(|e| {
                let name = e.file_name().to_string_lossy();
                name != CACHE_DIR && name != ".git"
            })
            .build();
        for entry in walker {
            let entry = entry.map_err(|e| StoreError::Io(e.to_string()))?;
            let path = entry.path();
            if !path.is_file() || path.extension().map(|e| e != "md").unwrap_or(true) {
                continue;
            }
            let rel = path
                .strip_prefix(&self.root)
                .unwrap_or(path)
                .to_string_lossy()
                .replace('\\', "/");
            let Ok(rp) = RelPath::new(&rel) else { continue };
            let content =
                std::fs::read_to_string(path).map_err(|e| StoreError::Io(e.to_string()))?;
            let (mtime, size) = fs_meta(path);
            out.push((rp, content, mtime, size));
        }
        Ok(out)
    }

    // --- síntesis / agregados (SQL == core, verificado por paridad) -------

    /// `hard_fail`/`warn_count` derivados de la tabla `diagnostics`.
    pub fn conformance_counts(&self) -> Result<(usize, usize), StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok((synth::hard_fail(&conn)?, synth::warn_count(&conn)?))
    }

    /// Concepts (no reservados), en orden estable.
    pub fn concepts(&self) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.conn.lock().unwrap();
        synth::concepts(&conn)
    }

    /// Backlinks entrantes de un concept (sintetizados sobre `links.dst`).
    pub fn backlinks(&self, path: &RelPath) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.conn.lock().unwrap();
        synth::backlinks(&conn, path)
    }

    /// Huérfanos sintetizados (`ORPHAN`).
    pub fn orphans(&self) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.conn.lock().unwrap();
        synth::orphans(&conn)
    }

    /// Destinos colgantes sintetizados (`LINK-STUB`/ghosts).
    pub fn dangling(&self) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.conn.lock().unwrap();
        synth::dangling(&conn)
    }

    /// Paths listados por algún `index.md`.
    pub fn in_index(&self) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.conn.lock().unwrap();
        synth::in_index(&conn)
    }

    /// Blast-radius direccional (`Direction::In`): CTE recursivo sobre aristas inversas.
    pub fn blast_radius(&self, root: &RelPath, depth: u32) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.conn.lock().unwrap();
        synth::blast_radius(&conn, root, depth)
    }

    /// Candidatos FTS5 (acelerador, con escapado de la expresión de usuario).
    pub fn fts_candidates(&self, needle: &str) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.conn.lock().unwrap();
        synth::fts_candidates(&conn, needle)
    }

    /// Búsqueda de subcadena (semántica del core; FTS solo acelera).
    pub fn search(&self, needle: &str) -> Result<Vec<RelPath>, StoreError> {
        let conn = self.conn.lock().unwrap();
        synth::search_substring(&conn, needle)
    }

    /// Conformidad de un commit cacheada por `tree_oid` (`§10` fila 20). `None` si no está cacheada.
    /// El `tree_oid` es inmutable, así que la entrada nunca caduca (sobrevive a rebuilds).
    pub fn get_conformance(
        &self,
        tree_oid: &str,
    ) -> Result<Option<lodestar_core::types::CommitConformance>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let row = conn
            .query_row(
                "SELECT hard_fail, warn_count, conform FROM commit_conformance WHERE tree_oid = ?1",
                [tree_oid],
                |r| {
                    Ok((
                        r.get::<_, i64>(0)?,
                        r.get::<_, i64>(1)?,
                        r.get::<_, i64>(2)?,
                    ))
                },
            )
            .ok();
        Ok(
            row.map(|(hf, wc, cf)| lodestar_core::types::CommitConformance {
                hard_fail: hf as usize,
                warn_count: wc as usize,
                conform: cf != 0,
            }),
        )
    }

    /// Guarda la conformidad de un `tree_oid` en la cache (idempotente).
    pub fn put_conformance(
        &self,
        tree_oid: &str,
        c: &lodestar_core::types::CommitConformance,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO commit_conformance (tree_oid, hard_fail, warn_count, conform)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![
                tree_oid,
                c.hard_fail as i64,
                c.warn_count as i64,
                c.conform as i64
            ],
        )?;
        Ok(())
    }

    /// Un `Bundle` del core servido desde la cache (vía el trait [`ConceptStore`]).
    /// Su análisis es idéntico al de `Bundle::from_files` sobre el mismo corpus.
    pub fn bundle(&self) -> Bundle {
        Bundle::from_store(self)
    }
}

/// El store sirve el corpus al core sin materializar todos los cuerpos en RAM de golpe
/// (lee `raw` por path desde SQL). El core sigue puro: la impl SQL vive aquí.
impl ConceptStore for Store {
    fn paths(&self) -> Vec<RelPath> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = match conn.prepare("SELECT path FROM files ORDER BY path") {
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
        let conn = self.conn.lock().unwrap();
        conn.query_row(
            "SELECT raw FROM files WHERE path = ?1",
            [path.as_str()],
            |r| r.get::<_, String>(0),
        )
        .ok()
    }
}

fn current_hash(conn: &Connection, path: &RelPath) -> Result<Option<Vec<u8>>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT hash FROM files WHERE path = ?1",
            [path.as_str()],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .ok())
}

fn current_hash_tx(
    tx: &rusqlite::Transaction,
    path: &RelPath,
) -> Result<Option<Vec<u8>>, StoreError> {
    Ok(tx
        .query_row(
            "SELECT hash FROM files WHERE path = ?1",
            [path.as_str()],
            |r| r.get::<_, Vec<u8>>(0),
        )
        .ok())
}

fn cached_paths(conn: &Connection) -> Result<std::collections::BTreeSet<RelPath>, StoreError> {
    let mut stmt = conn.prepare("SELECT path FROM files")?;
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
