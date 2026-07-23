//! Tests del **modelo documental genérico** (épica E16, `ARCHITECTURE.md §20.4`).
//!
//! Fase ROJA de **E16-H01** — `ParsedFrontmatter`: el frontmatter deja de tener forma conocida
//! (los 7 campos tipados de `types::Frontmatter` + `KNOWN_FM` + la coerción `js_string`) y pasa a
//! ser **metadata arbitraria del usuario**, conservada con su tipo YAML real y su texto original.
//!
//! Vive en un fichero propio y no en `core.rs` por tres motivos:
//!   1. `core.rs` (2661 líneas) es la suite de la era OKF y el implementador va a tener que
//!      migrarla durante E16; mezclar aquí la spec nueva multiplica el conflicto en su diff.
//!   2. Estos tests **no compilan** hasta que exista `ParsedFrontmatter`. Si vivieran en `core.rs`,
//!      el fallo de compilación tumbaría los ~60 tests verdes de ese target, violando la regla de
//!      que lo existente sigue compilando. Aislados, el rojo queda confinado a este binario.
//!   3. E16-H02..H05 aportan 16 tests más de la misma familia (documento genérico): este es su
//!      hogar natural, igual que E15 abrió `discovery.rs`/`config.rs` en `lodestar-workspace`.
//!
//! ---
//!
//! ## API que fija esta fase roja
//!
//! ```ignore
//! // lodestar_core::types
//!
//! /// El frontmatter es metadata ARBITRARIA. Sin campos conocidos, sin lista cerrada, sin
//! /// conversión de tipos, sin borrado de claves desconocidas.
//! pub struct ParsedFrontmatter {
//!     /// Siempre un `Mapping` (vacío si el bloque está vacío): `get` y el catálogo de E20
//!     /// necesitan una forma uniforme. La ausencia de frontmatter se modela con `Option`, no
//!     /// con `Value::Null`.
//!     pub value: serde_yaml::Value,
//!     /// Texto YAML EXACTO del bloque, **sin** los delimitadores `---`.
//!     pub raw: String,
//!     /// Rango de BYTES que ocupa `raw` dentro del raw del documento, de modo que
//!     /// `documento[span] == raw`. Excluye los delimitadores: el patch quirúrgico de E16-H04
//!     /// sustituye exactamente ese rango, y `§20.9` deriva de él el rango de diagnóstico.
//!     pub span: std::ops::Range<usize>,
//! }
//!
//! impl ParsedFrontmatter {
//!     /// LA única verdad de acceso a metadata (invariante #3): la reutilizan E18 (indexado),
//!     /// E19 (query) y E20 (`metadata_inspect`). Nadie vuelve a navegar el `Value` a mano.
//!     pub fn get(&self, path: &FieldPath) -> Option<&serde_yaml::Value>;
//! }
//!
//! /// Ruta a una propiedad del frontmatter: una secuencia NO vacía de segmentos ya resueltos.
//! /// Newtype validado (mismo patrón que `RelPath`), no un `String` crudo: la dot-notation es
//! /// una *sintaxis de entrada*, no la identidad del campo — por eso hay dos constructores.
//! pub struct FieldPath(/* Vec<String> */);
//!
//! impl FieldPath {
//!     /// Desde dot-notation (`"service.tier"`, `"release.target.date"`). Es lo que usan la
//!     /// consulta textual de E19 y el `"field"` del filtro JSON. Falla con path vacío o
//!     /// segmento vacío.
//!     pub fn parse(s: &str) -> Result<FieldPath, /* error Debug */>;
//!     /// Desde segmentos explícitos: la vía para direccionar una clave YAML **que contiene un
//!     /// punto**, que `parse` partiría. Falla con lista vacía o segmento vacío.
//!     pub fn from_segments<I, S>(segments: I) -> Result<FieldPath, /* error Debug */>
//!     where I: IntoIterator<Item = S>, S: Into<String>;
//!     pub fn segments(&self) -> &[String];
//! }
//!
//! // lodestar_core::model
//! pub struct Parsed { /* … */ pub frontmatter: Option<ParsedFrontmatter>, /* … */ }
//! pub fn build_raw(fm: Option<&ParsedFrontmatter>, body: &str) -> String;
//! ```
//!
//! `Option<ParsedFrontmatter>` es lo que distingue **sin frontmatter** (`None`) de **frontmatter
//! vacío** (`Some` con `value` = mapping vacío): son dos estados válidos y distintos, no dos
//! formas de lo mismo.

use std::collections::BTreeSet;

use lodestar_core::model;
use lodestar_core::types::{FieldPath, ParsedFrontmatter};
use serde_yaml::Value as Yaml;

// --- Utilidades ---------------------------------------------------------------

/// `FieldPath` desde dot-notation, para los casos en que el path es obviamente válido.
fn fp(s: &str) -> FieldPath {
    FieldPath::parse(s).unwrap_or_else(|e| panic!("`{s}` debe ser un FieldPath válido: {e:?}"))
}

/// `FieldPath` de un único segmento literal (no se parte por puntos).
fn fp_literal(s: &str) -> FieldPath {
    FieldPath::from_segments([s])
        .unwrap_or_else(|e| panic!("`{s}` debe ser un segmento válido: {e:?}"))
}

/// Claves de primer nivel del frontmatter, como conjunto (deliberadamente SIN orden: preservar
/// el orden de aparición es E16-H04, aquí solo se juzga que no se borra nada).
fn claves(pf: &ParsedFrontmatter) -> BTreeSet<String> {
    pf.value
        .as_mapping()
        .expect("`ParsedFrontmatter::value` debe ser un Mapping")
        .keys()
        .map(|k| {
            k.as_str()
                .expect("las claves del frontmatter de estos tests son escalares string")
                .to_string()
        })
        .collect()
}

/// Invariante del `span`: es el rango de bytes que ocupa `raw` DENTRO del raw del documento, sin
/// incluir los delimitadores `---`. Es lo que permite a E16-H04 sustituir el bloque in situ.
fn assert_span_coherente(doc_raw: &str, pf: &ParsedFrontmatter) {
    assert!(
        pf.span.end <= doc_raw.len() && pf.span.start <= pf.span.end,
        "span fuera del documento: {:?} sobre {} bytes",
        pf.span,
        doc_raw.len()
    );
    assert_eq!(
        &doc_raw[pf.span.clone()],
        pf.raw.as_str(),
        "`span` debe ser el rango de bytes que ocupa `raw` dentro del documento"
    );
    assert!(
        doc_raw[..pf.span.start].ends_with("---\n"),
        "`span` empieza justo DESPUÉS del delimitador de apertura (no lo incluye); antes de él hay {:?}",
        doc_raw[..pf.span.start].chars().rev().take(8).collect::<String>()
    );
    let cola = &doc_raw[pf.span.end..];
    assert!(
        cola.starts_with("---") || cola.starts_with("\n---") || cola.starts_with("\r\n---"),
        "`span` termina justo ANTES del delimitador de cierre (no lo incluye); tras él viene {:?}",
        cola.chars().take(8).collect::<String>()
    );
}

// --- E16-H01: ParsedFrontmatter ----------------------------------------------

/// Criterio 1: un `.md` sin frontmatter → `frontmatter` es `None`, el body es el fichero entero
/// y no se emite ningún diagnóstico.
///
/// El "ningún diagnóstico" se juzga **en el parseo** (`fm_err`), que es lo que E16-H01 controla:
/// la ausencia de frontmatter deja de ser una condición de error del modelo. La retirada de
/// `OKF-FM01` del catálogo de `conform` es E16-H05 (`sin_frontmatter_no_diagnostica`); hasta
/// entonces `conform` puede seguir emitiéndolo derivándolo de `frontmatter.is_none()`.
#[test]
fn sin_frontmatter_es_valido() {
    let raw = "# Rotación de tokens\n\nUn documento sin una sola línea de frontmatter.\n\n\
               Con [un enlace](otro.md) y un `---` que no abre nada.\n";
    let parsed = model::parse_file("docs/rotacion.md", raw);

    assert!(
        parsed.frontmatter.is_none(),
        "un documento sin frontmatter no tiene `ParsedFrontmatter`: es `None`, no un bloque vacío"
    );
    assert_eq!(
        parsed.body, raw,
        "sin frontmatter, el cuerpo es el fichero ENTERO byte a byte"
    );
    assert!(
        parsed.fm_err.is_none(),
        "la ausencia de frontmatter es válida: ya no es `FmError::Missing` ni ninguna otra \
         condición de error del parseo"
    );
}

/// Criterio 2: un `.md` con `---\n---\n` → frontmatter presente y **vacío**, distinguible del
/// caso anterior.
#[test]
fn frontmatter_vacio_es_valido() {
    let raw = "---\n---\n\n# Sin metadata\n\nCuerpo.\n";
    let parsed = model::parse_file("docs/vacio.md", raw);

    assert!(
        parsed.fm_err.is_none(),
        "`---\\n---\\n` es un bloque vacío perfectamente cerrado, no un frontmatter sin cierre"
    );
    let pf = parsed
        .frontmatter
        .as_ref()
        .expect("un bloque `---\\n---\\n` está PRESENTE (y vacío): no puede colapsar a `None`");

    let mapa = pf
        .value
        .as_mapping()
        .expect("el `value` de un frontmatter vacío es un Mapping vacío, no `Null`");
    assert!(
        mapa.is_empty(),
        "el frontmatter vacío no tiene claves, pero tiene {} entradas",
        mapa.len()
    );
    assert!(
        pf.get(&fp("status")).is_none(),
        "ninguna consulta acierta sobre un frontmatter vacío"
    );
    assert_eq!(
        pf.raw, "",
        "el texto YAML del bloque vacío es la cadena vacía"
    );
    assert_eq!(
        pf.span,
        4..4,
        "el span es el hueco entre delimitadores: el rango vacío justo tras `---\\n`"
    );
    assert_span_coherente(raw, pf);

    // Distinguible del caso anterior: `Some(vacío)` vs `None`.
    let sin = model::parse_file("docs/sin.md", "# Sin metadata\n\nCuerpo.\n");
    assert!(
        sin.frontmatter.is_none() && parsed.frontmatter.is_some(),
        "«sin frontmatter» y «frontmatter vacío» deben ser dos estados DISTINTOS del modelo"
    );

    // Misma clase, escrito con una línea en blanco dentro: sigue siendo un bloque presente y
    // vacío (hoy el modelo lo reporta como frontmatter ausente). Aquí no se fija el span exacto:
    // que el `\n` interior cuente como contenido o como parte del cierre es indiferente.
    let raw_blanco = "---\n\n---\n\n# Sin metadata\n";
    let con_blanco = model::parse_file("docs/blanco.md", raw_blanco);
    let pf_blanco = con_blanco
        .frontmatter
        .as_ref()
        .expect("`---\\n\\n---\\n` también es un frontmatter presente y vacío");
    assert!(
        pf_blanco
            .value
            .as_mapping()
            .is_some_and(serde_yaml::Mapping::is_empty),
        "un bloque solo con espacio en blanco es un frontmatter vacío"
    );
    assert!(
        pf_blanco.raw.trim().is_empty(),
        "el texto del bloque en blanco no tiene contenido: {:?}",
        pf_blanco.raw
    );
    assert_span_coherente(raw_blanco, pf_blanco);
}

