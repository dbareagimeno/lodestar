//! Tests de la **inspección genérica de metadata** (épica E20, `ARCHITECTURE.md §20.10`).
//!
//! Fase ROJA de **E20-H01** (catálogo de propiedades) y **E20-H02** (inspección de una propiedad):
//! las dos funciones puras de `metadata_inspect` que sustituyen a `schema_inspect`, para que un
//! agente comprenda las convenciones de una base desconocida **sin schema** (`REFACTOR_PHASE_2
//! §Fase 6`).
//!
//! Fichero propio (no `documento.rs` ni `consulta.rs`) por los mismos tres motivos que aislaron
//! aquellos:
//!   1. Estos tests **no pasan** hasta que existan `metadata::catalog`/`inspect_field` (hoy
//!      `todo!()`): aislados, su rojo no arrastra a los ~362 tests verdes de los demás binarios.
//!   2. E20-H02 (inspección de campo) aporta la misma familia —la inspección de metadata— y tiene
//!      aquí su hogar natural.
//!   3. El estilo del repo es «un fichero de integración por familia» (`consulta.rs`, `enlaces.rs`,
//!      `grafo.rs`); `metadata.rs` es esa familia.
//!
//! ---
//!
//! ## La API que fija esta fase roja (el contrato que hereda E20-H03)
//!
//! ```ignore
//! // lodestar_core::metadata  (módulo NUEVO, funciones puras sobre DocumentSet)
//! pub fn catalog(docs: &DocumentSet) -> MetadataCatalog;
//! pub fn inspect_field(docs: &DocumentSet, field: &FieldPath) -> FieldInspection;
//!
//! // lodestar_core::types  (la FORMA de los tipos de retorno = contrato de wire de E20-H03)
//! pub struct MetadataCatalog { pub fields: Vec<FieldStats> }
//! pub struct FieldStats { pub field: FieldPath, pub present_in: usize,
//!                         pub inferred_types: BTreeMap<ValueType, usize> }
//! pub struct FieldInspection { pub field: FieldPath, pub present_in: usize, pub missing_in: usize,
//!                              pub inferred_types: BTreeMap<ValueType, usize>,
//!                              pub values: Vec<ValueCount> }
//! pub struct ValueCount { pub value: serde_yaml::Value, pub count: usize }
//! ```
//!
//! ## Decisiones de criterio (autor de tests, documentadas y clavadas por los asserts)
//!
//! - **El catálogo INCLUYE los mapas intermedios** (`service` además de `service.name`/
//!   `service.tier`), reflejando [`walk`] 1:1. Justificación: `walk` ES la definición de «qué es un
//!   campo», y el store v2 (E18) indexa exactamente lo que `walk` emite; si el catálogo omitiera los
//!   mapas intermedios, catálogo y store discreparían sobre el conjunto de campos (invariante #3). Y
//!   `service` es direccionable (`get(service)` → el mapa; `has(service)` es contestable), así que
//!   es un campo consultable legítimo. Lo clava `catalogo_paths_anidados`.
//! - **`inferred_types` se teclea por [`ValueType`]** (no por su nombre de wire en `String`): una
//!   sola verdad de tipo. El mapeo a `"string"`/`"number"` en minúscula (`§Fase 6`) es serde, y se
//!   difiere a E20-H03 igual que `Expression` difirió el suyo a E19-H03.
//! - **`values` cuenta SOLO escalares**: un valor lista u objeto cuenta en `present_in` y su tipo en
//!   `inferred_types`, pero no aparece en `values`. Lo clava `inspecciona_anidado` (un `service.tier`
//!   que a veces es lista).
//! - **Orden de `values` determinista**: conteo desc, y a igual conteo, por el TEXTO del valor
//!   ascendente. Lo clava `inspecciona_valores_frecuentes` con un empate deliberado (`draft` y
//!   `review`, ambos 21) que solo el desempate por valor resuelve.

use std::collections::BTreeMap;

