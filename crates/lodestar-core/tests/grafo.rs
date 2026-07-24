//! Tests del **grafo universal** (épica E17-H04, `ARCHITECTURE.md §20.7`).
//!
//! Fase ROJA. Nodos = **todos** los documentos descubiertos; aristas = enlaces resueltos entre
//! ellos. `Analysis` deja de ser una adyacencia de strings y pasa a la forma de `§20.7`.
//!
//! Fichero propio (no se amplía `enlaces.rs` ni `core.rs`) por la misma razón que E17-H01/H02: cada
//! fichero de `tests/` es un binario independiente, y meter aquí símbolos que aún no existen tumba
//! solo este binario, no los ~300 tests verdes de los demás.
//!
//! ---
//!
//! ## API que fija esta fase roja
//!
//! ```ignore
//! // lodestar_core::types  (invariante #4: el contrato se define UNA vez aquí)
//!
//! pub struct Analysis {
//!     /// TODOS los documentos del workspace, ordenados por `RelPath`.
//!     pub documents: Vec<RelPath>,
//!     /// Enlaces salientes ya resueltos, en ORDEN DE APARICIÓN en el cuerpo. Una entrada por
//!     /// documento (vector vacío si no enlaza a nadie). Lleva TODOS los enlaces —también los
//!     /// externos, los anchors y los que apuntan a código—, no solo las aristas del grafo.
//!     pub outgoing: BTreeMap<RelPath, Vec<ResolvedLink>>,
//!     /// La inversa: quién enlaza a cada documento. Una entrada por documento.
//!     pub incoming: BTreeMap<RelPath, Vec<LinkReference>>,
//!     /// Sin enlaces INTERNOS entrantes ni salientes. Propiedad consultable, no diagnóstico.
//!     pub isolated: Vec<RelPath>,
//!     /// Los enlaces cuyo destino es `LinkTarget::Missing`, con su origen.
//!     pub dangling: Vec<DanglingLink>,
//!     /// Diagnósticos por documento (antes `per_file`). Una entrada por documento.
//!     pub diagnostics: BTreeMap<RelPath, Vec<Check>>,
//! }
//!
//! impl Analysis {
//!     /// Nº de documentos con al menos un `Severity::Err` (CONTEO de ficheros, no `.max()`).
//!     pub fn hard_fail(&self) -> usize;
//!     /// Nº total de diagnósticos `Severity::Warn` del workspace.
//!     pub fn warn_count(&self) -> usize;
//! }
//!
//! /// Un enlace visto DESDE SU DESTINO: quién lo escribe y cómo.
//! pub struct LinkReference { pub from: RelPath, pub link: ResolvedLink }
//!
//! /// Un enlace roto: quién lo escribe, qué destino pretendía y cómo lo escribió.
//! /// Invariante: `link.target == LinkTarget::Missing(target)`.
//! pub struct DanglingLink { pub from: RelPath, pub target: RelPath, pub link: ResolvedLink }
//!
//! pub struct GraphNode { pub id: RelPath, pub title: String, pub ghost: bool }
//!
//! // lodestar_core::DocumentSet
//! /// Como `from_files`, pero declarando además los ficheros del proyecto que NO son documentos
//! /// (código, imágenes…): sin ellos `resolve` no puede clasificar un `WorkspaceFile`.
//! pub fn with_other_files<I: IntoIterator<Item = RelPath>>(files: FileMap, other_files: I) -> Self;
//! ```
//!
//! ### Por qué `hard_fail`/`warn_count` pasan a ser MÉTODOS derivadas y no campos
//!
//! `§20.7` fija seis campos y ninguno es un contador, pero `lodestar check` (vía
//! `WorkspaceConfig::gate_blocked`) y `workspace_status`/`knowledge_check` los usan como **puerta de
//! CI**, así que no pueden desaparecer sin más. La salida es derivarlos de `diagnostics`: el
//! contrato de `§20.7` se cumple al pie de la letra, los consumidores solo añaden `()`, y —lo que
//! importa— **deja de existir un contador que pueda desincronizarse de la lista de la que sale**
//! (invariante #3: una sola verdad computada). Se conserva la semántica exacta de hoy: `hard_fail`
//! cuenta **ficheros** con algún `Err` (no diagnósticos, `§10` fila 4) y `warn_count` cuenta
//! **diagnósticos** `Warn`.
//!
//! ### Por qué `outgoing` lleva TODOS los enlaces y no solo las aristas
//!
//! Sus tres consumidores lo necesitan: `knowledge_get.outgoingLinks` (un agente quiere ver también
//! los enlaces externos del documento), `move_document` (reescribe destinos, y necesita los offsets
//! de todos) y la tabla `links` del store v2 (`§20.12`), que guarda `kind`/`fragment`/`target`. El
//! **grafo** filtra: nodo solo lo que es documento, arista solo `Document`/`Missing`.
//!
//! ### Qué cuenta como «enlace interno» para `isolated`
//!
//! `Document` (arista real) y `Missing` (arista a un fantasma: el documento **participa** en el
//! grafo). NO cuentan `ExternalUri`, `SelfAnchor`, `EscapesWorkspace` ni `WorkspaceFile`: ninguno
//! conecta con otro documento, y `§20.7` habla de enlaces *internos*.

