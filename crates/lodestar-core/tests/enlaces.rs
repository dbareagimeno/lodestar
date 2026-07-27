//! Tests de **enlaces universales** (épica E17, `ARCHITECTURE.md §20.6`).
//!
//! Fase ROJA de **E17-H01** (extracción) y **E17-H02** (resolución y clasificación). Los enlaces
//! dejan de ser dos regex que solo ven `[t](href)` y pasan a ser Markdown estándar completo,
//! resuelto **únicamente por path** y sin una sola heurística.
//!
//! Vive en un fichero propio, como `documento.rs` hizo con E16, por los mismos tres motivos:
//! `core.rs` es la suite de la era OKF que el implementador tendrá que migrar (mezclar aquí la
//! spec nueva multiplica su diff), estos tests no verdean hasta que `crate::links` exista, y
//! E17-H03/H04 aportarán más tests de la misma familia.
//!
//! ---
//!
//! ## API que fija esta fase roja
//!
//! ```ignore
//! // lodestar_core::types  (invariante #4: el contrato se define UNA vez aquí)
//!
//! /// La FORMA sintáctica del enlace (el `link_type` del parser), no su destino.
//! pub enum LinkKind { Inline, Reference, Collapsed, Shortcut, Autolink }
//!
//! pub struct RawLink {
//!     /// Destino CRUDO: sin percent-decoding, con fragmento y query. En un enlace de
//!     /// referencia, el destino de la DEFINICIÓN.
//!     pub href: String,
//!     pub text: String,
//!     /// Rango de bytes DEL DESTINO dentro del cuerpo: `body[span] == href`.
//!     pub span: std::ops::Range<usize>,
//!     pub kind: LinkKind,
//! }
//!
//! pub enum LinkTarget {
//!     Document(RelPath), WorkspaceFile(RelPath), ExternalUri(String),
//!     SelfAnchor(String), Missing(RelPath),
//!     /// AMPLIADO en E23-H11: un directorio CONTENIDO en el workspace (`None` = la raíz).
//!     WorkspaceDirectory(Option<RelPath>),
//!     EscapesWorkspace,
//! }
//!
//! pub struct ResolvedLink {
//!     pub href: String, pub text: String,
//!     pub span: std::ops::Range<usize>, pub kind: LinkKind,
//!     pub target: LinkTarget,
//!     /// El fragmento SIN almohadilla. Vive aquí, fuera de `LinkTarget`.
//!     pub fragment: Option<String>,
//! }
//!
//! /// Lo que el motor sabe que existe, SIN tocar el disco (invariante #2).
//! pub struct Inventory { /* documentos + demás ficheros del proyecto */ }
//! impl Inventory {
//!     pub fn new<D, F>(documents: D, other_files: F) -> Inventory
//!     where D: IntoIterator<Item = RelPath>, F: IntoIterator<Item = RelPath>;
//!     pub fn from_documents(files: &FileMap) -> Inventory;
//!     pub fn contains_document(&self, path: &RelPath) -> bool;
//!     pub fn contains_file(&self, path: &RelPath) -> bool;
//! }
//!
//! // lodestar_core::links
//! pub fn extract_links(body: &str) -> Vec<RawLink>;
//! pub fn resolve(raw: &RawLink, from: &RelPath, inventory: &Inventory) -> ResolvedLink;
//! ```
//!
//! ### Por qué el `span` acota el DESTINO y no el enlace entero
//!
//! Sus dos consumidores quieren el destino: `move_document` (`§20.11`) reescribe **solo el
//! destino** «conservando label y fragmento» —con el rango del enlace entero tendría que
//! re-parsearlo para encontrarlo—, y el `range` de los diagnósticos de `§20.9` señala lo que está
//! mal, que es el destino y no el texto del enlace. Además así el rango existe también para los
//! enlaces de **referencia**, donde el destino no está en el sitio del enlace sino en su
//! definición: `[t][id]` señala a la URL de `[id]: …`, que es exactamente el byte que hay que
//! reescribir al mover el documento.
//!
//! ### Por qué `resolve` recibe un `Inventory` y no el `FileMap`
//!
//! `WorkspaceFile` exige saber si existe un fichero que **no** es `.md`, y un `FileMap` solo tiene
//! documentos. El core no puede mirar el disco (invariante #2), así que la existencia le llega
//! como **dato**: quien hace I/O (el descubrimiento de `lodestar-workspace`) construye el
//! inventario. Consecuencia buscada: `resolve` es una función total y pura de sus tres argumentos
//! y el veredicto de `escape_del_workspace` no depende de que `/etc/passwd` exista.

use std::collections::BTreeSet;

use lodestar_core::types::{
    Check, CheckCode, FileMap, Inventory, LinkKind, LinkTarget, RawLink, RelPath, ResolvedLink,
    Severity,
};
use lodestar_core::{links, model, DocumentSet};

// --- Utilidades ---------------------------------------------------------------

/// `RelPath` para rutas obviamente válidas (invariante #6: nunca un string crudo).
fn rp(p: &str) -> RelPath {
    RelPath::new(p).unwrap_or_else(|e| panic!("`{p}` debe ser un RelPath válido: {e:?}"))
}

/// Los hrefs crudos de un cuerpo, en orden de aparición.
fn hrefs(body: &str) -> Vec<String> {
    links::extract_links(body)
        .into_iter()
        .map(|l| l.href)
        .collect()
}

/// El **único** enlace de un cuerpo. Falla si hay cero o más de uno: los tests que llaman aquí
/// juzgan un enlace concreto y no pueden permitirse que la extracción invente o pierda enlaces.
fn unico(body: &str) -> RawLink {
    let ls = links::extract_links(body);
    assert_eq!(
        ls.len(),
        1,
        "el cuerpo debe tener exactamente un enlace; se extrajeron {}: {:?}\n--- cuerpo ---\n{body}",
        ls.len(),
        ls.iter().map(|l| &l.href).collect::<Vec<_>>()
    );
    ls.into_iter().next().unwrap()
}

/// Invariante del `span` de E17-H01: acota **exactamente** el destino dentro del cuerpo.
fn assert_span_es_el_destino(body: &str, l: &RawLink) {
    assert!(
        l.span.start <= l.span.end && l.span.end <= body.len(),
        "span fuera del cuerpo: {:?} sobre {} bytes (href `{}`)",
        l.span,
        body.len(),
        l.href
    );
    assert_eq!(
        &body[l.span.clone()],
        l.href.as_str(),
        "`span` debe acotar el DESTINO dentro del cuerpo, no el enlace entero ni su texto"
    );
}

/// Inventario ad hoc: solo documentos Markdown.
fn inv(paths: &[&str]) -> Inventory {
    Inventory::new(paths.iter().map(|p| rp(p)), std::iter::empty())
}

/// Inventario de un `FileMap` de fixture (solo documentos).
fn inv_de(files: &FileMap) -> Inventory {
    Inventory::from_documents(files)
}

