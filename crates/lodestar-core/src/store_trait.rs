//! El trait [`ConceptStore`] (`ARCHITECTURE.md §3`, `§10` fila 1).
//!
//! Abstrae la **lectura** de concepts para que, a escala, el motor del core opere sobre
//! proyecciones (p. ej. SQL en `lodestar-store`) en vez de mantener todo el corpus en RAM.
//! El core sigue **puro**: el trait no arrastra `rusqlite`; la implementación SQL vive en `store`.

use crate::types::{FileMap, RelPath};
use crate::Bundle;

/// Lectura abstracta del corpus de un bundle. La impl trivial es un [`FileMap`] en RAM
/// (corpus pequeños); una impl SQL sirve las mismas filas desde `.lodestar/index.db`.
pub trait ConceptStore {
    /// Todas las rutas del bundle (concepts + reservados), en orden estable.
    fn paths(&self) -> Vec<RelPath>;

    /// El contenido crudo (`.md`) de una ruta, o `None` si no existe.
    fn raw(&self, path: &RelPath) -> Option<String>;

    /// Reconstruye el `FileMap` completo desde el store (por defecto vía `paths`+`raw`).
    fn file_map(&self) -> FileMap {
        self.paths()
            .into_iter()
            .filter_map(|p| self.raw(&p).map(|r| (p, r)))
            .collect()
    }
}

/// El `FileMap` en RAM es un `ConceptStore` trivial (la vía por defecto de v1).
impl ConceptStore for FileMap {
    fn paths(&self) -> Vec<RelPath> {
        self.keys().cloned().collect()
    }
    fn raw(&self, path: &RelPath) -> Option<String> {
        self.get(path).cloned()
    }
    fn file_map(&self) -> FileMap {
        self.clone()
    }
}

impl Bundle {
    /// Construye un `Bundle` sirviendo el corpus desde un [`ConceptStore`] (SQL o en RAM).
    /// El análisis resultante es idéntico al de [`Bundle::from_files`] sobre el mismo corpus.
    pub fn from_store<S: ConceptStore + ?Sized>(store: &S) -> Self {
        Bundle::from_files(store.file_map())
    }
}
