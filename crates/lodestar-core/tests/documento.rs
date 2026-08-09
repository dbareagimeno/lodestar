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

// ---------------------------------------------------------------------------
// E18-H01 — El recorrido recursivo de la metadata (la pieza que heredan E19/E20)
// ---------------------------------------------------------------------------
//
// UBICACIÓN Y RAZÓN DE SER: los cuatro criterios de E18-H01 son del store
// (`crates/lodestar-store/tests/store.rs`), pero la historia exige que el recorrido que puebla la
// tabla `metadata` **reutilice `FieldPath`/`ParsedFrontmatter::get` del core**, «nunca un segundo
// navegador del `Value` en SQL» (invariante #3). Hoy el core no expone ese recorrido: `entries()`
// solo da el primer nivel. Este test fija la firma que falta —y que van a heredar el evaluador de
// consultas (E19) y `metadata_inspect` (E20)— junto al resto de la spec de `ParsedFrontmatter`:
//
// ```ignore
// impl ParsedFrontmatter {
//     /// Pares (FieldPath, &Value) en profundidad y orden de aparición, padre antes que hijos.
//     pub fn walk(&self) -> Vec<(FieldPath, &serde_yaml::Value)>;
// }
// ```
//
// El invariante rector es uno solo: **para todo par devuelto, `get(path) == Some(value)`**. Es lo
// que impide que la cache materialice paths que la única verdad de acceso no sabe resolver.

/// Frontmatter de referencia del recorrido: mapas anidados a dos niveles, una lista de escalares,
/// una lista de mapas (el caso que tienta a inventar `contacts.0.nombre`) y una lista colgando de
/// un mapa.
const FM_ANIDADO: &str = concat!(
    "---\n",
    "service:\n",
    "  name: auth\n",
    "  tier: critical\n",
    "priority: 2\n",
    "owners: [platform, security]\n",
    "contacts:\n",
    "  - nombre: Ana\n",
    "    rol: sre\n",
    "  - nombre: Bea\n",
    "release:\n",
    "  target:\n",
    "    date: \"2026-07-23\"\n",
    "---\n",
    "\n",
    "# Servicio\n",
);

/// **Dado** un frontmatter con mapas anidados y listas, **Cuando** se recorre con `walk()`,
/// **Entonces** aparece un par por cada propiedad direccionable por `get` —mapas intermedios
/// incluidos—, en orden de aparición y con el padre antes que sus hijos, y **ninguno** por dentro
/// de una lista.
///
/// Fase ROJA: `ParsedFrontmatter::walk` no existe (se añade como stub `todo!()` para que el resto
/// del target siga compilando), así que el test entra en pánico en el `todo!()`.
#[test]
fn walk_recorre_la_metadata_direccionable() {
    let parsed = model::parse_file("servicio.md", FM_ANIDADO);
    let pf = parsed
        .frontmatter
        .expect("el documento de referencia tiene frontmatter");

    let pares = pf.walk();
    let paths: Vec<String> = pares.iter().map(|(p, _)| p.to_string()).collect();

    // (1) Cobertura y ORDEN: profundidad, aparición, padre antes que hijos. Los mapas intermedios
    //     (`service`, `release`, `release.target`) son propiedades direccionables y tienen par
    //     propio: `has(service)` (E19) y el catálogo de propiedades (E20) los necesitan.
    assert_eq!(
        paths,
        vec![
            "service",
            "service.name",
            "service.tier",
            "priority",
            "owners",
            "contacts",
            "release",
            "release.target",
            "release.target.date",
        ],
        "`walk()` debe recorrer en profundidad y en orden de aparición, emitiendo también los \
         mapas intermedios"
    );

    // (2) INVARIANTE RECTOR: cada par es exactamente lo que devuelve la única verdad de acceso.
    //     Si esto se cumple, la tabla `metadata` del store no puede inventar paths.
    for (path, valor) in &pares {
        assert_eq!(
            pf.get(path),
            Some(*valor),
            "`walk()` emitió `{path}`, pero `get(\"{path}\")` no devuelve ese mismo valor: el \
             recorrido sería un segundo navegador del `Value` (invariante #3)"
        );
    }

    // (3) Las LISTAS son hojas: `FieldPath` no direcciona posiciones, así que `owners.0` o
    //     `contacts.0.nombre` serían paths que `get` no resuelve.
    for path in &paths {
        assert!(
            !path.starts_with("owners.") && !path.starts_with("contacts."),
            "`{path}` desciende por dentro de una lista: `FieldPath` no direcciona posiciones"
        );
    }

    // (4) …y el valor de la lista viaja ENTERO en su propio par (con los mapas que contenga),
    //     que es lo que `owners contains "security"` (E19) y el recuento de valores (E20) usan.
    let (_, contacts) = pares
        .iter()
        .find(|(p, _)| p == &fp("contacts"))
        .expect("`contacts` debe tener su par");
    let elementos = contacts
        .as_sequence()
        .expect("`contacts` es una lista y su par la lleva entera");
    assert_eq!(elementos.len(), 2, "la lista no se trunca: {contacts:?}");
    assert_eq!(
        elementos[0]
            .as_mapping()
            .and_then(|m| m.iter().find(|(k, _)| k.as_str() == Some("rol")))
            .and_then(|(_, v)| v.as_str()),
        Some("sre"),
        "los mapas de dentro de la lista se conservan tal cual en el valor de la lista"
    );

    // (5) Y el tipo YAML real sobrevive al recorrido (no hay coerción: `2` sigue siendo número).
    let (_, priority) = pares
        .iter()
        .find(|(p, _)| p == &fp("priority"))
        .expect("`priority` debe tener su par");
    assert_eq!(
        priority.as_i64(),
        Some(2),
        "`walk()` no coerciona: `priority` sigue siendo el número 2, no \"2\" ({priority:?})"
    );
}

// ===========================================================================
// E23-H02 — `create` sin residuo OKF (`ARCHITECTURE.md §20.2` invariante 3, `§20.4`;
// `requirements/epica-23-cierre-migracion.md`). Fase ROJA.
//
// Síntoma reproducido con los binarios: `change_plan` con
// `{"op":"create","path":"notas/nueva.md","body":"# Nueva"}` + `change_apply` escribe en disco
//
//     ---
//     title: nueva
//     type: ''
//     ---
//
//     # Nueva
//
// Un `type` vacío heredado de OKF y un `title` que nadie pidió, en un producto cuyo invariante 3
// dice que las claves del frontmatter **no tienen semántica impuesta**. La causa es
// `plan::normalize_create`, que inserta `type` y `title` a fuego.
//
// ## Firma que fija esta fase roja (el «Alcance» de E23-H02)
//
// ```ignore
// pub fn normalize_create(
//     doc_set: &DocumentSet,
//     path: &RelPath,
//     frontmatter: Option<FrontmatterPatch>,   // ARBITRARIO: lo que pida el llamador, tal cual
//     body: Option<String>,
// ) -> Result<NormalizedOperation, CoreError>;
// ```
//
// Desaparecen los parámetros privilegiados `doctype: &str` y `title: Option<&str>`. **Sin**
// `frontmatter`, el `.md` sale SIN bloque de frontmatter (no un bloque vacío `---\n{}\n---`).
//
// ## Por qué viven aquí y no en `core.rs`
//
// Estos dos tests **no compilan** hasta que la firma cambie (arity + tipos), y un fallo de
// compilación tumba el binario de test entero. `documento.rs` es el hogar del modelo documental
// genérico (`§20.4`: frontmatter arbitrario, título derivado) y ya asume ese coste por diseño (ver
// la cabecera del fichero); dejarlos en `core.rs` habría enmascarado el rojo POR ASERCIÓN de los
// tests de E23-H03, que viven allí.
//
// ROJO esperado HOY: **compile-fail** (`normalize_create` toma 5 argumentos, no 4, y el tercero es
// `&str`). Es el rojo por símbolo/firma ausente, el mismo patrón que E12-H05/H06.
// ===========================================================================

use lodestar_core::plan;

/// El `.md` COMPLETO que produce un `create` normalizado: normaliza con la firma nueva y aplica la
/// operación con la simulación en memoria del core ([`plan::apply_normalized_ops`]), que es
/// exactamente la que materializa el contenido que luego escribe el único escritor.
fn raw_creado(path: &str, frontmatter: Option<FrontmatterPatch>, body: Option<&str>) -> String {
    let p = RelPath::new(path).expect("el path del fixture debe ser un RelPath válido");
    let vacio = DocumentSet::from_files(FileMap::new());
    let op = plan::normalize_create(&vacio, &p, frontmatter, body.map(str::to_string))
        .expect("crear un documento nuevo no debe fallar la normalización");
    let files = plan::apply_normalized_ops(&FileMap::new(), &[op])
        .expect("aplicar en memoria un `create` normalizado no debe fallar");
    files
        .get(&p)
        .cloned()
        .unwrap_or_else(|| panic!("el `create` debe materializar el documento `{path}`"))
}

/// Criterio `create_sin_frontmatter_no_inyecta` — **Dado** un `create` SIN `frontmatter`,
/// **Cuando** se aplica, **Entonces** el `.md` no contiene `type:` ni `title:` ni un bloque `---`
/// vacío.
///
/// La aserción fuerte es la igualdad byte a byte con el cuerpo pedido: «no inyecta» significa que
/// el documento es EXACTAMENTE lo que el llamador pidió, no que casualmente falten dos claves.
#[test]
fn create_sin_frontmatter_no_inyecta() {
    let raw = raw_creado("notas/nueva.md", None, Some("# Nueva\n"));

    assert!(
        !raw.contains("type:"),
        "un `create` sin `frontmatter` no puede inyectar la clave OKF `type` (§20.2 invariante 3: \
         las claves del frontmatter no tienen semántica impuesta); documento =\n{raw}",
    );
    assert!(
        !raw.contains("title:"),
        "un `create` sin `frontmatter` no puede inyectar `title`: el título se DERIVA (§20.4), no \
         se materializa como metadata que el usuario no pidió; documento =\n{raw}",
    );
    assert!(
        model::parse_frontmatter(&raw).is_none(),
        "sin `frontmatter` el `.md` sale SIN bloque de frontmatter, no con uno vacío; \
         documento =\n{raw}",
    );
    assert!(
        !raw.starts_with("---"),
        "el documento no debe abrir con un delimitador de frontmatter; documento =\n{raw}",
    );
    assert_eq!(
        raw, "# Nueva\n",
        "el `.md` creado debe ser EXACTAMENTE el cuerpo pedido, sin residuo",
    );
}