/// Resuelve el único enlace de `body`, escrito desde el documento `desde`.
fn resolver(body: &str, desde: &str, inventario: &Inventory) -> ResolvedLink {
    let l = unico(body);
    links::resolve(&l, &rp(desde), inventario)
}

/// Resuelve un href suelto envolviéndolo en el enlace inline mínimo.
fn resolver_href(href: &str, desde: &str, inventario: &Inventory) -> ResolvedLink {
    resolver(&format!("Enlace: [x]({href}).\n"), desde, inventario)
}

/// Todos los enlaces resueltos de un documento **real** de un fixture.
fn resueltos(files: &FileMap, path: &str, inventario: &Inventory) -> Vec<ResolvedLink> {
    let raw = files
        .get(&rp(path))
        .unwrap_or_else(|| panic!("el fixture debe traer `{path}`"));
    let parsed = model::parse_file(path, raw);
    links::extract_links(&parsed.body)
        .iter()
        .map(|l| links::resolve(l, &rp(path), inventario))
        .collect()
}

/// Los destinos clasificados de un documento real, en orden.
fn objetivos(files: &FileMap, path: &str, inventario: &Inventory) -> Vec<LinkTarget> {
    resueltos(files, path, inventario)
        .into_iter()
        .map(|l| l.target)
        .collect()
}

/// `LinkTarget::Document` de la ruta dada, para comparar sin ceremonia.
fn doc(p: &str) -> LinkTarget {
    LinkTarget::Document(rp(p))
}

// =============================================================================
// E17-H01 — Extracción de enlaces del documento
// =============================================================================
//
// `ARCHITECTURE.md §20.6` · `REFACTOR_PHASE_2 §Fase 7 (Tipos admitidos / no admitidos)`.
//
// Lo que retira: `model::LINK_RE` (que solo ve `[t](href)`), `out_links`, `out_links_with_href` y
// `raw_rel_links`. Nada de esto sobrevive a E17-H02.

/// Los tres sabores inline del criterio 1 en un solo cuerpo. `concat!` con una línea por literal
/// (la continuación de línea de Rust se come la indentación y desplazaría todos los spans).
const CUERPO_INLINE: &str = concat!(
    "# Guía\n",
    "\n",
    "Un enlace [simple](otro.md), uno con [fragmento](otro.md#seccion) y otro\n",
    "[con título](../guia.md \"Título del enlace\").\n",
);

/// Criterio 1: inline, con fragmento y con título → los 3, con su href crudo exacto.
#[test]
fn extrae_inline() {
    let ls = links::extract_links(CUERPO_INLINE);

    assert_eq!(
        ls.iter().map(|l| l.href.as_str()).collect::<Vec<_>>(),
        ["otro.md", "otro.md#seccion", "../guia.md"],
        "los tres enlaces salen en orden de aparición y con el href CRUDO"
    );
    assert_eq!(
        ls.iter().map(|l| l.text.as_str()).collect::<Vec<_>>(),
        ["simple", "fragmento", "con título"],
        "el texto del enlace se conserva aparte del destino"
    );
    assert!(
        ls.iter().all(|l| l.kind == LinkKind::Inline),
        "los tres son enlaces inline: {:?}",
        ls.iter().map(|l| l.kind).collect::<Vec<_>>()
    );

    // El fragmento es parte del href crudo: separarlo es E17-H02, no la extracción.
    assert_eq!(
        ls[1].href, "otro.md#seccion",
        "la extracción NO parte el fragmento: entrega el destino tal como está escrito"
    );
    // El título NO es parte del destino.
    assert_eq!(
        ls[2].href, "../guia.md",
        "`\"Título del enlace\"` es el title del enlace, no parte de su destino"
    );
    for l in &ls {
        assert_span_es_el_destino(CUERPO_INLINE, l);
    }
}

/// Cuerpo del criterio 2: el uso y su definición separados por 40 líneas de relleno, para que
/// ninguna implementación pueda apoyarse en que estén cerca.
fn cuerpo_referencia() -> String {
    let mut s = String::from("# Documento\n\nConsulta [la spec][spec].\n\n");
    for i in 0..40 {
        s.push_str(&format!("Línea de relleno número {i}, sin enlaces.\n\n"));
    }
    s.push_str("[spec]: ../../reference.md\n");
    s
}

/// Criterio 2: `[t][id]` + su definición lejana → un enlace con el href de la definición.
#[test]
fn extrae_referencia() {
    let body = cuerpo_referencia();
    let l = unico(&body);

    assert_eq!(
        l.href, "../../reference.md",
        "el href de un enlace de referencia es el destino de SU DEFINICIÓN"
    );
    assert_eq!(
        l.text, "la spec",
        "el texto visible es el del sitio de uso, no la etiqueta de la referencia"
    );
    assert_eq!(
        l.kind,
        LinkKind::Reference,
        "`[t][id]` es un enlace de referencia: la forma sintáctica se conserva"
    );
    // El destino vive en la definición: ahí es donde apunta el rango (y donde `move_document`
    // tendrá que escribir).
    assert_span_es_el_destino(&body, &l);
    assert!(
        body[..l.span.start].ends_with("[spec]: "),
        "el rango del destino de un enlace de referencia cae DENTRO de su definición; antes de \
         él hay {:?}",
        body[..l.span.start]
            .chars()
            .rev()
            .take(12)
            .collect::<String>()
    );

    // Colapsado `[id][]` y corto `[id]`: mismo destino, forma distinta.
    let colapsado = "Ver [spec][] aquí.\n\n[spec]: ../../reference.md\n";
    let c = unico(colapsado);
    assert_eq!(
        c.href, "../../reference.md",
        "`[id][]` resuelve su etiqueta"
    );
    assert_eq!(c.kind, LinkKind::Collapsed, "`[id][]` es colapsado");
    assert_span_es_el_destino(colapsado, &c);

    let corto = "Ver [spec] aquí.\n\n[spec]: ../../reference.md\n";
    let s = unico(corto);
    assert_eq!(s.href, "../../reference.md", "`[id]` resuelve su etiqueta");
    assert_eq!(s.kind, LinkKind::Shortcut, "`[id]` es un enlace corto");
    assert_span_es_el_destino(corto, &s);
}

/// Criterio 3: una referencia sin definición no inventa enlace y no revienta.
///
/// El enlace real del mismo cuerpo es el **control**: sin él, una implementación que devolviera
/// siempre `vec![]` pasaría este test sin hacer nada.
#[test]
fn referencia_sin_definicion() {
    let body = concat!(
        "# Documento\n",
        "\n",
        "Consulta [la spec][no-definida], el [colapsado][] y el [corto].\n",
        "\n",
        "Pero esto sí es un enlace: [real](real.md).\n",
        "\n",
        "[otra-etiqueta]: ../si-definida.md\n",
    );

    assert_eq!(
        hrefs(body),
        ["real.md"],
        "una etiqueta sin definición es texto plano: ni se inventa destino ni se toma la \
         definición de otra etiqueta"
    );
}

