//! El trait [`DocumentStore`] (`ARCHITECTURE.md §3`, `§10` fila 1).
//!
//! Abstrae la **lectura** de documentos para que, a escala, el motor del core opere sobre
//! proyecciones (p. ej. SQL en `lodestar-store`) en vez de mantener todo el corpus en RAM.
//! El core sigue **puro**: el trait no arrastra `rusqlite`; la implementación SQL vive en `store`.

use crate::types::{FileMap, RelPath};
use crate::DocumentSet;

/// Lectura abstracta del corpus de un workspace. La impl trivial es un [`FileMap`] en RAM
/// (corpus pequeños); una impl SQL sirve las mismas filas desde `.lodestar/index.db`.
pub trait DocumentStore {
    /// Todas las rutas del workspace, en orden estable (**todos** los `.md`: desde E16-H02 ningún
    /// nombre de fichero recibe trato especial).
    fn paths(&self) -> Vec<RelPath>;

    /// El contenido crudo (`.md`) de una ruta, o `None` si no existe.
    fn raw(&self, path: &RelPath) -> Option<String>;

    /// Los ficheros del proyecto que **no** son documentos (código, imágenes, `.md` excluidos del
    /// descubrimiento…): el inventario que declara un [`crate::types::LinkTarget::WorkspaceFile`].
    ///
    /// Por defecto **vacío**: un [`FileMap`] en RAM no conoce el resto del proyecto. Una impl que sí
    /// lo conozca (la cache SQL) lo sobreescribe para que [`DocumentSet::from_store`] resuelva los
    /// enlaces con el mismo inventario que el core (invariante #3), sin degradar un `WorkspaceFile`
    /// a `Missing`.
    fn other_files(&self) -> Vec<RelPath> {
        Vec::new()
    }

    /// Reconstruye el `FileMap` completo desde el store (por defecto vía `paths`+`raw`).
    fn file_map(&self) -> FileMap {
        self.paths()
            .into_iter()
            .filter_map(|p| self.raw(&p).map(|r| (p, r)))
            .collect()
    }
}

/// El `FileMap` en RAM es un `DocumentStore` trivial (la vía por defecto de v1).
impl DocumentStore for FileMap {
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

impl DocumentSet {
    /// Construye un `DocumentSet` sirviendo el corpus desde un [`DocumentStore`] (SQL o en RAM),
    /// declarando además sus [`DocumentStore::other_files`] para que la resolución de enlaces
    /// clasifique los `WorkspaceFile` igual que el core sobre disco (E18-H04). El análisis resultante
    /// es idéntico al de [`DocumentSet::with_other_files`] sobre el mismo corpus e inventario.
    pub fn from_store<S: DocumentStore + ?Sized>(store: &S) -> Self {
        DocumentSet::with_other_files(store.file_map(), store.other_files())
    }
}