use std::collections::{BTreeMap, BTreeSet};

use lodestar_core::types::{
    Analysis, DanglingLink, FileMap, LinkReference, LinkTarget, RelPath, ResolvedLink, Severity,
};
use lodestar_core::DocumentSet;

// --- Utilidades ---------------------------------------------------------------

/// `RelPath` para rutas obviamente válidas (invariante #6: nunca un string crudo).
fn rp(p: &str) -> RelPath {
    RelPath::new(p).unwrap_or_else(|e| panic!("`{p}` debe ser un RelPath válido: {e:?}"))
}

/// Los salientes de `path`, o pánico si `outgoing` no tiene entrada para él.
fn salientes<'a>(a: &'a Analysis, path: &str) -> &'a Vec<ResolvedLink> {
    a.outgoing.get(&rp(path)).unwrap_or_else(|| {
        panic!(
            "`outgoing` debe tener una entrada por documento; falta `{path}`: {:?}",
            a.outgoing.keys().collect::<Vec<_>>()
        )
    })
}

/// Los entrantes de `path`, o pánico si `incoming` no tiene entrada para él.
fn entrantes<'a>(a: &'a Analysis, path: &str) -> &'a Vec<LinkReference> {
    a.incoming.get(&rp(path)).unwrap_or_else(|| {
        panic!(
            "`incoming` debe tener una entrada por documento; falta `{path}`: {:?}",
            a.incoming.keys().collect::<Vec<_>>()
        )
    })
}

/// Los destinos clasificados de los salientes de `path`, en orden.
fn objetivos(a: &Analysis, path: &str) -> Vec<LinkTarget> {
    salientes(a, path)
        .iter()
        .map(|l| l.target.clone())
        .collect()
}

/// Los hrefs **crudos** de los salientes de `path`, en orden.
fn hrefs_salientes(a: &Analysis, path: &str) -> Vec<String> {
    salientes(a, path).iter().map(|l| l.href.clone()).collect()
}

/// Pares `(origen, href crudo)` de los entrantes de `path`, como conjunto.
fn entrantes_como_pares(a: &Analysis, path: &str) -> BTreeSet<(String, String)> {
    entrantes(a, path)
        .iter()
        .map(|r| (r.from.as_str().to_string(), r.link.href.clone()))
        .collect()
}

/// `LinkTarget::Document` de la ruta dada, para comparar sin ceremonia.
fn doc(p: &str) -> LinkTarget {
    LinkTarget::Document(rp(p))
}

/// Los `id` de los nodos del `GraphModel`, como conjunto de strings.
fn ids_del_grafo(ds: &DocumentSet) -> BTreeSet<String> {
    ds.graph_model()
        .nodes
        .iter()
        .map(|n| n.id.as_str().to_string())
        .collect()
}