/// Criterio 4: wikilinks y embeds de Obsidian no son enlaces (`§20.6`: sin soporte de Obsidian).
#[test]
fn wikilinks_ignorados() {
    let body = concat!(
        "# Documento\n",
        "\n",
        "Un [[wikilink]], un embed ![[embed]], uno con heading [[doc#h]] y uno con\n",
        "alias [[doc|alias]].\n",
        "\n",
        "El único enlace de verdad es [este](este.md).\n",
    );

    assert_eq!(
        hrefs(body),
        ["este.md"],
        "la sintaxis de Obsidian es texto plano; el enlace Markdown del mismo cuerpo es el \
         control de que la extracción sí funciona"
    );
}

/// Criterio 5: los enlaces dentro de código no cuentan (fence, bloque indentado y span).
#[test]
fn enlace_en_fence_ignorado() {
    let body = concat!(
        "# Documento\n",
        "\n",
        "Este sí cuenta: [visible](visible.md).\n",
        "\n",
        "```md\n",
        "[en-fence](en-fence.md)\n",
        "```\n",
        "\n",
        "    [indentado](indentado.md)\n",
        "\n",
        "Y en un span de código: `[en-span](en-span.md)`.\n",
    );

    assert_eq!(
        hrefs(body),
        ["visible.md"],
        "un enlace dentro de un bloque de código (fence o indentado) o de un span es CONTENIDO \
         del código: la regex actual no distingue, el parser sí"
    );
}

/// Cuerpo del criterio 6: la cadena `otro.md` aparece **antes** del primer enlace, de modo que un
/// `body.find(href)` ingenuo devolvería el offset equivocado.
const CUERPO_RANGO: &str = concat!(
    "# Rangos\n",
    "\n",
    "La cadena `otro.md` aparece suelta antes, y también en `[otro.md]`.\n",
    "\n",
    "Aquí sí: [el enlace](otro.md) y otro [con fragmento](otro.md#s).\n",
    "\n",
    "Y de referencia: [la spec][spec].\n",
    "\n",
    "[spec]: otro.md\n",
);

/// Criterio 6: el rango de bytes acota **exactamente el destino** dentro del cuerpo.
#[test]
fn rango_del_destino() {
    let ls = links::extract_links(CUERPO_RANGO);
    assert_eq!(
        ls.len(),
        3,
        "el cuerpo tiene 3 enlaces (dos inline y uno de referencia): {:?}",
        ls.iter().map(|l| &l.href).collect::<Vec<_>>()
    );

    for l in &ls {
        assert_span_es_el_destino(CUERPO_RANGO, l);
        assert_eq!(
            l.span.end - l.span.start,
            l.href.len(),
            "el rango mide exactamente lo que mide el destino (href `{}`)",
            l.href
        );
    }

    // Offsets EXACTOS, calculados desde el texto del enlace: matan la implementación que busca la
    // primera aparición del href en el cuerpo (que caería en el `otro.md` del span de código).
    let inicio_inline = CUERPO_RANGO.find("](otro.md)").expect("el enlace está ahí") + 2;
    assert_eq!(
        ls[0].span,
        inicio_inline..inicio_inline + "otro.md".len(),
        "el rango del primer enlace es el de SU destino, no el de la primera aparición del texto \
         `otro.md` en el cuerpo (que está dentro de un span de código, en el byte {})",
        CUERPO_RANGO.find("otro.md").unwrap()
    );
    assert!(
        ls[0].span.start > CUERPO_RANGO.find("otro.md").unwrap(),
        "el destino del primer enlace está DESPUÉS de los señuelos"
    );

    let inicio_frag = CUERPO_RANGO
        .find("](otro.md#s)")
        .expect("el enlace está ahí")
        + 2;
    assert_eq!(
        ls[1].span,
        inicio_frag..inicio_frag + "otro.md#s".len(),
        "el rango incluye el fragmento: el destino crudo es `otro.md#s`"
    );

    // El de referencia apunta a su definición.
    let inicio_def = CUERPO_RANGO
        .find("[spec]: ")
        .expect("la definición está ahí")
        + "[spec]: ".len();
    assert_eq!(
        ls[2].span,
        inicio_def..inicio_def + "otro.md".len(),
        "el rango de un enlace de referencia acota el destino DE SU DEFINICIÓN: es el byte que \
         `move_document` tendrá que reescribir"
    );

    // Y en ningún caso el enlace entero.
    assert_ne!(
        &CUERPO_RANGO[ls[0].span.clone()],
        "[el enlace](otro.md)",
        "el rango es el del destino, no el del enlace completo"
    );
}

// =============================================================================
// E17-H02 — Resolución y clasificación de destinos
// =============================================================================
//
// `ARCHITECTURE.md §20.6` · `REFACTOR_PHASE_2 §Fase 7 (Algoritmo / Modelo / Prohibiciones)`.
//
// Lo que retira de `model::resolve_link`: la conversión `foo/` → `foo/index.md`, el requisito de
// que el destino termine en `.md` para considerarse interno, y el filtro de «href relativo» de
// `raw_rel_links`. Y lo que arregla: hoy `normalize` **recorta** los `..` sobrantes contra la raíz
// (`parts.pop()` sobre un vector vacío), así que `docs/a.md` + `../../docs/a.md` vuelve a caer
// dentro del workspace en vez de escapar — ver `escape_del_workspace`.

/// Criterio 1: de la raíz a tres niveles de profundidad.
#[test]
fn raiz_hacia_tres_niveles() {
    let files = lodestar_fixtures::arbitrary();
    let i = inv_de(&files);

    assert_eq!(
        objetivos(&files, "README.md", &i),
        vec![doc("one/first.md"), doc("three/levels/deep/third.md")],
        "un documento de la raíz alcanza tanto al vecino de un nivel como al de tres"
    );

    // La forma literal del criterio, sobre el otro fixture: `packages/api/docs/…` desde la raíz.
    let edge = lodestar_fixtures::with_edge_cases();
    let ie = inv_de(&edge);
    assert_eq!(
        resolver_href("packages/api/docs/auth.md", "raiz.md", &ie).target,
        doc("packages/api/docs/auth.md"),
        "un href relativo sin `./` desde la raíz baja tres niveles sin ceremonia"
    );
}

/// Criterio 2: de tres niveles de profundidad a la raíz.
#[test]
fn tres_niveles_hacia_raiz() {
    let files = lodestar_fixtures::arbitrary();
    let i = inv_de(&files);

    assert_eq!(
        objetivos(&files, "three/levels/deep/third.md", &i),
        vec![doc("README.md")],
        "`../../../README.md` desde `three/levels/deep/third.md` es el README de la raíz"
    );

    // Se resuelve contra el DIRECTORIO del origen, no «subiendo hasta encontrarlo»: con un `../`
    // de menos el destino es otro, y no existe.
    assert_eq!(
        resolver_href("../../README.md", "three/levels/deep/third.md", &i).target,
        LinkTarget::Missing(rp("three/README.md")),
        "con un `../` de menos el destino es `three/README.md`, que no existe: NO se busca el \
         README «más cercano hacia arriba»"
    );
}