/// Frontmatter con los siete casos del criterio 3. Usa deliberadamente nombres de los antiguos
/// `KNOWN_FM` (`type`, `status`, `title`, `description`) con valores NO string: son exactamente
/// los que hoy pasan por `js_string` y pierden el tipo.
///
/// **`concat!` a propósito, una línea YAML por literal.** NO usar la continuación de línea de Rust
/// (`\` al final): se come el salto Y **toda la indentación** de la línea siguiente, con lo que las
/// estructuras anidadas llegan aplanadas al parser (`  name: auth` → `name: auth`, clave hermana) y
/// las listas de objetos ni siquiera son YAML válido. Aquí la indentación va DENTRO de las comillas.
const FM_TIPOS: &str = concat!(
    "---\n",
    "type: 2\n",
    "status: true\n",
    "title: Autenticación\n",
    "description:\n",
    "priority: 2\n",
    "owners:\n",
    "  - platform\n",
    "  - security\n",
    "service:\n",
    "  name: auth\n",
    "  tier: critical\n",
    "approvals:\n",
    "  - who: ana\n",
    "    ok: true\n",
    "  - who: luis\n",
    "    ok: false\n",
    "---\n",
    "\n",
    "# Autenticación\n",
);

/// Criterio 3: string, número, booleano, `null`, lista, objeto anidado y lista de objetos
/// conservan su **tipo YAML real**. Se asierta sobre el TIPO, nunca sobre el valor renderizado.
#[test]
fn preserva_tipos_yaml() {
    let parsed = model::parse_file("docs/auth.md", FM_TIPOS);
    let pf = parsed
        .frontmatter
        .as_ref()
        .expect("el documento tiene frontmatter");
    assert_span_coherente(FM_TIPOS, pf);

    // --- Número: un `2` sigue siendo número, en una clave antes «conocida» y en una nueva.
    for clave in ["type", "priority"] {
        let v = pf
            .get(&fp(clave))
            .unwrap_or_else(|| panic!("falta la clave `{clave}`"));
        assert!(
            matches!(v, Yaml::Number(_)),
            "`{clave}: 2` debe conservar el tipo número YAML; llegó {v:?}"
        );
        assert_eq!(
            v.as_i64(),
            Some(2),
            "`{clave}` debe valer el entero 2, no su renderizado"
        );
        assert_ne!(
            v,
            &Yaml::String("2".to_string()),
            "`{clave}` NO puede coercerse a string (era la paridad `String(v)` de `js_string`)"
        );
    }

    // --- Booleano.
    let status = pf.get(&fp("status")).expect("falta la clave `status`");
    assert_eq!(
        status,
        &Yaml::Bool(true),
        "`status: true` debe conservar el tipo booleano YAML; llegó {status:?}"
    );
    assert_ne!(
        status,
        &Yaml::String("true".to_string()),
        "`status` NO puede coercerse a string"
    );

    // --- String (el caso que ya funcionaba: sigue funcionando).
    assert_eq!(
        pf.get(&fp("title")),
        Some(&Yaml::String("Autenticación".to_string())),
        "un string sigue siendo un string"
    );

    // --- `null` explícito: clave PRESENTE con valor nulo, distinta de clave ausente.
    assert_eq!(
        pf.get(&fp("description")),
        Some(&Yaml::Null),
        "`description:` es una clave presente con valor `null`, no una ausencia"
    );
    assert_eq!(
        pf.get(&fp("no_existe")),
        None,
        "una clave que no está devuelve `None` (así se distingue de la que está a `null`)"
    );

    // --- Lista de escalares.
    let owners = pf.get(&fp("owners")).expect("falta la clave `owners`");
    assert_eq!(
        owners,
        &Yaml::Sequence(vec![
            Yaml::String("platform".to_string()),
            Yaml::String("security".to_string()),
        ]),
        "`owners` debe seguir siendo una secuencia YAML, no un `platform,security` unido"
    );

    // --- Objeto anidado.
    let service = pf.get(&fp("service")).expect("falta la clave `service`");
    assert!(
        matches!(service, Yaml::Mapping(_)),
        "`service` debe conservar el tipo mapping; llegó {service:?}"
    );
    assert_ne!(
        service,
        &Yaml::String("[object Object]".to_string()),
        "un objeto no se aplana a texto"
    );

    // --- Lista de objetos.
    let approvals = pf
        .get(&fp("approvals"))
        .and_then(Yaml::as_sequence)
        .expect("`approvals` debe ser una secuencia");
    assert_eq!(approvals.len(), 2, "`approvals` tiene 2 elementos");
    let primero = approvals[0]
        .as_mapping()
        .expect("cada elemento de `approvals` es un objeto");
    assert_eq!(
        primero.get("who"),
        Some(&Yaml::String("ana".to_string())),
        "el objeto de la lista conserva sus claves"
    );
    assert_eq!(
        primero.get("ok"),
        Some(&Yaml::Bool(true)),
        "el tipo se conserva también DENTRO de una lista de objetos"
    );
}

/// Criterio 4: `service.tier` → `critical`; `service.ausente` → `None`.
#[test]
fn dot_notation() {
    // `concat!` con una línea YAML por literal: la indentación va dentro de las comillas (ver la
    // nota de `FM_TIPOS` sobre la continuación de línea de Rust).
    let raw = concat!(
        "---\n",
        "service: {name: auth, tier: critical}\n",
        "release:\n",
        "  target:\n",
        "    date: \"2026-07-23\"\n",
        "---\n",
        "\n",
        "# Servicio\n",
    );
    let parsed = model::parse_file("docs/servicio.md", raw);
    let pf = parsed
        .frontmatter
        .as_ref()
        .expect("el documento tiene frontmatter");

    assert_eq!(
        pf.get(&fp("service.tier")),
        Some(&Yaml::String("critical".to_string())),
        "`service.tier` desciende por el mapa hasta el valor anidado"
    );
    assert_eq!(
        pf.get(&fp("service.name")),
        Some(&Yaml::String("auth".to_string())),
        "`service.name` desciende por el mapa hasta el valor anidado"
    );
    assert_eq!(
        pf.get(&fp("service.ausente")),
        None,
        "una clave que no existe dentro de un mapa existente devuelve `None`"
    );
    assert_eq!(
        pf.get(&fp("ausente.tier")),
        None,
        "descender por un mapa que no existe devuelve `None`, no revienta"
    );
    assert_eq!(
        pf.get(&fp("service.tier.loquesea")),
        None,
        "descender por un escalar es ausencia, no error"
    );
    assert_eq!(
        pf.get(&fp("release.target.date")),
        Some(&Yaml::String("2026-07-23".to_string())),
        "la dot-notation soporta más de dos niveles (lo exige `metadata_inspect` de E20)"
    );
    assert!(
        pf.get(&fp("service"))
            .and_then(Yaml::as_mapping)
            .is_some_and(|m| m.len() == 2),
        "un path de un solo segmento devuelve la clave de primer nivel entera"
    );

    // El `FieldPath` es una secuencia de segmentos, no un string con puntos: una clave YAML
    // PUEDE contener un punto y debe seguir siendo direccionable (lo necesitan el filtro JSON de
    // E19 y el catálogo de E20, que construyen paths sin pasar por la sintaxis textual).
    let raw_punto = concat!(
        "---\n",
        "\"service.tier\": literal\n",
        "service:\n",
        "  tier: anidado\n",
        "---\n",
        "\n",
        "# Punto\n",
    );
    let con_punto = model::parse_file("docs/punto.md", raw_punto);
    let pf_punto = con_punto
        .frontmatter
        .as_ref()
        .expect("el documento tiene frontmatter");
    assert_eq!(
        pf_punto.get(&fp("service.tier")),
        Some(&Yaml::String("anidado".to_string())),
        "la dot-notation SIEMPRE desciende: nunca resuelve a la clave literal con punto"
    );
    assert_eq!(
        pf_punto.get(&fp_literal("service.tier")),
        Some(&Yaml::String("literal".to_string())),
        "un segmento literal direcciona la clave que contiene el punto"
    );
    assert_eq!(
        fp("service.tier").segments(),
        ["service".to_string(), "tier".to_string()],
        "`parse` parte por puntos"
    );
    assert_eq!(
        fp_literal("service.tier").segments(),
        ["service.tier".to_string()],
        "`from_segments` NO parte por puntos"
    );

    // Un path sin segmentos no designa ningún campo: se rechaza en la construcción (E19 lo
    // recibe de texto de usuario, así que el error debe ser un dato, no un panic).
    assert!(
        FieldPath::parse("").is_err(),
        "un path vacío no es un campo válido"
    );
    assert!(
        FieldPath::parse("service.").is_err(),
        "un segmento vacío no es una clave válida"
    );
}

/// Frontmatter íntegramente compuesto por claves que Lodestar nunca ha visto, incluidos los tres
/// valores que el `dump_frontmatter` actual descarta o filtra.
const FM_DESCONOCIDAS: &str = concat!(
    "---\n",
    "owners: [platform, security]\n",
    "sla_minutes: 15\n",
    "deprecated_field: null\n",
    "nota_vacia: \"\"\n",
    "sin_duenos: []\n",
    "sonar.projectKey: lodestar\n",
    "nested:\n",
    "  vendor:\n",
    "    id: 42\n",
    "---\n",
    "\n",
    "# Doc\n",
    "\n",
    "Cuerpo.\n",
);

/// Criterio 5: un frontmatter con claves desconocidas sobrevive intacto a parse + serialize sin
/// patch.
///
/// Se juzga el CONJUNTO de claves y el valor de cada una — **no** su orden ni su formato: que la
/// reconstrucción preserve el orden de aparición es E16-H04 (`patch_preserva_orden_y_claves`).
#[test]
fn no_borra_desconocidas() {
    let esperadas: BTreeSet<String> = [
        "owners",
        "sla_minutes",
        "deprecated_field",
        "nota_vacia",
        "sin_duenos",
        "sonar.projectKey",
        "nested",
    ]
    .into_iter()
    .map(String::from)
    .collect();

    let parsed = model::parse_file("docs/desconocidas.md", FM_DESCONOCIDAS);
    let pf = parsed
        .frontmatter
        .as_ref()
        .expect("el documento tiene frontmatter");
    assert_eq!(
        claves(pf),
        esperadas,
        "el PARSEO no puede perder ninguna clave desconocida"
    );

    let salida = model::build_raw(Some(pf), &parsed.body);
    let reparsed = model::parse_file("docs/desconocidas.md", &salida);
    let re = reparsed
        .frontmatter
        .as_ref()
        .unwrap_or_else(|| panic!("el documento reconstruido debe tener frontmatter:\n{salida}"));
    assert_eq!(
        claves(re),
        esperadas,
        "la SERIALIZACIÓN no puede perder ninguna clave desconocida:\n{salida}"
    );

    // Cada valor, intacto y con su tipo.
    for clave in [
        "owners",
        "sla_minutes",
        "deprecated_field",
        "nota_vacia",
        "sin_duenos",
        "sonar.projectKey",
    ] {
        let path = fp_literal(clave);
        assert_eq!(
            re.get(&path),
            pf.get(&path),
            "la clave `{clave}` no sobrevive intacta al round-trip:\n{salida}"
        );
    }
    assert_eq!(
        re.get(&fp("nested.vendor.id")),
        pf.get(&fp("nested.vendor.id")),
        "el valor anidado no sobrevive intacto al round-trip:\n{salida}"
    );

    // Los tres valores que el filtrado heredado del prototipo borraba en silencio.
    assert_eq!(
        re.get(&fp_literal("nota_vacia")),
        Some(&Yaml::String(String::new())),
        "la cadena vacía es un VALOR del usuario, no una ausencia:\n{salida}"
    );
    assert_eq!(
        re.get(&fp_literal("deprecated_field")),
        Some(&Yaml::Null),
        "un `null` explícito es una clave presente:\n{salida}"
    );
    assert_eq!(
        re.get(&fp_literal("sin_duenos")),
        Some(&Yaml::Sequence(Vec::new())),
        "una lista vacía es un valor del usuario:\n{salida}"
    );
}