/// Todos los diagnósticos del análisis, aplanados.
fn todos_los_diagnosticos(a: &Analysis) -> Vec<(String, &'static str, Severity)> {
    a.diagnostics
        .iter()
        .flat_map(|(p, cs)| {
            cs.iter()
                .map(move |c| (p.as_str().to_string(), c.code.as_str(), c.level))
        })
        .collect()
}

// =============================================================================
// Criterio 1 — `grafo_cubre_todas_las_profundidades`
// =============================================================================

/// **Dado** el fixture `arbitrary()` (raíz + 3 niveles con enlaces cruzados), **Cuando** se analiza,
/// **Entonces** `documents` tiene los 4 y hay aristas en ambos sentidos entre raíz y profundo.
#[test]
fn grafo_cubre_todas_las_profundidades() {
    let ds = DocumentSet::from_files(lodestar_fixtures::arbitrary());
    let a = ds.analyze();

    // (1) Los 4 documentos son nodos, a cualquier profundidad y sin `index.md` que los liste.
    assert_eq!(
        a.documents,
        vec![
            rp("README.md"),
            rp("one/first.md"),
            rp("three/levels/deep/third.md"),
            rp("two/levels/second.md"),
        ],
        "`documents` son TODOS los `.md` descubiertos, ordenados por `RelPath`"
    );
    for p in &a.documents {
        assert!(
            a.outgoing.contains_key(p)
                && a.incoming.contains_key(p)
                && a.diagnostics.contains_key(p),
            "todo documento tiene entrada en `outgoing`/`incoming`/`diagnostics` (vacía si no hay \
             nada), incluido `{p}`"
        );
    }

    // (2) De la raíz hacia abajo: un nivel y TRES niveles, en orden de aparición.
    assert_eq!(
        objetivos(a, "README.md"),
        vec![doc("one/first.md"), doc("three/levels/deep/third.md")],
        "el README alcanza tanto al vecino de un nivel como al de tres"
    );
    assert_eq!(
        hrefs_salientes(a, "README.md"),
        ["one/first.md", "three/levels/deep/third.md"],
        "…conservando el href CRUDO de cada enlace (lo necesita `move_document`)"
    );

    // (3) De tres niveles hacia la raíz: la arista de vuelta.
    assert_eq!(
        objetivos(a, "three/levels/deep/third.md"),
        vec![doc("README.md")],
        "`../../../README.md` desde el documento profundo es el README de la raíz"
    );

    // (4) Aristas EN AMBOS SENTIDOS entre raíz y profundo — el criterio central de la épica.
    assert_eq!(
        entrantes_como_pares(a, "README.md"),
        BTreeSet::from([(
            "three/levels/deep/third.md".to_string(),
            "../../../README.md".to_string(),
        )]),
        "el README recibe el enlace del documento profundo, con su href crudo"
    );
    assert_eq!(
        entrantes_como_pares(a, "three/levels/deep/third.md"),
        BTreeSet::from([(
            "README.md".to_string(),
            "three/levels/deep/third.md".to_string(),
        )]),
        "…y el documento profundo recibe el del README: mismo grafo, los dos sentidos"
    );

    // (5) El hermano en otro árbol también es una arista (`one/` → `two/`), y el destino final del
    //     encadenado no tiene salientes.
    assert_eq!(
        objetivos(a, "one/first.md"),
        vec![doc("two/levels/second.md")]
    );
    assert!(
        salientes(a, "two/levels/second.md").is_empty(),
        "`two/levels/second.md` no enlaza a nadie: entrada presente pero vacía"
    );
    assert_eq!(
        entrantes_como_pares(a, "two/levels/second.md"),
        BTreeSet::from([(
            "one/first.md".to_string(),
            "../two/levels/second.md".to_string(),
        )]),
    );

    // (6) Un workspace sano: nadie aislado, nada colgante, ningún diagnóstico.
    assert!(
        a.isolated.is_empty(),
        "los 4 documentos participan en el grafo: {:?}",
        a.isolated
    );
    assert!(
        a.dangling.is_empty(),
        "ningún enlace del fixture está roto: {:?}",
        a.dangling
    );
    assert_eq!(
        todos_los_diagnosticos(a),
        Vec::new(),
        "un workspace sin frontmatter, sin `index.md` y con enlaces correctos es SILENCIOSO"
    );
    assert_eq!(a.hard_fail(), 0, "sin errores duros");
    assert_eq!(a.warn_count(), 0, "sin avisos");
}