use lodestar_core::metadata::{catalog, inspect_field};
use lodestar_core::types::{
    FieldPath, FieldStats, MetadataCatalog, RelPath, ValueCount, ValueType,
};
use lodestar_core::DocumentSet;
use serde_yaml::Value as Yaml;

// --- Utilidades --------------------------------------------------------------

/// `RelPath` para rutas obviamente válidas (invariante #6: nunca un string crudo).
fn rp(p: &str) -> RelPath {
    RelPath::new(p).unwrap_or_else(|e| panic!("`{p}` debe ser un RelPath válido: {e:?}"))
}

/// `FieldPath` desde dot-notation.
fn fp(s: &str) -> FieldPath {
    FieldPath::parse(s).unwrap_or_else(|e| panic!("`{s}` debe ser un FieldPath válido: {e:?}"))
}

/// Un documento mínimo con `yaml` como frontmatter (sin delimitadores) y un cuerpo trivial. El YAML
/// llega al modelo real, para que los tipos se parseen como en producción.
fn fm_doc(yaml: &str) -> String {
    format!("---\n{yaml}\n---\n\n# doc\n")
}

/// Construye un [`DocumentSet`] a partir de pares `(ruta, contenido)`.
fn ds(docs: Vec<(String, String)>) -> DocumentSet {
    let mut files: BTreeMap<RelPath, String> = BTreeMap::new();
    for (p, raw) in docs {
        files.insert(rp(&p), raw);
    }
    DocumentSet::from_files(files)
}

/// `n` documentos con `status: <status>`, rutas únicas por prefijo.
fn statuses(status: &str, n: usize) -> Vec<(String, String)> {
    (0..n)
        .map(|i| (format!("{status}-{i}.md"), fm_doc(&format!("status: {status}"))))
        .collect()
}

/// La entrada del catálogo para `field`, o panic con la lista de campos presentes.
fn stats<'a>(cat: &'a MetadataCatalog, field: &str) -> &'a FieldStats {
    let target = fp(field);
    cat.fields.iter().find(|e| e.field == target).unwrap_or_else(|| {
        let listados: Vec<String> = cat.fields.iter().map(|e| e.field.to_string()).collect();
        panic!("el catálogo debe listar `{field}`; lista {listados:?}");
    })
}

/// Los nombres de campo del catálogo, en su orden real (para clavar el determinismo del orden).
fn field_names(cat: &MetadataCatalog) -> Vec<String> {
    cat.fields.iter().map(|e| e.field.to_string()).collect()
}

/// Un [`ValueCount`] de un valor string.
fn vstr(s: &str, count: usize) -> ValueCount {
    ValueCount {
        value: Yaml::String(s.to_string()),
        count,
    }
}

// =============================================================================
// E20-H01 — Catálogo de propiedades
// =============================================================================

/// Criterio: 3 documentos con `status` string y 1 con `status` número →
/// `presentIn: 4`, `inferredTypes: {string: 3, number: 1}` (`catalogo_presencia_y_tipos`).
#[test]
fn catalogo_presencia_y_tipos() {
    // 3 docs con status string, 1 con status número, 1 sin status (para que `present_in` no sea el
    // total y el conteo discrimine).
    let docs = vec![
        ("d1.md".to_string(), fm_doc("status: draft")),
        ("d2.md".to_string(), fm_doc("status: accepted")),
        ("d3.md".to_string(), fm_doc("status: review")),
        ("d4.md".to_string(), fm_doc("status: 5")),
        ("d5.md".to_string(), "# Sin frontmatter\n".to_string()),
    ];
    let cat = catalog(&ds(docs));
    let s = stats(&cat, "status");

    assert_eq!(s.present_in, 4, "`status` aparece en 4 de los 5 documentos");
    assert_eq!(
        s.inferred_types.get(&ValueType::String),
        Some(&3),
        "3 documentos tienen `status` string"
    );
    assert_eq!(
        s.inferred_types.get(&ValueType::Number),
        Some(&1),
        "1 documento tiene `status` número (`status: 5`, sin coerción a string)"
    );
    assert_eq!(
        s.inferred_types.len(),
        2,
        "solo se observan dos tipos: string y number"
    );
    // Invariante rector: la suma de los conteos por tipo es exactamente `present_in`.
    assert_eq!(
        s.inferred_types.values().sum::<usize>(),
        s.present_in,
        "sum(inferred_types) == present_in (una observación de tipo por documento presente)"
    );
}