// =============================================================================
// E16-H02 — Ningún nombre de fichero activa reglas especiales
// =============================================================================
//
// `REFACTOR_PHASE_2 §Principios 3 y 4` («ningún nombre de archivo debe activar reglas
// especiales», «`index.md` no representa una colección»), `§Fase 8 (Eliminar)` y
// `ARCHITECTURE.md §20.4`/`§20.7`.
//
// ## API que fija esta fase roja
//
// ```ignore
// // lodestar_core::model
// /// Ya NO ramifica por basename: `index.md`, `log.md` y `README.md` se parsean como
// /// cualquier otro `.md` (hoy `model.rs:437-446` devuelve `fm: None` + raw entero como body).
// pub fn parse_file(path: &str, raw: &str) -> Parsed;   // `Parsed` SIN campo `kind`
//
// // lodestar_core::types::Analysis
// pub struct Analysis {
//     // …
//     /// Sustituye a `orphans` con la definición de `§20.7`: documentos SIN enlaces internos
//     /// entrantes NI salientes. Es una propiedad consultable, no un diagnóstico.
//     pub isolated: Vec<RelPath>,
//     // SIN `in_index`, SIN `okf_version`.
// }
// // `Backlinks` SIN `index_refs`.
// ```
//
// **Desaparecen** (`§20.4`): `FileKind`, `model::file_kind`, `model::is_reserved`,
// `RelPath::is_reserved`, `RelPath::concept_id`, `Bundle::root_okf_version`, el gating de
// fichero reservado de `query.rs:104-123` y el `is:reserved` de `is_predicate`.
//
// **Lo que estas pruebas NO fijan**: la forma final de `Analysis` (`documents`/`outgoing`/
// `incoming`/`dangling`/`diagnostics` con `ResolvedLink`/`DanglingLink` es E17-H04) ni el
// renombre `concepts` → `documents` (ya hecho en E16-H06). Aquí se usan los nombres vigentes —
// `documents`/`out`/`inn`/`per_file` — porque esta historia solo RETIRA campos.

use lodestar_core::types::{Analysis, FileMap, RelPath};
use lodestar_core::DocumentSet;

/// `RelPath` para rutas obviamente válidas (invariante #6: nunca un string crudo).
fn rp(p: &str) -> RelPath {
    RelPath::new(p).unwrap_or_else(|e| panic!("`{p}` debe ser un RelPath válido: {e:?}"))
}

/// `FileMap` desde pares (ruta, contenido).
fn mapa(pares: &[(&str, &str)]) -> FileMap {
    pares
        .iter()
        .map(|(p, c)| (rp(p), (*c).to_string()))
        .collect()
}

/// Claves de primer nivel del objeto JSON de un tipo del wire.
///
/// Se juzga la AUSENCIA de un campo por su serialización y no por el compilador (que no puede
/// aserir «este campo no existe»): es la única forma de fijar que `in_index`/`okf_version`/
/// `index_refs` se han **retirado** y no meramente ocultado.
fn claves_wire<T: serde::Serialize>(v: &T) -> BTreeSet<String> {
    serde_json::to_value(v)
        .expect("los tipos del wire deben serializar")
        .as_object()
        .expect("el tipo del wire es un objeto JSON")
        .keys()
        .cloned()
        .collect()
}

/// Códigos de diagnóstico emitidos para `p`, **como cadena de wire**.
///
/// Deliberadamente por serialización y no por la variante `CheckCode::Orphan`: E16-H05 borra esa
/// variante y el test debe sobrevivir a su desaparición sin dejar de significar lo mismo.
fn codigos(a: &Analysis, p: &RelPath) -> Vec<String> {
    a.diagnostics
        .get(p)
        .into_iter()
        .flatten()
        .map(|c| {
            serde_json::to_value(c.code)
                .ok()
                .and_then(|v| v.as_str().map(String::from))
                .unwrap_or_else(|| "<no serializable>".to_string())
        })
        .collect()
}

/// Un `index.md` **con frontmatter** que además enlaza a otro documento. Reúne los dos rasgos que
/// hoy reciben trato mágico: el basename reservado y los enlaces «de pertenencia».
const INDICE_CON_FM: &str = concat!(
    "---\n",
    "title: Índice del workspace\n",
    "okf_version: \"1.0\"\n",
    "owners:\n",
    "  - platform\n",
    "---\n",
    "\n",
    "# Índice\n",
    "\n",
    "- [Alfa](alfa.md)\n",
);

/// El cuerpo de `INDICE_CON_FM`: lo que queda tras el delimitador de cierre.
const INDICE_BODY: &str = "\n# Índice\n\n- [Alfa](alfa.md)\n";

const LOG_CON_FM: &str = concat!(
    "---\n",
    "updated: 2026-07-23\n",
    "---\n",
    "\n",
    "- 2026-07-23 — se creó el workspace\n",
);

/// Entrantes (desde `index.md` y desde `gamma.md`), ningún saliente.
const ALFA: &str = "---\nstatus: accepted\n---\n\n# Alfa\n\nSin enlaces salientes.\n";
/// Salientes (a `alfa.md`), ningún entrante.
const GAMMA: &str = "---\nstatus: draft\n---\n\n# Gamma\n\nEnlaza a [Alfa](alfa.md).\n";
/// Ni entrantes ni salientes: el único documento **aislado** del workspace.
const SOLO: &str = "---\nstatus: draft\n---\n\n# Solo\n\nNi entrantes ni salientes.\n";

/// Workspace de los criterios 2, 3, 4 y 5: un índice con frontmatter que enlaza a `alfa.md`, un
/// `gamma.md` que solo tiene salientes y un `solo.md` sin enlaces de ningún tipo.
fn ws_enlaces() -> FileMap {
    mapa(&[
        ("index.md", INDICE_CON_FM),
        ("log.md", LOG_CON_FM),
        ("alfa.md", ALFA),
        ("gamma.md", GAMMA),
        ("solo.md", SOLO),
    ])
}

/// Criterio 1: un `index.md` con frontmatter se parsea como cualquier otro documento.
///
/// Hoy `parse_file` corta por basename (`model.rs:437-446`) y devuelve `frontmatter: None` con el
/// raw entero como cuerpo, de modo que la metadata del índice es invisible para todo el motor.
#[test]
fn index_md_es_documento_normal() {
    let parsed = model::parse_file("index.md", INDICE_CON_FM);

    assert!(
        parsed.fm_err.is_none(),
        "el frontmatter del índice está bien formado"
    );
    let pf = parsed
        .frontmatter
        .as_ref()
        .expect("un `index.md` con frontmatter TIENE frontmatter: el basename no lo suprime");
    assert_eq!(
        pf.get(&fp("title")),
        Some(&Yaml::String("Índice del workspace".to_string())),
        "la metadata del índice se lee como la de cualquier documento"
    );
    assert!(
        matches!(pf.get(&fp("owners")), Some(Yaml::Sequence(_))),
        "y conserva sus tipos YAML: {:?}",
        pf.get(&fp("owners"))
    );
    assert_eq!(
        parsed.body, INDICE_BODY,
        "el cuerpo del índice EXCLUYE el bloque de frontmatter, como en cualquier documento"
    );
    assert_span_coherente(INDICE_CON_FM, pf);

    // Un `log.md` con frontmatter, igual.
    let log = model::parse_file("log.md", LOG_CON_FM);
    assert!(
        log.frontmatter
            .as_ref()
            .is_some_and(|f| f.contains_key("updated")),
        "`log.md` tampoco pierde su frontmatter por llamarse como se llama"
    );
    assert_eq!(
        log.body, "\n- 2026-07-23 — se creó el workspace\n",
        "el cuerpo de `log.md` excluye su frontmatter"
    );

    // La formulación exacta del principio 4: el MISMO contenido bajo CUALQUIER nombre produce
    // exactamente el mismo parseo.
    let referencia = model::parse_file("docs/cualquiera.md", INDICE_CON_FM);
    for nombre in ["index.md", "log.md", "README.md", "AGENTS.md", "a/index.md"] {
        let otro = model::parse_file(nombre, INDICE_CON_FM);
        assert_eq!(
            otro.body, referencia.body,
            "`{nombre}` debe parsearse igual que un documento cualquiera (cuerpo)"
        );
        assert_eq!(
            otro.frontmatter
                .as_ref()
                .map(|f| (&f.value, &f.raw, &f.span)),
            referencia
                .frontmatter
                .as_ref()
                .map(|f| (&f.value, &f.raw, &f.span)),
            "`{nombre}` debe parsearse igual que un documento cualquiera (frontmatter)"
        );
    }
}

/// Criterio 2: un enlace desde `index.md` es una **arista** normal, no una relación de
/// pertenencia.
///
/// Hoy `compute_analysis` (`document_set.rs:57-70`) se salta el índice como origen y vuelca sus enlaces
/// en `in_index`; `backlinks` (`document_set.rs:182-216`) los aparta en `index_refs`.
#[test]
fn enlace_desde_indice_es_arista() {
    let b = DocumentSet::from_files(ws_enlaces());
    let a = b.analyze();
    let index = rp("index.md");
    let alfa = rp("alfa.md");

    assert!(
        a.documents.contains(&index),
        "`index.md` es un documento más del análisis, no un fichero de servicio: {:?}",
        a.documents
    );
    // MIGRADO en E17-H04: `out`/`inn` pasaron a `outgoing`/`incoming`, con el enlace resuelto.
    assert!(
        a.outgoing
            .get(&index)
            .is_some_and(|ls| ls.iter().any(|l| l.target.internal_path() == Some(&alfa))),
        "el enlace del índice a `alfa.md` es una arista SALIENTE de `index.md`: {:?}",
        a.outgoing.get(&index)
    );
    assert!(
        a.incoming
            .get(&alfa)
            .is_some_and(|v| v.iter().any(|r| r.from == index)),
        "y se invierte como cualquier otra: `alfa.md` tiene a `index.md` entre sus entrantes: {:?}",
        a.incoming.get(&alfa)
    );

    // Indistinguible de un enlace desde un documento cualquiera: `index.md` y `gamma.md` entran
    // por la MISMA puerta.
    let bl = b.backlinks(&alfa);
    let entrantes: BTreeSet<&str> = bl.inbound.iter().map(|l| l.from.as_str()).collect();
    assert!(
        entrantes.contains("index.md") && entrantes.contains("gamma.md"),
        "los entrantes de `alfa.md` son `index.md` Y `gamma.md`, sin distinción de origen: {entrantes:?}"
    );
    assert!(
        bl.inbound
            .iter()
            .any(|l| l.from == index && l.link.href == "alfa.md"),
        "el enlace del índice conserva su href como cualquier otro: {:?}",
        bl.inbound
    );

    // La pertenencia determinada por índices desaparece del contrato, no se limita a quedar vacía.
    let claves_analysis = claves_wire(a);
    assert!(
        !claves_analysis.contains("inIndex") && !claves_analysis.contains("in_index"),
        "`Analysis` ya no tiene `in_index`: la pertenencia por índices no existe. Claves: {claves_analysis:?}"
    );
    let claves_backlinks = claves_wire(&bl);
    assert!(
        !claves_backlinks.contains("indexRefs") && !claves_backlinks.contains("index_refs"),
        "`Backlinks` ya no tiene `index_refs`: un índice que te enlaza es un entrante más. Claves: {claves_backlinks:?}"
    );
}

