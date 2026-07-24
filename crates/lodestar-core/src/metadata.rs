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

use std::collections::{BTreeMap, HashMap};

use crate::types::{
    FieldInspection, FieldPath, FieldStats, MetadataCatalog, ParsedFrontmatter, ValueCount,
    ValueType,
};
use crate::DocumentSet;

/// El **catálogo de propiedades** del workspace (E20-H01): por cada `field_path` que emite
/// [`crate::types::ParsedFrontmatter::walk`] en algún documento, en cuántos documentos aparece
/// (`present_in`) y qué tipos toma (`inferred_types`). Incluye los mapas intermedios (`service`)
/// además de las hojas (`service.name`, `service.tier`).
///
/// Se construye recorriendo cada frontmatter con [`walk`](ParsedFrontmatter::walk) —una fila por
/// par `(FieldPath, &Value)`— y clasificando cada valor con [`ValueType::of`]. `walk` emite cada
/// `FieldPath` como mucho una vez por documento, así que cada par es exactamente **una** observación:
/// `present_in` suma 1 por documento presente e `inferred_types` una observación de tipo por él
/// (invariante `sum(inferred_types) == present_in`). Un workspace sin frontmatter en ningún
/// documento produce un catálogo vacío, sin error.
///
/// El acumulador es un [`BTreeMap`] tecleado por [`FieldPath`], de modo que `fields` sale ordenado
/// por `FieldPath` sin un paso de ordenación aparte (`service` < `service.name` < `service.tier`).
pub fn catalog(docs: &DocumentSet) -> MetadataCatalog {
    // (present_in, {ValueType: conteo}) por campo. BTreeMap por FieldPath → orden determinista.
    let mut acc: BTreeMap<FieldPath, (usize, BTreeMap<ValueType, usize>)> = BTreeMap::new();
    for fm in frontmatters(docs) {
        for (field, value) in fm.walk() {
            let (present_in, tipos) = acc.entry(field).or_default();
            *present_in += 1;
            *tipos.entry(ValueType::of(value)).or_insert(0) += 1;
        }
    }
    let fields = acc
        .into_iter()
        .map(|(field, (present_in, inferred_types))| FieldStats {
            field,
            present_in,
            inferred_types,
        })
        .collect();
    MetadataCatalog { fields }
}

/// La **inspección de una propiedad** (E20-H02): `present_in`/`missing_in`, `inferred_types` y los
/// valores escalares más frecuentes (`values`). Funciona sobre paths anidados (`service.tier`,
/// `release.target.date`).
///
/// La presencia y el valor del campo se resuelven con [`ParsedFrontmatter::get`] —el mismo accesor
/// canónico que usa el evaluador de consultas (E19), no una navegación propia del `Value`— y el
/// tipo con [`ValueType::of`]. `present_in` cuenta los documentos donde `get` devuelve algo (aunque
/// sea `null`); `missing_in` es el resto del **total** de documentos del workspace (los sin
/// frontmatter y los con frontmatter pero sin este campo), de modo que
/// `present_in + missing_in == nº de documentos`.
///
/// `values` cuenta **solo escalares** (`null`/bool/número/string): un valor lista u objeto suma en
/// `present_in` y su tipo en `inferred_types`, pero no aparece en `values`. Un `null` presente es un
/// escalar y **sí** aparece —distinto de la ausencia, que no llega a `present_in`—.
///
/// # Orden de `values` (determinista)
/// Por conteo **descendente** y, a igual conteo, por el **texto** del valor **ascendente** (el
/// render de `scalar_text`: el número `2` y el string `"2"` rinden ambos a `"2"`; el `null` se
/// ordena bajo `"null"`). Un tercer desempate por [`ValueType`] cierra el no-determinismo
/// latente cuando dos valores **distintos** rinden al mismo texto con el mismo conteo (el número `2`
/// antes que el string `"2"`, por `Number` < `String`): ningún test lo fija, pero deja el orden
/// **total** y reproducible.
pub fn inspect_field(docs: &DocumentSet, field: &FieldPath) -> FieldInspection {
    let total = docs.files().len();
    let mut present_in = 0usize;
    let mut inferred_types: BTreeMap<ValueType, usize> = BTreeMap::new();
    // Conteo por valor escalar. `serde_yaml::Value` es `Hash + Eq` (no `Ord`), así que se agrupa en
    // un HashMap y se ordena al final con un comparador total explícito.
    let mut conteos: HashMap<serde_yaml::Value, usize> = HashMap::new();

    for fm in frontmatters(docs) {
        let Some(value) = fm.get(field) else {
            continue;
        };
        present_in += 1;
        let tipo = ValueType::of(value);
        *inferred_types.entry(tipo).or_insert(0) += 1;
        // Solo los escalares entran en `values`; lista y objeto quedan fuera (sí en inferred_types).
        if es_escalar(tipo) {
            *conteos.entry(value.clone()).or_insert(0) += 1;
        }
    }

    let mut values: Vec<ValueCount> = conteos
        .into_iter()
        .map(|(value, count)| ValueCount { value, count })
        .collect();
    // Orden total: conteo descendente → texto del valor ascendente → `ValueType` (este último cierra
    // el desempate cuando dos valores distintos rinden al mismo texto, p. ej. el nº `2` y el str `"2"`).
    values.sort_by(|a, b| {
        b.count
            .cmp(&a.count)
            .then_with(|| texto_orden(&a.value).cmp(&texto_orden(&b.value)))
            .then_with(|| ValueType::of(&a.value).cmp(&ValueType::of(&b.value)))
    });

    FieldInspection {
        field: field.clone(),
        present_in,
        missing_in: total - present_in,
        inferred_types,
        values,
    }
}

/// Los frontmatter parseados de los documentos que tienen bloque, reutilizando el parseo que ya
/// hizo el [`DocumentSet`] (no reparsea). Base común de [`catalog`] e [`inspect_field`].
fn frontmatters(docs: &DocumentSet) -> impl Iterator<Item = &ParsedFrontmatter> + '_ {
    docs.files().keys().filter_map(|p| {
        docs.parsed(p)
            .and_then(|parsed| parsed.frontmatter.as_ref())
    })
}

/// `true` si el [`ValueType`] es un escalar contable en `values` (`null`/bool/número/string); lista
/// y objeto no lo son.
fn es_escalar(tipo: ValueType) -> bool {
    !matches!(tipo, ValueType::List | ValueType::Mapping)
}

/// El texto con el que un valor escalar entra en el orden de `values`. Reutiliza
/// [`crate::types::scalar_text`] (única verdad del render de escalar) y ordena el `null` —que no
/// tiene texto de escalar— bajo su representación canónica `"null"`.
fn texto_orden(v: &serde_yaml::Value) -> String {
    crate::types::scalar_text(v).unwrap_or_else(|| "null".to_owned())
}
