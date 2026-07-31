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
///
/// # El nombre que se rinde es DIRECCIONABLE (E26-H09)
///
/// El `field` de cada [`FieldStats`] es el texto con el que un agente vuelve a preguntar por ese
/// campo (`metadata_inspect{mode:"field"}`, `where`, `has(...)`), así que tiene que significar en
/// esas superficies **el mismo campo** que aquí. Cuando la clave del usuario colisiona con un
/// namespace reservado (una clave de primer nivel `graph:` o `document:`), el nombre desnudo que
/// emite [`walk`](ParsedFrontmatter::walk) NO cumple: `graph.backlinks` se resuelve contra el
/// **grafo** y `graph.nota` ni siquiera parsea (E24-H07/H08). Por eso esas —y solo esas— se rinden
/// **ancladas** ([`FieldPath::anclado`]): `frontmatter.graph.backlinks`. Lo que cambia es cómo se
/// rinde el nombre, no cómo se construye el path: [`walk`](ParsedFrontmatter::walk) y
/// [`FieldPath::from_segments`] siguen intactos (validar ahí reventaría el catálogo de cualquier
/// documento con una clave `graph`, la restricción vigente desde E24-H07).
///
/// **Dos claves siguen sin ser direccionables, por límites del lenguaje y no del catálogo**: una
/// clave con **punto literal** (`"sonar.projectKey"`), que `walk` emite como un segmento único cuyo
/// `Display` es indistinguible del path anidado —desambiguarla exigiría comillas en el lenguaje de
/// consulta—, y una clave de primer nivel llamada literalmente `frontmatter`, que el evaluador
/// interpreta como el anclaje. Ninguna se ancla aquí: el anclaje no las arreglaría (el `where`
/// seguiría sin resolverlas) y cambiaría su nombre sin ganancia. En el caso límite de un documento
/// que tenga a la vez una clave `graph:` y una clave `frontmatter:` **con** una subclave `graph`,
/// los dos campos rinden al mismo nombre y comparten UNA entrada del catálogo: la ambigüedad es
/// del lenguaje (no hay forma de escribir «la clave literal `frontmatter`»), y desambiguarla es el
/// mismo trabajo de quoting que la clave con punto.
///
/// **Pero la estadística no miente en esa fusión**: `present_in` cuenta **documentos**, así que dos
/// campos del **mismo** documento que rinden al mismo nombre suman **1**, nunca 2 —el conteo no
/// puede exceder el total de documentos—, y la observación de tipo que se registra es la del campo
/// que [`inspect_field`] resolvería (la forma **anclada**, que tiene prioridad), para que catálogo e
/// inspección describan el mismo valor. Lo que se une son **conjuntos de documentos**, no conteos.
pub fn catalog(docs: &DocumentSet) -> MetadataCatalog {
    // (present_in, {ValueType: conteo}) por campo. BTreeMap por FieldPath → orden determinista.
    let mut acc: BTreeMap<FieldPath, (usize, BTreeMap<ValueType, usize>)> = BTreeMap::new();
    for fm in frontmatters(docs) {
        // Los campos de ESTE documento, por su nombre rendido, con el tipo observado y si ese
        // nombre vino del anclaje. Un mapa por documento —y no un incremento directo sobre el
        // acumulador global— es lo que mantiene `present_in` contando DOCUMENTOS cuando dos campos
        // distintos del mismo documento rinden al mismo nombre (la fusión de arriba).
        let mut del_documento: BTreeMap<FieldPath, (ValueType, bool)> = BTreeMap::new();
        for (field, value) in fm.walk() {
            // El anclaje se aplica al ACUMULAR, no al servir, para que `fields` siga saliendo
            // ordenado por el nombre que realmente se publica.
            let anclado = field.es_namespace_reservado();
            let nombre = if anclado { field.anclado() } else { field };
            let tipo = ValueType::of(value);
            // En una colisión dentro del mismo documento gana la forma ANCLADA: es la que
            // `inspect_field` resuelve, así que es su tipo el que describe lo que la inspección
            // va a devolver. `walk` no repite un mismo path dentro de un documento, de modo que
            // esta es la ÚNICA colisión posible.
            let gana = match del_documento.get(&nombre) {
                None => true,
                Some((_, previo_anclado)) => anclado && !previo_anclado,
            };
            if gana {
                del_documento.insert(nombre, (tipo, anclado));
            }
        }
        for (nombre, (tipo, _)) in del_documento {
            let (present_in, tipos) = acc.entry(nombre).or_default();
            *present_in += 1;
            *tipos.entry(tipo).or_insert(0) += 1;
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
/// `values` es el **vocabulario** del campo. Un valor escalar (`null`/bool/número/string) aporta su
/// propio valor; un valor **lista aporta cada uno de sus elementos escalares** (E23-H11), que es lo
/// que hace útil `metadata_inspect` sobre un `tags: [a, b]` — hasta E23-H11 devolvía `values: []` y
/// era imposible sacar el vocabulario de tags de una base. Un valor mapa no aporta nada (sus hojas
/// ya son campos propios del catálogo, con su field path). Un `null` presente es un escalar y **sí**
/// aparece —distinto de la ausencia, que no llega a `present_in`—.
///
/// **Consecuencia contraintuitiva y deliberada**: `present_in` cuenta **documentos**, así que un
/// documento con `tags: [a, b, c]` suma 1 a `present_in` pero 3 al vocabulario. La suma de los
/// `count` de `values` **puede superar** `present_in` en un campo multivalor. Un elemento repetido
/// dentro de la misma lista (`tags: [a, a]`) cuenta 2: `values` mide observaciones, no documentos.
/// La explosión **no es recursiva**: un elemento que a su vez sea lista u objeto se ignora.
///
/// # Orden de `values` (determinista)
/// Por conteo **descendente** y, a igual conteo, por el **texto** del valor **ascendente** (el
/// render de `scalar_text`: el número `2` y el string `"2"` rinden ambos a `"2"`; el `null` se
/// ordena bajo `"null"`). Un tercer desempate por [`ValueType`] cierra el no-determinismo
/// latente cuando dos valores **distintos** rinden al mismo texto con el mismo conteo (el número `2`
/// antes que el string `"2"`, por `Number` < `String`): ningún test lo fija, pero deja el orden
/// **total** y reproducible.
///
/// # Paths ANCLADOS (E26-H09)
///
/// Un `field` anclado (`frontmatter.graph.backlinks`) se resuelve contra la clave del **usuario**,
/// igual que hace el evaluador de consultas desde E24-H08: el prefijo se recorta con
/// `FieldPath::sin_anclaje` antes de consultar el frontmatter. Sin esto, el catálogo anunciaría
/// (desde esta misma historia) un nombre que su propia tool no sabría inspeccionar. El `field` que
/// viaja en la respuesta es el que se **pidió** —con su anclaje—, para que el round-trip
/// catálogo → inspección sea estable texto a texto.
pub fn inspect_field(docs: &DocumentSet, field: &FieldPath) -> FieldInspection {
    // La clave real que se busca en cada frontmatter: la desanclada si el path venía anclado, el
    // propio path si no.
    let real = field.sin_anclaje();
    let real = real.as_ref().unwrap_or(field);
    let total = docs.files().len();
    let mut present_in = 0usize;
    let mut inferred_types: BTreeMap<ValueType, usize> = BTreeMap::new();
    // Conteo por valor escalar. `serde_yaml::Value` es `Hash + Eq` (no `Ord`), así que se agrupa en
    // un HashMap y se ordena al final con un comparador total explícito.
    let mut conteos: HashMap<serde_yaml::Value, usize> = HashMap::new();

    for fm in frontmatters(docs) {
        let Some(value) = fm.get(real) else {
            continue;
        };
        present_in += 1;
        let tipo = ValueType::of(value);
        *inferred_types.entry(tipo).or_insert(0) += 1;
        match value {
            // E23-H11: una LISTA aporta cada uno de sus elementos escalares al vocabulario. Sin
            // esto, `metadata_inspect(field: "tags")` devolvía `values: []` sobre cualquier base de
            // notas —el caso de uso número uno de una KB— y la tool incumplía su propósito
            // declarado: «descubrir qué campos usa una base desconocida **y qué valores toma**».
            //
            // No es recursivo: un elemento que a su vez sea lista u objeto se ignora. Explotar en
            // profundidad convertiría el vocabulario en una mezcla de niveles distintos, y las hojas
            // de un objeto ya son campos propios del catálogo por su field path (`tags.x`).
            serde_yaml::Value::Sequence(items) => {
                for item in items {
                    if es_escalar(ValueType::of(item)) {
                        *conteos.entry(item.clone()).or_insert(0) += 1;
                    }
                }
            }
            // Un escalar aporta su propio valor; un mapa no aporta nada (sí su tipo, arriba).
            _ if es_escalar(tipo) => {
                *conteos.entry(value.clone()).or_insert(0) += 1;
            }
            _ => {}
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