/// Criterio `create_frontmatter_arbitrario` — **Dado** un `create` con
/// `frontmatter: {estado: "borrador", tags: [a, b]}`, **Cuando** se aplica, **Entonces** el `.md`
/// lleva exactamente esas claves con sus tipos YAML.
///
/// «Exactamente» en los dos sentidos: ni falta ninguna de las pedidas (con su TIPO: `tags` sigue
/// siendo una lista, no un string) ni sobra ninguna que el motor añada por su cuenta.
#[test]
fn create_frontmatter_arbitrario() {
    let patch = FrontmatterPatch(
        [
            (
                "estado".to_string(),
                Some(Yaml::String("borrador".to_string())),
            ),
            (
                "tags".to_string(),
                Some(Yaml::Sequence(vec![
                    Yaml::String("a".to_string()),
                    Yaml::String("b".to_string()),
                ])),
            ),
        ]
        .into_iter()
        .collect(),
    );

    let raw = raw_creado("notas/nueva.md", Some(patch), Some("# Nueva\n"));
    let pf = model::parse_frontmatter(&raw).unwrap_or_else(|| {
        panic!("un `create` CON `frontmatter` debe escribir su bloque; documento =\n{raw}")
    });

    // 1) Las claves son EXACTAMENTE las pedidas: nada de `type`/`title` de propina.
    assert_eq!(
        claves(&pf),
        BTreeSet::from(["estado".to_string(), "tags".to_string()]),
        "el frontmatter debe llevar exactamente las claves pedidas por el llamador; \
         documento =\n{raw}",
    );

    // 2) Con sus tipos YAML reales (`§20.4`: sin coerción).
    assert_eq!(
        pf.get(&fp("estado")),
        Some(&Yaml::String("borrador".to_string())),
        "`estado` debe escribirse tal cual, como string; documento =\n{raw}",
    );
    let tags = pf
        .get(&fp("tags"))
        .unwrap_or_else(|| panic!("`tags` debe estar en el frontmatter; documento =\n{raw}"));
    assert_eq!(
        tags.as_sequence().map(Vec::len),
        Some(2),
        "`tags` debe seguir siendo una LISTA de 2 elementos, no un escalar aplanado ({tags:?}); \
         documento =\n{raw}",
    );
    assert_eq!(
        tags.as_sequence()
            .map(|s| s.iter().filter_map(Yaml::as_str).collect::<Vec<_>>()),
        Some(vec!["a", "b"]),
        "los elementos de `tags` deben ser `a` y `b`, en ese orden; documento =\n{raw}",
    );

    // 3) Y el cuerpo pedido sigue ahí, tras el bloque.
    assert!(
        raw.ends_with("# Nueva\n"),
        "el cuerpo pedido debe cerrar el documento; documento =\n{raw}",
    );
}

// ===========================================================================
// E24-H01 — Un BOM deja de tragarse el frontmatter
// (`requirements/epica-24-cierre-defectos-v031.md §E24-H01`, `ARCHITECTURE.md §20.4`,
// `CLAUDE.md` invariante #1). Fase ROJA.
//
// ## Síntoma (reproducido EJECUTANDO el binario `lodestar-mcp` por JSON-RPC, no leyendo código)
//
// Fichero en disco:
//     b'\xef\xbb\xbf---\nstatus: draft\nowner: ana\n---\n\n# Con BOM\n\ncuerpo original\n'
//
//   - `knowledge_get` → `frontmatter: {}` y `body` = el fichero ENTERO (bloque incluido).
//   - `document.has_frontmatter` = `false`.
//   - Y al escribir encima, `patch_frontmatter {"status":"review"}` publica en disco:
//         b'---\nstatus: review\n---\n\n\xef\xbb\xbf---\nstatus: draft\nowner: ana\n---\n\n# Con
//           BOM\n\ncuerpo original\n'
//     Dos bloques; el original degradado a texto del cuerpo. Un `replace_body` posterior
//     **destruye `owner: ana` y `status: draft` para siempre** — pérdida silenciosa de datos
//     mientras la validación responde VÁLIDO.
//
// ## Causa
//
// `model::split_front` (línea 57) hace `if !raw.starts_with("---") { return SplitFront::Sin }`;
// con BOM, `raw` empieza por `\u{feff}---` y cae en `Sin`. De ahí, la rama `SplitFront::Sin` de
// `patch_frontmatter` (líneas 367-382) antepone un bloque nuevo POR DELANTE del BOM.
//
// ## Qué NO se fija aquí (deliberado: no inventar API)
//
// Estos tests **no** tocan la firma de `model::build_raw`: la reemisión del BOM se ejerce por el
// camino real de escritura (`plan::apply_normalized_ops`, que es lo que materializa el `.md` que
// publica el único escritor), para que el implementador elija cómo propaga la marca —un campo en
// `ParsedFrontmatter`, un parámetro de `build_raw`, lo que sea— sin que la fase roja se lo dicte.
// Tampoco se exige que el patch sea quirúrgico (`reserialized == false`): sobre este fixture
// ambos caminos dan los mismos bytes, y el criterio es la conservación, no el camino.
//
// ## Rojo esperado HOY (por ASERCIÓN, no por compilación: no hace falta ningún stub)
//
//   - `bom_no_se_traga_el_frontmatter`  → `parse_file` devuelve `frontmatter: None`.
//   - `patch_sobre_bom_no_duplica_bloque` → el resultado empieza por `---`, no por el BOM.
//   - `bom_roundtrip_byte_a_byte` → el cuerpo leído ES el fichero entero (precondición) y el
//     patch duplica el bloque, así que ni los bytes ni la `WorkspaceRevision` se conservan.
// ===========================================================================

use lodestar_core::types::workspace_revision;

/// El BOM UTF-8 (`EF BB BF`) como `&str`.
const BOM: &str = "\u{feff}";

/// El documento del síntoma, byte a byte: BOM + un frontmatter válido de **tres** claves. La
/// segunda clave (`owner`) es la que la corrupción destruye, así que es el testigo del daño real.
///
/// **E31-H02 (`decisiones §26`)**: la tercera clave, `tags: [a, b]`, está escrita en estilo *flow*
/// **a propósito**. Sin ella el fixture estaba en forma canónica *block*, o sea exactamente la que
/// `serde_yaml` emite al reserializar, y `bom_roundtrip_byte_a_byte` pasaba **por casualidad**: un
/// `ReplaceBody` reserializaba el bloque entero y devolvía los mismos bytes de vuelta. Con `tags`
/// en *flow*, reserializar produce `tags:\n- a\n- b`, así que el round-trip solo cierra si el
/// camino de escritura **preserva los bytes** del bloque en vez de reconstruirlo. Es el mismo
/// defecto de método que originó `§26` (ver su «Nota de método»).
const DOC_CON_BOM: &str = concat!(
    "\u{feff}",
    "---\n",
    "status: draft\n",
    "owner: ana\n",
    "tags: [a, b]\n",
    "---\n",
    "\n",
    "# Con BOM\n",
    "\n",
    "cuerpo original\n",
);

/// El **mismo** documento sin el BOM. Es el oráculo del último criterio de la historia («un `.md`
/// sin BOM se parsea igual que en v0.3.0»): comparar contra él, y no contra literales copiados,
/// hace imposible que el arreglo del BOM se pague cambiando el parseo del caso normal.
const DOC_SIN_BOM: &str = concat!(
    "---\n",
    "status: draft\n",
    "owner: ana\n",
    "tags: [a, b]\n",
    "---\n",
    "\n",
    "# Con BOM\n",
    "\n",
    "cuerpo original\n",
);

/// Guarda de fixture y aserción de criterio a la vez: los tres primeros bytes son `EF BB BF`.
fn assert_empieza_por_bom(raw: &str, contexto: &str) {
    assert_eq!(
        raw.as_bytes().get(..3),
        Some([0xEF, 0xBB, 0xBF].as_slice()),
        "{contexto}: debe empezar por el BOM UTF-8 (EF BB BF) — el BOM es del usuario y se \
         conserva byte a byte, ni se strippea ni se desplaza. Primeros bytes: {:02X?}; \
         documento = {raw:?}",
        raw.as_bytes().get(..8),
    );
}

/// Número de líneas delimitadoras de frontmatter (`---`) del documento, tolerando el BOM en la
/// primera. Un documento con **un** bloque tiene exactamente dos; la corrupción de esta historia
/// produce cuatro.
fn delimitadores(raw: &str) -> usize {
    raw.lines()
        .filter(|l| l.trim_start_matches(BOM) == "---")
        .count()
}

