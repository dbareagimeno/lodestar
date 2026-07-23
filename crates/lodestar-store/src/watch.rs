//! Watcher incremental (`ARCHITECTURE.md §5`, `§10` fila 8): `notify-debouncer-full` (~250 ms)
//! → `reconcile_all()` (gate por hash blake3, que descarta no-ops y los echoes de nuestras propias
//! escrituras). El watcher es el **único escritor** de la cache una vez compuesto en la workspace.

use std::sync::Arc;
use std::time::Duration;

use notify::{RecursiveMode, Watcher as _};
use notify_debouncer_full::{new_debouncer, DebounceEventResult, Debouncer, FileIdMap};

use crate::error::StoreError;
use crate::Store;

/// Handle vivo del watcher. Mientras exista, los cambios en disco se reconcilian con la cache.
/// Al dropearlo, se detiene la observación.
pub struct Watcher {
    _debouncer: Debouncer<notify::RecommendedWatcher, FileIdMap>,
}

impl Store {
    /// Arranca el watcher del workspace. Cada tanda de eventos dispara `reconcile_all()`
    /// (gate por hash: los no-ops y echoes no generan `IndexEvent`).
    pub fn watch(self: &Arc<Self>) -> Result<Watcher, StoreError> {
        let store = Arc::clone(self);
        let cache_dir = self.root().join(crate::CACHE_DIR);
        let git_dir = self.root().join(".git");
        let mut debouncer = new_debouncer(
            Duration::from_millis(250),
            None,
            move |res: DebounceEventResult| {
                // Filtro: los eventos de `.lodestar/` (¡las escrituras de la PROPIA cache!) y de
                // `.git/` no disparan reconcile — sin él, cada upsert genera un eco que re-lee y
                // re-hashea el workspace entero, y cada commit/switch dispara decenas de reconciles.
                let relevant = match &res {
                    Ok(events) => events.iter().any(|ev| {
                        ev.paths
                            .iter()
                            .any(|p| !p.starts_with(&cache_dir) && !p.starts_with(&git_dir))
                    }),
                    Err(_) => false,
                };
                if relevant {
                    // El gate por hash de reconcile_all descarta los no-ops; los errores se
                    // reportan a stderr (un reconcile posterior repara el drift).
                    if let Err(e) = store.reconcile_all() {
                        eprintln!("lodestar-store: aviso: reconcile fallido: {e}");
                    }
                }
            },
        )
        .map_err(|e| StoreError::Watch(e.to_string()))?;

        debouncer
            .watcher()
            .watch(self.root(), RecursiveMode::Recursive)
            .map_err(|e| StoreError::Watch(e.to_string()))?;

        Ok(Watcher {
            _debouncer: debouncer,
        })
    }
}