/// Criterio 3: hermanos en árboles distintos.
#[test]
fn hermanos_en_arboles_distintos() {
    let files = lodestar_fixtures::arbitrary();
    let i = inv_de(&files);

    assert_eq!(
        objetivos(&files, "one/first.md", &i),
        vec![doc("two/levels/second.md")],
        "`../two/levels/second.md` desde `one/first.md` cruza a otro árbol: sube uno y baja dos"
    );
}

/// Criterio 4: `./doc.md` y `doc.md` designan el mismo destino.
#[test]
fn punto_barra_equivale() {
    let i = inv(&["docs/a.md", "docs/doc.md"]);
    let esperado = doc("docs/doc.md");

    for href in ["doc.md", "./doc.md", "././doc.md", "../docs/doc.md"] {
        let r = resolver_href(href, "docs/a.md", &i);
        assert_eq!(
            r.target, esperado,
            "`{href}` debe resolver al mismo destino que `doc.md`"
        );
        assert_eq!(
            r.href, href,
            "…pero el href ORIGINAL se conserva byte a byte (paso 10 del algoritmo)"
        );
    }
}

/// Criterio 5: percent-decoding del path antes de resolver.
#[test]
fn percent_encoding() {
    let files = lodestar_fixtures::with_edge_cases();
    let i = inv_de(&files);

    let r = resolver_href("notas/con%20espacios.md", "raiz.md", &i);
    assert_eq!(
        r.target,
        doc("notas/con espacios.md"),
        "`%20` se decodifica ANTES de buscar en el inventario: el documento real tiene un espacio"
    );
    assert_eq!(
        r.href, "notas/con%20espacios.md",
        "el href original se registra tal cual: la decodificación no reescribe el documento"
    );

    // El fixture lo trae escrito así de verdad: el documento real resuelve igual.
    assert!(
        objetivos(&files, "raiz.md", &i).contains(&doc("notas/con espacios.md")),
        "el `raiz.md` del fixture enlaza al documento con espacios vía `%20`: {:?}",
        objetivos(&files, "raiz.md", &i)
    );

    // Y sin decodificar no existe ningún documento llamado `con%20espacios.md`.
    assert!(
        !i.contains_document(&rp("notas/con%20espacios.md")),
        "el inventario no tiene el nombre codificado: si `resolve` no decodifica, el destino es \
         `Missing` y este criterio no se cumple"
    );
}

/// Criterio 6: el fragmento se separa del destino y se conserva aparte.
#[test]
fn fragmento_separado() {
    let i = inv(&["raiz.md", "otro.md"]);

    let r = resolver_href("otro.md#seccion", "raiz.md", &i);
    assert_eq!(
        r.target,
        doc("otro.md"),
        "el destino es el documento `otro.md`: el fragmento no forma parte del path"
    );
    assert_eq!(
        r.fragment.as_deref(),
        Some("seccion"),
        "…y el fragmento se conserva aparte, sin la almohadilla"
    );
    assert_eq!(
        r.href, "otro.md#seccion",
        "el href original conserva el fragmento (lo necesita `move_document` para reescribir solo \
         el destino)"
    );
    if let LinkTarget::Document(p) = &r.target {
        assert!(
            !p.as_str().contains('#'),
            "el `#` jamás se cuela en el `RelPath`: {p}"
        );
    }

    // Sin fragmento, no hay fragmento: `None` y `Some("")` son cosas distintas.
    assert_eq!(
        resolver_href("otro.md", "raiz.md", &i).fragment,
        None,
        "un enlace sin `#` no tiene fragmento"
    );
}

/// Criterio 7: `[x](#instalacion)` es un anchor del propio documento.
#[test]
fn anchor_propio() {
    // `instalacion.md` existe a propósito: un self-anchor NO se convierte en enlace a documento.
    let i = inv(&["docs/guia.md", "instalacion.md", "docs/instalacion.md"]);

    let r = resolver_href("#instalacion", "docs/guia.md", &i);
    assert_eq!(
        r.target,
        LinkTarget::SelfAnchor("instalacion".to_string()),
        "un href que empieza por `#` apunta dentro del propio documento, sin almohadilla y sin \
         pasar por el inventario (aunque exista un `instalacion.md`)"
    );
    assert_eq!(
        r.fragment.as_deref(),
        Some("instalacion"),
        "el fragmento se rellena también en un anchor propio: siempre vive en el mismo sitio"
    );
    assert_eq!(
        r.href, "#instalacion",
        "el href original conserva la almohadilla"
    );
}

/// Criterio 8: URIs externas.
#[test]
fn uri_externa() {
    // `docs/otro.md` existe: una URI externa que termina en `.md` sigue siendo externa.
    let i = inv(&["docs/guia.md", "docs/otro.md"]);

    for href in [
        "https://example.com",
        "http://example.com/a?b=c#d",
        "mailto:a@b.c",
        "https://example.com/docs/otro.md",
    ] {
        assert_eq!(
            resolver_href(href, "docs/guia.md", &i).target,
            LinkTarget::ExternalUri(href.to_string()),
            "`{href}` tiene esquema: es una URI externa, y se registra entera"
        );
    }

    // Un autolink `<uri>` es la misma clase de destino, con otra forma sintáctica.
    let auto = unico("Ver <https://example.com> para más.\n");
    assert_eq!(
        auto.kind,
        LinkKind::Autolink,
        "`<uri>` es un autolink (`§20.6` lo lista entre los tipos admitidos)"
    );
    assert_eq!(
        links::resolve(&auto, &rp("docs/guia.md"), &i).target,
        LinkTarget::ExternalUri("https://example.com".to_string()),
        "y su destino se clasifica como cualquier otra URI externa"
    );
}

