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
//! - **`values` cuenta escalares**: un valor objeto no aparece en `values` (sus hojas son campos
//!   propios, direccionables por su `FieldPath`). **CORREGIDO en E23-H11**: una LISTA sí aporta —se
//!   cuentan sus elementos escalares uno a uno— porque sin eso es imposible obtener el vocabulario
//!   de `tags` de una base de notas. Ver la sección E23-H11 al final del fichero.
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
        .map(|i| {
            (
                format!("{status}-{i}.md"),
                fm_doc(&format!("status: {status}")),
            )
        })
        .collect()
}

/// La entrada del catálogo para `field`, o panic con la lista de campos presentes.
fn stats<'a>(cat: &'a MetadataCatalog, field: &str) -> &'a FieldStats {
    let target = fp(field);
    cat.fields
        .iter()
        .find(|e| e.field == target)
        .unwrap_or_else(|| {
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
    assert_eq!(
        name.present_in, 2,
        "`service.name` aparece en los 2 documentos"
    );
    assert_eq!(name.inferred_types.get(&ValueType::String), Some(&2));

    let tier = stats(&cat, "service.tier");
    assert_eq!(
        tier.present_in, 2,
        "`service.tier` aparece en los 2 documentos"
    );
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
        (
            "a.md".to_string(),
            "# A\n\nSolo cuerpo, sin frontmatter.\n".to_string(),
        ),
        (
            "b.md".to_string(),
            "# B\n\nTampoco tengo frontmatter.\n".to_string(),
        ),
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

    assert_eq!(
        insp.present_in, 105,
        "57 + 21 + 21 + 6 documentos tienen `status`"
    );
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
/// Clava además cómo entra un `service.tier` LISTA: cuenta en `present_in` (un documento) y su tipo
/// en `inferred_types` (`list`), y **sus elementos escalares se cuentan uno a uno en `values`**
/// (E23-H11 — antes la lista no aportaba ningún valor, lo que dejaba `values` vacío en cualquier
/// campo multivalor).
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

    // (3) `values` cuenta escalares: `critical`×2, `normal`×1 y —desde E23-H11— los dos elementos
    //     de la lista de `d.md`, cada uno con su conteo propio. Orden: conteo desc y, en el triple
    //     empate a 1, por texto asc (`normal` < `x` < `y`).
    assert_eq!(
        insp.values,
        vec![
            vstr("critical", 2),
            vstr("normal", 1),
            vstr("x", 1),
            vstr("y", 1)
        ],
        "`values` lista los escalares directos (critical×2, normal×1) y los ELEMENTOS escalares de \
         la lista de `d.md` (x, y): la lista se explota, no se descarta (E23-H11)"
    );
    assert!(
        insp.values.iter().all(|v| v.value.is_string()),
        "ningún ValueCount es una lista u objeto: lo que entra son sus elementos escalares, no el \
         contenedor: {:?}",
        insp.values
    );
}

// =============================================================================
// E23-H11 — El vocabulario de un campo MULTIVALOR (fase roja)
// =============================================================================
//
// ## El defecto (reproducido con el binario real, no deducido leyendo código)
//
//     metadata_inspect(mode:"field", field:"tags")
//     → {"field":"tags","inferredTypes":{"list":3},"missingIn":2,"presentIn":3,"values":[]}
//
// `values` VACÍO. `metadata::inspect_field` solo cuenta valores ESCALARES (`es_escalar` excluye
// `List`/`Mapping`), así que un campo cuyo valor es una lista suma en `present_in` y en
// `inferred_types` pero no aporta ni un valor. Consecuencia: es imposible obtener el vocabulario de
// tags de una base de notas —el caso de uso número uno de una KB— con la tool cuyo propósito
// declarado es «descubrir qué campos usa una base desconocida y qué valores toma»
// (`ARCHITECTURE.md §20.10`).
//
// ## El contrato que fija esta fase roja
//
// **Los elementos ESCALARES de un valor lista se cuentan individualmente**: un documento con
// `tags: [a, b]` aporta 1 a `a` y 1 a `b`. Sus cuatro consecuencias, todas aseveradas abajo porque
// son el contrato y una de ellas es contraintuitiva:
//
// 1. **`present_in` sigue contando DOCUMENTOS**, no elementos: un documento con 5 tags suma 1, no 5.
//    Es lo que hace comparables `present_in`/`missing_in` (`present_in + missing_in == nº de
//    documentos`, invariante ya fijado por `inspecciona_presencia`) y lo que mantiene el invariante
//    `sum(inferred_types) == present_in` (una observación de TIPO por documento presente).
//    **Por tanto la suma de los `count` de `values` PUEDE superar `present_in`** — lo correcto para
//    un campo multivalor, pero hay que dejarlo clavado para que nadie lo «arregle» después.
// 2. **`inferred_types` sigue diciendo `list`**: el tipo del campo es lista, no string. Explotar es
//    una decisión sobre `values` (el vocabulario), no sobre la inferencia de tipos.
// 3. **Un elemento NO escalar dentro de la lista** (una lista anidada, un objeto) **se ignora** en
//    `values`: `values` es un vocabulario de valores, y un contenedor no es un valor de vocabulario.
//    Tampoco se explota recursivamente — eso multiplicaría el mismo problema un nivel más abajo.
// 4. **Los mapas NO se explotan** (control): las hojas de un objeto ya son campos propios,
//    direccionables por su `FieldPath` (`service.tier`), y `walk`/`catalog` ya las listan. Explotar
//    un mapa duplicaría esa verdad con un segundo camino (invariante #3).
//
// Y una decisión de criterio propia del autor de tests, que la historia no fija:
//
// 5. **Un elemento repetido dentro de la MISMA lista cuenta dos veces** (`tags: [a, a]` → `a`×2).
//    `values` cuenta OBSERVACIONES de un valor, no documentos que lo contienen —esa es justamente la
//    columna `present_in`—, así que deduplicar por documento introduciría una tercera semántica de
//    conteo en la misma respuesta. `tags: [a, a]` es un error del autor de la nota, y el conteo fiel
//    es lo que se lo enseña.
//
// El **orden determinista** ya documentado por `inspect_field` (conteo desc → texto asc →
// `ValueType`) se conserva íntegro: los elementos explotados entran en el mismo comparador.

/// `n` documentos cuyo `tags` es la lista dada (rutas únicas por prefijo).
fn con_tags(prefijo: &str, tags: &[&str], n: usize) -> Vec<(String, String)> {
    let lista = tags
        .iter()
        .map(|t| format!("  - {t}\n"))
        .collect::<String>();
    (0..n)
        .map(|i| {
            (
                format!("{prefijo}-{i}.md"),
                fm_doc(format!("tags:\n{lista}").trim_end()),
            )
        })
        .collect()
}

/// Criterio E23-H11: **Dado** un workspace con `tags: [a, b]` en 3 documentos, **Cuando** se pide
/// `metadata_inspect(field:"tags")`, **Entonces** `values` trae `a` y `b` con sus frecuencias
/// (`metadata_inspect_explota_listas`).
///
/// El fixture reproduce el síntoma medido con el binario: 3 documentos con `tags` y 2 sin él, o sea
/// `presentIn: 3`, `missingIn: 2`, `inferredTypes: {list: 3}` — exactamente la respuesta de hoy,
/// salvo que hoy `values` sale **vacío**.
#[test]
fn metadata_inspect_explota_listas() {
    let docs = vec![
        (
            "d1.md".to_string(),
            fm_doc("tags:\n  - diseño\n  - backend"),
        ),
        ("d2.md".to_string(), fm_doc("tags:\n  - backend")),
        (
            "d3.md".to_string(),
            fm_doc("tags:\n  - diseño\n  - backend\n  - urgente"),
        ),
        ("d4.md".to_string(), fm_doc("status: draft")), // frontmatter SIN tags
        ("d5.md".to_string(), "# Sin frontmatter\n".to_string()),
    ];
    let insp = inspect_field(&ds(docs), &fp("tags"));

    // (1) El vocabulario: cada elemento escalar con su frecuencia, en el orden determinista de
    //     `inspect_field` (conteo desc → texto asc).
    assert_eq!(
        insp.values,
        vec![vstr("backend", 3), vstr("diseño", 2), vstr("urgente", 1)],
        "`values` debe traer el VOCABULARIO de tags con sus frecuencias (backend×3, diseño×2, \
         urgente×1); hoy sale vacío porque `inspect_field` solo cuenta escalares y descarta la \
         lista entera"
    );

    // (2) `present_in`/`missing_in` NO cambian: cuentan documentos, no elementos.
    assert_eq!(
        insp.present_in, 3,
        "`tags` está en 3 documentos: un documento con 3 tags sigue sumando 1"
    );
    assert_eq!(
        insp.missing_in, 2,
        "los 2 documentos sin `tags` (uno con otro frontmatter, otro sin bloque) siguen faltando"
    );

    // (3) `inferred_types` sigue diciendo `list`: el TIPO del campo es lista.
    assert_eq!(
        insp.inferred_types.get(&ValueType::List),
        Some(&3),
        "los 3 documentos observan el tipo `list`: explotar la lista no reclasifica el campo como \
         string"
    );
    assert_eq!(
        insp.inferred_types.len(),
        1,
        "solo se observa un tipo (`list`): {:?}",
        insp.inferred_types
    );
    assert_eq!(
        insp.inferred_types.values().sum::<usize>(),
        insp.present_in,
        "se conserva `sum(inferred_types) == present_in`: una observación de TIPO por documento"
    );

    // (4) La consecuencia contraintuitiva, clavada a propósito: la suma de los conteos de `values`
    //     supera `present_in`. Es lo correcto en un campo multivalor y no debe «arreglarse».
    let total: usize = insp.values.iter().map(|v| v.count).sum();
    assert_eq!(
        total, 6,
        "6 observaciones de tag repartidas en 3 documentos: {:?}",
        insp.values
    );
    assert!(
        total > insp.present_in,
        "sum(values.count) ({total}) PUEDE superar present_in ({}): `values` cuenta observaciones \
         de valor y `present_in` cuenta documentos",
        insp.present_in
    );
}

/// Un campo que a veces es escalar y a veces lista (lo más común en una KB real escrita a mano):
/// las dos formas alimentan el MISMO vocabulario, y el orden determinista se mantiene con empates.
#[test]
fn metadata_inspect_lista_y_escalar_alimentan_el_mismo_vocabulario() {
    let docs = vec![
        ("a.md".to_string(), fm_doc("tags: backend")), // escalar
        ("b.md".to_string(), fm_doc("tags: backend")), // escalar
        ("c.md".to_string(), fm_doc("tags:\n  - backend\n  - diseño")),
        (
            "d.md".to_string(),
            fm_doc("tags:\n  - diseño\n  - zeta\n  - alfa"),
        ),
    ];
    let insp = inspect_field(&ds(docs), &fp("tags"));

    assert_eq!(
        insp.values,
        vec![
            vstr("backend", 3), // 2 escalares + 1 elemento de lista
            vstr("diseño", 2),  // 2 elementos de lista
            vstr("alfa", 1),    // empate a 1 resuelto por texto asc: alfa < zeta
            vstr("zeta", 1),
        ],
        "un `tags` escalar y un `tags` lista cuentan en el mismo vocabulario, y el empate a 1 se \
         desempata por el texto del valor (alfa antes que zeta)"
    );
    assert_eq!(insp.present_in, 4, "`tags` está en los 4 documentos");
    assert_eq!(
        insp.inferred_types.get(&ValueType::String),
        Some(&2),
        "2 documentos observan `string` (el campo es escalar en ellos)"
    );
    assert_eq!(
        insp.inferred_types.get(&ValueType::List),
        Some(&2),
        "y 2 observan `list`: la heterogeneidad del campo se sigue reportando"
    );
    assert_eq!(
        insp.inferred_types.values().sum::<usize>(),
        insp.present_in,
        "sum(inferred_types) == present_in también con tipos mezclados"
    );
}

/// Un elemento NO escalar dentro de la lista (lista anidada u objeto) se ignora; un `null` de la
/// lista, en cambio, es un escalar y **sí** cuenta —igual que un `null` de primer nivel, que ya
/// entraba en `values`—.
#[test]
fn metadata_inspect_ignora_elementos_no_escalares_de_la_lista() {
    let docs = vec![
        (
            "a.md".to_string(),
            fm_doc(concat!(
                "tags:\n",
                "  - suelto\n",
                "  - [anidada, dentro]\n",
                "  - {clave: valor}"
            )),
        ),
        ("b.md".to_string(), fm_doc("tags:\n  - ~\n  - suelto")),
    ];
    let insp = inspect_field(&ds(docs), &fp("tags"));

    assert_eq!(
        insp.values,
        vec![
            vstr("suelto", 2),
            ValueCount {
                value: Yaml::Null,
                count: 1
            },
        ],
        "solo entran los elementos ESCALARES: `suelto`×2 y el `null` de `b.md`. La lista anidada y \
         el objeto no son valores de vocabulario, y no se explotan recursivamente"
    );
    for prohibido in ["anidada", "dentro", "valor", "clave"] {
        assert!(
            !insp
                .values
                .iter()
                .any(|v| v.value == Yaml::String(prohibido.to_string())),
            "`{prohibido}` vive dentro de un contenedor anidado: explotar es UN nivel, no \
             recursivo: {:?}",
            insp.values
        );
    }
    assert_eq!(
        insp.present_in, 2,
        "los 2 documentos tienen `tags` (los contenedores anidados no cambian la presencia)"
    );
    assert_eq!(
        insp.inferred_types.get(&ValueType::List),
        Some(&2),
        "ambos observan el tipo `list`"
    );
}

/// Control del alcance: explotar es cosa de LISTAS. Un valor objeto no aporta valores —sus hojas ya
/// son campos propios del catálogo (`service.name`), y duplicarlas aquí sería una segunda verdad—.
#[test]
fn metadata_inspect_no_explota_mapas() {
    let nested = concat!("service:\n", "  name: authentication\n", "  tier: critical");
    let docs = vec![
        ("a.md".to_string(), fm_doc(nested)),
        ("b.md".to_string(), fm_doc(nested)),
    ];
    let insp = inspect_field(&ds(docs), &fp("service"));

    assert!(
        insp.values.is_empty(),
        "un campo objeto no aporta valores: sus hojas son campos propios (`service.name`, \
         `service.tier`), no vocabulario de `service`: {:?}",
        insp.values
    );
    assert_eq!(insp.present_in, 2, "`service` está en los 2 documentos");
    assert_eq!(
        insp.inferred_types.get(&ValueType::Mapping),
        Some(&2),
        "y se sigue clasificando como objeto"
    );

    // Y sus hojas siguen siendo inspeccionables por su propio path (la vía correcta).
    let hoja = inspect_field(
        &ds(vec![("a.md".to_string(), fm_doc(nested))]),
        &fp("service.name"),
    );
    assert_eq!(
        hoja.values,
        vec![vstr("authentication", 1)],
        "la hoja del objeto se inspecciona por su `FieldPath`, que es donde vive su vocabulario"
    );
}

/// Decisión 5: un valor repetido dentro de la misma lista cuenta dos veces (`values` cuenta
/// observaciones, `present_in` cuenta documentos).
#[test]
fn metadata_inspect_cuenta_repeticiones_dentro_de_la_lista() {
    let docs = con_tags("dup", &["a", "a", "b"], 1);
    let insp = inspect_field(&ds(docs), &fp("tags"));

    assert_eq!(
        insp.values,
        vec![vstr("a", 2), vstr("b", 1)],
        "`tags: [a, a, b]` en UN documento cuenta `a`×2: `values` cuenta observaciones del valor, \
         no documentos que lo contienen (para eso está `present_in`)"
    );
    assert_eq!(
        insp.present_in, 1,
        "…y el documento sigue siendo uno solo (`present_in` no se ve afectado)"
    );
}

/// Regresión: un campo ESCALAR normal no cambia de comportamiento. Su marca distintiva —la que lo
/// separa de un multivalor— es que la suma de los conteos de `values` es exactamente `present_in`.
#[test]
fn metadata_inspect_escalar_no_cambia() {
    let mut docs = statuses("accepted", 2);
    docs.extend(statuses("draft", 1));
    docs.push(("plain.md".to_string(), "# Sin frontmatter\n".to_string()));

    let insp = inspect_field(&ds(docs), &fp("status"));

    assert_eq!(
        insp.values,
        vec![vstr("accepted", 2), vstr("draft", 1)],
        "un `status: accepted` escalar sigue contando exactamente como antes"
    );
    assert_eq!(insp.present_in, 3, "3 documentos tienen `status`");
    assert_eq!(insp.missing_in, 1, "el cuarto no tiene frontmatter");
    assert_eq!(
        insp.values.iter().map(|v| v.count).sum::<usize>(),
        insp.present_in,
        "en un campo ESCALAR sum(values.count) == present_in: cada documento aporta exactamente un \
         valor (es justo lo que deja de cumplirse en un campo multivalor)"
    );
    assert_eq!(
        insp.inferred_types.get(&ValueType::String),
        Some(&3),
        "y el tipo observado sigue siendo `string`"
    );
}

/// Vocabulario de tags a escala: el orden determinista se mantiene con conteos grandes salidos de
/// listas, y `values` no deduplica documentos ni recorta la cola.
#[test]
fn metadata_inspect_vocabulario_ordenado_por_frecuencia() {
    let mut docs = Vec::new();
    docs.extend(con_tags("api", &["backend", "api"], 30)); // backend 30, api 30
    docs.extend(con_tags("ui", &["frontend"], 12)); // frontend 12
    docs.extend(con_tags("nota", &["backend"], 5)); // backend +5 → 35

    let insp = inspect_field(&ds(docs), &fp("tags"));

    assert_eq!(
        insp.values,
        vec![vstr("backend", 35), vstr("api", 30), vstr("frontend", 12),],
        "el vocabulario sale por frecuencia descendente, sumando los elementos de todas las listas"
    );
    assert_eq!(insp.present_in, 47, "30 + 12 + 5 documentos tienen `tags`");
    assert_eq!(
        insp.values.iter().map(|v| v.count).sum::<usize>(),
        77,
        "77 observaciones de tag en 47 documentos: el multivalor rompe la igualdad con present_in"
    );
}

// =============================================================================
// E26-H09 — El catálogo es DIRECCIONABLE y el anclaje `frontmatter.` llega al core
// =============================================================================
//
// La mitad de la historia que se ve por el wire (`metadata_inspect` normalizando su `field` con
// `parse::build_field_path` y el rechazo del namespace reservado) vive en
// `crates/lodestar-mcp/tests/mcp.rs` y en `descubribilidad.rs`. Lo que se fija AQUÍ es la mitad que
// vive en el core, y que es la premisa de aquella:
//
//   1. **Cómo se RINDE el nombre en el catálogo** (`catalog`). Un `name` del catálogo tiene que ser
//      un texto que el lenguaje de consulta acepte y resuelva al MISMO campo. Cuando la clave del
//      usuario colisiona con un namespace reservado (`graph:`, `document:`), el nombre desnudo que
//      hoy emite `walk` (`graph.backlinks`) NO cumple: `where`/`has` lo resuelven contra el GRAFO
//      (E24-H07/H08) y `graph.nota` ni siquiera parsea. La forma direccionable de esa clave es la
//      **anclada** (`frontmatter.graph.backlinks`).
//   2. **Que `inspect_field` entienda ese mismo path anclado**, porque si no el catálogo anunciaría
//      un nombre que su propia tool no sabe inspeccionar — el defecto que la historia cierra, del
//      revés.
//
// Se prueba en el core, y no solo por el wire, por el invariante #3 (*una sola verdad computada*):
// el catálogo lo computa `core::metadata`, y si la fachada reescribiera los nombres al servirlos,
// el catálogo del core y el del wire dirían cosas distintas sobre el mismo workspace. La ALCANCE de
// la historia lo dice explícitamente: no se toca `FieldPath::from_segments` ni `walk` —la
// restricción de E24-H07 sigue vigente—; lo que cambia es **cómo se rinde** el nombre.
//
// NO se prueba aquí la clave con **punto literal** (`"sonar.projectKey"` como clave única del
// mapa): `walk` la emite como un `FieldPath` de UN segmento con punto, y su `Display` es
// indistinguible del path anidado `sonar → projectKey`. Hacerla direccionable exigiría una sintaxis
// de escape en el lenguaje de consulta, que está fuera del alcance declarado de esta historia.

/// Frontmatter que colisiona con los DOS namespaces reservados y, a la vez, tiene claves que **no**
/// deben cambiar de nombre:
///   · `graph:`/`document:` de primer nivel → colisión real (los nombres que hoy no son
///     direccionables);
///   · `meta.graph.x` → un `graph` que NO es primer segmento, así que ya es direccionable tal cual
///     (control: quien «ancle todo» lo rompe);
///   · `status` → una clave normal (mismo control).
const FM_COLISION: &str = concat!(
    "status: draft\n",
    "graph:\n",
    "  backlinks: 7\n",
    "  nota: manual\n",
    "document:\n",
    "  path: falso.md\n",
    "meta:\n",
    "  graph:\n",
    "    x: 1"
);

/// Un workspace con el frontmatter en colisión + un documento sin frontmatter (para que
/// `present_in`/`missing_in` discriminen).
fn docs_colision() -> Vec<(String, String)> {
    vec![
        ("alfa.md".to_string(), fm_doc(FM_COLISION)),
        (
            "bravo.md".to_string(),
            "# Bravo\n\nsin frontmatter.\n".to_string(),
        ),
    ]
}

/// **E26-H09** — el catálogo rinde ANCLADA la clave que colisiona con un namespace reservado, y
/// **solo** esa.
///
/// Hoy emite `graph.backlinks`/`graph.nota`/`document.path` (los nombres crudos de `walk`), que son
/// justo los tres textos que el lenguaje de consulta interpreta como propiedades calculadas —o
/// rechaza—: la tool que existe para hacer descubrible una base desconocida anuncia nombres que
/// ninguna otra superficie acepta.
#[test]
fn catalogo_rinde_anclado_el_nombre_en_colision() {
    let cat = catalog(&ds(docs_colision()));
    let nombres = field_names(&cat);

    for anclado in [
        "frontmatter.graph",
        "frontmatter.graph.backlinks",
        "frontmatter.graph.nota",
        "frontmatter.document",
        "frontmatter.document.path",
    ] {
        assert!(
            nombres.iter().any(|n| n == anclado),
            "el catálogo debe anunciar «{anclado}»: es la ÚNICA forma con la que un agente puede \
             después inspeccionar o consultar esa clave (`where`/`has` resuelven el texto desnudo \
             contra el grafo, E24-H07/H08). Nombres emitidos: {nombres:?}"
        );
    }

    for desnudo in [
        "graph",
        "graph.backlinks",
        "graph.nota",
        "document",
        "document.path",
    ] {
        assert!(
            !nombres.iter().any(|n| n == desnudo),
            "…y NO debe anunciar «{desnudo}» a secas: es un nombre no direccionable (o significa \
             otra cosa —el grafo— en el resto de la superficie). Nombres emitidos: {nombres:?}"
        );
    }

    // Control anti-vacuo 1: lo que YA era direccionable no cambia de texto. La consecuencia que la
    // historia declara es que cambian los nombres «para las claves que colisionan con un
    // namespace», no todos.
    for intacto in ["status", "meta", "meta.graph", "meta.graph.x"] {
        assert!(
            nombres.iter().any(|n| n == intacto),
            "«{intacto}» ya es direccionable tal cual (su primer segmento no es un namespace \
             reservado): anclarlo también sería cambiar el contrato sin motivo. Nombres emitidos: \
             {nombres:?}"
        );
    }

    // Control anti-vacuo 2: la ESTADÍSTICA no se toca; lo que cambia es cómo se rinde el nombre.
    let backlinks = stats_por_nombre(&cat, "frontmatter.graph.backlinks");
    assert_eq!(
        backlinks.present_in, 1,
        "la clave del usuario sigue estando en 1 de los 2 documentos: {backlinks:?}"
    );
    assert_eq!(
        backlinks.inferred_types.get(&ValueType::Number),
        Some(&1),
        "…y su tipo observado sigue siendo `number` (el 7 del frontmatter): {backlinks:?}"
    );
}

/// **E26-H09** — el catálogo es direccionable **desde el propio core**: cada `FieldPath` que emite
/// `catalog` se puede pasar a `inspect_field` y devuelve la misma presencia.
///
/// Es la mitad de la propiedad de round-trip que `descubribilidad.rs` verifica por el wire (allí se
/// añade la tercera pata: `knowledge_search{where}`). Hoy pasa —los nombres desnudos los resuelve
/// `ParsedFrontmatter::get` sin más—; el día que el catálogo rinda anclados los nombres en colisión
/// **exige** que `inspect_field` entienda ese anclaje, que es la premisa del criterio
/// `anclaje_frontmatter_alcanza_la_clave_reservada`.
#[test]
fn cada_nombre_del_catalogo_se_inspecciona() {
    let docs = ds(docs_colision());
    let cat = catalog(&docs);
    assert!(
        cat.fields.len() >= 8,
        "el fixture debe producir un catálogo rico (≥ 8 campos) o el bucle sería vacuo: {:?}",
        field_names(&cat)
    );

    for entrada in &cat.fields {
        let insp = inspect_field(&docs, &entrada.field);
        assert_eq!(
            insp.present_in, entrada.present_in,
            "el nombre «{}» que anuncia el catálogo debe inspeccionarse tal cual y describir el \
             MISMO campo (present_in del catálogo vs. de la inspección)",
            entrada.field
        );
        assert_eq!(
            insp.inferred_types, entrada.inferred_types,
            "…con los mismos tipos observados, para «{}»",
            entrada.field
        );
    }
}

/// **E26-H09** — `inspect_field` resuelve un [`FieldPath`] **anclado** contra el frontmatter del
/// usuario, igual que hace el evaluador de consultas desde E24-H08.
///
/// Hoy busca una clave de primer nivel literalmente llamada `frontmatter` y devuelve `present_in:
/// 0`: silenciosamente equivocado. El path se construye con `FieldPath::from_segments` (público) y
/// no con el normalizador del parser, para no acoplar este test a un símbolo que la historia aún
/// tiene que promover a `pub`.
#[test]
fn inspect_field_alcanza_la_clave_reservada_por_el_anclaje() {
    let docs = ds(docs_colision());
    let anclado = FieldPath::from_segments(["frontmatter", "graph", "backlinks"])
        .expect("un path anclado de 3 segmentos es válido");

    let insp = inspect_field(&docs, &anclado);

    assert_eq!(
        insp.present_in, 1,
        "`frontmatter.graph.backlinks` debe alcanzar la clave del USUARIO (el 7 de `alfa.md`), no \
         una clave de primer nivel llamada literalmente `frontmatter`: {insp:?}"
    );
    assert_eq!(
        insp.missing_in, 1,
        "…y el documento sin frontmatter sigue contando como ausencia: {insp:?}"
    );
    assert_eq!(
        insp.values,
        vec![ValueCount {
            value: serde_yaml::from_str::<Yaml>("7").unwrap(),
            count: 1
        }],
        "…con el valor 7 en su tipo YAML real (número), que es el dato que hoy es inalcanzable: \
         {insp:?}"
    );

    // Control anti-vacuo: el anclaje no es un comodín que haga aparecer cualquier cosa. Una clave
    // inexistente bajo el mismo anclaje sigue ausente.
    let fantasma = FieldPath::from_segments(["frontmatter", "graph", "inventada"]).unwrap();
    assert_eq!(
        inspect_field(&docs, &fantasma).present_in,
        0,
        "una subclave que no existe sigue sin estar presente"
    );
}

/// La entrada del catálogo cuyo `name` (el `Display` del `FieldPath`) es `nombre`, o panic con la
/// lista. Complementa a [`stats`], que busca por `FieldPath::parse` y por tanto no puede pedir un
/// nombre anclado (`parse` no aplica la abreviatura: construiría `["frontmatter","graph",…]` sí,
/// pero por una vía distinta de la del lenguaje).
fn stats_por_nombre<'a>(cat: &'a MetadataCatalog, nombre: &str) -> &'a FieldStats {
    cat.fields
        .iter()
        .find(|e| e.field.to_string() == nombre)
        .unwrap_or_else(|| {
            panic!(
                "el catálogo debe listar «{nombre}»; lista {:?}",
                field_names(cat)
            )
        })
}