/// Criterio: documentos con `service: {name, tier}` → `service.name` y `service.tier` son campos del
/// catálogo (`catalogo_paths_anidados`).
#[test]
fn catalogo_paths_anidados() {
    // 2 docs con `service: {name, tier}` y NADA más en el frontmatter, para que el conjunto de
    // campos sea exactamente el árbol de `service`.
    let nested = concat!("service:\n", "  name: authentication\n", "  tier: critical");
    let docs = vec![
        ("a.md".to_string(), fm_doc(nested)),
        ("b.md".to_string(), fm_doc(nested)),
    ];
    let cat = catalog(&ds(docs));

    // (1) DECISIÓN de criterio: el catálogo lista el mapa intermedio `service` ADEMÁS de las dos
    //     hojas, reflejando `walk` 1:1, y en orden determinista por `FieldPath`
    //     (`service` < `service.name` < `service.tier`).
    assert_eq!(
        field_names(&cat),
        vec!["service", "service.name", "service.tier"],
        "el catálogo lista el mapa intermedio y las dos hojas anidadas, ordenados por FieldPath"
    );

    // (2) Las hojas anidadas son campos propios con su presencia y su tipo (`§Fase 6`).
    let name = stats(&cat, "service.name");
    assert_eq!(name.present_in, 2, "`service.name` aparece en los 2 documentos");
    assert_eq!(name.inferred_types.get(&ValueType::String), Some(&2));

    let tier = stats(&cat, "service.tier");
    assert_eq!(tier.present_in, 2, "`service.tier` aparece en los 2 documentos");
    assert_eq!(tier.inferred_types.get(&ValueType::String), Some(&2));

    // (3) El mapa intermedio `service` aparece con tipo Mapping: informa al agente de que es un
    //     objeto en el que puede descender.
    let service = stats(&cat, "service");
    assert_eq!(service.present_in, 2);
    assert_eq!(
        service.inferred_types.get(&ValueType::Mapping),
        Some(&2),
        "`service` se clasifica como objeto (Mapping)"
    );
}

/// Criterio: un workspace sin frontmatter en ningún documento → catálogo vacío, sin error
/// (`catalogo_vacio`).
#[test]
fn catalogo_vacio() {
    let docs = vec![
        ("a.md".to_string(), "# A\n\nSolo cuerpo, sin frontmatter.\n".to_string()),
        ("b.md".to_string(), "# B\n\nTampoco tengo frontmatter.\n".to_string()),
    ];
    let cat = catalog(&ds(docs));

    assert!(
        cat.fields.is_empty(),
        "sin frontmatter, el catálogo es vacío (sin error); lista {:?}",
        field_names(&cat)
    );
}

// =============================================================================
// E20-H02 — Inspección de una propiedad
// =============================================================================

