//! Enlaces Markdown: extracción y resolución (`ARCHITECTURE.md §20.6`, épica E17).
//!
//! Sustituye a `model::LINK_RE`/`resolve_link`/`out_links`/`raw_rel_links`: solo **Markdown
//! estándar** (inline, con fragmento, de referencia, anchors propios y URIs externas), resuelto
//! **únicamente por path** y sin una sola heurística — nada de buscar por basename o título, añadir
//! `.md`, tratar un directorio como `index.md` ni interpretar wikilinks.
//!
//! > **STUB de la fase roja de E17-H01/H02**: las funciones existen para que `tests/enlaces.rs`
//! > compile; están sin implementar. El parser Markdown (`pulldown-cmark`) es un **detalle de
//! > implementación**: ningún tipo suyo asoma en esta API.

use crate::types::{Inventory, RawLink, RelPath, ResolvedLink};

/// Todos los enlaces del cuerpo, en orden de aparición, con su href crudo y el rango de bytes de
/// su destino **relativo a `body`** (E17-H01).
///
/// No cuentan los enlaces dentro de bloques de código (fence o indentado) ni de spans de código,
/// ni los wikilinks/embeds de Obsidian, ni las imágenes (no son enlaces de navegación).
///
/// > **STUB de la fase roja de E17-H01**: sin implementar.
pub fn extract_links(body: &str) -> Vec<RawLink> {
    let _ = body;
    todo!("E17-H01: extracción de enlaces del cuerpo Markdown")
}

/// Resuelve y clasifica un enlace crudo con los 10 pasos de `§20.6` (E17-H02).
///
/// `from` es el documento **origen** (los paths relativos se resuelven contra su directorio) y
/// `inventory` es lo que el motor sabe que existe. Función total y pura: no toca el disco, así que
/// el veredicto depende solo de sus tres argumentos.
///
/// > **STUB de la fase roja de E17-H02**: sin implementar.
pub fn resolve(raw: &RawLink, from: &RelPath, inventory: &Inventory) -> ResolvedLink {
    let _ = (raw, from, inventory);
    todo!("E17-H02: resolución y clasificación del destino de un enlace")
}