// =============================================================================
// Criterio 2 — `backlinks_globales`
// =============================================================================

/// Workspace donde `docs/api/target.md` es enlazado desde **3 profundidades distintas**, cada una
/// con un href diferente; un decoy que enlaza a otro sitio, y un origen que lo enlaza **dos veces**.
fn corpus_backlinks() -> FileMap {
    lodestar_fixtures::file_map(&[
        (
            "docs/api/target.md",
            "# Target\n\nMe enlazan desde todas partes.\n",
        ),
        // (a) Desde la raíz, bajando dos niveles.
        (
            "README.md",
            "# Proyecto\n\nVer [la API](docs/api/target.md).\n",
        ),
        // (b) Desde un hermano, con `../`.
        ("docs/guia.md", "# Guía\n\nVer [la API](api/target.md).\n"),
        // (c) Desde tres niveles abajo, en otro árbol.
        (
            "packages/api/docs/notas.md",
            "# Notas\n\nVer [la API](../../../docs/api/target.md).\n",
        ),
        // (d) Decoy: enlaza a OTRO documento. Sin él, un stub que devolviera «todos los
        //     documentos» como entrantes pasaría el criterio.
        ("docs/otro.md", "# Otro\n\nVer [la guía](guia.md).\n"),
        // (e) Un origen que enlaza DOS veces al mismo destino, con hrefs distintos.
        (
            "docs/doble.md",
            "# Doble\n\nUna vez [así](api/target.md) y otra [asá](../docs/api/target.md).\n",
        ),
    ])
}

/// **Dado** un documento enlazado desde 3 orígenes distintos, **Cuando** se analiza, **Entonces**
/// `incoming` lista los 3 con su href crudo.
#[test]
fn backlinks_globales() {
    let ds = DocumentSet::from_files(corpus_backlinks());
    let a = ds.analyze();

    let pares = entrantes_como_pares(a, "docs/api/target.md");
    for (origen, href) in [
        ("README.md", "docs/api/target.md"),
        ("docs/guia.md", "api/target.md"),
        ("packages/api/docs/notas.md", "../../../docs/api/target.md"),
    ] {
        assert!(
            pares.contains(&(origen.to_string(), href.to_string())),
            "`incoming` debe listar el entrante `{origen}` con SU href crudo (`{href}`): {pares:?}"
        );
    }

    // No vacuo: el decoy no enlaza al target y no puede aparecer.
    assert!(
        !pares.iter().any(|(o, _)| o == "docs/otro.md"),
        "«docs/otro.md» enlaza a la guía, no al target: {pares:?}"
    );

    // Una entrada por ENLACE, no por documento: `docs/doble.md` enlaza dos veces y aparece dos
    // veces, con sus dos hrefs. Es lo que necesita `move_document` para reescribirlos todos.
    let dobles: Vec<&LinkReference> = entrantes(a, "docs/api/target.md")
        .iter()
        .filter(|r| r.from == rp("docs/doble.md"))
        .collect();
    assert_eq!(
        dobles.len(),
        2,
        "un origen que enlaza dos veces produce DOS referencias entrantes: {dobles:?}"
    );
    assert_eq!(
        dobles
            .iter()
            .map(|r| r.link.href.as_str())
            .collect::<BTreeSet<_>>(),
        BTreeSet::from(["api/target.md", "../docs/api/target.md"]),
        "…cada una con el href que se escribió, sin deduplicar por destino"
    );
    assert_eq!(
        entrantes(a, "docs/api/target.md").len(),
        5,
        "3 orígenes distintos + los 2 enlaces del origen doble"
    );

    // La referencia entrante lleva el enlace COMPLETO, no solo el href: es el mismo `ResolvedLink`
    // que su origen tiene en `outgoing` (invariante #3: una sola verdad computada).
    let desde_readme = entrantes(a, "docs/api/target.md")
        .iter()
        .find(|r| r.from == rp("README.md"))
        .expect("el README enlaza al target");
    assert_eq!(
        desde_readme.link.target,
        doc("docs/api/target.md"),
        "el enlace de la referencia entrante ya viene resuelto y clasificado"
    );
    assert_eq!(
        &desde_readme.link,
        &salientes(a, "README.md")[0],
        "`incoming` es EXACTAMENTE la inversa de `outgoing`: el mismo enlace visto desde el otro \
         extremo, no una copia recalculada"
    );

    // El decoy sí es entrante de quien enlaza de verdad (el grafo no es un espejismo).
    assert_eq!(
        entrantes_como_pares(a, "docs/guia.md"),
        BTreeSet::from([("docs/otro.md".to_string(), "guia.md".to_string())]),
    );
}

