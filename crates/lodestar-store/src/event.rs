//! Bus de eventos `IndexEvent` (`ARCHITECTURE.md §5`, `§9`) — `crossbeam`, síncrono y
//! runtime-neutral. El MCP lo puentea a tokio; Tauri a `app.emit`; la CLI lo ignora.

use std::sync::Mutex;

use crossbeam_channel::{unbounded, Receiver, Sender};
use lodestar_core::types::RelPath;

/// Un cambio en la cache: qué paths se upsertaron y cuáles se borraron.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IndexEvent {
    /// Paths creados o modificados (upsert efectivo, ya pasado el gate de hash).
    pub changed: Vec<RelPath>,
    /// Paths eliminados de la cache.
    pub removed: Vec<RelPath>,
}

impl IndexEvent {
    /// `true` si el evento no representa ningún cambio real.
    pub fn is_empty(&self) -> bool {
        self.changed.is_empty() && self.removed.is_empty()
    }
}

/// Difusor de `IndexEvent` a N suscriptores (broadcast). Sin suscriptores no bloquea.
#[derive(Default)]
pub(crate) struct Bus {
    subs: Mutex<Vec<Sender<IndexEvent>>>,
}

impl Bus {
    /// Nuevo suscriptor: devuelve un `Receiver` propio (canal ilimitado, no bloquea al productor).
    pub(crate) fn subscribe(&self) -> Receiver<IndexEvent> {
        let (tx, rx) = unbounded();
        self.subs.lock().unwrap().push(tx);
        rx
    }

    /// Emite un evento a todos los suscriptores vivos; purga los cerrados.
    pub(crate) fn emit(&self, ev: IndexEvent) {
        if ev.is_empty() {
            return;
        }
        let mut subs = self.subs.lock().unwrap();
        subs.retain(|tx| tx.send(ev.clone()).is_ok());
    }
}