/// Criterio: `status` con 21 `draft`, 57 `accepted`, 6 `deprecated` → `values` los lista con su
/// conteo, ordenados (`inspecciona_valores_frecuentes`).
///
/// Se añaden 21 `review` (empatan en conteo con `draft`) para clavar el desempate: a igual conteo,
/// el orden es por el TEXTO del valor ascendente (`draft` antes que `review`). Sin ese desempate el
/// orden sería no determinista (aviso 2).
#[test]
fn inspecciona_valores_frecuentes() {
    let mut docs = Vec::new();
    docs.extend(statuses("accepted", 57));
    docs.extend(statuses("draft", 21));
    docs.extend(statuses("review", 21)); // empata con `draft` en conteo
    docs.extend(statuses("deprecated", 6));

    let insp = inspect_field(&ds(docs), &fp("status"));

    assert_eq!(insp.present_in, 105, "57 + 21 + 21 + 6 documentos tienen `status`");
    assert_eq!(
        insp.inferred_types.get(&ValueType::String),
        Some(&105),
        "todos los `status` son string"
    );

    // Orden determinista: conteo DESC y, en el empate a 21, por valor ASC (`draft` < `review`).
    assert_eq!(
        insp.values,
        vec![
            vstr("accepted", 57),
            vstr("draft", 21),
            vstr("review", 21),
            vstr("deprecated", 6),
        ],
        "`values` va por conteo desc y, a igual conteo, por valor asc: draft antes que review"
    );
}

/// Criterio: `status` presente en 84 de 110 documentos → `presentIn: 84`, `missingIn: 26`
/// (`inspecciona_presencia`).
#[test]
fn inspecciona_presencia() {
    let mut docs = statuses("accepted", 84); // 84 con status
    for i in 0..26 {
        // 26 sin frontmatter → `status` ausente
        docs.push((format!("plain-{i}.md"), format!("# Plain {i}\n")));
    }
    assert_eq!(docs.len(), 110, "el fixture tiene 110 documentos");

    let insp = inspect_field(&ds(docs), &fp("status"));

    assert_eq!(insp.present_in, 84, "`status` aparece en 84 documentos");
    assert_eq!(insp.missing_in, 26, "falta en 26 documentos");
    assert_eq!(
        insp.present_in + insp.missing_in,
        110,
        "present_in + missing_in == nº total de documentos"
    );
    assert_eq!(
        insp.inferred_types.get(&ValueType::String),
        Some(&84),
        "los 84 presentes son string"
    );
}

/// Criterio: `service.tier` se puede inspeccionar sobre el path anidado (`inspecciona_anidado`).
///
/// Clava además que **`values` cuenta solo escalares**: un documento con `service.tier` LISTA cuenta
/// en `present_in` y su tipo en `inferred_types`, pero la lista no aparece entre los valores frecuentes.
#[test]
fn inspecciona_anidado() {
    let critical = concat!("service:\n", "  tier: critical");
    let normal = concat!("service:\n", "  tier: normal");
    let lista = concat!("service:\n", "  tier:\n", "    - x\n", "    - y");
    let docs = vec![
        ("a.md".to_string(), fm_doc(critical)),
        ("b.md".to_string(), fm_doc(critical)),
        ("c.md".to_string(), fm_doc(normal)),
        ("d.md".to_string(), fm_doc(lista)), // `service.tier` es una LISTA
        ("e.md".to_string(), "# Sin service\n".to_string()), // sin `service`
    ];
    let insp = inspect_field(&ds(docs), &fp("service.tier"));

    // (1) Funciona sobre el path anidado: presencia/ausencia correctas.
    assert_eq!(insp.present_in, 4, "4 documentos tienen `service.tier`");
    assert_eq!(insp.missing_in, 1, "`e.md` no tiene `service`");

    // (2) Tipos heterogéneos: 3 string + 1 list — el tipo de la lista SÍ cuenta.
    assert_eq!(insp.inferred_types.get(&ValueType::String), Some(&3));
    assert_eq!(
        insp.inferred_types.get(&ValueType::List),
        Some(&1),
        "la lista cuenta en inferred_types aunque no sea un valor frecuente"
    );

    // (3) `values` cuenta SOLO escalares: `critical`×2 y `normal`×1; la lista queda fuera.
    assert_eq!(
        insp.values,
        vec![vstr("critical", 2), vstr("normal", 1)],
        "`values` lista solo los escalares (critical×2, normal×1); la lista no aparece"
    );
    assert!(
        insp.values.iter().all(|v| v.value.is_string()),
        "ningún ValueCount es una lista u objeto: {:?}",
        insp.values
    );
}