/// Criterio `bom_no_se_traga_el_frontmatter` — **Dado** un `.md` con BOM y frontmatter válido,
/// **Cuando** se lee, **Entonces** `frontmatter` trae las claves reales y `has_frontmatter` es
/// `true`.
///
/// `document.has_frontmatter` se computa (`eval.rs:160`) como `doc.frontmatter.is_some()` sobre lo
/// que devuelve `model::parse_file`, así que aquí se juzga en su origen; por el wire lo fija el
/// test homónimo de `crates/lodestar-mcp/tests/mcp.rs`.
#[test]
fn bom_no_se_traga_el_frontmatter() {
    assert_empieza_por_bom(DOC_CON_BOM, "el fixture de la historia");

    let con = model::parse_file("bom.md", DOC_CON_BOM);
    let sin = model::parse_file("bom.md", DOC_SIN_BOM);

    // (a) El bloque está PRESENTE: `has_frontmatter` es `true`.
    assert!(
        con.fm_err.is_none(),
        "un BOM delante de un frontmatter perfectamente cerrado no es un error de parseo: \
         fm_err = {:?}",
        con.fm_err
    );
    let pf = con.frontmatter.as_ref().unwrap_or_else(|| {
        panic!(
            "un `.md` que empieza por BOM SÍ tiene frontmatter: `document.has_frontmatter` debe \
             ser `true`. Hoy `split_front` compara `raw.starts_with(\"---\")` sobre un raw que \
             empieza por `\\u{{feff}}---` y cae en `SplitFront::Sin`, así que la metadata del \
             usuario se vuelve invisible para el motor. Cuerpo interpretado = {:?}",
            con.body
        )
    });

    // (b) Las claves REALES, con su tipo YAML (`§20.4`: metadata arbitraria, sin coerción).
    assert_eq!(
        claves(pf),
        BTreeSet::from([
            "status".to_string(),
            "owner".to_string(),
            "tags".to_string()
        ]),
        "el frontmatter tras el BOM debe traer sus TRES claves reales, no el mapa vacío"
    );
    assert_eq!(
        pf.get(&fp("status")),
        Some(&Yaml::String("draft".to_string())),
        "`status` debe leerse como el string «draft»"
    );
    assert_eq!(
        pf.get(&fp("owner")),
        Some(&Yaml::String("ana".to_string())),
        "`owner` debe leerse como el string «ana»: es la clave que la corrupción destruye"
    );

    // (c) El cuerpo es el CUERPO, no el fichero entero: el bloque no se degrada a texto.
    assert_eq!(
        con.body, sin.body,
        "el BOM no cambia dónde empieza el cuerpo: debe ser exactamente el mismo que el del \
         documento sin BOM. Hoy el cuerpo se traga el bloque `---…---` entero"
    );
    assert!(
        !con.body.contains("status: draft"),
        "el frontmatter no puede aparecer dentro del cuerpo: cuerpo = {:?}",
        con.body
    );
    assert!(
        !con.body.starts_with(BOM),
        "el BOM pertenece a la cabecera del fichero, no al cuerpo: cuerpo = {:?}",
        con.body
    );

    // (d) Mismo veredicto que sin BOM: el BOM no cambia la INTERPRETACIÓN, solo los bytes.
    let pf_sin = sin
        .frontmatter
        .as_ref()
        .expect("guarda: el documento SIN BOM tiene frontmatter ya en v0.3.0");
    assert_eq!(
        pf.value, pf_sin.value,
        "el frontmatter interpretado debe ser idéntico con y sin BOM"
    );

    // (e) Los offsets siguen siendo posiciones de BYTE válidas sobre el raw CON BOM: la invariante
    // `raw[span] == fm.raw` no se rompe (alcance de la historia), que es lo que necesitan el patch
    // quirúrgico y los rangos de diagnóstico.
    assert_span_coherente(DOC_CON_BOM, pf);
    assert!(
        matches!(
            model::split_front(DOC_CON_BOM),
            model::SplitFront::Bloque { .. }
        ),
        "`split_front` debe reconocer el bloque tras el BOM: {:?}",
        model::split_front(DOC_CON_BOM)
    );
}

/// Criterio `patch_sobre_bom_no_duplica_bloque` — **Dado** ese documento, **Cuando** se le aplica
/// `patch_frontmatter`, **Entonces** el resultado tiene **un solo** bloque, conserva las claves que
/// no se tocaron y **empieza por el BOM**.
#[test]
fn patch_sobre_bom_no_duplica_bloque() {
    let res = parchear(DOC_CON_BOM, &parche(&[("status", s("review"))]));

    // (a) El resultado EMPIEZA POR EL BOM: nada se antepone al BOM del usuario.
    assert_empieza_por_bom(
        &res.raw,
        "el documento parcheado (hoy se le antepone un bloque nuevo POR DELANTE del BOM)",
    );

    // (b) UN SOLO bloque: dos delimitadores, no cuatro.
    assert_eq!(
        delimitadores(&res.raw),
        2,
        "el documento parcheado debe tener UN solo bloque de frontmatter (2 líneas «---»); la \
         corrupción de esta historia produce 4 y deja el bloque original como texto del cuerpo. \
         Documento resultante:\n{:?}",
        res.raw
    );

    // (c) Las claves NO tocadas se conservan, y la tocada toma el valor nuevo.
    let pf = model::parse_frontmatter(&res.raw).unwrap_or_else(|| {
        panic!(
            "el documento parcheado debe tener un bloque de frontmatter legible:\n{:?}",
            res.raw
        )
    });
    assert_eq!(
        claves(&pf),
        BTreeSet::from([
            "status".to_string(),
            "owner".to_string(),
            "tags".to_string()
        ]),
        "el patch toca `status` y NADA más: `owner` y `tags` deben seguir en el bloque. Documento \
         resultante:\n{:?}",
        res.raw
    );
    assert_eq!(
        pf.get(&fp("status")),
        Some(&Yaml::String("review".to_string())),
        "`status` debe tomar el valor nuevo. Documento resultante:\n{:?}",
        res.raw
    );
    assert_eq!(
        pf.get(&fp("owner")),
        Some(&Yaml::String("ana".to_string())),
        "`owner: ana` es la clave que hoy se pierde: parchear `status` no puede degradarla a \
         texto del cuerpo. Documento resultante:\n{:?}",
        res.raw
    );

    // (d) El cuerpo queda intacto byte a byte (el mismo que el del documento sin BOM: el BOM no
    // desplaza dónde empieza el cuerpo).
    let re = model::parse_file("bom.md", &res.raw);
    assert_eq!(
        re.body,
        model::parse_file("bom.md", DOC_SIN_BOM).body,
        "el cuerpo debe llegar intacto al resultado del patch. Documento resultante:\n{:?}",
        res.raw
    );

    // (e) Y el documento resultante se vuelve a leer entero, sin pérdidas: un `replace_body`
    // posterior (el segundo paso del síntoma) ya no tendría nada que destruir.
    let refm = re
        .frontmatter
        .as_ref()
        .expect("el documento parcheado debe volver a leerse CON frontmatter");
    assert_eq!(
        claves(refm),
        BTreeSet::from([
            "status".to_string(),
            "owner".to_string(),
            "tags".to_string()
        ]),
        "releer el resultado debe dar las tres claves: si el patch dejó dos bloques, el segundo \
         (con `owner`) es texto muerto que la siguiente escritura borra para siempre. Documento \
         resultante:\n{:?}",
        res.raw
    );
}

/// Criterio `bom_roundtrip_byte_a_byte` (**control anti-vacuo de la historia**) — **Dado** ese
/// documento, **Cuando** se lee y se reescribe sin cambios, **Entonces** los bytes son idénticos y
/// la `WorkspaceRevision` no cambia.
///
/// Sin este criterio, un arreglo que se limitase a **strippear** el BOM al leer pasaría los otros
/// dos: aquí no, porque `types::workspace_revision` hashea los bytes CRUDOS del `FileMap`
/// (`types.rs:1196`), así que strippear al leer y no al reemitir declararía un cambio espurio en
/// cada round-trip — exactamente lo que el alcance de la historia prohíbe.
///
/// Se ejercen las **dos** rutas de escritura que el motor usa de verdad, ambas por
/// `plan::apply_normalized_ops` (lo que materializa el `.md` que publica el único escritor):
///
///   - **A. `replace_body`** con el cuerpo tal y como se acaba de leer. Es la ruta que obliga a
///     **reemitir** el BOM: reconstruye el documento desde `(frontmatter, cuerpo)`.
///   - **B. `patch_frontmatter`** escribiendo en una clave el valor que **ya** tenía. Es la ruta
///     que obliga a **conservarlo**: hoy antepone un bloque y cambia los bytes.
///
/// La ruta A necesita una **precondición explícita** para no ser vacua: si el cuerpo leído fuese
/// el fichero entero (el defecto de hoy), reescribirlo daría los mismos bytes de casualidad. Por
/// eso se asevera primero que lo leído es el cuerpo de verdad.
///
/// > **E31-H02 (`decisiones §26`)** — este test pasaba **por la razón equivocada**: `DOC_CON_BOM`
/// > estaba en forma canónica *block*, así que la ruta A reserializaba el bloque y le salían los
/// > mismos bytes de casualidad. El fixture lleva ahora `tags: [a, b]` en estilo *flow* (ver su
/// > doc), que reserializar convierte en `tags:\n- a\n- b`: la igualdad byte a byte de la ruta A
/// > deja de ser un accidente y pasa a exigir de verdad que `ReplaceBody` **no** reconstruya el
/// > bloque. Por eso este test es hoy ROJO y forma parte de la fase roja de E31-H02.
#[test]
fn bom_roundtrip_byte_a_byte() {
    let path = rp("bom.md");
    let files: FileMap = [(path.clone(), DOC_CON_BOM.to_string())]
        .into_iter()
        .collect();
    let rev_antes = workspace_revision(&files, &[]);
    let doc_set = DocumentSet::from_files(files.clone());

    // --- Ruta A: leer el cuerpo y volver a escribirlo TAL CUAL.
    let leido = model::parse_file(path.as_str(), DOC_CON_BOM);
    // Precondición anti-vacua de la ruta A (ver el rustdoc): lo leído debe ser el CUERPO.
    assert!(
        !leido.body.starts_with(BOM) && !leido.body.contains("status: draft"),
        "precondición del round-trip: el cuerpo leído debe ser el cuerpo, no el fichero entero — \
         si no, reescribirlo devolvería los mismos bytes por accidente y el criterio sería vacuo. \
         Cuerpo leído = {:?}",
        leido.body
    );
    let op_a = plan::normalize_replace_body(&doc_set, &path, leido.body.clone())
        .expect("reescribir el cuerpo de un documento existente no debe fallar la normalización");
    let despues_a = plan::apply_normalized_ops(&files, &[op_a])
        .expect("aplicar en memoria un `replace_body` no debe fallar");
    let raw_a = despues_a
        .get(&path)
        .expect("el documento debe seguir existiendo tras el `replace_body`");
    assert_empieza_por_bom(raw_a, "el documento reescrito por `replace_body`");
    assert_eq!(
        raw_a.as_str(),
        DOC_CON_BOM,
        "leer y reescribir el cuerpo sin cambios debe devolver los MISMOS bytes: el BOM se reemite \
         y ni el bloque ni el cuerpo se reordenan"
    );
    assert_eq!(
        workspace_revision(&despues_a, &[]),
        rev_antes,
        "un round-trip sin cambios no puede mover la `WorkspaceRevision`: declararía un cambio \
         espurio (y con él, conflictos de escritura) en cada lectura-escritura de un `.md` con BOM"
    );

    // --- Ruta B: reescribir una clave con el valor que YA tenía.
    let op_b =
        plan::normalize_patch_frontmatter(&doc_set, &path, parche(&[("status", s("draft"))]))
            .expect("parchear un documento existente no debe fallar la normalización");
    let despues_b = plan::apply_normalized_ops(&files, &[op_b])
        .expect("aplicar en memoria un `patch_frontmatter` no debe fallar");
    let raw_b = despues_b
        .get(&path)
        .expect("el documento debe seguir existiendo tras el `patch_frontmatter`");
    assert_empieza_por_bom(raw_b, "el documento reescrito por `patch_frontmatter`");
    assert_eq!(
        raw_b.as_str(),
        DOC_CON_BOM,
        "escribir en `status` el valor que ya tenía no cambia el documento: mismos bytes, BOM \
         incluido"
    );
    assert_eq!(
        workspace_revision(&despues_b, &[]),
        rev_antes,
        "un patch que no cambia ningún valor no puede mover la `WorkspaceRevision`"
    );
}

