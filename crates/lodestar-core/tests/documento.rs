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