/// Criterio 3: un documento sin entrantes pero **con** salientes NO es aislado (`§20.7`).
#[test]
fn con_salientes_no_es_aislado() {
    let b = DocumentSet::from_files(ws_enlaces());
    let a = b.analyze();

    assert!(
        !a.isolated.contains(&rp("gamma.md")),
        "`gamma.md` no tiene entrantes, pero enlaza a `alfa.md`: NO está aislado. isolated={:?}",
        a.isolated
    );
    assert!(
        !a.isolated.contains(&rp("alfa.md")),
        "`alfa.md` no tiene salientes, pero le entran dos enlaces: NO está aislado. isolated={:?}",
        a.isolated
    );
    assert!(
        !a.isolated.contains(&rp("index.md")),
        "`index.md` enlaza a `alfa.md`: tampoco está aislado (ni recibe trato especial). isolated={:?}",
        a.isolated
    );
    assert!(
        a.isolated.contains(&rp("solo.md")),
        "el contraste: `solo.md` no tiene enlaces de ningún tipo y SÍ está aislado. isolated={:?}",
        a.isolated
    );
    assert!(
        a.isolated.contains(&rp("log.md")),
        "`log.md` tampoco tiene enlaces: se juzga con la misma regla que los demás. isolated={:?}",
        a.isolated
    );
}

/// Criterio 4: un documento sin enlaces de ningún tipo es aislado y **no genera diagnóstico**.
///
/// El «no genera diagnóstico» se juzga a nivel del código `ORPHAN` —el que hoy emite `conform`
/// por esta causa (`conform.rs:204-211`)—, no exigiendo cero diagnósticos: el resto del catálogo
/// OKF cae en E16-H05 y no puede bloquear a esta historia.
#[test]
fn aislado_no_es_error() {
    let b = DocumentSet::from_files(ws_enlaces());
    let a = b.analyze();
    let solo = rp("solo.md");

    assert!(
        a.isolated.contains(&solo),
        "`solo.md` no tiene entrantes ni salientes: está aislado. isolated={:?}",
        a.isolated
    );
    let cs = codigos(a, &solo);
    assert!(
        !cs.iter().any(|c| c == "ORPHAN"),
        "el aislamiento es una PROPIEDAD consultable, no un diagnóstico: {cs:?}"
    );

    // El renombre es un renombre: `orphans` (que además significaba otra cosa —«sin entrantes y
    // no listado en un índice»—) no sobrevive junto a `isolated`.
    let claves = claves_wire(a);
    assert!(
        claves.contains("isolated"),
        "`Analysis` expone `isolated` en el wire. Claves: {claves:?}"
    );
    assert!(
        !claves.contains("orphans"),
        "`orphans` no coexiste con `isolated`: es el mismo campo, renombrado y redefinido. Claves: {claves:?}"
    );
}

/// Criterio 5: `okf_version` es metadata consultable normal y no aparece en `Analysis`.
#[test]
fn okf_version_es_metadata_normal() {
    // (a) Como dato del usuario, se lee por el accesor como cualquier otra clave (`§20.13`: se
    //     conserva, deja de ser un concepto del motor).
    let parsed = model::parse_file("index.md", INDICE_CON_FM);
    let pf = parsed
        .frontmatter
        .as_ref()
        .expect("el índice tiene frontmatter");
    assert_eq!(
        pf.get(&fp("okf_version")),
        Some(&Yaml::String("1.0".to_string())),
        "`okf_version` se consulta como cualquier otra clave del frontmatter"
    );
    assert!(
        claves(pf).contains("okf_version"),
        "y sigue ahí entre las demás claves, sin trato aparte: {:?}",
        claves(pf)
    );

    // (b) Como concepto del motor, desaparece: `Analysis` no lo promociona a campo propio.
    let b = DocumentSet::from_files(ws_enlaces());
    let a = b.analyze();
    let cl = claves_wire(a);
    assert!(
        !cl.contains("okfVersion") && !cl.contains("okf_version"),
        "`Analysis` ya no tiene `okf_version`: el motor no lee la versión OKF del índice raíz. \
         Claves: {cl:?}"
    );
}

// =============================================================================
// E16-H03 — Título derivado
// =============================================================================
//
// `ARCHITECTURE.md §20.4` («`frontmatter.title` → primer heading H1 → nombre del fichero. Es
// **solo una heurística de presentación**: `title` no se convierte en propiedad reservada») y
// `REFACTOR_PHASE_2 §Fase 4 (Título derivado)`.
//
// ## API que fija esta fase roja
//
// ```ignore
// // lodestar_core::model
// /// Título presentable de un documento. Función PURA (el core no hace I/O) y total: siempre
// /// devuelve algo, porque el último eslabón de la cadena —el nombre del fichero— existe
// /// siempre. La consumen `DocumentSummary`/`DocumentSummary` y `GraphNode` (E17-H05) y el FTS
// /// del store (E18); recibe las tres piezas por separado —y no un `&Parsed`— para que el store
// /// pueda derivarlo sin re-parsear el documento entero.
// pub fn derived_title(
//     fm: Option<&ParsedFrontmatter>,
//     body: &str,
//     path: &RelPath,
// ) -> String;
// ```
//
// **Desaparece** `model::title_from_path` (Title Case con el quirk del `\b` de JS: `año.md` →
// `AñO`), y con ella el test de paridad `title_from_path_boundaries_como_js` de `core.rs:585`.

/// Deriva el título de un documento a partir de su texto crudo, que es como llegan siempre los
/// documentos: se parsea y se pasan las tres piezas a `derived_title`.
fn titulo(path: &str, raw: &str) -> String {
    let parsed = model::parse_file(path, raw);
    model::derived_title(parsed.frontmatter.as_ref(), &parsed.body, &rp(path))
}

/// Criterio 1: con `title` en el frontmatter y un H1 distinto, gana el del frontmatter.
#[test]
fn titulo_frontmatter_gana() {
    let raw = concat!(
        "---\n",
        "title: Autenticación\n",
        "status: accepted\n",
        "---\n",
        "\n",
        "# Rotación de tokens\n",
        "\n",
        "Cuerpo.\n",
    );
    assert_eq!(
        titulo("docs/auth.md", raw),
        "Autenticación",
        "el primer eslabón de la cadena es `frontmatter.title`"
    );

    // Se toma TAL CUAL: sin Title Case, sin recortes, sin reescrituras.
    let literal = concat!(
        "---\n",
        "title: rotación de tokens (v2)\n",
        "---\n",
        "\n",
        "# Otro\n",
    );
    assert_eq!(
        titulo("docs/auth.md", literal),
        "rotación de tokens (v2)",
        "el título del frontmatter se usa literalmente: no es un slug ni se capitaliza"
    );

    // Un `title` vacío no es un título presentable: la cadena continúa. (Es la semántica que ya
    // tiene `DocumentSet::list_documents` con su `.filter(|s| !s.is_empty())`, `document_set.rs:160`.)
    let vacio = concat!(
        "---\n",
        "title: \"\"\n",
        "---\n",
        "\n",
        "# Rotación de tokens\n",
    );
    assert_eq!(
        titulo("docs/auth.md", vacio),
        "Rotación de tokens",
        "`title: \"\"` no es un título: se cae al siguiente eslabón de la cadena"
    );
}

/// Criterio 2: sin `title`, gana el **primer H1** del cuerpo.
#[test]
fn titulo_del_h1() {
    // Con frontmatter, pero sin `title`.
    let raw = concat!(
        "---\n",
        "status: draft\n",
        "---\n",
        "\n",
        "# Rotación de tokens\n",
        "\n",
        "## Detalle\n",
    );
    assert_eq!(
        titulo("docs/rotacion.md", raw),
        "Rotación de tokens",
        "sin `title`, el título es el primer H1 del cuerpo"
    );

    // Sin frontmatter en absoluto: el cuerpo es el fichero entero y el H1 sigue encontrándose.
    assert_eq!(
        titulo("docs/rotacion.md", "# Rotación de tokens\n\nCuerpo.\n"),
        "Rotación de tokens",
        "un documento sin frontmatter también tiene H1"
    );

    // **H1**, no «primer heading»: un `##` previo no es un título de documento.
    let con_h2_delante = concat!(
        "## Contexto\n",
        "\n",
        "Texto.\n",
        "\n",
        "# Rotación de tokens\n",
        "\n",
        "### Detalle\n",
    );
    assert_eq!(
        titulo("docs/rotacion.md", con_h2_delante),
        "Rotación de tokens",
        "la cadena dice H1: un `##` que aparece antes no gana"
    );

    // El texto del heading llega recortado y sin las almohadillas.
    assert_eq!(
        titulo("docs/x.md", "#    Rotación de tokens   \n"),
        "Rotación de tokens",
        "el título es el TEXTO del heading, sin `#` ni espacios de relleno"
    );
}

/// Criterio 3: un `#` dentro de un bloque de código no es un H1.
///
/// `model::parse_headings` (`model.rs:536`) ya reconoce los fences ` ``` `: esta es la razón de
/// reutilizarlo en vez de reimplementar la detección de headings.
#[test]
fn h1_en_fence_no_cuenta() {
    let raw = concat!(
        "Texto introductorio.\n",
        "\n",
        "```md\n",
        "# No soy un título\n",
        "```\n",
        "\n",
        "# Sí soy el título\n",
    );
    assert_eq!(
        titulo("docs/ejemplo.md", raw),
        "Sí soy el título",
        "el `#` de dentro del fence es contenido del bloque de código, no un heading"
    );

    // Si el ÚNICO `#` del documento vive dentro de un fence, NO hay H1: la cadena sigue hasta el
    // nombre del fichero (es lo que distingue reconocer fences de limitarse a ignorarlos).
    let solo_fence = concat!(
        "```sh\n",
        "# instala las dependencias\n",
        "npm ci\n",
        "```\n",
        "\n",
        "Fin.\n",
    );
    assert_eq!(
        titulo("docs/instalacion.md", solo_fence),
        "instalacion",
        "un comentario de shell dentro de un fence no puede convertirse en el título del documento"
    );
}

/// Criterio 4: sin `title` ni H1, el título es el **nombre del fichero tal cual**, sin `.md`.
#[test]
fn titulo_del_nombre_de_fichero() {
    let cuerpo = "Un documento sin metadata y sin encabezados.\n";
    assert_eq!(
        titulo("docs/decisions/auth-tokens.md", cuerpo),
        "auth-tokens",
        "el último eslabón es el NOMBRE del fichero: sin directorios, sin `.md`, sin retoques"
    );
    assert_ne!(
        titulo("docs/decisions/auth-tokens.md", cuerpo),
        "Auth Tokens",
        "el Title Case de `title_from_path` era paridad con el prototipo, ya retirado"
    );

    // El quirk del `\b` de JS (`año.md` → `AñO`, `foo.bar.md` → `Foo.Bar`) se va con él.
    assert_eq!(
        titulo("año_fiscal.md", cuerpo),
        "año_fiscal",
        "ni se capitaliza ni se sustituyen `-`/`_` por espacios"
    );
    assert_eq!(
        titulo("docs/foo.bar.md", cuerpo),
        "foo.bar",
        "solo se quita la extensión `.md` final"
    );
    // Y ningún nombre es especial (E16-H02).
    assert_eq!(
        titulo("README.md", cuerpo),
        "README",
        "`README.md` deriva su título con la misma regla que cualquier otro documento"
    );
    assert_eq!(
        titulo("docs/index.md", cuerpo),
        "index",
        "`index.md` tampoco hereda el título de su carpeta: no representa una colección"
    );
}