/// Criterio 9: un enlace a un fichero del proyecto que **no** es Markdown.
///
/// Lo que aquí se juzga es la **clasificación**; que `WorkspaceFile` no entre en el grafo como
/// nodo lo verifica `codigo_no_es_nodo` (E17-H04), que es quien construye el grafo.
#[test]
fn enlace_a_codigo() {
    let files = lodestar_fixtures::with_edge_cases();
    let codigo = rp("src/auth/token_service.rs");

    // El fichero de código EXISTE en el proyecto (lo materializa `materialize_disk_only`), pero
    // no es un documento: entra al inventario por la otra puerta.
    let con_codigo = Inventory::new(files.keys().cloned(), [codigo.clone()]);

    assert_eq!(
        resolver_href("src/auth/token_service.rs", "raiz.md", &con_codigo).target,
        LinkTarget::WorkspaceFile(codigo.clone()),
        "un fichero del proyecto que existe y no es `.md` se clasifica como `WorkspaceFile`"
    );
    // La forma literal del criterio, desde dos niveles de profundidad.
    assert_eq!(
        resolver_href(
            "../../src/auth/token_service.rs",
            "docs/deep/nota.md",
            &con_codigo
        )
        .target,
        LinkTarget::WorkspaceFile(codigo.clone()),
        "el path relativo a código se resuelve igual que el de un documento"
    );
    assert!(
        !con_codigo.contains_document(&codigo),
        "…y ese fichero NO está entre los documentos: no puede ser nodo del grafo"
    );

    // El contraste que impide clasificar por extensión: un `.rs` que NO existe es `Missing`
    // (E17-H03 lo degradará a warning), no `WorkspaceFile`.
    let ausente = resolver_href("src/auth/no_existe.rs", "raiz.md", &con_codigo).target;
    assert_eq!(
        ausente,
        LinkTarget::Missing(rp("src/auth/no_existe.rs")),
        "`WorkspaceFile` afirma que el fichero EXISTE: si no está en el inventario, es `Missing`"
    );

    // Y el mismo enlace con un inventario que no conoce el fichero tampoco puede afirmarlo.
    let sin_codigo = inv_de(&files);
    assert_eq!(
        resolver_href("src/auth/token_service.rs", "raiz.md", &sin_codigo).target,
        LinkTarget::Missing(codigo),
        "la existencia sale del inventario, no de la extensión ni del disco"
    );
}

/// Criterio 10: destino inexistente.
#[test]
fn destino_inexistente() {
    let files = lodestar_fixtures::with_edge_cases();
    let i = inv_de(&files);

    assert_eq!(
        resolver_href("no-existe.md", "raiz.md", &i).target,
        LinkTarget::Missing(rp("no-existe.md")),
        "un destino contenido en el workspace que no está en el inventario es `Missing`"
    );
    // El fixture lo trae escrito de verdad.
    assert!(
        objetivos(&files, "raiz.md", &i).contains(&LinkTarget::Missing(rp("no-existe.md"))),
        "el `raiz.md` del fixture enlaza a un documento inexistente: {:?}",
        objetivos(&files, "raiz.md", &i)
    );

    // `Missing` lleva el path YA NORMALIZADO, no el href crudo: es lo que necesita el diagnóstico
    // de E17-H03 para decir qué documento falta.
    let r = resolver_href("no-existe.md", "docs/auth.md", &i);
    assert_eq!(
        r.target,
        LinkTarget::Missing(rp("docs/no-existe.md")),
        "el destino perdido se nombra por su path resuelto desde el origen"
    );
    assert_eq!(
        r.href, "no-existe.md",
        "…sin perder el href original, que es lo que el usuario escribió"
    );
}

/// Criterio 11: un destino que sale del workspace se rechaza, y el veredicto no depende del disco.
#[test]
fn escape_del_workspace() {
    let files = lodestar_fixtures::with_edge_cases();
    let i = inv_de(&files);

    assert_eq!(
        resolver_href("../../../../../../etc/passwd", "raiz.md", &i).target,
        LinkTarget::EscapesWorkspace,
        "seis `..` desde la raíz salen del workspace"
    );
    // El fixture lo trae escrito de verdad, con tres `..`.
    assert!(
        objetivos(&files, "raiz.md", &i).contains(&LinkTarget::EscapesWorkspace),
        "el `raiz.md` del fixture intenta escapar: {:?}",
        objetivos(&files, "raiz.md", &i)
    );

    // El caso que hoy se cuela: el path sale de la raíz y **vuelve a entrar**. `model::normalize`
    // recorta los `..` sobrantes (`parts.pop()` sobre un vector vacío), de modo que
    // `docs/` + `../../docs/auth.md` acaba siendo `docs/auth.md`, que SÍ existe. Contener es
    // contar profundidad, no recortar.
    assert_eq!(
        resolver_href("../../docs/auth.md", "docs/auth.md", &i).target,
        LinkTarget::EscapesWorkspace,
        "sube dos desde `docs/` (queda por encima de la raíz) y vuelve a bajar: escapa igual, \
         aunque el path recortado (`docs/auth.md`) exista en el inventario"
    );
    assert_eq!(
        resolver_href("../vecino.md", "raiz.md", &i).target,
        LinkTarget::EscapesWorkspace,
        "un solo `..` desde un documento de la raíz ya está fuera"
    );

    // Nunca se toca el disco: el mismo enlace da el mismo veredicto con el inventario vacío y con
    // uno que contenga hasta el propio `/etc/passwd` como fichero del proyecto.
    let vacio = Inventory::new(std::iter::empty(), std::iter::empty());
    let generoso = Inventory::new(files.keys().cloned(), [rp("etc/passwd")]);
    for (nombre, inventario) in [("vacío", &vacio), ("generoso", &generoso)] {
        assert_eq!(
            resolver_href("../../../../../../etc/passwd", "raiz.md", inventario).target,
            LinkTarget::EscapesWorkspace,
            "el veredicto de contención es puro: con el inventario {nombre} sigue siendo el mismo"
        );
    }
}

/// Criterio 12: un directorio **no** se resuelve como su `index.md` (`§20.6`, prohibiciones).
#[test]
fn directorio_no_es_index() {
    let i = inv(&["raiz.md", "guias/index.md", "guias/uno.md"]);

    for href in ["guias/", "guias", "./guias/"] {
        let t = resolver_href(href, "raiz.md", &i).target;
        assert_ne!(
            t,
            doc("guias/index.md"),
            "`{href}` NO puede resolver al `index.md` del directorio: es la heurística que \
             `§20.6` prohíbe explícitamente"
        );
        assert!(
            !matches!(t, LinkTarget::Document(_)),
            "`{href}` designa un directorio, y un directorio no es un documento: {t:?}"
        );
        assert_eq!(
            t,
            LinkTarget::Missing(rp("guias")),
            "un directorio es un destino que no está en el inventario: `Missing`"
        );
    }

    // El contraste: el mismo `index.md`, nombrado, sí resuelve.
    assert_eq!(
        resolver_href("guias/index.md", "raiz.md", &i).target,
        doc("guias/index.md"),
        "nombrado explícitamente, `index.md` es un documento como cualquier otro"
    );
    // Y tampoco se añade `.md` automáticamente (misma lista de prohibiciones).
    assert_eq!(
        resolver_href("guias/uno", "raiz.md", &i).target,
        LinkTarget::Missing(rp("guias/uno")),
        "`guias/uno` no resuelve a `guias/uno.md`: añadir la extensión está prohibido"
    );
}