// ===========================================================================
// E24-H01 (cobertura) — El BOM de un documento SIN frontmatter
// (`requirements/epica-24-cierre-defectos-v031.md §E24-H01`, `ARCHITECTURE.md §20.4`,
// `CLAUDE.md` invariante #1).
//
// Los tres tests de arriba usan un fixture CON bloque, así que entran por el brazo
// `SplitFront::Bloque` y dejan sin red las dos ramas que sirven al documento **sin** bloque —
// justo la familia del defecto de E23-H03 (el `README.md` sin frontmatter que se corrompe al
// reescribirle el cuerpo), ahora con el BOM como dato a conservar. Un juez ciego lo confirmó
// mutando la implementación: las dos ramas de abajo sobrevivían con la suite ENTERA en verde.
//
// Hueco 1 — `model::build_raw_with_bom` con `fm = None`: sustituir
// `return format!("{prefijo}{body}")` por `return body.to_string()` (es decir, que un
// `replace_body` sobre un `.md` con BOM y sin bloque se coma la marca) no rompía ningún test.
// Lo cubre `move_no_se_come_el_bom_del_enlazante_sin_frontmatter`.
//
// Hueco 2 — brazo `SplitFront::Sin` de `model::patch_frontmatter`: revertirlo a
// `format!("---\n{bloque}\n---\n\n{raw}")` (bloque nuevo POR DELANTE del BOM, marca de
// codificación en mitad del fichero) tampoco rompía ningún test. Lo cubre
// `patch_sobre_bom_sin_bloque_no_precede_a_la_marca`.
//
// Ambos llevan un **gemelo sin BOM** como oráculo: el resultado con BOM debe ser exactamente el
// resultado sin BOM más la marca delante. Así ninguna de las dos mutaciones opuestas pasa — ni
// tragarse el BOM, ni emitirlo donde no lo había.
// ===========================================================================

/// El enlazante del escenario real: un `README.md` con BOM, **sin frontmatter**, que enlaza a otro
/// documento. Es el fichero que `move --rewriteInboundLinks` reescribe en cadena.
const ENLAZANTE_CON_BOM: &str = concat!("\u{feff}", "# Notas\n", "\n", "[a](x.md)\n");

/// Su gemelo exacto sin la marca: el oráculo diferencial (mismo resultado, un BOM menos).
const ENLAZANTE_SIN_BOM: &str = concat!("# Notas\n", "\n", "[a](x.md)\n");

/// Documento con BOM y **sin** bloque de frontmatter, para el patch que tiene que crear uno.
const DOC_CON_BOM_SIN_FRONTMATTER: &str = concat!("\u{feff}", "# Notas\n", "\n", "cuerpo\n");

/// Su gemelo sin la marca.
const DOC_SIN_BOM_SIN_FRONTMATTER: &str = concat!("# Notas\n", "\n", "cuerpo\n");

/// Cobertura del hueco 1 — **Dado** un `.md` con BOM y **sin** frontmatter que enlaza a otro
/// documento, **Cuando** se mueve el documento enlazado con `rewriteInboundLinks`, **Entonces** el
/// enlazante conserva su BOM byte a byte (y sigue sin frontmatter).
///
/// Es el camino por el que el daño se **propaga**: `normalize_move` emite un `ReplaceBody` por
/// **cada** documento entrante, así que un solo `move` reescribe el cuerpo de medio workspace. Si
/// `build_raw_with_bom` no reemitiera la marca cuando el documento no tiene bloque, ese `move`
/// cambiaría en silencio la codificación de todos los enlazantes sin frontmatter y movería la
/// `WorkspaceRevision` sin que nadie lo pidiera.
///
/// El gemelo `LEEME.md` (sin BOM, mismo cuerpo) es el control anti-vacuo en la otra dirección: el
/// arreglo no puede consistir en emitir el BOM siempre.
#[test]
fn move_no_se_come_el_bom_del_enlazante_sin_frontmatter() {
    assert_empieza_por_bom(ENLAZANTE_CON_BOM, "el fixture del enlazante");
    let files = mapa(&[
        ("README.md", ENLAZANTE_CON_BOM),
        ("LEEME.md", ENLAZANTE_SIN_BOM),
        ("x.md", "---\ntitle: X\n---\n\n# X\n"),
    ]);
    // Guarda del fixture: el enlazante NO tiene bloque de frontmatter (si lo tuviera, el test
    // entraría por el brazo `Bloque`, que ya está cubierto, y no probaría nada nuevo).
    assert!(
        model::parse_file("README.md", ENLAZANTE_CON_BOM)
            .frontmatter
            .is_none(),
        "guarda del fixture: el enlazante debe ser un `.md` SIN bloque de frontmatter"
    );

    let doc_set = DocumentSet::from_files(files.clone());
    let ops = plan::normalize_move(&doc_set, &rp("x.md"), &rp("docs/x.md"), true)
        .expect("mover un documento existente no debe fallar la normalización");
    let despues = plan::apply_normalized_ops(&files, &ops)
        .expect("aplicar en memoria un `move` con reescritura de entrantes no debe fallar");

    let readme = despues
        .get(&rp("README.md"))
        .expect("el enlazante debe seguir existiendo tras el `move`");
    let leeme = despues
        .get(&rp("LEEME.md"))
        .expect("el enlazante gemelo debe seguir existiendo tras el `move`");

    // (a) Guarda de no vacuidad: el `move` REESCRIBIÓ de verdad los dos enlazantes. Sin esto, un
    // `move` que no tocara nada dejaría el BOM intacto por accidente.
    assert!(
        readme.contains("docs/x.md") && leeme.contains("docs/x.md"),
        "guarda: `rewriteInboundLinks` debe haber reapuntado el enlace de AMBOS enlazantes a \
         `docs/x.md` — si no, el criterio sería vacuo.\nREADME = {readme:?}\nLEEME  = {leeme:?}"
    );

    // (b) El BOM sobrevive a la reescritura del cuerpo.
    assert_empieza_por_bom(
        readme,
        "el enlazante reescrito por el `move` (reescribir su cuerpo no puede comerse su BOM)",
    );

    // (c) …y el documento sigue SIN bloque de frontmatter: no se le inventa uno (E23-H03) ni se le
    // cuela nada entre la marca y el cuerpo.
    assert_eq!(
        delimitadores(readme),
        0,
        "un documento sin frontmatter sigue sin frontmatter tras el `move`: no se le inyecta un \
         bloque.\nREADME = {readme:?}"
    );
    assert!(
        model::parse_file("README.md", readme).frontmatter.is_none(),
        "releer el enlazante reescrito no puede dar un bloque de frontmatter: {readme:?}"
    );

    // (d) Oráculo diferencial: el resultado con BOM es EXACTAMENTE el resultado sin BOM más la
    // marca delante. Cierra las dos mutaciones opuestas de una vez (comerse el BOM y emitirlo
    // donde no lo había).
    assert!(
        !leeme.starts_with(BOM),
        "el enlazante que NO tenía BOM no puede ganar uno: {leeme:?}"
    );
    assert_eq!(
        readme.as_str(),
        format!("{BOM}{leeme}"),
        "el enlazante con BOM debe quedar igual que su gemelo sin BOM, más la marca delante"
    );

    // (e) La misma exigencia por la ruta desnuda (`replace_body` directo), que es en lo que
    // `move`, `replace_text`, `edit_section` y `delete remove_links` se normalizan todos.
    let cuerpo = model::parse_file("README.md", ENLAZANTE_CON_BOM).body;
    let op = plan::normalize_replace_body(&doc_set, &rp("README.md"), cuerpo)
        .expect("reescribir el cuerpo de un documento existente no debe fallar la normalización");
    let tras_replace = plan::apply_normalized_ops(&files, &[op])
        .expect("aplicar en memoria un `replace_body` no debe fallar");
    let readme_replace = tras_replace
        .get(&rp("README.md"))
        .expect("el documento debe seguir existiendo tras el `replace_body`");
    assert_eq!(
        readme_replace.as_str(),
        ENLAZANTE_CON_BOM,
        "leer y reescribir el cuerpo de un `.md` con BOM y SIN frontmatter debe devolver los \
         MISMOS bytes"
    );
    assert_eq!(
        workspace_revision(&tras_replace, &[]),
        workspace_revision(&files, &[]),
        "un round-trip sin cambios sobre un documento sin frontmatter no puede mover la \
         `WorkspaceRevision`"
    );
}

