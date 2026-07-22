//! `core::schema` — el catálogo de tipos de un bundle (`DocType`, campos, relaciones tipadas,
//! plantillas), **puro** (`ARCHITECTURE.md §19.2`, `docs/REFACTOR.md §4/§9.4`).
//!
//! Este módulo solo modela y deserializa: **nunca abre ficheros**. La lectura de
//! `<root>/.lodestar/schema.yaml` es I/O de `lodestar-workspace` (patrón `Config::load`,
//! `crates/lodestar-workspace/src/config.rs`), que deserializa el texto a [`Schema`] y se lo
//! entrega ya construido a quien lo consuma. El wire YAML usa claves `camelCase`
//! (`requiredFields`/`allowedStatuses`/`bodyTemplate`/`targetTypes`) mapeadas a los campos
//! `snake_case` de estos tipos — mismo convenio que `WorkspaceConfig`.
//!
//! Un bundle **sin** `schema.yaml` se modela como [`Schema::default()`]: `types` vacío, lo que
//! deja el bundle sin restricciones adicionales (compat con bundles OKF actuales que no declaran
//! esquema). La validación schema-driven (`SCHEMA-REQFIELD`/`SCHEMA-STATUS`/…) es E10-H07 y queda
//! fuera de este módulo.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Catálogo de esquemas de un bundle: versión + `DocType`s indexados por nombre de tipo.
///
/// `Schema::default()` es el esquema **vacío y permisivo** (sin `DocType`s declarados): el que
/// se usa cuando un bundle no tiene `.lodestar/schema.yaml`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Schema {
    /// Versión del formato de esquema (independiente de `okf_version` del frontmatter).
    #[serde(default = "default_schema_version")]
    pub version: String,
    /// `DocType`s declarados, indexados por su nombre de tipo (la clave del mapa YAML).
    pub types: BTreeMap<String, DocType>,
}

impl Default for Schema {
    fn default() -> Self {
        Schema {
            version: default_schema_version(),
            types: BTreeMap::new(),
        }
    }
}

fn default_schema_version() -> String {
    "1".to_string()
}

/// Definición de un tipo de documento (`docs/REFACTOR.md §9.4`, salida de `schema_inspect`).
///
/// Todos los campos llevan `#[serde(default)]`: un YAML parcial (solo `requiredFields`, por
/// ejemplo) deserializa sin error, con el resto en su valor por defecto.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DocType {
    /// Nombre del tipo. Puede omitirse en el YAML (la clave de `Schema::types` ya lo da); si el
    /// wire lo trae explícito, debe coincidir con la clave — el loader de `workspace` no lo
    /// fuerza aquí (el core no valida contra sí mismo), pero sí lo rellena cuando falta
    /// (`WorkspaceSchema::load`).
    pub name: String,
    /// Descripción humana del tipo (para `schema_inspect`, UI, etc.).
    pub description: String,
    /// Campos de frontmatter obligatorios para este tipo (`SCHEMA-REQFIELD` en E10-H07).
    pub required_fields: Vec<String>,
    /// Valores permitidos para `status` de este tipo (`SCHEMA-STATUS` en E10-H07). Vacío =
    /// cualquier `status` es válido (sin restricción).
    pub allowed_statuses: Vec<String>,
    /// Campos adicionales declarados por el tipo, indexados por nombre (forma simple: solo la
    /// descripción del campo; la validación de tipo/forma queda fuera de alcance de esta
    /// historia).
    pub fields: BTreeMap<String, FieldDef>,
    /// Relaciones tipadas declaradas por el tipo, indexadas por el nombre del campo de relación
    /// (p. ej. `implemented_by`). Validadas en E11-H03 (`REL-TARGET`/`REL-CARD`/`REL-TYPE`).
    pub relations: BTreeMap<String, RelationDef>,
    /// Reglas adicionales en lenguaje libre (documentales; sin mecánica de validación asociada
    /// todavía).
    pub rules: Vec<String>,
    /// Plantilla de cuerpo para `create_concept` de este tipo (aplicación en E12-H05).
    pub body_template: Option<String>,
}

/// Definición simple de un campo adicional de un [`DocType`].
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FieldDef {
    /// Descripción humana del campo.
    pub description: String,
}

/// Definición de una relación tipada (`docs/REFACTOR.md §9.4`; validación en E11-H03).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RelationDef {
    /// Tipos de `DocType` que puede referenciar esta relación. Vacío = cualquier tipo destino.
    pub target_types: Vec<String>,
    /// Cardinalidad de la relación (`"one"`/`"many"`), en forma simple de `String` — el enum
    /// cerrado se decide junto con su validación (E11-H03), no aquí.
    #[serde(default = "default_cardinality")]
    pub cardinality: String,
}

impl Default for RelationDef {
    fn default() -> Self {
        RelationDef {
            target_types: Vec::new(),
            cardinality: default_cardinality(),
        }
    }
}

fn default_cardinality() -> String {
    "many".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_vacio_es_permisivo() {
        let s = Schema::default();
        assert_eq!(s.version, "1");
        assert!(s.types.is_empty());
    }
}
