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
    /// Arranca el watcher del bundle. Cada tanda de eventos dispara `reconcile_all()`
    /// (gate por hash: los no-ops y echoes no generan `IndexEvent`).
    pub fn watch(self: &Arc<Self>) -> Result<Watcher, StoreError> {
        let store = Arc::clone(self);
        let mut debouncer = new_debouncer(
            Duration::from_millis(250),
            None,
            move |res: DebounceEventResult| {
                if res.is_ok() {
                    // El gate por hash de reconcile_all descarta los no-ops; los errores del
                    // watcher se ignoran (un reconcile posterior repara el drift).
                    let _ = store.reconcile_all();
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