/// Criterio 5: con `title: 42` la derivación no revienta **y** `title` sigue siendo metadata
/// consultable con su tipo numérico.
#[test]
fn title_no_es_reservada() {
    let raw = concat!(
        "---\n",
        "title: 42\n",
        "status: accepted\n",
        "---\n",
        "\n",
        "# Encabezado del cuerpo\n",
    );
    let parsed = model::parse_file("docs/numerico.md", raw);
    let pf = parsed
        .frontmatter
        .as_ref()
        .expect("el documento tiene frontmatter");

    // (a) No revienta: un escalar no-string se rinde a texto para presentar.
    assert_eq!(
        model::derived_title(Some(pf), &parsed.body, &rp("docs/numerico.md")),
        "42",
        "`title: 42` se presenta como «42»: la derivación es tolerante, no valida el tipo"
    );

    // (b) Y `title` NO se convierte en propiedad reservada: sigue siendo metadata del usuario,
    //     con su tipo YAML real (si la heurística coercionase el dato, volvería `js_string`).
    let v = pf
        .get(&fp("title"))
        .expect("`title` sigue en el frontmatter");
    assert!(
        matches!(v, Yaml::Number(_)),
        "`title` conserva su tipo numérico para la consulta; llegó {v:?}"
    );
    assert_eq!(v.as_i64(), Some(42), "y su valor es el entero 42");
    assert_ne!(
        v,
        &Yaml::String("42".to_string()),
        "derivar un título NO puede reescribir el dato del usuario a string"
    );

    // (c) Un `title` sin rendición textual (lista, mapa, `null`) no es un título: la cadena sigue.
    let lista = concat!(
        "---\n",
        "title:\n",
        "  - uno\n",
        "  - dos\n",
        "---\n",
        "\n",
        "# Título real\n",
    );
    assert_eq!(
        titulo("docs/lista.md", lista),
        "Título real",
        "una lista no tiene rendición textual: no puede ser el título, pero tampoco un error"
    );
    let nulo = concat!("---\n", "title:\n", "---\n", "\n", "# Título real\n");
    assert_eq!(
        titulo("docs/nulo.md", nulo),
        "Título real",
        "`title:` a `null` es una clave presente sin valor presentable: la cadena continúa"
    );
}

// =============================================================================
// E16-H04 — `patch_frontmatter` quirúrgico
// =============================================================================
//
// `ARCHITECTURE.md §20.4` («modifica solo las claves pedidas, preserva las demás, no reordena
// innecesariamente, mantiene el cuerpo intacto y **distingue explícitamente asignar `null` de
// eliminar una clave**. El plan debe declarar si el bloque se reserializará entero») y
// `REFACTOR_PHASE_2 §Fase 4 (Requisitos de edición)`.
//
// ## API que fija esta fase roja
//
// ```ignore
// // lodestar_core::model
//
// /// El documento resultante de aplicar un `FrontmatterPatch`, con la declaración que
// /// `change_plan` (E21) necesita para avisar al agente.
// pub struct PatchedDocument {
//     /// El `.md` COMPLETO resultante (frontmatter + cuerpo), listo para el único escritor.
//     pub raw: String,
//     /// `true` si el bloque de frontmatter se **reserializó entero** en vez de editarse in situ.
//     /// Es el «campo booleano del resultado» de la historia: significa *se ha perdido el texto
//     /// original del bloque* (formato, estilo de comillas, comentarios YAML, saltos), no
//     /// meramente *el fichero ha cambiado*.
//     pub reserialized: bool,
// }
//
// /// Aplica un patch de frontmatter sobre el texto crudo de UN documento. Pura (`§CLAUDE` #2):
// /// ni toca disco ni necesita el resto del workspace — por eso recibe el `raw` entero y no un
// /// `&DocumentSet`: el patch quirúrgico necesita el `span` del bloque DENTRO del documento.
// pub fn patch_frontmatter(
//     raw: &str,
//     patch: &FrontmatterPatch,
// ) -> Result<PatchedDocument, CoreError>;
// ```
//
// ## Contrato de «el patch lo permite» (lo fija esta fase roja, `§20.4` no lo detalla)
//
// El patch se aplica **quirúrgicamente** (`reserialized == false`) si, para **cada** clave que
// toca, se cumple una de estas dos:
//
//   1. la clave **no existe** en el bloque — `set` añade una línea al final, `remove` es no-op; o
//   2. la clave existe en el **primer nivel** y su valor está escrito **en una sola línea**
//      (`clave: escalar`, `clave: [a, b]` en flow style, `clave:` vacío): esa línea —y solo
//      esa— se sustituye o se borra.
//
// Es **reserialización** (`reserialized == true`) si alguna clave tocada existe con un valor
// **multilínea** (un mapa o una lista en block style, que ocupan varias líneas del bloque), o si
// el bloque tiene cualquier otra forma que impida localizar líneas con seguridad. Es el «la clave
// está dentro de una estructura anidada compleja» de la historia, llevado al único direccionamiento
// que `FrontmatterPatch` sabe expresar (claves de primer nivel): tocar la estructura ENTERA.
//
// **Deliberadamente NO se fija** (queda como reserialización, que siempre es correcta): claves
// duplicadas, anchors/alias YAML, documentos multi-doc `---` internos, block scalars `|`/`>`.
//
// ## Hasta dónde llega «su formato original»
//
// Hasta el **byte**, y solo en el camino quirúrgico: las líneas del bloque que el patch no toca
// llegan al resultado **idénticas y en el mismo orden** — el flow style sigue en flow style, las
// comillas siguen como estaban, la indentación del mapa anidado se conserva y **un comentario YAML
// sobrevive** (serde_yaml los descarta: un comentario en el bloque es el testigo más limpio de que
// no ha habido round-trip). De la línea **sí** tocada no se exige formato alguno, solo que su
// valor sea el nuevo. En el camino de reserialización no se exige formato: se exige que no se
// pierda ninguna clave, que los valores conserven su tipo y que el cuerpo siga intacto.
//
// ## El `Err`: por qué la firma devuelve `Result` y qué variante
//
// Un frontmatter que Lodestar **no puede interpretar** (sin cerrar, o con YAML inválido) no se
// puede parchear: no hay mapa sobre el que aplicar el merge-patch. El peligro no es teórico —
// `parse_file` devuelve `frontmatter: None` tanto para «no hay bloque» como para «hay un bloque
// ilegible» (solo `fm_err` los distingue), así que una implementación que se guíe por
// `frontmatter.is_none()` creará un bloque nuevo **encima del ilegible y lo borrará**. En un motor
// que promete garantías transaccionales, ese es el peor fallo posible; de ahí el `Result` y
// `patch_sobre_frontmatter_ilegible_falla`.
//
// La variante **debe ser nueva**: `CoreError::UnreadableFrontmatter`. Ninguna existente sirve —
// `NormalizeTargetNotFound` mapea a `DocumentNotFound` y mentiría (el documento existe), y
// `OperationNotApplicable` mapea a `InternalIoError`, que culparía al motor de un estado del
// fichero del usuario. El agente necesita oír «el frontmatter de este documento no es
// interpretable: repáralo (o escríbelo crudo) antes de tocar su metadata». Su mapeo a `ErrorCode`
// se decide en E21, cuando la operación llegue a la superficie MCP; aquí solo se exige que la
// variante exista con ese nombre.
//
// La aserción es **agnóstica a la forma del payload** (tupla/struct/unit): comprueba que el nombre
// de la variante aparece en el `Debug` del error — misma convención que `delete_referenciado_rechaza`
// con `CoreError::InboundLinksExist` (`core.rs:2172-2177`). Se hace así, y no añadiendo la variante
// en el stub, porque `lodestar_app::error_code` (`lodestar-app/src/lib.rs:121`) hace `match`
// **exhaustivo** sobre `CoreError`: añadirla sin su brazo rompería la compilación de producción.

use lodestar_core::model::PatchedDocument;
use lodestar_core::types::{FmError, FrontmatterPatch};

/// `FrontmatterPatch` desde pares: `Some(v)` escribe, `None` borra (los 3 estados de `§20.4`).
fn parche(entradas: &[(&str, Option<Yaml>)]) -> FrontmatterPatch {
    FrontmatterPatch(
        entradas
            .iter()
            .map(|(k, v)| ((*k).to_string(), v.clone()))
            .collect(),
    )
}

/// Aplica un patch sobre un documento bien formado (todos los de esta sección lo son).
fn parchear(raw: &str, patch: &FrontmatterPatch) -> PatchedDocument {
    model::patch_frontmatter(raw, patch)
        .expect("el patch debe aplicarse sobre un documento con frontmatter bien formado")
}

/// El texto YAML del bloque de un documento, **sin** los delimitadores: lo que hay que comparar
/// para juzgar «formato original». Reutiliza [`model::parse_frontmatter`] (E16-H01).
fn bloque(raw: &str) -> String {
    model::parse_frontmatter(raw)
        .unwrap_or_else(|| panic!("el documento debe tener un bloque de frontmatter:\n{raw}"))
        .raw
}

/// Claves de primer nivel **en orden de aparición** (la `claves` de E16-H01 devuelve un conjunto
/// a propósito; aquí el orden es justamente lo que se juzga).
fn claves_ordenadas(raw: &str) -> Vec<String> {
    model::parse_frontmatter(raw)
        .unwrap_or_else(|| panic!("el documento debe tener un bloque de frontmatter:\n{raw}"))
        .mapping()
        .keys()
        .map(|k| {
            k.as_str()
                .expect("las claves de estos tests son escalares string")
                .to_string()
        })
        .collect()
}

/// Un valor string YAML, para los patches.
fn s(v: &str) -> Option<Yaml> {
    Some(Yaml::String(v.to_string()))
}

/// Documento con **6 claves en orden no alfabético** (alfabético sería `owners`, `priority`,
/// `reviewed`, `service`, `status`, `type`), y con las tres formas de escritura que un round-trip
/// por `serde_yaml` destruiría: una lista en **flow style**, un **comentario** y un mapa anidado en
/// **block style**. `status` es la clave «del medio» que se parchea.
const DOC_SEIS_CLAVES: &str = concat!(
    "---\n",
    "type: decision\n",
    "status: draft\n",
    "# el equipo que mantiene el servicio\n",
    "owners: [platform, security]\n",
    "priority: 2\n",
    "service:\n",
    "  name: auth\n",
    "  tier: critical\n",
    "reviewed: false\n",
    "---\n",
    "\n",
    "# Autenticación\n",
    "\n",
    "Cuerpo.\n",
);

/// Las 6 claves de `DOC_SEIS_CLAVES`, en su orden de aparición.
const SEIS_CLAVES: [&str; 6] = [
    "type", "status", "owners", "priority", "service", "reviewed",
];