/// Criterio 13: dos documentos con el mismo basename en árboles distintos, sin ambigüedad.
#[test]
fn mismo_basename_inequivoco() {
    let files = lodestar_fixtures::with_edge_cases();
    let i = inv_de(&files);

    // El fixture tiene los dos `auth.md`, y son documentos distintos.
    let dos: BTreeSet<&str> = files
        .keys()
        .map(RelPath::as_str)
        .filter(|p| p.ends_with("auth.md"))
        .collect();
    assert_eq!(
        dos,
        BTreeSet::from(["docs/auth.md", "packages/api/docs/auth.md"]),
        "el fixture debe traer DOS `auth.md` en árboles distintos, o el test no prueba nada"
    );

    // (a) El enlace real del fixture: desde el de `packages/`, subiendo tres niveles.
    assert_eq!(
        objetivos(&files, "packages/api/docs/auth.md", &i),
        vec![doc("docs/auth.md")],
        "`../../../docs/auth.md` apunta al `auth.md` de `docs/`, no a sí mismo"
    );

    // (b) El camino inverso.
    assert_eq!(
        resolver_href("../packages/api/docs/auth.md", "docs/auth.md", &i).target,
        doc("packages/api/docs/auth.md"),
        "y desde `docs/` se alcanza el de `packages/` por su path completo"
    );

    // (c) El MISMO href desde dos orígenes distintos: cada uno cae en su propio árbol.
    assert_eq!(
        resolver_href("auth.md", "docs/auth.md", &i).target,
        doc("docs/auth.md"),
        "`auth.md` desde `docs/` es el de `docs/`"
    );
    assert_eq!(
        resolver_href("auth.md", "packages/api/docs/auth.md", &i).target,
        doc("packages/api/docs/auth.md"),
        "`auth.md` desde `packages/api/docs/` es el de `packages/api/docs/`: el destino depende \
         del origen, nunca del basename"
    );

    // (d) Y desde la raíz, donde no hay ningún `auth.md`, no se «encuentra» ninguno de los dos:
    //     buscar por basename está prohibido.
    assert_eq!(
        resolver_href("auth.md", "raiz.md", &i).target,
        LinkTarget::Missing(rp("auth.md")),
        "existen dos `auth.md` en el workspace, pero ninguno está en la raíz: el destino falta, \
         no se desempata por similitud"
    );
}

// =============================================================================
// E23-H11 — Un enlace a un DIRECTORIO no es un escape (fase roja)
// =============================================================================
//
// ## El defecto (deuda registrada al cerrar E17 y aplazada a E20/E21, que cerraron sin hacerlo)
//
// `links::clasificar` devuelve `LinkTarget::EscapesWorkspace` en dos casos que **no** salen del
// workspace: cuando el href es **pura navegación** (no nombra ningún segmento: `../`, `./`, `.`,
// `..`) y cuando el destino **normaliza a la raíz** (`../docs/..`). Y `links::diagnose` convierte
// `EscapesWorkspace` en `Severity::Err` → `LINK-ESCAPES-WORKSPACE`. Resultado: un `[volver](../)`
// —un enlace legítimo que GitHub resuelve perfectamente— hace **fallar `lodestar check` con exit
// 1**. El propio `links.rs` lo documenta como caso degenerado consciente y anota que «el arreglo
// correcto es ampliar `LinkTarget`, no parchear el diagnóstico».
//
// ## El contrato que fija esta fase roja
//
// ```ignore
// pub enum LinkTarget {
//     …,
//     /// El destino es un DIRECTORIO contenido en el workspace, alcanzado por navegación pura
//     /// (`./`, `.`, `../`, `..`) o por un path que normaliza a la raíz (`../docs/..`).
//     /// `None` = la RAÍZ del workspace: no hay `RelPath` que la nombre (`RelPath::new("")` es un
//     /// error por diseño, invariante #6). `Some(p)` = un directorio de dentro.
//     WorkspaceDirectory(Option<RelPath>),
//     /// El destino sale POR ENCIMA de la raíz. Sigue sin llevar path y sigue siendo `Err`.
//     EscapesWorkspace,
// }
// ```
//
// **Wire** (`knowledge_get.outgoingLinks[].target`, etiquetado adyacente en camelCase como el
// resto de variantes): `{"kind":"workspaceDirectory","value":"docs"}` y —para la raíz—
// `{"kind":"workspaceDirectory","value":null}`.
//
// ### Por qué UNA variante con `Option` y no dos (`WorkspaceRoot` + `Directory(RelPath)`)
//
// Para un cliente MCP es **un solo concepto** —«esto apunta a un directorio, no a un documento»— y
// un solo `kind` que comprobar; la raíz es solo el directorio sin nombre. Dos `kind` obligarían a
// todo consumidor a acordarse de los dos, y el día que uno se olvide del segundo tendrá el mismo
// bug que hoy. Con `Option` el caso raíz es **inevitable de mirar**: no hay `RelPath` que sacar sin
// pasar por él.
//
// ### Las cuatro decisiones de criterio que van clavadas abajo
//
// 1. **No es un error de ninguna severidad**: `diagnose` no emite NADA para un directorio, igual
//    que no emite nada para `Document`/`WorkspaceFile`/`ExternalUri`/`SelfAnchor`. Un directorio del
//    workspace existe (el documento origen vive dentro de él, o de uno bajo él): no hay nada roto
//    que reportar, y el diagnóstico sigue siendo **función pura del `LinkTarget`** —el principio de
//    E17-H03— porque la distinción la hace ahora la CLASIFICACIÓN, no una segunda lectura del href.
// 2. **No es nodo del grafo ni fantasma**: `internal_path()` devuelve `None` (un directorio no es
//    un documento, y `§20.6` prohíbe convertirlo en su `index.md`), así que no aparece en
//    `dangling` ni crea vértices. Es la misma respuesta que ya dan `WorkspaceFile` y `ExternalUri`.
// 3. **El escape de verdad NO se toca**: subir por encima de la raíz sigue siendo
//    `EscapesWorkspace`/`Err`/`LINK-ESCAPES-WORKSPACE`. Es el chokepoint de seguridad (invariante
//    #6) y el control anti-vacuo obligatorio de esta historia: sin él, «no tumbar el check» se
//    satisface borrando el diagnóstico.
// 4. **Un href que NOMBRA un directorio (`guias/`) sigue siendo `Missing("guias")`**, no
//    `WorkspaceDirectory`. El core no tiene inventario de directorios —`Inventory` son documentos y
//    ficheros—, así que decidir «esto es un directorio» por la barra final sería exactamente la
//    heurística que `§20.6` prohíbe. Y no hace falta: `Missing` de un destino no-`.md` ya es `Warn`,
//    o sea que tampoco tumba la puerta de CI. Lo que arregla esta historia es el caso que el core
//    **sí** puede decidir estructuralmente: el href que no nombra nada.

/// Un [`DocumentSet`] a partir de pares `(ruta, contenido crudo)`. Es el camino que recorre
/// `lodestar check`: `analyze()` resuelve los enlaces y emite los diagnósticos.
fn docset(files: &[(&str, &str)]) -> DocumentSet {
    let mut m: FileMap = FileMap::new();
    for (p, raw) in files {
        m.insert(rp(p), (*raw).to_string());
    }
    DocumentSet::from_files(m)
}