// =============================================================================
// Criterio 3 — `dangling_identifica_origen`
// =============================================================================

/// **Dado** un enlace roto, **Cuando** se analiza, **Entonces** `dangling` identifica origen, href
/// crudo y destino pretendido.
#[test]
fn dangling_identifica_origen() {
    // DOS orígenes distintos, a distinta profundidad, apuntan al MISMO destino inexistente con
    // hrefs distintos: es justo lo que la forma vieja (`Vec<RelPath>` de destinos perdidos) no
    // podía expresar — decía «falta `docs/no-existe.md`» sin decir quién lo enlazaba.
    let files = lodestar_fixtures::file_map(&[
        (
            "README.md",
            "# Proyecto\n\nVer [lo que falta](docs/no-existe.md).\n",
        ),
        (
            "packages/api/docs/notas.md",
            "# Notas\n\nVer [lo mismo](../../../docs/no-existe.md).\n",
        ),
        // Control: un enlace que SÍ resuelve no puede aparecer en `dangling`.
        (
            "docs/existe.md",
            "# Existe\n\nVer [el README](../README.md).\n",
        ),
    ]);
    let ds = DocumentSet::from_files(files);
    let a = ds.analyze();

    assert_eq!(
        a.dangling.len(),
        2,
        "un enlace roto por origen, no una entrada por destino perdido: {:?}",
        a.dangling
    );

    let triples: BTreeSet<(String, String, String)> = a
        .dangling
        .iter()
        .map(|d| {
            (
                d.from.as_str().to_string(),
                d.target.as_str().to_string(),
                d.link.href.clone(),
            )
        })
        .collect();
    assert_eq!(
        triples,
        BTreeSet::from([
            (
                "README.md".to_string(),
                "docs/no-existe.md".to_string(),
                "docs/no-existe.md".to_string(),
            ),
            (
                "packages/api/docs/notas.md".to_string(),
                "docs/no-existe.md".to_string(),
                "../../../docs/no-existe.md".to_string(),
            ),
        ]),
        "cada colgante dice QUIÉN enlaza, QUÉ destino pretendía (ya normalizado) y CÓMO lo escribió"
    );

    // El destino pretendido y el `LinkTarget` del enlace no pueden divergir.
    for d in &a.dangling {
        assert_eq!(
            d.link.target,
            LinkTarget::Missing(d.target.clone()),
            "`DanglingLink::target` es el payload de `LinkTarget::Missing`, no un segundo cálculo"
        );
        assert!(
            !a.documents.contains(&d.target),
            "un destino colgante no es un documento del workspace: {}",
            d.target
        );
    }

    // No vacuo: el enlace que resuelve no está entre los colgantes.
    assert!(
        !a.dangling.iter().any(|d| d.from == rp("docs/existe.md")),
        "«docs/existe.md» enlaza a un documento que existe: {:?}",
        a.dangling
    );

    // Y el diagnóstico correspondiente (E17-H03) señala al ORIGEN, con el destino en `related`.
    let diags = todos_los_diagnosticos(a);
    assert_eq!(
        diags
            .iter()
            .filter(|(_, code, _)| *code == "LINK-TARGET-MISSING")
            .count(),
        2,
        "un `LINK-TARGET-MISSING` por enlace roto, en el documento que lo contiene: {diags:?}"
    );
    assert_eq!(
        a.hard_fail(),
        2,
        "`hard_fail` cuenta FICHEROS con algún error (los dos orígenes), no diagnósticos"
    );
}

