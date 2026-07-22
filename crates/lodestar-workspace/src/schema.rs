//! Loader de `<root>/.lodestar/schema.yaml` → `lodestar_core::schema::Schema`
//! (`ARCHITECTURE.md §19.2`, `docs/REFACTOR.md §4/§9.4`; patrón `WorkspaceConfig::load`,
//! `crates/lodestar-workspace/src/config.rs`).
//!
//! El TIPO `Schema` (y sus `DocType`/`RelationDef`) vive en `lodestar-core` (puro, sin I/O); este
//! módulo es el ÚNICO sitio que abre `schema.yaml` en disco y se lo entrega ya deserializado — el
//! core nunca abre ficheros (invariante #2).

use std::path::Path;

use lodestar_core::schema::Schema;

/// Ruta del fichero de esquemas, relativa al root del bundle.
pub const SCHEMA_FILE: &str = ".lodestar/schema.yaml";

/// Loader de `.lodestar/schema.yaml`.
pub struct WorkspaceSchema;

impl WorkspaceSchema {
    /// Carga `<root>/.lodestar/schema.yaml` si existe; si no, devuelve `Schema::default()` (vacío
    /// y permisivo — mismo patrón que `Config::load`/`WorkspaceConfig::load`: la ausencia de
    /// fichero no es un error, compat con bundles OKF actuales sin esquema declarado). YAML
    /// presente pero malformado sí es un error explícito.
    ///
    /// Rellena `DocType::name` desde la clave del mapa `types` cuando el YAML no lo trae
    /// explícito (`name` es opcional en el wire: la clave ya identifica el tipo).
    pub fn load(root: &Path) -> Result<Schema, String> {
        let path = root.join(SCHEMA_FILE);
        let mut schema = match std::fs::read_to_string(&path) {
            Ok(text) => serde_yaml::from_str::<Schema>(&text)
                .map_err(|e| format!("{SCHEMA_FILE} inválido: {e}"))?,
            Err(_) => Schema::default(),
        };
        for (key, doc_type) in schema.types.iter_mut() {
            if doc_type.name.is_empty() {
                doc_type.name = key.clone();
            }
        }
        Ok(schema)
    }
}
