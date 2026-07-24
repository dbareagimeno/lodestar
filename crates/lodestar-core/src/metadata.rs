//! Inspección genérica de metadata (`ARCHITECTURE.md §20.10`, `REFACTOR_PHASE_2 §Fase 6`, épica E20).
//!
//! Dos funciones **puras** sobre un [`DocumentSet`]: [`catalog`] (el catálogo de propiedades de
//! E20-H01) e [`inspect_field`] (la inspección de una propiedad de E20-H02). Permiten a un agente
//! comprender las convenciones de una base desconocida **sin necesitar un schema**.
//!
//! Ambas se construyen sobre [`crate::types::ParsedFrontmatter::walk`] (E18, el iterador
//! `(FieldPath, &Value)`) y clasifican cada valor con [`crate::types::ValueType::of`] (E19): una
//! sola verdad de qué es un campo y de qué tipo (invariante #3). La FORMA de sus tipos de retorno
//! ([`MetadataCatalog`]/[`FieldInspection`]) vive en `crate::types` (invariante #4) y es el contrato
//! de wire que hereda la tool `metadata_inspect` (E20-H03).

use crate::types::{FieldInspection, FieldPath, MetadataCatalog};
use crate::DocumentSet;

/// El **catálogo de propiedades** del workspace (E20-H01): por cada `field_path` que emite
/// [`crate::types::ParsedFrontmatter::walk`] en algún documento, en cuántos documentos aparece
/// (`present_in`) y qué tipos toma (`inferred_types`). Incluye los mapas intermedios (`service`)
/// además de las hojas (`service.name`, `service.tier`).
pub fn catalog(docs: &DocumentSet) -> MetadataCatalog {
    let _ = docs;
    todo!("E20-H01: catálogo de propiedades sobre walk + ValueType::of")
}

/// La **inspección de una propiedad** (E20-H02): `present_in`/`missing_in`, `inferred_types` y los
/// valores escalares más frecuentes (`values`, orden determinista por conteo desc y luego por valor).
/// Funciona sobre paths anidados (`service.tier`, `release.target.date`).
pub fn inspect_field(docs: &DocumentSet, field: &FieldPath) -> FieldInspection {
    let _ = (docs, field);
    todo!("E20-H02: inspección de un campo")
}