// =============================================================================
// Criterio 4 — `isolated_sin_diagnostico`
// =============================================================================

/// **Dado** un documento sin enlaces de ningún tipo, **Cuando** se analiza, **Entonces** está en
/// `isolated` y no genera diagnóstico.
#[test]
fn isolated_sin_diagnostico() {
    let files = lodestar_fixtures::file_map(&[
        // (a) Sin un solo enlace: aislado de libro.
        ("suelto.md", "# Suelto\n\nNi enlazo ni me enlazan.\n"),
        // (b) Solo enlaces que NO son internos: sigue aislado.
        (
            "solo-externo.md",
            "# Externo\n\nWeb: [ej](https://example.com).\nCorreo: [c](mailto:a@b.c).\n\
             Anchor: [aquí](#externo).\nCódigo: [svc](src/auth/token_service.rs).\n",
        ),
        // (c) Un enlace roto es participación en el grafo (hay un fantasma al otro lado).
        ("roto.md", "# Roto\n\nVer [lo que falta](no-existe.md).\n"),
        // (d) y (e) Emisor y receptor: ninguno aislado.
        ("emisor.md", "# Emisor\n\nVer [receptor](receptor.md).\n"),
        ("receptor.md", "# Receptor\n\nMe enlazan.\n"),
    ]);
    // El fichero de código EXISTE (por eso es `WorkspaceFile` y no `Missing`): sin declararlo, el
    // enlace de (b) sería un colgante y el documento dejaría de estar aislado por accidente.
    let ds = DocumentSet::with_other_files(files, [rp("src/auth/token_service.rs")]);
    let a = ds.analyze();

    assert_eq!(
        a.isolated,
        vec![rp("solo-externo.md"), rp("suelto.md")],
        "aislado = sin enlaces INTERNOS entrantes ni salientes. Las URIs externas, los anchors \
         propios y los enlaces a código no conectan con ningún documento"
    );

    // No vacuo por partida triple: ni el emisor, ni el receptor, ni el que enlaza a un fantasma.
    for p in ["emisor.md", "receptor.md", "roto.md"] {
        assert!(
            !a.isolated.contains(&rp(p)),
            "«{p}» participa en el grafo y no puede estar aislado: {:?}",
            a.isolated
        );
    }

    // El aislamiento NO es un diagnóstico (`ORPHAN` murió en E16-H02) y `§20.7` lo confirma.
    for p in ["suelto.md", "solo-externo.md"] {
        assert_eq!(
            a.diagnostics.get(&rp(p)).map(Vec::len),
            Some(0),
            "un documento aislado es válido y silencioso: {:?}",
            a.diagnostics.get(&rp(p))
        );
    }

    // Los contadores derivados: solo `roto.md` tiene un error duro, y no hay ningún aviso.
    assert_eq!(
        a.hard_fail(),
        1,
        "`hard_fail` = nº de FICHEROS con algún `Err`, derivado de `diagnostics`: {:?}",
        todos_los_diagnosticos(a)
    );
    assert_eq!(
        a.warn_count(),
        0,
        "ni el aislamiento ni un enlace externo producen avisos: {:?}",
        todos_los_diagnosticos(a)
    );

    // El enlace a código sí se registra como saliente (aunque no sea arista): `outgoing` lleva
    // todos los enlaces del documento.
    assert_eq!(
        objetivos(a, "solo-externo.md"),
        vec![
            LinkTarget::ExternalUri("https://example.com".to_string()),
            LinkTarget::ExternalUri("mailto:a@b.c".to_string()),
            LinkTarget::SelfAnchor("externo".to_string()),
            LinkTarget::WorkspaceFile(rp("src/auth/token_service.rs")),
        ],
        "`outgoing` registra TODOS los enlaces resueltos, en orden de aparición; el filtro de qué \
         es arista lo hace el grafo, no la lista"
    );
}