/// Líneas del bloque de `raw` que **no** pertenecen a la clave de primer nivel `clave` (se
/// reconoce por el inicio de línea, tolerando que el valor se escriba entrecomillado).
fn lineas_salvo(raw: &str, clave: &str) -> Vec<String> {
    bloque(raw)
        .lines()
        .filter(|l| {
            !l.trim_start()
                .trim_start_matches(['"', '\''])
                .starts_with(clave)
        })
        .map(str::to_string)
        .collect()
}

/// Criterio 1: con 6 claves en orden no alfabético, parchear una del medio deja a las otras 5 con
/// su orden **y su formato original**.
#[test]
fn patch_preserva_orden_y_claves() {
    let original = model::parse_frontmatter(DOC_SEIS_CLAVES).expect("el fixture tiene frontmatter");

    // --- (a) Modificar la clave del medio.
    let res = parchear(DOC_SEIS_CLAVES, &parche(&[("status", s("accepted"))]));

    assert!(
        !res.reserialized,
        "sustituir el valor escalar de una clave de primer nivel es una edición QUIRÚRGICA: no \
         reserializa el bloque.\nbloque resultante:\n{}",
        bloque(&res.raw)
    );
    assert_eq!(
        claves_ordenadas(&res.raw),
        SEIS_CLAVES.map(String::from).to_vec(),
        "las 6 claves siguen en su orden de aparición: el patch no canonicaliza nada.\nbloque \
         resultante:\n{}",
        bloque(&res.raw)
    );

    // El formato original, byte a byte: TODA línea del bloque que no sea la de `status` llega
    // idéntica y en el mismo orden. Esto fija el flow style de `owners`, el comentario YAML, la
    // indentación de `service` y las comillas de todo lo demás.
    assert_eq!(
        lineas_salvo(&res.raw, "status"),
        lineas_salvo(DOC_SEIS_CLAVES, "status"),
        "las 5 claves no tocadas conservan su TEXTO original línea a línea (flow style, \
         comentario, indentación); reserializar el bloque las reescribiría.\nbloque \
         resultante:\n{}",
        bloque(&res.raw)
    );

    // Y el patch hizo su trabajo, con el tipo YAML del nuevo valor.
    let re = model::parse_frontmatter(&res.raw).expect("el resultado tiene frontmatter");
    assert_eq!(
        re.get(&fp("status")),
        Some(&Yaml::String("accepted".to_string())),
        "la clave parcheada toma el valor nuevo:\n{}",
        bloque(&res.raw)
    );
    for clave in ["type", "owners", "priority", "service", "reviewed"] {
        let path = fp(clave);
        assert_eq!(
            re.get(&path),
            original.get(&path),
            "la clave `{clave}` no se ha tocado: mismo valor y mismo tipo.\nbloque \
             resultante:\n{}",
            bloque(&res.raw)
        );
    }

    // --- (b) Añadir una clave nueva: va al FINAL, sin mover ni reescribir nada de lo anterior.
    let add = parchear(DOC_SEIS_CLAVES, &parche(&[("reviewed_by", s("ana"))]));
    assert!(
        !add.reserialized,
        "añadir una clave escalar es añadir una línea al final del bloque: tampoco reserializa.\n\
         bloque resultante:\n{}",
        bloque(&add.raw)
    );
    let mut esperadas = SEIS_CLAVES.map(String::from).to_vec();
    esperadas.push("reviewed_by".to_string());
    assert_eq!(
        claves_ordenadas(&add.raw),
        esperadas,
        "las claves nuevas se añaden al final, sin reordenar las existentes.\nbloque \
         resultante:\n{}",
        bloque(&add.raw)
    );
    let previas: Vec<String> = bloque(DOC_SEIS_CLAVES)
        .lines()
        .map(str::to_string)
        .collect();
    let nuevas: Vec<String> = bloque(&add.raw).lines().map(str::to_string).collect();
    assert_eq!(
        nuevas.get(..previas.len()).map(<[String]>::to_vec),
        Some(previas.clone()),
        "el bloque original queda como PREFIJO exacto del nuevo: nada se reescribe al añadir.\n\
         bloque resultante:\n{}",
        bloque(&add.raw)
    );
}

/// Criterio 2: `{status: null}` deja la clave con valor nulo; eliminarla la hace desaparecer. Son
/// dos estados distintos, no dos formas de lo mismo.
#[test]
fn null_no_es_borrado() {
    // Asignar `null` (`Some(Value::Null)` en el patch).
    let nulo = parchear(DOC_SEIS_CLAVES, &parche(&[("status", Some(Yaml::Null))]));
    let re_nulo = model::parse_frontmatter(&nulo.raw).expect("el resultado tiene frontmatter");
    assert!(
        re_nulo.contains_key("status"),
        "asignar `null` deja la clave PRESENTE:\n{}",
        bloque(&nulo.raw)
    );
    assert_eq!(
        re_nulo.get(&fp("status")),
        Some(&Yaml::Null),
        "…y con valor nulo:\n{}",
        bloque(&nulo.raw)
    );
    assert_eq!(
        claves_ordenadas(&nulo.raw),
        SEIS_CLAVES.map(String::from).to_vec(),
        "asignar `null` no altera el juego de claves ni su orden:\n{}",
        bloque(&nulo.raw)
    );

    // Eliminar la clave (`None` en el patch).
    let borrado = parchear(DOC_SEIS_CLAVES, &parche(&[("status", None)]));
    let re_borrado =
        model::parse_frontmatter(&borrado.raw).expect("el resultado tiene frontmatter");
    assert!(
        !re_borrado.contains_key("status"),
        "eliminar la clave la hace DESAPARECER del bloque:\n{}",
        bloque(&borrado.raw)
    );
    assert!(
        !bloque(&borrado.raw).contains("status"),
        "…también del texto: no queda una línea `status:` huérfana:\n{}",
        bloque(&borrado.raw)
    );
    assert_eq!(
        claves_ordenadas(&borrado.raw),
        ["type", "owners", "priority", "service", "reviewed"].map(String::from),
        "y solo desaparece esa: el resto conserva su orden:\n{}",
        bloque(&borrado.raw)
    );

    // Los dos estados son distinguibles (es la razón de ser de `Option<Value>` en el patch).
    assert_ne!(
        nulo.raw, borrado.raw,
        "«asignar null» y «eliminar» no pueden producir el mismo documento"
    );
    assert_ne!(
        re_nulo.get(&fp("status")),
        re_borrado.get(&fp("status")),
        "`Some(Null)` es presencia con valor nulo; `None` es ausencia"
    );
}

/// Documento cuyo **cuerpo contiene una línea `---`** (una regla horizontal Markdown) y que además
/// deja DOS líneas en blanco tras el bloque: cualquier reconstrucción que normalice separadores
/// —como hace hoy `build_raw` con su `trim_start_matches('\n')`— lo altera.
const DOC_CUERPO_CON_RAYA: &str = concat!(
    "---\n",
    "type: decision\n",
    "status: draft\n",
    "service:\n",
    "  name: auth\n",
    "  tier: critical\n",
    "---\n",
    "\n",
    "\n",
    "# Documento\n",
    "\n",
    "Un párrafo.\n",
    "\n",
    "---\n",
    "\n",
    "Otro párrafo, tras la regla horizontal.\n",
);

/// El cuerpo de `DOC_CUERPO_CON_RAYA`: todo lo que sigue al delimitador de cierre y a su salto.
const CUERPO_CON_RAYA: &str = concat!(
    "\n",
    "\n",
    "# Documento\n",
    "\n",
    "Un párrafo.\n",
    "\n",
    "---\n",
    "\n",
    "Otro párrafo, tras la regla horizontal.\n",
);

/// Criterio 3: el cuerpo queda **byte a byte** idéntico tras parchear el frontmatter.
///
/// Se comprueba por los **dos** caminos —quirúrgico y reserialización—, porque «el cuerpo del
/// documento queda intacto byte a byte» es incondicional en la historia: el `---` del cuerpo no
/// puede confundirse con un delimitador ni bajo reserialización.
#[test]
fn cuerpo_intacto() {
    assert_eq!(
        model::parse_file("docs/raya.md", DOC_CUERPO_CON_RAYA).body,
        CUERPO_CON_RAYA,
        "premisa del fixture: el cuerpo empieza tras el delimitador de cierre"
    );

    let casos: [(&str, FrontmatterPatch); 3] = [
        // Quirúrgico: escalar de primer nivel.
        ("escalar", parche(&[("status", s("accepted"))])),
        // Quirúrgico: borrado de un escalar de primer nivel.
        ("borrado", parche(&[("status", None)])),
        // Reserialización: la clave tocada es un mapa anidado multilínea.
        (
            "anidado",
            parche(&[("service", Some(Yaml::String("auth".to_string())))]),
        ),
    ];

    for (nombre, patch) in casos {
        let res = parchear(DOC_CUERPO_CON_RAYA, &patch);
        assert!(
            res.raw.ends_with(CUERPO_CON_RAYA),
            "[{nombre}] el cuerpo debe sobrevivir byte a byte al final del documento; \
             resultado:\n{}",
            res.raw
        );
        assert_eq!(
            model::parse_file("docs/raya.md", &res.raw).body,
            CUERPO_CON_RAYA,
            "[{nombre}] al reparsear, el cuerpo es EXACTAMENTE el original: ni se normalizan las \
             líneas en blanco de separación ni el `---` del cuerpo cierra nada; resultado:\n{}",
            res.raw
        );
    }
}

/// Criterio 4: un patch que obliga a reserializar el bloque entero **lo señala explícitamente**.
#[test]
fn declara_reserializacion() {
    // `service` está escrito como mapa anidado en block style (3 líneas del bloque): sustituirlo
    // no es sustituir una línea, así que el bloque se reserializa entero.
    let res = parchear(DOC_SEIS_CLAVES, &parche(&[("service", s("auth"))]));
    assert!(
        res.reserialized,
        "tocar una clave cuyo valor es una estructura anidada multilínea reserializa el bloque, y \
         el resultado DEBE declararlo (`change_plan` de E21 lo consume para avisar al agente).\n\
         bloque resultante:\n{}",
        bloque(&res.raw)
    );

    // La bandera describe el camino tomado, no es una constante: el MISMO documento, parcheado en
    // una clave escalar de una sola línea, no reserializa.
    let quirurgico = parchear(
        DOC_SEIS_CLAVES,
        &parche(&[("priority", Some(Yaml::Number(7.into())))]),
    );
    assert!(
        !quirurgico.reserialized,
        "sobre el mismo documento, tocar `priority` (escalar en una línea) NO reserializa: la \
         bandera distingue los dos caminos.\nbloque resultante:\n{}",
        bloque(&quirurgico.raw)
    );

    // Reserializar es perder el TEXTO del bloque, nunca perder DATOS: siguen las 6 claves, con sus
    // valores y sus tipos, y el cuerpo intacto.
    let original = model::parse_frontmatter(DOC_SEIS_CLAVES).expect("el fixture tiene frontmatter");
    let re = model::parse_frontmatter(&res.raw).expect("el resultado tiene frontmatter");
    assert_eq!(
        claves_ordenadas(&res.raw),
        SEIS_CLAVES.map(String::from).to_vec(),
        "ni reserializando se pierde o reordena una clave.\nbloque resultante:\n{}",
        bloque(&res.raw)
    );
    for clave in ["type", "status", "owners", "priority", "reviewed"] {
        let path = fp(clave);
        assert_eq!(
            re.get(&path),
            original.get(&path),
            "reserializar conserva el VALOR y el TIPO de `{clave}`.\nbloque resultante:\n{}",
            bloque(&res.raw)
        );
    }
    assert_eq!(
        re.get(&fp("service")),
        Some(&Yaml::String("auth".to_string())),
        "y la clave pedida toma el valor nuevo.\nbloque resultante:\n{}",
        bloque(&res.raw)
    );
    assert!(
        res.raw.ends_with("\n# Autenticación\n\nCuerpo.\n"),
        "el cuerpo sigue intacto tras la reserialización:\n{}",
        res.raw
    );
}