/// Cobertura del hueco 2 — **Dado** un `.md` con BOM y **sin** bloque de frontmatter, **Cuando** se
/// le aplica `patch_frontmatter`, **Entonces** el bloque nuevo se inserta **después** de la marca,
/// no por delante.
///
/// El brazo `SplitFront::Sin` es el que crea el bloque donde no había ninguno. Anteponerlo al BOM
/// deja la marca de codificación en mitad del fichero: deja de ser un BOM (que por definición es el
/// primer byte) y se convierte en un carácter invisible dentro del cuerpo, que además vuelve
/// ilegible el propio bloque que se acaba de escribir para cualquier lector que sí respete la
/// marca.
#[test]
fn patch_sobre_bom_sin_bloque_no_precede_a_la_marca() {
    assert_empieza_por_bom(DOC_CON_BOM_SIN_FRONTMATTER, "el fixture sin frontmatter");
    // Guarda del fixture: hoy no hay bloque, así que el patch entra por el brazo `Sin`.
    assert!(
        matches!(
            model::split_front(DOC_CON_BOM_SIN_FRONTMATTER),
            model::SplitFront::Sin
        ),
        "guarda del fixture: el documento no debe tener bloque de frontmatter, para que el patch \
         entre por el brazo que lo CREA"
    );

    let res = parchear(
        DOC_CON_BOM_SIN_FRONTMATTER,
        &parche(&[("status", s("draft"))]),
    );
    // El mismo patch sobre el gemelo sin la marca: el oráculo de todo lo que sigue.
    let gemelo = parchear(
        DOC_SIN_BOM_SIN_FRONTMATTER,
        &parche(&[("status", s("draft"))]),
    );

    // (a) El resultado empieza por la marca: nada se antepone al BOM del usuario.
    assert_empieza_por_bom(
        &res.raw,
        "el documento parcheado (el bloque nuevo va DESPUÉS de la marca, nunca por delante)",
    );

    // (b) …y la marca aparece UNA sola vez: no queda un BOM huérfano en mitad del fichero.
    assert_eq!(
        res.raw.matches(BOM).count(),
        1,
        "el BOM debe aparecer exactamente una vez, y en el primer byte: si el bloque se antepone, \
         la marca queda enterrada dentro del documento.\nDocumento resultante = {:?}",
        res.raw
    );

    // (c) El bloque creado es legible y trae la clave del patch; el cuerpo llega intacto y SIN la
    // marca (que pertenece a la cabecera del fichero).
    let re = model::parse_file("bom.md", &res.raw);
    let pf = re.frontmatter.as_ref().unwrap_or_else(|| {
        panic!(
            "el documento parcheado debe tener un bloque de frontmatter legible:\n{:?}",
            res.raw
        )
    });
    assert_eq!(
        pf.get(&fp("status")),
        Some(&Yaml::String("draft".to_string())),
        "el bloque creado debe traer la clave del patch.\nDocumento resultante = {:?}",
        res.raw
    );
    assert_eq!(
        re.body,
        model::parse_file("bom.md", &gemelo.raw).body,
        "el cuerpo debe llegar intacto y sin la marca: si el bloque se antepusiera al BOM, el \
         cuerpo empezaría por él.\nDocumento resultante = {:?}",
        res.raw
    );
    assert_span_coherente(&res.raw, pf);

    // (d) Oráculo diferencial: mismo patch sobre el gemelo sin BOM ⇒ mismos bytes, una marca menos.
    assert!(
        !gemelo.raw.starts_with(BOM),
        "el documento que NO tenía BOM no puede ganar uno: {:?}",
        gemelo.raw
    );
    assert_eq!(
        res.raw,
        format!("{BOM}{}", gemelo.raw),
        "el documento con BOM debe quedar igual que su gemelo sin BOM, más la marca delante"
    );
}

// ---------------------------------------------------------------------------
// E24-H02 — Un BOM es VISIBLE, no silencioso
//
// H01 hizo que el BOM dejara de tragarse el frontmatter, pero un `.md` con BOM sigue sin producir
// ni un diagnóstico: `knowledge_check` responde VÁLIDO con 0 avisos sobre un fichero cuya marca de
// codificación no es portable. Es el mismo problema de portabilidad que `LINK-CASE-MISMATCH`
// (capitalización que solo funciona en un tipo de volumen), y se trata igual: **aviso**, no error.
//
// Criterio de severidad (decisión propia de la historia): `Warn` y NO configurable. `§20.9` fija
// cinco familias reclasificables desde `validation:` y esta no es una de ellas; añadir una sexta
// sería ampliar el contrato de config, no arreglar un defecto. Queda como `LINK-ESCAPES-WORKSPACE`
// o `SYMLINK-UNSUPPORTED`: severidad intrínseca (`family_of` devuelve `None`).
// ---------------------------------------------------------------------------

/// **E24-H02** — un `.md` con BOM emite el aviso de portabilidad y NO bloquea.
///
/// La pareja de aserciones importa: si fuera `Err`, un `lodestar check` sobre cualquier repo con un
/// fichero guardado por Notepad saldría con exit 1, que es exactamente el tipo de falso positivo
/// que `§20.9` quiere evitar («solo lo que impide interpretar o modificar con seguridad»).
#[test]
fn bom_emite_aviso_sin_bloquear() {
    let con_bom = format!("{BOM}---\nstatus: draft\n---\n\n# Con BOM\n");
    let ds = DocumentSet::from_files(mapa(&[("bom.md", &con_bom)]));
    let a = ds.analyze();

    let cs = codigos(a, &rp("bom.md"));
    assert!(
        cs.iter().any(|c| c == "DOC-BOM"),
        "un `.md` que empieza por BOM UTF-8 debe emitir el aviso de portabilidad `DOC-BOM`; \
         hoy no emite nada y `knowledge_check` responde VÁLIDO sobre él. Códigos: {cs:?}"
    );

    let bom = a
        .diagnostics
        .get(&rp("bom.md"))
        .into_iter()
        .flatten()
        .find(|c| c.code.as_str() == "DOC-BOM")
        .expect("el diagnóstico DOC-BOM debe estar presente");

    assert_eq!(
        bom.level,
        lodestar_core::types::Severity::Warn,
        "el BOM es un problema de PORTABILIDAD, no de interpretabilidad: es aviso, no error. \
         Con `Err`, `lodestar check` tumbaría cualquier repo con un fichero guardado por Notepad"
    );
    assert_eq!(
        bom.targets,
        vec![rp("bom.md")],
        "`targets` es el documento que lleva la marca (el que hay que editar si molesta)"
    );
    assert_eq!(
        a.hard_fail(),
        0,
        "un BOM no puede tumbar la puerta de CI: el workspace sigue siendo válido"
    );

    // El frontmatter se sigue leyendo (no se ha roto lo que arregló H01).
    assert!(
        !cs.iter()
            .any(|c| c == "FM-UNCLOSED" || c == "FM-YAML-INVALID"),
        "el aviso de BOM no puede venir acompañado de un diagnóstico de frontmatter ilegible: \
         H01 hizo que el bloque se interprete bien. Códigos: {cs:?}"
    );
}

/// **E24-H02** — control anti-vacuo: sin BOM no hay aviso.
///
/// Sin esto, una implementación que emitiera `DOC-BOM` para todo documento pasaría el test de
/// arriba.
#[test]
fn sin_bom_no_hay_aviso() {
    let sin_bom = "---\nstatus: draft\n---\n\n# Sin BOM\n";
    let ds = DocumentSet::from_files(mapa(&[("limpio.md", sin_bom)]));
    let a = ds.analyze();

    let cs = codigos(a, &rp("limpio.md"));
    assert!(
        !cs.iter().any(|c| c == "DOC-BOM"),
        "un `.md` SIN BOM no puede emitir `DOC-BOM`: {cs:?}"
    );
    assert!(
        cs.is_empty(),
        "un documento limpio no diagnostica nada: {cs:?}"
    );
}

/// **E24-H02** — el aviso también aparece en un documento con BOM y SIN frontmatter.
///
/// La marca es del fichero, no del bloque: no puede depender de que haya frontmatter. Cierra por
/// delante el hueco de cobertura que el juez de H01 encontró por mutación en la familia gemela.
#[test]
fn bom_sin_frontmatter_tambien_avisa() {
    let con_bom = format!("{BOM}# Solo cuerpo\n\ntexto\n");
    let ds = DocumentSet::from_files(mapa(&[("desnudo.md", &con_bom)]));
    let a = ds.analyze();

    let cs = codigos(a, &rp("desnudo.md"));
    assert!(
        cs.iter().any(|c| c == "DOC-BOM"),
        "el aviso es del FICHERO (su codificación), no del bloque de frontmatter: un `.md` con \
         BOM y sin frontmatter también debe avisarlo. Códigos: {cs:?}"
    );
}