/// Todos los diagnósticos del workspace, aplanados.
fn diagnosticos(ds: &DocumentSet) -> Vec<Check> {
    ds.analyze()
        .diagnostics
        .values()
        .flatten()
        .cloned()
        .collect()
}

/// Los códigos de una lista de diagnósticos, con su severidad, para mensajes de error legibles.
fn resumen(cs: &[Check]) -> Vec<(&'static str, Severity)> {
    cs.iter().map(|c| (c.code.as_str(), c.level)).collect()
}

/// El JSON de wire de un destino: [`LinkTarget`] viaja en `knowledge_get.outgoingLinks[].target`,
/// así que su forma serializada **es** contrato (invariante #4).
fn wire(t: &LinkTarget) -> serde_json::Value {
    serde_json::to_value(t).expect("`LinkTarget` es un tipo de wire: tiene que serializar")
}

/// El wire esperado de un destino directorio; `None` es la raíz del workspace.
fn wire_directorio(dir: Option<&str>) -> serde_json::Value {
    serde_json::json!({ "kind": "workspaceDirectory", "value": dir })
}

/// Clava el contrato completo de un destino directorio: wire, no-escape, no-nodo y round-trip.
fn assert_es_directorio(t: &LinkTarget, dir: Option<&str>, ctx: &str) {
    assert_ne!(
        *t,
        LinkTarget::EscapesWorkspace,
        "{ctx}: el destino es un directorio DE DENTRO del workspace, no un escape (hoy sale \
         `EscapesWorkspace` y `diagnose` lo convierte en un `Err` que tumba `lodestar check`)"
    );
    assert_eq!(
        wire(t),
        wire_directorio(dir),
        "{ctx}: el wire de un destino directorio es \
         `{{\"kind\":\"workspaceDirectory\",\"value\":<dir|null>}}`; salió {}",
        wire(t)
    );
    assert!(
        t.internal_path().is_none(),
        "{ctx}: un directorio NO es un documento del grafo — ni arista real ni fantasma \
         (`§20.6` prohíbe resolverlo a su `index.md`): {t:?}"
    );
    assert!(
        !t.is_internal(),
        "{ctx}: …y por tanto tampoco conecta el grafo: {t:?}"
    );
    assert_eq!(
        serde_json::from_value::<LinkTarget>(wire(t)).expect("el wire debe deserializar"),
        *t,
        "{ctx}: el destino directorio debe hacer round-trip por el wire"
    );
}