/// Documento **sin una sola línea de frontmatter**, con un `---` suelto en el cuerpo.
const DOC_SIN_BLOQUE: &str = concat!(
    "# Documento pelado\n",
    "\n",
    "Sin frontmatter.\n",
    "\n",
    "---\n",
    "\n",
    "Fin.\n",
);

/// Criterio 5: parchear un documento **sin** frontmatter crea el bloque al principio y deja el
/// cuerpo intacto.
#[test]
fn patch_crea_bloque() {
    let res = parchear(
        DOC_SIN_BLOQUE,
        &parche(&[
            ("status", s("accepted")),
            ("priority", Some(Yaml::Number(2.into()))),
        ]),
    );

    // El cuerpo entero sobrevive byte a byte como sufijo del documento; delante solo puede haber
    // el bloque recién creado (con, a lo sumo, una línea en blanco de separación).
    let cabecera = res.raw.strip_suffix(DOC_SIN_BLOQUE).unwrap_or_else(|| {
        panic!(
            "el cuerpo debe quedar intacto al final del documento:\n{}",
            res.raw
        )
    });
    assert!(
        cabecera.starts_with("---\n"),
        "el bloque se crea AL PRINCIPIO del documento; cabecera: {cabecera:?}"
    );
    let cerrado = cabecera.trim_end_matches('\n');
    assert!(
        cerrado.ends_with("\n---"),
        "y se cierra con su delimitador; cabecera: {cabecera:?}"
    );
    assert!(
        cabecera.len() - cerrado.len() <= 2,
        "entre el bloque y el cuerpo cabe como mucho una línea en blanco; cabecera: {cabecera:?}"
    );

    // Y es un frontmatter de verdad: se reparsea con los valores del patch, con su tipo YAML.
    let parsed = model::parse_file("docs/pelado.md", &res.raw);
    let re = parsed
        .frontmatter
        .as_ref()
        .expect("el documento parcheado ya tiene frontmatter");
    assert_eq!(
        re.get(&fp("status")),
        Some(&Yaml::String("accepted".to_string())),
        "el bloque creado lleva las claves del patch:\n{}",
        res.raw
    );
    assert!(
        matches!(re.get(&fp("priority")), Some(Yaml::Number(_))),
        "…con su tipo YAML, no coercionadas a texto: {:?}",
        re.get(&fp("priority"))
    );
    assert!(
        parsed.body.ends_with(DOC_SIN_BLOQUE),
        "el `---` del cuerpo no se ha convertido en delimitador: el cuerpo sigue entero:\n{}",
        res.raw
    );

    // Crear el bloque NO es reserializarlo: la bandera significa «se ha perdido el texto original
    // del bloque», y aquí no había bloque que perder. (Lo contrario haría que `change_plan`
    // avisara de una pérdida de formato inexistente en toda creación de metadata.)
    assert!(
        !res.reserialized,
        "crear un bloque donde no había ninguno no destruye formato del usuario: no es \
         reserialización"
    );
}

/// Frontmatter que abre `---` y **nunca cierra**.
///
/// Ojo al montar este fixture (E16-H01 reescribió `split_front`): el bloque se cierra con la
/// PRIMERA línea posterior que empiece por `---`, así que el cuerpo no puede contener ninguna —
/// ni siquiera una regla horizontal `----` o un separador `-----`, que empiezan por `---` y
/// cerrarían el bloque, convirtiendo el documento en uno perfectamente legible y este test en una
/// tautología. Por el mismo cambio, `---\n---\n` **ya no** es «sin cerrar» sino un bloque vacío
/// válido: tampoco sirve como fixture de este caso.
const DOC_FM_SIN_CERRAR: &str = concat!(
    "---\n",
    "type: decision\n",
    "status: draft\n",
    "owners: [platform, security]\n",
    "\n",
    "# Aquí arriba falta el cierre del bloque\n",
    "\n",
    "Este cuerpo tampoco debe perderse.\n",
);

/// Frontmatter con bloque bien delimitado pero **YAML sintácticamente inválido**.
const DOC_FM_YAML_ROTO: &str = concat!(
    "---\n",
    "type: : :\n",
    "  - x\n",
    ": bad\n",
    "---\n",
    "\n",
    "# Documento\n",
    "\n",
    "Y este cuerpo tampoco.\n",
);

/// Criterio 6 (añadido tras la fase roja): parchear un documento cuyo frontmatter **no es
/// interpretable** falla, y el documento queda **intacto byte a byte**. El bloque ilegible no se
/// sustituye nunca por uno nuevo.
///
/// Es el escenario destructivo: `parse_file` devuelve `frontmatter: None` **tanto** para «no hay
/// bloque» (→ `patch_crea_bloque`: se crea, correcto) **como** para «hay un bloque y no se puede
/// leer» (→ aquí: se falla). Solo `fm_err` los distingue. Una implementación que se guíe por
/// `frontmatter.is_none()` pasa `patch_crea_bloque` y **borra el frontmatter del usuario** en este.
/// Por eso el test comprueba las dos caras: que este falla **y** que el de ausencia sigue creando.
#[test]
fn patch_sobre_frontmatter_ilegible_falla() {
    for (caso, doc, err_esperado) in [
        ("sin cerrar", DOC_FM_SIN_CERRAR, FmError::Unclosed),
        (
            "YAML inválido",
            DOC_FM_YAML_ROTO,
            FmError::Malformed(String::new()),
        ),
    ] {
        // --- Premisa del fixture: el documento es ilegible por la razón que se dice, y su
        //     frontmatter llega como `None` (que es justo lo que lo hace confundible con la
        //     ausencia de bloque). Si `split_front` derivase, el fixture dejaría de probar nada.
        let parsed = model::parse_file("docs/roto.md", doc);
        assert!(
            parsed.frontmatter.is_none(),
            "[{caso}] premisa: un frontmatter ilegible llega como `frontmatter: None`"
        );
        assert_eq!(
            std::mem::discriminant(
                parsed
                    .fm_err
                    .as_ref()
                    .unwrap_or_else(|| panic!("[{caso}] premisa: el documento debe ser ilegible"))
            ),
            std::mem::discriminant(&err_esperado),
            "[{caso}] premisa: el documento debe ser ilegible por ESTA razón, no por otra"
        );

        // --- (a) Cualquier patch que modifique algo FALLA.
        for (que, patch) in [
            ("sobrescribir una clave", parche(&[("status", s("done"))])),
            ("borrar una clave", parche(&[("status", None)])),
            ("añadir una clave nueva", parche(&[("reviewed", s("si"))])),
        ] {
            let res = model::patch_frontmatter(doc, &patch);
            assert!(
                res.is_err(),
                "[{caso}] {que}: parchear un frontmatter que no se puede interpretar DEBE fallar. \
                 Devolver `Ok` significa haber reconstruido el bloque encima del ilegible, es \
                 decir, haber borrado la metadata del usuario. Devolvió:\n{}",
                res.as_ref().map(|d| d.raw.as_str()).unwrap_or_default()
            );
        }

        // --- (b) El error NOMBRA el problema. `change_plan` (E21) tiene que poder decirle al
        //     agente qué reparar; un error genérico lo dejaría adivinando.
        let err = model::patch_frontmatter(doc, &parche(&[("status", s("done"))]))
            .expect_err("ya comprobado en (a)");
        assert!(
            format!("{err:?}").contains("UnreadableFrontmatter"),
            "[{caso}] el error debe ser `CoreError::UnreadableFrontmatter` (variante nueva: \
             ninguna existente sirve — `NormalizeTargetNotFound` mentiría diciendo que el \
             documento no existe y `OperationNotApplicable` culparía al motor de un estado del \
             fichero del usuario). Llegó: {err:?}"
        );

        // --- (c) La garantía fuerte, y la que no bastaría con satisfacer devolviendo `Err`: de
        //     esta operación NUNCA sale un documento distinto del original. Ni con el patch vacío,
        //     que un implementador podría querer tratar como no-op: o falla, o devuelve el
        //     original byte a byte. No hay tercera opción, y desde luego no una en la que el
        //     bloque ilegible haya sido sustituido.
        if let Ok(d) = model::patch_frontmatter(doc, &parche(&[])) {
            assert_eq!(
                d.raw, doc,
                "[{caso}] un patch vacío sobre un documento ilegible puede ser un no-op, pero \
                 entonces devuelve el documento ORIGINAL: jamás uno con el bloque reconstruido"
            );
        }
    }

    // --- (d) Contraste, para que el fallo no se pueda satisfacer fallando siempre:
    //     el mismo patch funciona sobre un documento legible…
    let patch = parche(&[("status", s("done"))]);
    assert!(
        model::patch_frontmatter(DOC_SEIS_CLAVES, &patch).is_ok(),
        "el mismo patch sobre un documento legible tiene que seguir funcionando"
    );
    //     …y sobre uno SIN frontmatter, que también llega con `frontmatter: None`. Ahí sí se crea
    //     el bloque (`patch_crea_bloque`): la frontera no es «no hay mapa que parchear», es «hay
    //     un bloque del usuario que no sé leer y no voy a pisar».
    assert!(
        model::patch_frontmatter(DOC_SIN_BLOQUE, &patch).is_ok(),
        "un documento SIN frontmatter no es un documento ilegible: ahí el patch crea el bloque"
    );
}

// =============================================================================
// E16-H05 — Diagnósticos mínimos: retirar el catálogo OKF
// =============================================================================
//
// `ARCHITECTURE.md §20.9` («¿puede Lodestar interpretar y modificar este workspace de forma
// consistente y segura?», **no** «¿cumple el workspace una especificación documental?») y
// `REFACTOR_PHASE_2 §Fase 10`.
//
// ## Lo que fija esta fase roja
//
// El catálogo de `CheckCode` pasa a ser el de `§20.9`. **Se borran** `OKF-FM01` (la falta de
// frontmatter deja de ser error), `OKF-TYPE`, `REC-TITLE`, `REC-DESC`, `FMT-TAGS`, `FMT-TS`,
// `BODY-STRUCT`, `ORPHAN` (ya sin productor desde E16-H02), `OKF-IDX`, `OKF-LOG`, y las familias
// `SCHEMA-*`/`REL-*`/`EXTREF-MISSING` dejan de producirse. **Renombres**: `OKF-FM02` →
// `FM-UNCLOSED`, `OKF-FM03` → `FM-YAML-INVALID`, `OKF-CONFLICT` → `DOC-CONFLICT-MARKER`. Mueren
// también `conform::validate_index`, `conform::validate_log` y `model::is_iso` (existía solo para
// `FMT-TS`).
//
// `Check` **conserva su forma** (`level`/`code`/`msg`/`targets` + los aditivos `id`/`range`/
// `related`/`fixes`, `§10` fila #3): cambia el catálogo de códigos, no la estructura.
//
// Los códigos se comparan **por su cadena de wire** (serializando `Check::code`) y nunca por la
// variante de la enum: así estos tests no dependen de cómo se llame la variante en Rust, y
// sobreviven al borrado de las variantes viejas sin dejar de significar lo mismo.
//
// **Fuera de alcance aquí**: `LINK-STUB`/`LINK-REL` siguen vivos hasta E17 (donde se convierten en
// `LINK-TARGET-MISSING`/`LINK-CASE-MISMATCH`/…), así que ningún fixture de esta sección tiene
// enlaces; y `DOC-NOT-UTF8`/`DOC-TOO-LARGE`/`PATH-NOT-UTF8`/`SYMLINK-UNSUPPORTED` los produce el
// descubrimiento de `lodestar-workspace` (E15-H07), no `conform`.