// ===========================================================================
// E31-H02 — El frontmatter no se reserializa cuando nadie pidió tocarlo
// (`requirements/epica-31-seguimientos-campana.md §E31-H02`,
// `decisiones/26-replace-text-noop-reserializa.md §26`, `ARCHITECTURE.md §20.4`,
// `CLAUDE.md` invariante #3 —una sola verdad de patcheo—). Fase ROJA.
//
// ## El defecto
//
// El brazo `ReplaceBody` de `plan::apply_one` (`plan.rs:1262-1279`) reconstruye el documento con
// `model::build_raw_with_bom`, que **serializa `fm.value` e ignora `fm.raw`**. O sea: reescribir el
// CUERPO reformatea la CABECERA, que nadie pidió tocar. Reproducido ejecutando el core:
//
//     entrada: "---\n# comentario\ntags: [a, b]\ntitle: \"Con comillas\"\n---\n\n# H\n\nviejo\n"
//     salida : "---\ntags:\n- a\n- b\ntitle: Con comillas\n---\n\n# H\n\nviejo\n"
//
// Se pierden el comentario YAML, el estilo *flow* de `tags` y las comillas de `title`. Es
// exactamente lo que E16-H04 arregló para `patch_frontmatter` (patch quirúrgico, `reserialized`) y
// que este brazo hermano nunca recibió.
//
// ## El radio, que es mucho mayor que el síntoma
//
// TODO lo que normaliza a `ReplaceBody` comparte el defecto: `replace_text` (`plan.rs:533`),
// `edit_section` (`:581`), `replace_body` (`:638`), `move` —incluido `rewriteInboundLinks`, que
// reescribe el cuerpo de CADA enlazante (`:1021`, `:1045`)— y `delete remove_links` (`:1100`). Un
// solo `move` puede hoy reformatear el frontmatter de medio workspace.
//
// ## Dos defectos más de la misma familia, hoy sin test
//
//   - **Línea en blanco inyectada**: `build_raw_with_bom` normaliza el separador a `---\n\n`, así
//     que un `.md` escrito `---\n…\n---\ncuerpo` vuelve con un `\n` de más.
//   - **Frontmatter ilegible BORRADO (pérdida de datos)**: `parse_file` devuelve
//     `frontmatter: None` tanto para «no hay bloque» como para «hay bloque con YAML inválido», así
//     que un `ReplaceBody` sobre un documento con el bloque roto **elimina el bloque entero del
//     usuario**. Es la trampa que el rustdoc de `patch_frontmatter` (`model.rs:420-424`) documenta
//     y esquiva; este brazo cae en ella.
//
// ## Qué fija esta fase roja (y qué NO)
//
// El criterio es **la conservación de bytes**, no el camino: los tests se escriben contra
// `plan::apply_normalized_ops` (lo que materializa el `.md` que publica el único escritor) y NO
// contra la función nueva que el alcance propone
// (`model::replace_body_preservando_cabecera(raw, body)`), para que el implementador elija cómo
// la estructura sin que la fase roja se lo dicte. No hace falta ningún stub: el rojo es POR
// ASERCIÓN, no por compilación.
//
// ## El caso `SinCerrar`, razonado (lo pide el alcance)
//
// Con el bloque **abierto y nunca cerrado**, `SplitFront::body_offset` vale `bom_len(raw)`, así que
// `SplitFront::body` devuelve el documento ENTERO —el bloque incluido, degradado a texto del
// cuerpo—. El comportamiento coherente, y el que la simetría lectura/escritura del splice produce
// sola, es:
//
//   - una operación que reescribe el cuerpo **derivándolo del que leyó** (`replace_text`,
//     `edit_section`, `move`, `delete remove_links`) conserva el bloque, porque el bloque viaja
//     DENTRO de ese cuerpo. Es lo que se asevera abajo, y lo que hoy ya ocurre: aquí el test es
//     **anti-regresión** (el arreglo no puede romperlo), no rojo.
//   - un `replace_body` desnudo, en cambio, sustituye el documento entero — y debe hacerlo: el
//     llamador pidió reemplazar exactamente lo que `knowledge_get` le devolvió como cuerpo, que
//     en este documento es todo el fichero. Fingir una cabecera que el motor no sabe leer sería
//     inventarse un corte que `SplitFront` no reconoce.
//
// El caso que SÍ es rojo hoy es su gemelo `Bloque` + YAML inválido: ahí el corte existe, el bloque
// está fuera del cuerpo, y el brazo lo borra.
//
// ## Rojo esperado HOY (todo por aserción)
//
//   - `replace_body_preserva_frontmatter_flow`             → `tags: [a, b]` vuelve en block style.
//   - `replace_body_no_inyecta_linea_en_blanco`            → un `\n` de más tras el `---`.
//   - `replace_body_no_borra_frontmatter_ilegible`         → el bloque con YAML inválido desaparece.
//   - `edit_section_preserva_la_cabecera`                  → (familia 1/4).
//   - `move_con_reescritura_de_entrantes_preserva_la_cabecera` → (familia 2/4, el radio grande).
//   - `delete_remove_links_preserva_la_cabecera`           → (familia 3/4).
//   - `replace_text_sin_coincidencias_no_toca_un_byte`     → (familia 4/4, el síntoma de la ficha).
//   - `preservar_la_cabecera_no_mueve_la_revision`         → la `WorkspaceRevision` avanza sin cambio.
//   - `bom_roundtrip_byte_a_byte` (arriba)                 → con el fixture ya en *flow*, ruta A.
// ===========================================================================

use lodestar_core::types::{EditSectionMode, InboundLinksPolicy};

/// El documento del síntoma: frontmatter en estilo **flow** (`tags: [a, b]`), con un **comentario
/// YAML** y un valor **entrecomillado**. Los tres rasgos son texto del usuario que la
/// reserialización destruye, y ninguno sobrevive a un `serde_yaml::to_string`.
const DOC_FLOW: &str = concat!(
    "---\n",
    "# el porqué de estas etiquetas\n",
    "tags: [a, b]\n",
    "title: \"Con comillas\"\n",
    "status: draft\n",
    "---\n",
    "\n",
    "# Documento\n",
    "\n",
    "cuerpo viejo\n",
);

/// Documento escrito **sin línea en blanco** tras el `---` de cierre. Es una forma real (hay
/// fixtures así en `crates/lodestar-mcp/tests/e2e_migracion.rs`), y `build_raw_with_bom` la
/// normaliza a `---\n\n`, o sea le inyecta un `\n` que el usuario no escribió.
const DOC_SEPARADOR_PEGADO: &str = concat!("---\n", "title: X\n", "---\n", "# H\n");

/// La cabecera de `raw` según el corte del propio core: `raw[..body_offset]`. Es el prefijo que
/// una operación de CUERPO no puede tocar, expresado con la misma verdad (`model::split_front`)
/// que usa la lectura — y no con un literal copiado, que se desincronizaría del fixture.
fn cabecera(raw: &str) -> &str {
    &raw[..model::split_front(raw).body_offset(raw)]
}

/// Aplica `ops` sobre `files` y devuelve el `.md` completo de `path` tras aplicarlas.
fn tras_aplicar(
    files: &FileMap,
    ops: &[lodestar_core::types::NormalizedOperation],
    path: &RelPath,
) -> String {
    plan::apply_normalized_ops(files, ops)
        .expect("aplicar en memoria un change set normalizado no debe fallar")
        .get(path)
        .cloned()
        .unwrap_or_else(|| {
            panic!(
                "`{}` debe seguir existiendo tras la operación",
                path.as_str()
            )
        })
}

/// Criterio `replace_body_preserva_frontmatter_flow` — **Dado** un documento con frontmatter en
/// estilo *flow*, con comentario YAML y comillas, **Cuando** se le cambia **solo el cuerpo**,
/// **Entonces** la cabecera queda **byte a byte idéntica** y el cuerpo es el nuevo (E31-H02, §26).
///
/// La aserción fuerte es la igualdad byte a byte de `raw[..body_offset]`, no «que `tags` siga
/// siendo una lista»: lo que se pierde hoy no son datos sino **texto del usuario** (formato,
/// comentarios, comillas), y solo la comparación literal lo detecta.
///
/// **Anti-vacuo obligatorio**: se asevera además que el cuerpo **SÍ cambió**. Sin esa guarda, un
/// `apply_one` que no escribiera nunca nada —o un fixture cuyo cuerpo nuevo fuese el viejo— pasaría
/// este test sin probar absolutamente nada.
#[test]
fn replace_body_preserva_frontmatter_flow() {
    let path = rp("flow.md");
    let files = mapa(&[("flow.md", DOC_FLOW)]);

    // Guarda del fixture: el bloque está en una forma que NO es la canónica de `serde_yaml`. Si
    // lo estuviera, reserializar devolvería los mismos bytes y el criterio sería vacuo (es
    // exactamente el defecto de método que originó `§26`).
    let bloque_original = cabecera(DOC_FLOW);
    assert!(
        bloque_original.contains("tags: [a, b]")
            && bloque_original.contains("# el porqué")
            && bloque_original.contains("\"Con comillas\""),
        "guarda del fixture: la cabecera debe llevar estilo flow, comentario YAML y comillas — los \
         tres rasgos que la reserialización destruye. Cabecera = {bloque_original:?}"
    );

    let doc_set = DocumentSet::from_files(files.clone());
    let cuerpo_nuevo = "\n# Documento\n\ncuerpo NUEVO\n".to_string();
    let op = plan::normalize_replace_body(&doc_set, &path, cuerpo_nuevo.clone())
        .expect("reescribir el cuerpo de un documento existente no debe fallar la normalización");
    let resultado = tras_aplicar(&files, &[op], &path);

    // (a) ANTI-VACUO: el cuerpo SÍ cambió. Sin esto, un no-op universal pasaría el test.
    let releido = model::parse_file(path.as_str(), &resultado);
    assert_eq!(
        releido.body, cuerpo_nuevo,
        "anti-vacuo: la operación debe haber escrito el cuerpo NUEVO. Si el cuerpo no cambia, la \
         igualdad de la cabecera no prueba nada (cualquier implementación que no escriba nunca \
         pasaría).\nDocumento resultante = {resultado:?}"
    );
    assert_ne!(
        releido.body,
        model::parse_file(path.as_str(), DOC_FLOW).body,
        "anti-vacuo: el cuerpo nuevo debe ser DISTINTO del viejo"
    );

    // (b) El criterio: la cabecera —bloque, delimitadores y separador— es la MISMA, byte a byte.
    assert_eq!(
        cabecera(&resultado),
        bloque_original,
        "reescribir el CUERPO no puede reformatear la CABECERA: nadie pidió tocar el frontmatter. \
         Hoy `apply_one` reconstruye el documento con `build_raw_with_bom`, que serializa \
         `fm.value` e ignora `fm.raw`, así que se pierden el comentario YAML, el estilo flow de \
         `tags` y las comillas de `title` (§26).\n  esperado = {bloque_original:?}\n  \
         obtenido = {:?}",
        cabecera(&resultado)
    );

    // (c) …y por tanto el documento entero es exactamente «cabecera original + cuerpo nuevo».
    assert_eq!(
        resultado,
        format!("{bloque_original}{cuerpo_nuevo}"),
        "el documento resultante debe ser el splice exacto: los bytes de la cabecera original más \
         el cuerpo pedido, sin nada más en medio"
    );

    // (d) Los datos, además del texto, siguen ahí (por si alguien «arreglase» esto vaciando el
    // bloque): las tres claves con sus tipos YAML.
    let pf = model::parse_frontmatter(&resultado).unwrap_or_else(|| {
        panic!("el documento resultante debe seguir teniendo frontmatter legible: {resultado:?}")
    });
    assert_eq!(
        claves(&pf),
        BTreeSet::from([
            "tags".to_string(),
            "title".to_string(),
            "status".to_string()
        ]),
        "las tres claves del usuario siguen en el bloque tras reescribir el cuerpo"
    );
}