/// Criterio E23-H11: **Dado** un documento con `[volver](../)`, **Cuando** se corre `check`,
/// **Entonces** **no** es un error bloqueante (`enlace_a_la_raiz_no_tumba_el_check`).
///
/// Se juzga por el mismo camino que `lodestar check`: `DocumentSet::analyze()`, que resuelve los
/// enlaces y emite los diagnósticos de `links::diagnose` (`document_set.rs`).
///
/// **El control anti-vacuo vive DENTRO de este mismo test**: sin él, la forma más barata de
/// verdear sería dejar de emitir `LINK-ESCAPES-WORKSPACE`, que es justo el chokepoint de seguridad
/// del motor (invariante #6).
#[test]
fn enlace_a_la_raiz_no_tumba_el_check() {
    // La familia entera de hrefs de navegación pura, más el que normaliza a la raíz nombrando algo
    // por el camino (`../docs/..`). Los cinco apuntan a un directorio DE DENTRO del workspace.
    //
    // Se evalúan TODOS y se asevera al final (en vez de un assert por vuelta) para que el rojo
    // liste de una vez los cinco casos y su diagnóstico real.
    let mut ofensores: Vec<(&str, Vec<(&'static str, Severity)>)> = Vec::new();
    for href in ["../", "..", "./", ".", "../docs/.."] {
        let ws = docset(&[
            ("README.md", "# Índice\n\nUn workspace de dos documentos.\n"),
            (
                "docs/guia.md",
                &format!("# Guía\n\nY [volver]({href}) al índice.\n"),
            ),
        ]);
        let cs = diagnosticos(&ws);
        // Ofende lo que bloquea la puerta de CI (`Err`) y, sea cual sea su severidad, cualquier
        // `LINK-ESCAPES-WORKSPACE`: el destino NO sale de la raíz.
        let malos: Vec<(&'static str, Severity)> = resumen(&cs)
            .into_iter()
            .filter(|(code, level)| {
                *level == Severity::Err || *code == CheckCode::LinkEscapesWorkspace.as_str()
            })
            .collect();
        if !malos.is_empty() {
            ofensores.push((href, malos));
        }
    }
    assert!(
        ofensores.is_empty(),
        "un `[volver](href)` desde `docs/guia.md` que apunta a un directorio del workspace es un \
         enlace legítimo —GitHub lo resuelve— y no puede tumbar la puerta de CI (exit 1) ni salir \
         como `LINK-ESCAPES-WORKSPACE`. Ofensores: {ofensores:?}"
    );

    // CONTROL ANTI-VACUO (obligatorio): el escape de verdad sigue siendo un error bloqueante. Si
    // este assert se cae, el arreglo fue borrar el diagnóstico, no ampliar la clasificación.
    let fuga = docset(&[
        ("README.md", "# Índice\n\nUn workspace de dos documentos.\n"),
        (
            "docs/guia.md",
            "# Guía\n\nY [fuera](../../../etc/passwd) del workspace.\n",
        ),
    ]);
    let cs = diagnosticos(&fuga);
    assert_eq!(
        resumen(&cs),
        vec![("LINK-ESCAPES-WORKSPACE", Severity::Err)],
        "un destino que sube POR ENCIMA de la raíz sigue siendo `LINK-ESCAPES-WORKSPACE` con \
         severidad error: es el chokepoint de path-traversal (invariante #6), no una deuda"
    );
}

/// La clasificación nueva, destino a destino: qué directorio designa cada href de navegación pura,
/// y su forma exacta en el wire (la raíz incluida, que no es nombrable como [`RelPath`]).
#[test]
fn destino_directorio_dentro_del_workspace() {
    let i = inv(&["raiz.md", "docs/guia.md", "docs/deep/nota.md"]);

    // `(href, origen, directorio designado)`. Desde tres niveles: `.` es su propio directorio, cada
    // `..` sube uno, y dos `..` llegan a la raíz —que no tiene `RelPath`, por eso el `Option`—.
    // El último caso NOMBRA segmentos pero vuelve a la raíz por el camino: mismo destino.
    const CASOS: [(&str, &str, Option<&str>); 9] = [
        ("./", "docs/deep/nota.md", Some("docs/deep")),
        (".", "docs/deep/nota.md", Some("docs/deep")),
        ("../", "docs/deep/nota.md", Some("docs")),
        ("..", "docs/deep/nota.md", Some("docs")),
        ("../../", "docs/deep/nota.md", None),
        ("../..", "docs/deep/nota.md", None),
        ("./", "raiz.md", None),
        (".", "raiz.md", None),
        ("../docs/..", "docs/guia.md", None),
    ];

    // Primero los 9 destinos de una vez: el rojo enseña la tabla entera, no solo el primer caso.
    let obtenidos: Vec<(&str, &str, serde_json::Value)> = CASOS
        .iter()
        .map(|(href, desde, _)| (*href, *desde, wire(&resolver_href(href, desde, &i).target)))
        .collect();
    let esperados: Vec<(&str, &str, serde_json::Value)> = CASOS
        .iter()
        .map(|(href, desde, dir)| (*href, *desde, wire_directorio(*dir)))
        .collect();
    assert_eq!(
        obtenidos, esperados,
        "un href de navegación pura designa el DIRECTORIO al que llega, y viaja al wire como \
         `{{\"kind\":\"workspaceDirectory\",\"value\":<dir|null>}}` (`null` = la raíz)"
    );

    // Y luego, caso a caso, el resto del contrato: no-escape, no-nodo y round-trip por el wire.
    for (href, desde, dir) in CASOS {
        let r = resolver_href(href, desde, &i);
        assert_es_directorio(&r.target, dir, &format!("`{href}` desde `{desde}`"));
        assert_eq!(
            r.href, href,
            "el href original se conserva byte a byte (paso 10 del algoritmo de §20.6)"
        );
    }

    // La raíz y un directorio de dentro son destinos DISTINTOS: el wire los separa por su `value`.
    assert_ne!(
        wire(&resolver_href("../", "docs/deep/nota.md", &i).target),
        wire(&resolver_href("../../", "docs/deep/nota.md", &i).target),
        "`../` designa `docs` y `../../` la raíz: no pueden colapsar en el mismo destino"
    );
}

/// Control anti-vacuo, en su propio test para que su rojo/verde se lea por separado: el escape
/// **real** sigue clasificándose `EscapesWorkspace` y diagnosticándose como error.
///
/// Es la parte del contrato que esta historia **no** puede aflojar: `EscapesWorkspace` es lo que
/// impide que un enlace nombre `/etc/passwd`, y `RelPath` (invariante #6) el que impide escribirlo.
#[test]
fn escape_real_sigue_siendo_error() {
    let files = lodestar_fixtures::with_edge_cases();
    let i = inv_de(&files);

    let escapes = [
        // (href, origen, por qué escapa)
        ("../", "raiz.md", "un solo `..` desde la raíz ya está fuera"),
        ("..", "raiz.md", "idem sin barra final"),
        (
            "../../..",
            "docs/deep/nota.md",
            "tres `..` desde dos niveles de profundidad",
        ),
        (
            "../../../../../../etc/passwd",
            "raiz.md",
            "seis `..` y un destino nombrado fuera",
        ),
        (
            "../../docs/auth.md",
            "docs/auth.md",
            "sale por encima de la raíz y vuelve a entrar: contener es contar profundidad",
        ),
    ];
    for (href, desde, porque) in escapes {
        assert_eq!(
            resolver_href(href, desde, &i).target,
            LinkTarget::EscapesWorkspace,
            "`{href}` desde `{desde}` escapa del workspace ({porque}): la variante nueva de \
             directorio NO puede tragarse este caso"
        );
    }

    // …y por el camino del diagnóstico: error bloqueante, con su código.
    let ws = docset(&[
        ("docs/deep/nota.md", "# Nota\n\n[fuera](../../../x.md)\n"),
        ("README.md", "# Índice\n"),
    ]);
    assert_eq!(
        resumen(&diagnosticos(&ws)),
        vec![("LINK-ESCAPES-WORKSPACE", Severity::Err)],
        "el escape real sigue siendo el único `LINK-ESCAPES-WORKSPACE` del catálogo, y sigue \
         siendo `Err`"
    );
}

/// Consecuencia sobre el GRAFO: un directorio no es nodo, no es fantasma y no es colgante.
///
/// Es lo que separa «ampliar `LinkTarget`» de «parchear el diagnóstico»: si el destino directorio se
/// hubiera modelado como `Missing`, dejaría de tumbar el check (sería `Warn`) pero metería un
/// vértice inexistente en el grafo y una fila en `dangling`.
#[test]
fn directorio_no_es_nodo_del_grafo() {
    let ws = docset(&[
        ("README.md", "# Índice\n\nSin enlaces.\n"),
        ("docs/guia.md", "# Guía\n\n[volver](../) al índice.\n"),
    ]);
    let a = ws.analyze();

    assert_eq!(
        a.documents,
        vec![rp("README.md"), rp("docs/guia.md")],
        "los nodos del grafo son los documentos descubiertos: ningún directorio se cuela"
    );
    assert!(
        a.dangling.is_empty(),
        "un enlace a un directorio NO es un colgante: no hay ningún destino roto que reparar; \
         dangling = {:?}",
        a.dangling.iter().map(|d| &d.target).collect::<Vec<_>>()
    );

    let salientes = &a.outgoing[&rp("docs/guia.md")];
    assert_eq!(salientes.len(), 1, "el documento tiene un enlace saliente");
    assert_es_directorio(
        &salientes[0].target,
        None,
        "el saliente de `docs/guia.md` (`../` desde `docs/`)",
    );

    // Un enlace a un directorio no conecta con ningún documento: `docs/guia.md` sigue AISLADO. Es
    // la consecuencia buscada de que `internal_path()` sea `None`.
    assert_eq!(
        a.isolated,
        vec![rp("README.md"), rp("docs/guia.md")],
        "enlazar a un directorio no saca a un documento del aislamiento: no es una arista del grafo"
    );
}

/// Control del alcance (decisión 4): un href que **nombra** un directorio sigue siendo `Missing`,
/// y `Missing` de un destino que no es `.md` ya era `Warn` — nunca fue lo que tumbaba el check.
#[test]
fn directorio_nombrado_sigue_siendo_missing_y_no_bloquea() {
    let i = inv(&["raiz.md", "guias/index.md"]);

    for href in ["guias/", "guias", "./guias/"] {
        assert_eq!(
            resolver_href(href, "raiz.md", &i).target,
            LinkTarget::Missing(rp("guias")),
            "`{href}` NOMBRA un segmento, y el core no tiene inventario de directorios: decidir \
             que es un directorio por la barra final sería la heurística que `§20.6` prohíbe"
        );
    }

    let ws = docset(&[
        ("raiz.md", "# Raíz\n\nVer [las guías](guias/).\n"),
        ("guias/index.md", "# Guías\n"),
    ]);
    assert_eq!(
        resumen(&diagnosticos(&ws)),
        vec![("LINK-TARGET-MISSING", Severity::Warn)],
        "un destino nombrado que no existe y no es `.md` es un aviso (`missingWorkspaceFiles: \
         warning`, §20.9): informa, pero no tumba la puerta de CI"
    );
}