use lodestar_core::types::{Check, Range as RangoLineas, Severity};

/// Los diagnósticos emitidos para `p` (vacío si no hay ninguno).
fn diagnosticos<'a>(a: &'a Analysis, p: &RelPath) -> &'a [Check] {
    a.diagnostics.get(p).map_or(&[], Vec::as_slice)
}

/// Analiza un workspace de un solo documento y devuelve su análisis y su ruta.
fn analiza_uno(path: &str, raw: &str) -> (Analysis, RelPath) {
    let b = DocumentSet::from_files(mapa(&[(path, raw)]));
    (b.analyze().clone(), rp(path))
}

/// Criterio 1: un documento sin frontmatter, sin `type` y sin `status` no emite **ningún**
/// diagnóstico.
///
/// Ojo con la deuda de E16-H02: allí se migraron 55 fixtures `index.md` a `type:`/`title:`/
/// `description:` porque `OKF-TYPE` seguía vivo. El punto de este test es el contrario: un
/// documento **pelado** no produce nada — ni `OKF-FM01`, ni `OKF-TYPE`, ni `REC-*`, ni
/// `BODY-STRUCT`.
#[test]
fn sin_frontmatter_no_diagnostica() {
    // Ni siquiera tiene encabezados (lo que hoy dispara además `BODY-STRUCT`).
    let (a, p) = analiza_uno(
        "docs/pelado.md",
        "Un documento pelado: sin frontmatter, sin `type` y sin `status`.\n",
    );
    assert_eq!(
        codigos(&a, &p),
        Vec::<String>::new(),
        "un `.md` cualquiera es un documento de primera clase: no incumple nada. Diagnósticos: {:?}",
        diagnosticos(&a, &p)
    );
    assert_eq!(
        a.hard_fail(),
        0,
        "y no es un hard-fail: la puerta de CI no puede caerse por un README sin metadata"
    );

    // Con encabezados y sin frontmatter, igual: nada.
    let (b, q) = analiza_uno("README.md", "# Proyecto\n\nQué es esto.\n");
    assert_eq!(
        codigos(&b, &q),
        Vec::<String>::new(),
        "tampoco un `README.md` con encabezados. Diagnósticos: {:?}",
        diagnosticos(&b, &q)
    );

    // Y un frontmatter vacío tampoco: no hay «campos obligatorios» que echar de menos.
    let (c, r) = analiza_uno("docs/vacio.md", "---\n---\n\n# Vacío\n");
    assert_eq!(
        codigos(&c, &r),
        Vec::<String>::new(),
        "un bloque vacío es válido y silencioso. Diagnósticos: {:?}",
        diagnosticos(&c, &r)
    );
}

/// Criterio 2: `tags: "no-es-lista"` y `timestamp: "ayer"` no producen diagnóstico — son metadata
/// arbitraria del usuario, no un formato de Lodestar.
#[test]
fn formato_de_tags_no_diagnostica() {
    let raw = concat!(
        "---\n",
        "tags: \"no-es-lista\"\n",
        "timestamp: \"ayer\"\n",
        "---\n",
        "\n",
        "# Documento\n",
    );
    let (a, p) = analiza_uno("docs/tags.md", raw);
    assert_eq!(
        codigos(&a, &p),
        Vec::<String>::new(),
        "el formato de `tags`/`timestamp` es cosa del usuario: ni `FMT-TAGS`, ni `FMT-TS`, ni \
         `OKF-TYPE`, ni `REC-*`. Diagnósticos: {:?}",
        diagnosticos(&a, &p)
    );
    assert_eq!(a.hard_fail(), 0, "y desde luego no es un hard-fail");

    // El caso simétrico: los mismos nombres con «buen» formato tampoco dicen nada (no hay `Pass`
    // que informe de conformidad: Lodestar ya no juzga especificaciones documentales).
    let bueno = concat!(
        "---\n",
        "type: decision\n",
        "title: Autenticación\n",
        "description: Cómo se autentica el servicio\n",
        "tags:\n",
        "  - auth\n",
        "timestamp: 2026-07-23T10:00:00Z\n",
        "---\n",
        "\n",
        "# Autenticación\n",
    );
    let (b, q) = analiza_uno("docs/bueno.md", bueno);
    assert_eq!(
        codigos(&b, &q),
        Vec::<String>::new(),
        "cumplir OKF tampoco genera checks `Pass`: el catálogo entero se retira. Diagnósticos: {:?}",
        diagnosticos(&b, &q)
    );
}

/// Documento cuyo frontmatter abre `---` y nunca cierra.
const DOC_SIN_CIERRE: &str = concat!(
    "---\n",
    "type: decision\n",
    "status: draft\n",
    "\n",
    "# El bloque de arriba nunca se cierra\n",
);

/// Criterio 3: frontmatter sin cierre → `FM-UNCLOSED` con severidad error.
#[test]
fn frontmatter_sin_cierre() {
    let (a, p) = analiza_uno("docs/sin-cierre.md", DOC_SIN_CIERRE);
    assert_eq!(
        codigos(&a, &p),
        vec!["FM-UNCLOSED".to_string()],
        "un bloque sin cerrar impide interpretar el documento: es exactamente `FM-UNCLOSED` (el \
         antiguo `OKF-FM02`), y nada más. Diagnósticos: {:?}",
        diagnosticos(&a, &p)
    );
    let d = &diagnosticos(&a, &p)[0];
    assert_eq!(
        d.level,
        Severity::Err,
        "con severidad error: Lodestar no puede modificar con seguridad lo que no sabe leer"
    );
    assert_eq!(
        d.targets,
        vec![p.clone()],
        "y apunta al documento afectado (`targets` nunca es null)"
    );
    assert_eq!(
        a.hard_fail(),
        1,
        "sigue siendo hard-fail: es de lo poco que queda en el catálogo"
    );
}

/// Frontmatter con YAML sintácticamente inválido. Numerado para el rango esperado:
/// 1 `---` · 2 `type: : :` · 3 `  - x` · 4 `: bad` · 5 `---`.
const DOC_YAML_INVALIDO: &str = concat!(
    "---\n",       // línea 1 (delimitador de apertura)
    "type: : :\n", // línea 2
    "  - x\n",     // línea 3
    ": bad\n",     // línea 4
    "---\n",       // línea 5 (delimitador de cierre)
    "\n",
    "# Documento\n",
);

/// Criterio 4: YAML inválido → `FM-YAML-INVALID` **con el rango de líneas del bloque**.
///
/// El rango son las líneas de **contenido** del bloque (1-based, ambas inclusive), sin los
/// delimitadores: es la traducción a líneas del `span` de `ParsedFrontmatter` (E16-H01), que se
/// define igual — «excluye los delimitadores».
#[test]
fn yaml_invalido_con_rango() {
    let (a, p) = analiza_uno("docs/malo.md", DOC_YAML_INVALIDO);
    assert_eq!(
        codigos(&a, &p),
        vec!["FM-YAML-INVALID".to_string()],
        "YAML inválido es exactamente `FM-YAML-INVALID` (el antiguo `OKF-FM03`), y nada más. \
         Diagnósticos: {:?}",
        diagnosticos(&a, &p)
    );
    let d = &diagnosticos(&a, &p)[0];
    assert_eq!(d.level, Severity::Err, "con severidad error");
    assert_eq!(
        d.range,
        Some(RangoLineas {
            start_line: 2,
            end_line: 4,
        }),
        "el diagnóstico acota el bloque: líneas 2..4 (1-based, delimitadores excluidos). Es lo que \
         `§20.9` hace posible con el `span` de E16-H01. Diagnóstico: {d:?}"
    );
}

/// Criterio 5: marcadores de merge → `DOC-CONFLICT-MARKER` con severidad error.
#[test]
fn marcadores_de_merge() {
    let raw = concat!(
        "---\n",
        "status: draft\n",
        "---\n",
        "\n",
        "# Documento\n",
        "\n",
        "<<<<<<< HEAD\n",
        "una versión\n",
        "=======\n",
        "otra versión\n",
        ">>>>>>> rama\n",
    );
    let (a, p) = analiza_uno("docs/conflicto.md", raw);
    assert_eq!(
        codigos(&a, &p),
        vec!["DOC-CONFLICT-MARKER".to_string()],
        "unos marcadores sin resolver impiden modificar el documento con seguridad: \
         `DOC-CONFLICT-MARKER` (el antiguo `OKF-CONFLICT`), y nada más. Diagnósticos: {:?}",
        diagnosticos(&a, &p)
    );
    assert_eq!(
        diagnosticos(&a, &p)[0].level,
        Severity::Err,
        "con severidad error"
    );
    assert_eq!(
        a.hard_fail(),
        1,
        "y hard-fail: el documento está a medio mergear"
    );
}

/// Criterio 6: un documento **aislado** y uno con **estructura de headings arbitraria** no
/// producen diagnóstico.
#[test]
fn aislado_y_headings_no_diagnostican() {
    let headings = concat!(
        "---\n",
        "status: draft\n",
        "---\n",
        "\n",
        "### Empieza por un H3\n",
        "\n",
        "Texto.\n",
        "\n",
        "# Y luego un H1\n",
        "\n",
        "###### Y un H6\n",
    );
    let b = DocumentSet::from_files(mapa(&[
        (
            "docs/aislado.md",
            "---\nstatus: draft\n---\n\n# Aislado\n\nNi entrantes ni salientes.\n",
        ),
        ("docs/headings.md", headings),
        // Sin encabezados de ningún tipo: `BODY-STRUCT` tampoco sobrevive.
        (
            "docs/plano.md",
            "---\nstatus: draft\n---\n\nSolo un párrafo, sin apartados.\n",
        ),
    ]));
    let a = b.analyze();

    for path in ["docs/aislado.md", "docs/headings.md", "docs/plano.md"] {
        let p = rp(path);
        assert_eq!(
            codigos(a, &p),
            Vec::<String>::new(),
            "`{path}` no incumple nada: la estructura del cuerpo y el aislamiento dejaron de ser \
             diagnósticos. Diagnósticos: {:?}",
            diagnosticos(a, &p)
        );
    }
    assert_eq!(a.hard_fail(), 0, "ninguno es hard-fail");

    // El aislamiento sigue siendo una PROPIEDAD consultable del grafo (`§20.7`, E16-H02): lo que
    // se retira es el diagnóstico, no la información.
    assert!(
        a.isolated.contains(&rp("docs/aislado.md")),
        "el aislamiento sigue reportándose como propiedad: isolated={:?}",
        a.isolated
    );
}