/// Criterio `replace_body_no_inyecta_linea_en_blanco` — **Dado** un documento escrito
/// `---\n…\n---\ncuerpo` (sin línea en blanco tras el delimitador de cierre), **Cuando** se le
/// reescribe el cuerpo con **el mismo contenido**, **Entonces** los bytes son idénticos: no se
/// inyecta separador (E31-H02, hallazgo 5 de la épica).
///
/// Hoy `build_raw_with_bom` normaliza el separador a `---\n\n` **y** recorta los `\n` iniciales del
/// cuerpo (`body.trim_start_matches('\n')`), así que la escritura impone su propia forma sobre la
/// del usuario. El único escritor no está para reformatear ficheros que nadie le pidió cambiar
/// (invariante #1: los `.md` en disco son la fuente de verdad).
#[test]
fn replace_body_no_inyecta_linea_en_blanco() {
    let path = rp("pegado.md");
    let files = mapa(&[("pegado.md", DOC_SEPARADOR_PEGADO)]);

    // Guarda del fixture: tras el `---` de cierre viene el cuerpo DIRECTAMENTE.
    assert!(
        DOC_SEPARADOR_PEGADO.contains("---\n# H\n"),
        "guarda del fixture: el cuerpo debe empezar pegado al delimitador de cierre, sin línea en \
         blanco de por medio. Documento = {DOC_SEPARADOR_PEGADO:?}"
    );

    let doc_set = DocumentSet::from_files(files.clone());
    // El MISMO cuerpo que se acaba de leer: la operación es un no-op semántico, así que también
    // debe serlo en bytes.
    let cuerpo = model::parse_file(path.as_str(), DOC_SEPARADOR_PEGADO).body;
    let op = plan::normalize_replace_body(&doc_set, &path, cuerpo)
        .expect("reescribir el cuerpo de un documento existente no debe fallar la normalización");
    let resultado = tras_aplicar(&files, &[op], &path);

    assert_eq!(
        resultado, DOC_SEPARADOR_PEGADO,
        "leer el cuerpo y volver a escribirlo TAL CUAL debe devolver los MISMOS bytes: el motor no \
         normaliza el separador del usuario. Hoy `build_raw_with_bom` fuerza `---\\n\\n` y añade \
         una línea en blanco que nadie escribió, ensuciando el `git diff` de quien versione su \
         workspace.\n  esperado = {DOC_SEPARADOR_PEGADO:?}\n  obtenido = {resultado:?}"
    );
    assert_eq!(
        workspace_revision(
            &plan::apply_normalized_ops(
                &files,
                &[plan::normalize_replace_body(
                    &doc_set,
                    &path,
                    model::parse_file(path.as_str(), DOC_SEPARADOR_PEGADO).body
                )
                .expect("normalizar no debe fallar")]
            )
            .expect("aplicar no debe fallar"),
            &[]
        ),
        workspace_revision(&files, &[]),
        "y como los bytes no cambian, la `WorkspaceRevision` tampoco puede moverse"
    );
}

/// Criterio `replace_body_no_borra_frontmatter_ilegible` — **Dado** un documento con frontmatter
/// **ilegible**, **Cuando** una operación reescribe su cuerpo, **Entonces** el bloque **sobrevive
/// literal** (E31-H02, hallazgo 5: hoy se borra, y es **pérdida de datos**).
///
/// Los dos documentos ilegibles del repo (`DOC_FM_YAML_ROTO`, `DOC_FM_SIN_CERRAR`) llegan al brazo
/// `ReplaceBody` como `frontmatter: None` —indistinguibles de «no hay bloque»—, así que el brazo
/// reconstruye el documento **sin** bloque y la metadata del usuario desaparece para siempre. Es la
/// trampa que el rustdoc de `model::patch_frontmatter` documenta y evita; este hermano cae en ella.
///
/// **Los dos casos NO son el mismo criterio**, y el test lo declara por separado:
///
///   - **`Bloque` + YAML inválido**: hay corte, el bloque está FUERA del cuerpo, y hoy se borra →
///     **rojo**. Debe sobrevivir byte a byte.
///   - **`SinCerrar`**: no hay corte (`body_offset == bom_len`), así que el bloque viaja DENTRO del
///     cuerpo que la operación lee y reescribe → sobrevive solo. Aquí el test es
///     **anti-regresión**: el arreglo tiene que seguir preservándolo, y el camino por el que lo
///     preserva es la simetría exacta con `SplitFront::body` (que es lo que el splice da gratis).
#[test]
fn replace_body_no_borra_frontmatter_ilegible() {
    for (caso, doc, esperado_split) in [
        (
            "bloque cerrado con YAML inválido",
            DOC_FM_YAML_ROTO,
            "Bloque",
        ),
        (
            "bloque que abre y nunca cierra",
            DOC_FM_SIN_CERRAR,
            "SinCerrar",
        ),
    ] {
        let path = rp("roto.md");
        let files = mapa(&[("roto.md", doc)]);

        // Premisa: el documento es ilegible y llega como `frontmatter: None` — que es justo lo que
        // lo hace confundible con la ausencia de bloque.
        let parsed = model::parse_file(path.as_str(), doc);
        assert!(
            parsed.frontmatter.is_none() && parsed.fm_err.is_some(),
            "[{caso}] premisa: el documento debe ser ilegible y llegar como `frontmatter: None`"
        );
        assert!(
            format!("{:?}", model::split_front(doc)).starts_with(esperado_split),
            "[{caso}] premisa: el corte debe ser `{esperado_split}`, que es lo que distingue los \
             dos casos. Obtenido: {:?}",
            model::split_front(doc)
        );

        // Una operación de CUERPO: `replace_text` sobre una cadena que sí está en el cuerpo (así el
        // test no es vacuo por «no cambió nada»).
        let doc_set = DocumentSet::from_files(files.clone());
        let op = plan::normalize_replace_text(&doc_set, &path, "cuerpo", "CUERPO", None)
            .expect("normalizar un `replace_text` sobre un documento existente no debe fallar");
        let resultado = tras_aplicar(&files, &[op], &path);

        // (a) ANTI-VACUO: la operación escribió de verdad.
        assert!(
            resultado.contains("CUERPO"),
            "[{caso}] anti-vacuo: el `replace_text` debe haber tocado el cuerpo. \
             Resultado = {resultado:?}"
        );

        // (b) El criterio: el bloque ilegible sobrevive LITERAL. Se compara la primera línea del
        // bloque y su contenido, que es lo que hoy desaparece entero.
        let primera = doc.lines().next().unwrap_or_default();
        assert!(
            resultado.starts_with(primera),
            "[{caso}] el bloque de frontmatter del usuario debe sobrevivir: reescribir el CUERPO \
             no puede borrar una cabecera que el motor no sabe leer — eso es pérdida de datos \
             silenciosa (§26, hallazgo 5). El documento resultante debería empezar por \
             {primera:?}.\n  original  = {doc:?}\n  resultante = {resultado:?}"
        );
        for linea in doc.lines().take_while(|l| !l.contains("cuerpo")) {
            assert!(
                resultado.contains(linea),
                "[{caso}] la línea {linea:?} del documento original se ha perdido al reescribir el \
                 cuerpo.\n  original   = {doc:?}\n  resultante = {resultado:?}"
            );
        }

        // (c) Y el resultado es exactamente «cabecera original + cuerpo reescrito»: ni un byte de
        // más entre una y otro.
        assert_eq!(
            resultado,
            format!(
                "{}{}",
                cabecera(doc),
                parsed.body.replace("cuerpo", "CUERPO")
            ),
            "[{caso}] el documento resultante debe ser el splice exacto de la cabecera original \
             (`raw[..body_offset]`, la misma que lee `SplitFront::body`) con el cuerpo reescrito"
        );
    }
}

// --- La familia entera (criterio «el radio ampliado» de E31-H02) -------------------------------
//
// El defecto NO es de `replace_text`: es del brazo `ReplaceBody`, y **todo** lo que normaliza a él
// lo hereda. Los cuatro tests que siguen ejercen los cuatro caminos, uno por test (y no cuatro
// bloques dentro de uno) para que cada uno sea rojo, y luego verde, **por separado**: en un solo
// test el primer `assert_eq!` que falla oculta a los otros tres, y el implementador no sabría si
// arregló la familia o solo su primer miembro.
//
// Cada uno lleva su propia guarda anti-vacua (la operación reescribió el cuerpo de verdad), porque
// sin ella una implementación que no escribiera nunca nada los pasaría todos.

/// Un enlazante con frontmatter en estilo *flow*: es el documento que `move --rewriteInboundLinks`
/// y `delete --remove_links` reescriben **en cadena**, sin que el usuario los haya nombrado.
const ENLAZANTE_FLOW: &str = concat!(
    "---\n",
    "tags: [x, y]\n",
    "title: \"Enlazante\"\n",
    "---\n",
    "\n",
    "# Enlazante\n",
    "\n",
    "Enlaza a [Destino](destino.md).\n",
);

/// El documento enlazado, también en *flow*. Tiene a su vez un enlace **saliente** relativo para
/// que `normalize_move` emita además el `ReplaceBody` que rebasa su propio cuerpo (`plan.rs:1021`).
/// Sin ese saliente el documento movido no se reescribiría, y aseverar sobre SU cabecera sería
/// vacuo: pasaría hoy mismo, sin arreglo ninguno.
const DESTINO_FLOW: &str =
    "---\ntags: [d]\n---\n\n# Destino\n\nVuelve a [Enlazante](enlazante.md).\n";

/// Criterio «la familia entera» (1/4) — **Dado** un documento con frontmatter *flow*, **Cuando** un
/// `edit_section` reescribe una sección con **el contenido que ya tenía**, **Entonces** la cabecera
/// no se reformatea (E31-H02, §26).
///
/// El contenido idéntico hace de la operación un no-op semántico: no hay ninguna excusa para que el
/// fichero cambie un solo byte, y menos en una zona que la operación ni siquiera nombra.
#[test]
fn edit_section_preserva_la_cabecera() {
    let path = rp("flow.md");
    let files = mapa(&[("flow.md", DOC_FLOW)]);
    let doc_set = DocumentSet::from_files(files.clone());
    let op = plan::normalize_edit_section(
        &doc_set,
        &path,
        &["Documento".to_string()],
        EditSectionMode::Replace,
        "cuerpo viejo",
    )
    .expect("editar una sección existente no debe fallar la normalización");
    let resultado = tras_aplicar(&files, &[op], &path);

    // Anti-vacuo: la operación materializó el contenido pedido.
    assert!(
        resultado.contains("cuerpo viejo"),
        "anti-vacuo: el contenido pedido debe estar en el cuerpo resultante: {resultado:?}"
    );
    assert_eq!(
        cabecera(&resultado),
        cabecera(DOC_FLOW),
        "editar una SECCIÓN del cuerpo no puede reformatear el frontmatter: la operación ni \
         siquiera nombra la cabecera, y aun así hoy la reserializa porque `edit_section` normaliza \
         a `ReplaceBody` (§26, el radio de la ficha).\n  esperado = {:?}\n  obtenido = {:?}",
        cabecera(DOC_FLOW),
        cabecera(&resultado)
    );
}