// =============================================================================
// Criterio 5 — `codigo_no_es_nodo`
// =============================================================================

/// **Dado** un enlace a un fichero de código, **Cuando** se analiza, **Entonces** ese fichero **no**
/// es nodo del grafo, aunque el enlace se registre.
#[test]
fn codigo_no_es_nodo() {
    let codigo = rp("src/auth/token_service.rs");
    let files = lodestar_fixtures::file_map(&[
        (
            "docs/deep/nota.md",
            "# Nota\n\nAl [servicio](../../src/auth/token_service.rs) y al \
             [vecino](../vecino.md).\n",
        ),
        ("docs/vecino.md", "# Vecino\n\nUn documento normal.\n"),
    ]);
    let ds = DocumentSet::with_other_files(files, [codigo.clone()]);
    let a = ds.analyze();

    // (1) El enlace SÍ se registra, clasificado como fichero del proyecto.
    assert_eq!(
        objetivos(a, "docs/deep/nota.md"),
        vec![
            LinkTarget::WorkspaceFile(codigo.clone()),
            doc("docs/vecino.md"),
        ],
        "el enlace a código se registra con su clasificación; el enlace al documento vecino es el \
         contraste que hace el criterio no vacuo"
    );

    // (2) …pero el fichero de código no es documento, ni nodo, ni tiene entrantes.
    assert!(
        !a.documents.contains(&codigo),
        "los nodos son los documentos descubiertos: {:?}",
        a.documents
    );
    assert!(
        !a.incoming.contains_key(&codigo),
        "un fichero que no es nodo no puede tener backlinks: {:?}",
        a.incoming.keys().collect::<Vec<_>>()
    );
    assert_eq!(
        ids_del_grafo(&ds),
        BTreeSet::from([
            "docs/deep/nota.md".to_string(),
            "docs/vecino.md".to_string(),
        ]),
        "el `GraphModel` tiene exactamente los dos documentos: ni nodo real ni fantasma para el `.rs`"
    );
    for e in ds.graph_model().edges {
        assert_ne!(
            e.target, codigo,
            "un enlace a código no produce arista: {e:?}"
        );
    }

    // (3) Y no es un colgante ni produce diagnóstico: el fichero existe.
    assert!(a.dangling.is_empty(), "{:?}", a.dangling);
    assert_eq!(todos_los_diagnosticos(a), Vec::new());

    // (4) El nodo del documento lleva el título derivado (E16-H03) y no `type`/`status`.
    let nodo = ds.node(&rp("docs/vecino.md"));
    assert_eq!(
        nodo.title, "Vecino",
        "`GraphNode` pierde `type`/`status` (campos OKF) y gana el TÍTULO derivado del H1"
    );
    assert!(!nodo.ghost, "un documento que existe no es fantasma");
}

// =============================================================================
// Criterio 6 — `analisis_determinista`
// =============================================================================

/// Corpus grande: los dos fixtures universales juntos (raíz + 3 niveles, espacios, `%20`, mismo
/// basename en dos árboles, capitalización errónea, externo, anchor, inexistente y escape).
fn corpus_completo() -> FileMap {
    let mut files = lodestar_fixtures::arbitrary();
    files.extend(lodestar_fixtures::with_edge_cases());
    files
}