/// **E26-H09** — cuando dos campos de un MISMO documento rinden al mismo nombre, la estadística no
/// se infla: `present_in` cuenta **documentos**.
///
/// El caso límite lo destapó la revisión de la historia: un documento con una clave `graph:` (que se
/// rinde anclada, `frontmatter.graph`) **y** una clave de primer nivel llamada literalmente
/// `frontmatter` con una subclave `graph` (que se rinde tal cual, y da el mismo texto). Los dos
/// caen en la misma entrada del catálogo. Sumar sus observaciones daría `presentIn: 2` sobre **un**
/// documento —un conteo que excede el total y que ninguna otra tool podría reproducir—: la
/// ambigüedad del lenguaje es tolerable, una estadística imposible no.
///
/// La segunda mitad es la coherencia con la inspección: en la fusión gana la forma **anclada**,
/// porque es la que `inspect_field` resuelve, así que catálogo e inspección describen el mismo
/// valor. Aquí es observable sin fragilidad porque los dos campos tienen **tipos distintos**: el
/// `graph:` del usuario es un mapa y la subclave de la clave literal es un string.
#[test]
fn la_fusion_no_infla_present_in() {
    let docs = ds(vec![(
        "alfa.md".to_string(),
        fm_doc(concat!(
            "graph:\n",
            "  backlinks: 7\n",
            "frontmatter:\n",
            "  graph: solo-un-string"
        )),
    )]);

    let cat = catalog(&docs);
    let fusionada = stats_por_nombre(&cat, "frontmatter.graph");

    assert_eq!(
        fusionada.present_in, 1,
        "los dos campos que rinden a «frontmatter.graph» viven en el MISMO documento: la entrada \
         cuenta 1, nunca 2. `present_in` cuenta documentos, y el workspace tiene uno solo: {cat:?}"
    );
    assert_eq!(
        fusionada.inferred_types.values().sum::<usize>(),
        1,
        "…y una sola observación de tipo, por el mismo motivo (el invariante \
         `sum(inferred_types) == present_in` sigue en pie): {fusionada:?}"
    );
    assert_eq!(
        fusionada.inferred_types.get(&ValueType::Mapping),
        Some(&1),
        "…y esa observación es la de la forma ANCLADA —el `graph:` del usuario, que es un mapa—, \
         porque es la que `inspect_field` resuelve; registrar el string de la clave literal haría \
         que el catálogo describiera un valor distinto del que devuelve la inspección: {fusionada:?}"
    );

    // La coherencia, aseverada de verdad y no por lectura del código: el mismo nombre, inspeccionado.
    let insp = inspect_field(
        &docs,
        &FieldPath::from_segments(["frontmatter", "graph"]).unwrap(),
    );
    assert_eq!(
        insp.present_in, fusionada.present_in,
        "catálogo e inspección deben decir lo mismo sobre «frontmatter.graph»: {insp:?}"
    );
    assert_eq!(
        insp.inferred_types, fusionada.inferred_types,
        "…también en el tipo observado: {insp:?}"
    );
    assert_eq!(
        insp.missing_in, 0,
        "…y `present_in + missing_in` sigue siendo el total de documentos (1): {insp:?}"
    );

    // Control anti-vacuo: la fusión no se come nada. Las dos hojas siguen anunciadas, cada una con
    // su nombre, y el catálogo no se queda en una sola entrada.
    let nombres = field_names(&cat);
    for esperado in [
        "frontmatter",
        "frontmatter.graph",
        "frontmatter.graph.backlinks",
    ] {
        assert!(
            nombres.iter().any(|n| n == esperado),
            "«{esperado}» debe seguir en el catálogo: fusionar dos nombres iguales no puede \
             hacer desaparecer campos distintos: {nombres:?}"
        );
    }
}