/// Criterio «la familia entera» (2/4) — **Dado** un documento enlazado desde otro con frontmatter
/// *flow*, **Cuando** se mueve con `rewriteInboundLinks`, **Entonces** ni el **enlazante** ni el
/// documento **movido** cambian de cabecera (E31-H02, §26).
///
/// Este es el **radio grande** de la ficha: `normalize_move` emite un `ReplaceBody` por CADA
/// documento entrante, así que un solo `move` puede hoy reformatear el frontmatter de medio
/// workspace — de documentos que el usuario ni mencionó en su petición.
#[test]
fn move_con_reescritura_de_entrantes_preserva_la_cabecera() {
    let files = mapa(&[
        ("enlazante.md", ENLAZANTE_FLOW),
        ("destino.md", DESTINO_FLOW),
    ]);
    let doc_set = DocumentSet::from_files(files.clone());
    let ops = plan::normalize_move(&doc_set, &rp("destino.md"), &rp("docs/destino.md"), true)
        .expect("mover un documento existente no debe fallar la normalización");
    let publicado = plan::apply_normalized_ops(&files, &ops)
        .expect("aplicar un `move` con reescritura de entrantes no debe fallar");

    let reescrito = publicado
        .get(&rp("enlazante.md"))
        .expect("el enlazante debe seguir existiendo tras el `move`");
    let movido = publicado
        .get(&rp("docs/destino.md"))
        .expect("el documento movido debe existir en su destino");

    // Anti-vacuo (los dos): el `move` reescribió de verdad el cuerpo de ambos.
    assert!(
        reescrito.contains("docs/destino.md"),
        "anti-vacuo: `rewriteInboundLinks` debe haber reapuntado el enlace del enlazante a \
         `docs/destino.md`: {reescrito:?}"
    );
    assert!(
        movido.contains("../enlazante.md"),
        "anti-vacuo: el saliente del documento movido debe haberse rebasado a `../enlazante.md` — \
         si su cuerpo no se reescribiera, aseverar sobre su cabecera sería vacuo: {movido:?}"
    );

    assert_eq!(
        cabecera(reescrito),
        cabecera(ENLAZANTE_FLOW),
        "mover un documento no puede reformatear el frontmatter de sus ENLAZANTES: el usuario pidió \
         mover `destino.md`, no reescribir la cabecera de `enlazante.md`. Este es el radio grande \
         de §26 — un solo `move` reformatea el frontmatter de medio workspace.\n  \
         esperado = {:?}\n  obtenido = {:?}",
        cabecera(ENLAZANTE_FLOW),
        cabecera(reescrito)
    );
    assert_eq!(
        cabecera(movido),
        cabecera(DESTINO_FLOW),
        "el documento MOVIDO cambia de path y de enlaces salientes, no de frontmatter: su cabecera \
         debe llegar al destino byte a byte.\n  esperado = {:?}\n  obtenido = {:?}",
        cabecera(DESTINO_FLOW),
        cabecera(movido)
    );
}

/// Criterio «la familia entera» (3/4) — **Dado** un documento enlazado desde otro con frontmatter
/// *flow*, **Cuando** se borra con `remove_links`, **Entonces** el enlazante conserva su cabecera
/// (E31-H02, §26).
///
/// Mismo daño colateral que el `move`: se pidió borrar un documento, y el precio es la cabecera
/// reformateada de todo el que lo enlazaba.
#[test]
fn delete_remove_links_preserva_la_cabecera() {
    let files = mapa(&[
        ("enlazante.md", ENLAZANTE_FLOW),
        ("destino.md", DESTINO_FLOW),
    ]);
    let doc_set = DocumentSet::from_files(files.clone());
    let ops = plan::normalize_delete(&doc_set, &rp("destino.md"), InboundLinksPolicy::RemoveLinks)
        .expect("borrar con `remove_links` no debe fallar la normalización");
    let publicado = plan::apply_normalized_ops(&files, &ops)
        .expect("aplicar un `delete remove_links` no debe fallar");

    let reescrito = publicado
        .get(&rp("enlazante.md"))
        .expect("el enlazante debe seguir existiendo tras el `delete`");

    // Anti-vacuo: el `delete` desenlazó de verdad.
    assert!(
        !reescrito.contains("(destino.md)"),
        "anti-vacuo: `remove_links` debe haber desenlazado el enlace al documento borrado: \
         {reescrito:?}"
    );
    assert_eq!(
        cabecera(reescrito),
        cabecera(ENLAZANTE_FLOW),
        "desenlazar a los entrantes no puede reformatear su frontmatter: se pidió borrar \
         `destino.md`, no reescribir la cabecera de quien lo enlazaba.\n  esperado = {:?}\n  \
         obtenido = {:?}",
        cabecera(ENLAZANTE_FLOW),
        cabecera(reescrito)
    );
}

/// Criterio «la familia entera» (4/4) — **Dado** un documento con frontmatter *flow*, **Cuando** un
/// `replace_text` **no casa ninguna ocurrencia**, **Entonces** el documento queda byte a byte igual
/// (E31-H02, §26: el síntoma literal con el que se abrió la ficha).
///
/// Es el caso que el juez ciego reprodujo por el wire sobre `examples/demo/overview.md`. La
/// aserción es la igualdad del documento ENTERO, no solo de la cabecera: un no-op tiene que ser un
/// no-op de verdad, cabecera y cuerpo.
#[test]
fn replace_text_sin_coincidencias_no_toca_un_byte() {
    let path = rp("flow.md");
    let files = mapa(&[("flow.md", DOC_FLOW)]);
    let doc_set = DocumentSet::from_files(files.clone());
    let op = plan::normalize_replace_text(&doc_set, &path, "no-casa-con-nada", "z", None)
        .expect("normalizar un `replace_text` sobre un documento existente no debe fallar");
    let resultado = tras_aplicar(&files, &[op], &path);

    assert_eq!(
        resultado, DOC_FLOW,
        "un `replace_text` cuyo patrón no casa NINGUNA ocurrencia debe dejar el documento byte a \
         byte igual: es el síntoma con el que se abrió §26. Hoy `tags: [a, b]` vuelve como lista en \
         block style, se pierden el comentario YAML y las comillas de `title`, y el documento entra \
         en `semanticDiff.modified` sin un solo cambio semántico — churn que contamina el `git \
         diff` de quien versione su workspace.\n  esperado = {DOC_FLOW:?}\n  obtenido = {resultado:?}"
    );
}

/// Criterio `preservar_la_cabecera_no_mueve_la_revision` — **Dado** cualquiera de los casos
/// anteriores en que los bytes no deben cambiar, **Cuando** se aplican, **Entonces** la
/// `WorkspaceRevision` **no se mueve** (E31-H02).
///
/// `types::workspace_revision` hashea los bytes CRUDOS del `FileMap`, así que el churn de bytes de
/// §26 no es cosmético: hace avanzar la revisión del workspace por una operación vacía, y con ella
/// invalida caches, dispara reconciliaciones del watcher y puede provocar conflictos de escritura
/// optimista. Mismo patrón que la aserción de `bom_roundtrip_byte_a_byte`.
#[test]
fn preservar_la_cabecera_no_mueve_la_revision() {
    let path = rp("flow.md");
    let files = mapa(&[("flow.md", DOC_FLOW)]);
    let rev_antes = workspace_revision(&files, &[]);
    let doc_set = DocumentSet::from_files(files.clone());

    // (a) `replace_text` sin coincidencias: la operación de la ficha.
    let op = plan::normalize_replace_text(&doc_set, &path, "no-casa-con-nada", "z", None)
        .expect("normalizar no debe fallar");
    let despues = plan::apply_normalized_ops(&files, &[op]).expect("aplicar no debe fallar");
    assert_eq!(
        workspace_revision(&despues, &[]),
        rev_antes,
        "un `replace_text` sin coincidencias no puede mover la `WorkspaceRevision`: no cambió nada \
         y la revisión hashea los bytes crudos. Hoy avanza porque la escritura reserializa el \
         frontmatter (§26).\n  documento resultante = {:?}",
        despues.get(&path)
    );

    // (b) Round-trip de cuerpo: leer el cuerpo y volver a escribirlo tal cual.
    let cuerpo = model::parse_file(path.as_str(), DOC_FLOW).body;
    let op =
        plan::normalize_replace_body(&doc_set, &path, cuerpo).expect("normalizar no debe fallar");
    let despues = plan::apply_normalized_ops(&files, &[op]).expect("aplicar no debe fallar");
    assert_eq!(
        despues.get(&path).map(String::as_str),
        Some(DOC_FLOW),
        "leer el cuerpo y reescribirlo TAL CUAL debe devolver los mismos bytes"
    );
    assert_eq!(
        workspace_revision(&despues, &[]),
        rev_antes,
        "y por tanto tampoco puede mover la `WorkspaceRevision`"
    );

    // (c) Anti-vacuo de este test: cuando el cuerpo SÍ cambia, la revisión SÍ se mueve. Sin esta
    // aserción, una `workspace_revision` constante pasaría las dos de arriba.
    let op =
        plan::normalize_replace_body(&doc_set, &path, "\n# Documento\n\notra cosa\n".to_string())
            .expect("normalizar no debe fallar");
    let despues = plan::apply_normalized_ops(&files, &[op]).expect("aplicar no debe fallar");
    assert_ne!(
        workspace_revision(&despues, &[]),
        rev_antes,
        "anti-vacuo: un cambio REAL del cuerpo sí debe mover la `WorkspaceRevision` — si no, las \
         aserciones de arriba se cumplirían con una revisión constante"
    );
}