/// **Dado** el mismo conjunto de ficheros analizado dos veces, **Cuando** se comparan los
/// resultados, **Entonces** son idénticos (inventario, grafo, backlinks, diagnósticos).
#[test]
fn analisis_determinista() {
    let otros = [rp("src/auth/token_service.rs")];

    let primero = DocumentSet::with_other_files(corpus_completo(), otros.clone());
    let segundo = DocumentSet::with_other_files(corpus_completo(), otros.clone());

    // El mismo `FileMap` construido en ORDEN DE INSERCIÓN INVERSO: el análisis no puede depender
    // del orden en que llegaron los ficheros, solo de su contenido.
    let mut invertido: FileMap = BTreeMap::new();
    let mut pares: Vec<(RelPath, String)> = corpus_completo().into_iter().collect();
    pares.reverse();
    for (p, raw) in pares {
        invertido.insert(p, raw);
    }
    let tercero = DocumentSet::with_other_files(invertido, otros);

    let a = primero.analyze();
    let b = segundo.analyze();
    let c = tercero.analyze();

    assert_eq!(a, b, "dos análisis del mismo corpus deben ser IDÉNTICOS");
    assert_eq!(
        a, c,
        "…y no pueden depender del orden de inserción de los ficheros"
    );

    // Igualdad también en el wire (el contrato que ven MCP/CLI y el store v2).
    let json_a = serde_json::to_string(a).expect("`Analysis` serializa");
    let json_c = serde_json::to_string(c).expect("`Analysis` serializa");
    assert_eq!(
        json_a, json_c,
        "la serialización es byte a byte la misma: sin `HashMap` ni orden de iteración inestable"
    );

    // No vacuo: el corpus ejercita de verdad todas las piezas del análisis.
    assert_eq!(a.documents.len(), 9, "los 4 + los 5 de casos límite");
    let mut ordenados = a.documents.clone();
    ordenados.sort();
    assert_eq!(
        a.documents, ordenados,
        "`documents` va ordenado por `RelPath`"
    );
    assert!(
        !a.dangling.is_empty(),
        "el fixture de casos límite trae un enlace roto"
    );
    assert!(
        a.outgoing
            .values()
            .any(|ls| ls.iter().any(|l| l.target == LinkTarget::EscapesWorkspace)),
        "…y un intento de escape del workspace"
    );
    assert!(
        a.warn_count() >= 1,
        "…y una capitalización errónea (`Docs/Auth.md` sobre `docs/auth.md`), que es un aviso: {:?}",
        todos_los_diagnosticos(a)
    );
    assert_eq!(
        (a.hard_fail(), a.warn_count()),
        (c.hard_fail(), c.warn_count()),
        "los contadores derivados son función de `diagnostics`: no pueden divergir entre dos \
         análisis iguales"
    );

    // El grafo completo también es estable.
    assert_eq!(
        ids_del_grafo(&primero),
        ids_del_grafo(&tercero),
        "los nodos del grafo (documentos + fantasmas) son los mismos"
    );
    assert_eq!(
        serde_json::to_string(&primero.graph_model()).unwrap(),
        serde_json::to_string(&tercero.graph_model()).unwrap(),
        "…y el `GraphModel` serializa idéntico"
    );

    // Y `DanglingLink`/`LinkReference` no son cajas negras: viajan en el wire con su origen.
    let d: &DanglingLink = a.dangling.first().expect("hay al menos un colgante");
    let wire = serde_json::to_value(d).expect("`DanglingLink` serializa");
    for clave in ["from", "target", "link"] {
        assert!(
            wire.get(clave).is_some(),
            "`DanglingLink` expone `{clave}` en el wire: {wire}"
        );
    }
    let r: &LinkReference = a
        .incoming
        .values()
        .flatten()
        .next()
        .expect("hay al menos un entrante");
    let wire = serde_json::to_value(r).expect("`LinkReference` serializa");
    for clave in ["from", "link"] {
        assert!(
            wire.get(clave).is_some(),
            "`LinkReference` expone `{clave}` en el wire: {wire}"
        );
    }
}
