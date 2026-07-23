//! Tests de los **diagnósticos de enlaces** (épica E17-H03, `ARCHITECTURE.md §20.9`).
//!
//! Fase ROJA. Los tres códigos de enlace del catálogo mínimo —`LINK-TARGET-MISSING`,
//! `LINK-ESCAPES-WORKSPACE` y `LINK-CASE-MISMATCH`— sustituyen a los `LINK-STUB`/`LINK-REL` de
//! `conform.rs`, que mueren aquí.
//!
//! Fichero propio (no se amplía `enlaces.rs`) por la misma razón que E17-H01/H02 no ampliaron
//! `core.rs`: `enlaces.rs` está **verde** con sus 19 tests y meter aquí símbolos que aún no existen
//! tumbaría su binario entero. Cada fichero de `tests/` es un binario independiente.
//!
//! ---
//!
//! ## API que fija esta fase roja
//!
//! ```ignore
//! // lodestar_core::links
//!
//! /// Diagnósticos de enlaces de UN documento. `raw` es el documento entero (con frontmatter):
//! /// el `range` de un `Check` va en líneas del FICHERO y el `span` de un `ResolvedLink` en bytes
//! /// del CUERPO, así que el productor suma el `body_start` de `model::split_front`.
//! /// `links` son los enlaces ya resueltos (`links::resolve`), para no resolver dos veces.
//! pub fn diagnose(
//!     path: &RelPath, raw: &str, links: &[ResolvedLink], inventory: &Inventory,
//! ) -> Vec<Check>;
//!
//! // lodestar_core::types::CheckCode  (dos variantes nuevas)
//! LinkTargetMissing     // wire `LINK-TARGET-MISSING`
//! LinkEscapesWorkspace  // wire `LINK-ESCAPES-WORKSPACE`
//! // `LinkCaseMismatch` (wire `LINK-CASE-MISMATCH`) YA existe: lo emite el descubrimiento para
//! // las colisiones de capitalización (`§20.5`). E17-H03 le añade un segundo productor.
//! ```
//!
//! ### La forma del `Check` que fijan estos tests
//!
//! | | `level` | `targets` | `related` | `range` |
//! |---|---|---|---|---|
//! | `LINK-TARGET-MISSING` (destino `.md`) | `Err` | `[documento origen]` | `[destino perdido]` | líneas del destino |
//! | `LINK-TARGET-MISSING` (otro fichero) | `Warn` | `[documento origen]` | `[destino perdido]` | líneas del destino |
//! | `LINK-ESCAPES-WORKSPACE` | `Err` | `[documento origen]` | `[]` | líneas del destino |
//! | `LINK-CASE-MISMATCH` | `Warn` | `[documento origen]` | `[ruta REAL del inventario]` | líneas del destino |
//!
//! `targets` es el **documento origen** y no el destino (al revés que el `LINK-STUB` heredado):
//! `Analysis::diagnostics` indexa por documento, `lodestar check` señala el fichero que hay que
//! editar, y el destino de un `EscapesWorkspace` no es nombrable como `RelPath`. El path
//! relacionado viaja en `related`, que es justo para lo que se añadió (`§19.3`).
//!
//! ### Decisiones de criterio propio (fase roja)
//!
//! 1. **`Err` para `LINK-ESCAPES-WORKSPACE`**. `§20.9` fija la severidad de las dos familias de
//!    destino ausente (`danglingDocumentLinks: error`, `missingWorkspaceFiles: warning`) pero no la
//!    del escape. Va a `Err`: un destino fuera de la raíz no es un enlace roto recuperable sino algo
//!    que el motor **no puede** seguir, indexar ni reescribir en un `move_document`.
//! 2. **El caso degenerado del destino que normaliza a la RAÍZ** (`[x](../)` desde un
//!    subdirectorio) **también emite `LINK-ESCAPES-WORKSPACE`**. E17-H02 lo clasifica así porque la
//!    raíz no es nombrable como `RelPath`, y el principio de esta historia es que **el diagnóstico
//!    es función pura del `LinkTarget`**: si H03 volviera a mirar el href para distinguir «escapa de
//!    verdad» de «apunta a la raíz» estaría reimplementando la clasificación de H02 con un segundo
//!    algoritmo divergente (lo que prohíbe el invariante #3). El precio —un `Err` sobre un enlace a
//!    la raíz del repo, que un render de GitHub sí resuelve— queda fijado por test y **reportado**:
//!    arreglarlo es ampliar `LinkTarget` (§20.6), no parchear el diagnóstico.
//! 3. **Un destino `Missing` produce UN solo diagnóstico**, nunca dos: o `LINK-CASE-MISMATCH` (si el
//!    inventario tiene esa ruta salvo capitalización) o `LINK-TARGET-MISSING`.
//! 4. **La familia de severidad se decide por el destino, no por el disco**: un destino perdido que
//!    termina en `.md` sería un documento (`Err`); cualquier otro, un fichero del proyecto (`Warn`).
//!    Es el único discriminador determinista disponible en el core, que no ve la política de
//!    descubrimiento. No es «resolver por extensión» (lo que `§20.6` prohíbe es *encontrar* el
//!    destino por heurística, y eso ya ocurrió en H02): aquí el destino ya está resuelto y lo único
//!    que se elige es a qué familia de `§20.9` pertenece.
//!
//! ### Portabilidad de `link_case_mismatch`
//!
//! La detección se hace contra el [`Inventory`] **en memoria**, jamás contra el disco: el veredicto
//! es idéntico en APFS (case-insensitive) y en ext4. Ningún test de este fichero toca el sistema de
//! ficheros.

use lodestar_core::plan::{self, PlanPolicy};
use lodestar_core::schema::Schema;
use lodestar_core::types::{Check, CheckCode, Inventory, Range, RelPath, ResolvedLink, Severity};
use lodestar_core::{links, model, DocumentSet};

// --- Utilidades ---------------------------------------------------------------

/// `RelPath` para rutas obviamente válidas (invariante #6: nunca un string crudo).
fn rp(p: &str) -> RelPath {
    RelPath::new(p).unwrap_or_else(|e| panic!("`{p}` debe ser un RelPath válido: {e:?}"))
}

/// Inventario ad hoc: `documentos` Markdown + `otros` ficheros del proyecto.
fn inv(documentos: &[&str], otros: &[&str]) -> Inventory {
    Inventory::new(
        documentos.iter().map(|p| rp(p)),
        otros.iter().map(|p| rp(p)),
    )
}

/// Los enlaces resueltos de un documento entero (`raw` con su frontmatter).
fn resueltos(raw: &str, desde: &str, inventario: &Inventory) -> Vec<ResolvedLink> {
    let parsed = model::parse_file(desde, raw);
    links::extract_links(&parsed.body)
        .iter()
        .map(|l| links::resolve(l, &rp(desde), inventario))
        .collect()
}

/// El pipeline completo de un documento: parsear → extraer → resolver → diagnosticar.
fn diagnosticos(raw: &str, desde: &str, inventario: &Inventory) -> Vec<Check> {
    let ls = resueltos(raw, desde, inventario);
    links::diagnose(&rp(desde), raw, &ls, inventario)
}

/// El **único** diagnóstico de un documento. Falla si hay cero o más de uno: cada criterio juzga un
/// enlace concreto y no puede permitirse que el productor diagnostique de más ni de menos.
fn unico(raw: &str, desde: &str, inventario: &Inventory) -> Check {
    let cs = diagnosticos(raw, desde, inventario);
    assert_eq!(
        cs.len(),
        1,
        "el documento debe producir exactamente un diagnóstico; salieron {}: {:?}",
        cs.len(),
        cs.iter().map(|c| c.code.as_str()).collect::<Vec<_>>()
    );
    cs.into_iter().next().unwrap()
}

/// Los códigos de una lista de diagnósticos, en orden.
fn codigos(cs: &[Check]) -> Vec<&'static str> {
    cs.iter().map(|c| c.code.as_str()).collect()
}

/// Rango de una sola línea (1-based, ambas inclusive).
fn linea(n: u32) -> Option<Range> {
    Some(Range {
        start_line: n,
        end_line: n,
    })
}

// =============================================================================
// Criterio 1 — `link_missing_con_rango`
// =============================================================================

/// Documento con frontmatter **a propósito**: el `span` del enlace es un offset del CUERPO y el
/// `range` del diagnóstico va en líneas del FICHERO. Sin frontmatter, un productor que confunda
/// ambos sistemas de coordenadas pasaría el test sin enterarse.
///
/// ```text
/// 1  ---
/// 2  title: Guía
/// 3  ---
/// 4
/// 5  # Guía
/// 6
/// 7  Falta el [destino](no-existe.md) de este enlace.
/// ```
const DOC_MISSING: &str = concat!(
    "---\n",
    "title: Guía\n",
    "---\n",
    "\n",
    "# Guía\n",
    "\n",
    "Falta el [destino](no-existe.md) de este enlace.\n",
);

/// **Dado** un enlace a un `.md` inexistente, **Cuando** se valida, **Entonces**
/// `LINK-TARGET-MISSING` con severidad error y el rango del destino.
#[test]
fn link_missing_con_rango() {
    let i = inv(&["docs/guia.md", "docs/existe.md"], &[]);
    let c = unico(DOC_MISSING, "docs/guia.md", &i);

    assert_eq!(
        c.code,
        CheckCode::LinkTargetMissing,
        "un destino Markdown que no está en el inventario es `LINK-TARGET-MISSING`"
    );
    assert_eq!(
        c.level,
        Severity::Err,
        "un enlace roto a un DOCUMENTO es error (`danglingDocumentLinks: error`, §20.9)"
    );
    assert_eq!(
        c.targets,
        vec![rp("docs/guia.md")],
        "`targets` es el documento que CONTIENE el enlace roto (el que hay que editar)"
    );
    assert_eq!(
        c.related,
        vec![rp("docs/no-existe.md")],
        "`related` lleva el destino perdido YA NORMALIZADO desde el origen (`docs/`), que es lo \
         que `Missing` trae"
    );
    assert_eq!(
        c.range,
        linea(7),
        "el rango señala la línea del DESTINO dentro del FICHERO (la 7). Si el productor usa el \
         `span` del cuerpo sin sumar el `body_start` del frontmatter, saldría la 4"
    );

    // No vacuo: el mismo cuerpo con un destino que sí existe no diagnostica nada.
    let bueno = DOC_MISSING.replace("no-existe.md", "existe.md");
    assert!(
        diagnosticos(&bueno, "docs/guia.md", &i).is_empty(),
        "un enlace a un documento que existe no produce diagnóstico: {:?}",
        codigos(&diagnosticos(&bueno, "docs/guia.md", &i))
    );

    // Y los códigos heredados que esta historia retira NO reaparecen por otra puerta.
    let cs = diagnosticos(DOC_MISSING, "docs/guia.md", &i);
    assert!(
        !cs.iter()
            .any(|c| matches!(c.code, CheckCode::LinkStub | CheckCode::LinkRel)),
        "`LINK-STUB`/`LINK-REL` se retiran del catálogo en E17-H03: {:?}",
        codigos(&cs)
    );
}

// =============================================================================
// Criterio 2 — `link_escapa`
// =============================================================================

/// **Dado** un enlace a `../../fuera.md` que escapa de la raíz, **Cuando** se valida, **Entonces**
/// `LINK-ESCAPES-WORKSPACE`.
#[test]
fn link_escapa() {
    let i = inv(&["docs/a.md", "docs/vecino.md"], &[]);

    // ```text
    // 1  # A
    // 2
    // 3  Se va [fuera](../../fuera.md) del workspace.
    // ```
    let raw = concat!(
        "# A\n",
        "\n",
        "Se va [fuera](../../fuera.md) del workspace.\n"
    );
    let c = unico(raw, "docs/a.md", &i);

    assert_eq!(
        c.code,
        CheckCode::LinkEscapesWorkspace,
        "dos `..` desde `docs/` dejan el destino por encima de la raíz"
    );
    assert_eq!(
        c.level,
        Severity::Err,
        "un destino fuera de la raíz no es un enlace roto recuperable: el motor no puede seguirlo, \
         indexarlo ni reescribirlo (decisión de criterio propio de la fase roja)"
    );
    assert_eq!(
        c.targets,
        vec![rp("docs/a.md")],
        "`targets` es el documento origen"
    );
    assert!(
        c.related.is_empty(),
        "un escape NO tiene destino nombrable como `RelPath` — por eso `EscapesWorkspace` no lleva \
         path: {:?}",
        c.related
    );
    assert_eq!(
        c.range,
        linea(3),
        "el rango señala la línea del destino dentro del fichero"
    );

    // Un escape que VUELVE a entrar sigue siendo un escape (E17-H02 `escape_del_workspace`): el
    // diagnóstico no puede «recortar» los `..` para redimirlo.
    let vuelve = "Sube y baja: [x](../../docs/vecino.md).\n";
    assert_eq!(
        codigos(&diagnosticos(vuelve, "docs/a.md", &i)),
        ["LINK-ESCAPES-WORKSPACE"],
        "aunque el path recortado (`docs/vecino.md`) exista, el destino salió de la raíz"
    );

    // El caso DEGENERADO que dejó abierto E17-H02: un destino que normaliza a la RAÍZ del
    // workspace no es nombrable como `RelPath` y se clasifica `EscapesWorkspace` — el único punto
    // donde esa variante no significa literalmente «sale del workspace». Decisión fijada aquí: el
    // diagnóstico es función PURA del `LinkTarget`, así que emite el mismo código. Re-inspeccionar
    // el href para distinguirlo sería reimplementar la clasificación de H02 (invariante #3).
    for href in ["../", "..", "./", "../docs/.."] {
        let raiz = format!("A la raíz: [x]({href}).\n");
        assert_eq!(
            codigos(&diagnosticos(&raiz, "docs/a.md", &i)),
            ["LINK-ESCAPES-WORKSPACE"],
            "`{href}` desde `docs/a.md` normaliza a la raíz del workspace: E17-H02 lo clasifica \
             `EscapesWorkspace` y el diagnóstico lo sigue sin re-interpretar el href"
        );
    }

    // No vacuo: subir un nivel SIN pasarse no escapa (y no diagnostica nada).
    assert!(
        diagnosticos("Vecino: [x](../docs/vecino.md).\n", "docs/a.md", &i).is_empty(),
        "`../docs/vecino.md` desde `docs/a.md` es `docs/vecino.md`, que existe"
    );
}

// =============================================================================
// Criterio 3 — `link_case_mismatch`
// =============================================================================

/// **Dado** `docs/auth.md` en el inventario y un enlace a `Docs/Auth.md`, **Cuando** se valida,
/// **Entonces** `LINK-CASE-MISMATCH` con severidad warning, **en cualquier sistema de ficheros**.
///
/// El veredicto sale del [`Inventory`] en memoria: este test no toca el disco, así que da lo mismo
/// en APFS (donde el enlace «funciona») que en ext4 (donde no) — que es exactamente el problema de
/// portabilidad que el código denuncia.
#[test]
fn link_case_mismatch() {
    let i = inv(&["raiz.md", "docs/auth.md"], &[]);

    // ```text
    // 1  # Raíz
    // 2
    // 3  Capitalización errónea: [auth](Docs/Auth.md).
    // ```
    let raw = concat!(
        "# Raíz\n",
        "\n",
        "Capitalización errónea: [auth](Docs/Auth.md).\n",
    );
    let c = unico(raw, "raiz.md", &i);

    assert_eq!(
        c.code,
        CheckCode::LinkCaseMismatch,
        "el inventario tiene esa ruta salvo capitalización: es un problema de PORTABILIDAD, no un \
         destino ausente"
    );
    assert_eq!(
        c.level,
        Severity::Warn,
        "`LINK-CASE-MISMATCH` es warning: en el volumen del autor el enlace funciona"
    );
    assert_eq!(
        c.targets,
        vec![rp("raiz.md")],
        "`targets` es el documento que contiene el enlace mal capitalizado"
    );
    assert_eq!(
        c.related,
        vec![rp("docs/auth.md")],
        "`related` lleva la ruta REAL del inventario: es la reparación que el agente necesita"
    );
    assert_eq!(c.range, linea(3), "…con el rango del destino");

    // Un solo diagnóstico, nunca dos: `LINK-CASE-MISMATCH` sustituye a `LINK-TARGET-MISSING`, no
    // se suma a él.
    assert_ne!(
        c.code,
        CheckCode::LinkTargetMissing,
        "el destino no «falta»: está ahí con otra capitalización"
    );

    // No vacuo (a): la capitalización correcta no diagnostica nada.
    let bien = raw.replace("Docs/Auth.md", "docs/auth.md");
    assert!(
        diagnosticos(&bien, "raiz.md", &i).is_empty(),
        "el mismo enlace bien capitalizado es silencioso: {:?}",
        codigos(&diagnosticos(&bien, "raiz.md", &i))
    );

    // No vacuo (b): un destino que difiere en algo MÁS que la capitalización falta de verdad.
    let otro = raw.replace("Docs/Auth.md", "Docs/Otro.md");
    let cs = diagnosticos(&otro, "raiz.md", &i);
    assert_eq!(
        codigos(&cs),
        ["LINK-TARGET-MISSING"],
        "`Docs/Otro.md` no es `docs/auth.md` ni plegando mayúsculas: es un destino ausente"
    );
    assert_eq!(
        cs[0].level,
        Severity::Err,
        "…y como sería un documento Markdown, es error"
    );

    // No vacuo (c): la capitalización también se juzga en el ÚLTIMO segmento y en un solo carácter.
    for href in ["docs/Auth.md", "Docs/auth.md", "docs/aUth.md"] {
        let uno = raw.replace("Docs/Auth.md", href);
        assert_eq!(
            codigos(&diagnosticos(&uno, "raiz.md", &i)),
            ["LINK-CASE-MISMATCH"],
            "`{href}` difiere de `docs/auth.md` solo en capitalización"
        );
    }

    // Un `LINK-CASE-MISMATCH` es un warning y NADA MÁS: no bloquea la puerta de conformidad por sí
    // solo. (`Severity::Warn` se había quedado sin productor desde E16-H05; este código lo resucita.)
    assert!(
        c.level < Severity::Err,
        "el orden de `Severity` es deliberado (`Err` es el máximo): un warning no es hard-fail"
    );
}

// =============================================================================
// Criterio 4 — `workspace_file_ausente_es_warning`
// =============================================================================

/// **Dado** un enlace a un `.rs` inexistente, **Cuando** se valida, **Entonces**
/// `LINK-TARGET-MISSING` con severidad **warning**, no error.
///
/// Y —porque `Severity::Warn` se había quedado **sin productor** en todo el pipeline desde
/// E16-H05— este criterio comprueba además, de punta a punta, que el warning vuelve a hacer
/// alcanzable la rama `allowWarnings` de `plan::can_apply`.
#[test]
fn workspace_file_ausente_es_warning() {
    let i = inv(&["raiz.md"], &["src/auth/token_service.rs"]);

    // ```text
    // 1  # Raíz
    // 2
    // 3  Al servicio: [código](src/auth/no_existe.rs).
    // ```
    let raw = concat!(
        "# Raíz\n",
        "\n",
        "Al servicio: [código](src/auth/no_existe.rs).\n",
    );
    let c = unico(raw, "raiz.md", &i);

    assert_eq!(
        c.code,
        CheckCode::LinkTargetMissing,
        "un fichero del proyecto que no existe usa el MISMO código que un documento ausente"
    );
    assert_eq!(
        c.level,
        Severity::Warn,
        "…pero con severidad warning (`missingWorkspaceFiles: warning`, §20.9), no error: el \
         destino no es un documento del grafo"
    );
    assert_eq!(c.targets, vec![rp("raiz.md")]);
    assert_eq!(
        c.related,
        vec![rp("src/auth/no_existe.rs")],
        "`related` lleva el fichero que falta"
    );
    assert_eq!(c.range, linea(3));

    // El contraste que fija la familia: el mismo enlace a un fichero que SÍ existe es silencioso…
    let existe = raw.replace("no_existe.rs", "token_service.rs");
    assert!(
        diagnosticos(&existe, "raiz.md", &i).is_empty(),
        "`WorkspaceFile` afirma que el fichero existe: no hay nada que diagnosticar"
    );
    // …y el mismo enlace a un `.md` ausente sube a error.
    let markdown = raw.replace("src/auth/no_existe.rs", "src/auth/no_existe.md");
    let cs = markdown_checks(&markdown, &i);
    assert_eq!(codigos(&cs), ["LINK-TARGET-MISSING"]);
    assert_eq!(
        cs[0].level,
        Severity::Err,
        "el MISMO código cambia de severidad según la familia del destino: `.md` ⇒ documento ⇒ error"
    );

    // --- Punta a punta: el warning vuelve a llegar al gate de aplicación ------------------
    //
    // `plan::validate_result` compone el universo completo de diagnósticos que ve `lodestar check`
    // (`Analysis` + schema). Con un enlace roto a un fichero del proyecto el workspace es
    // CONFORME (0 errores) pero tiene 1 warning, así que `allowWarnings` deja de ser una rama
    // muerta: `false` bloquea el plan y `true` lo deja pasar.
    let files = lodestar_fixtures::file_map(&[(
        "raiz.md",
        "# Raíz\n\nAl servicio: [código](src/auth/no_existe.rs).\n",
    )]);
    let doc_set = DocumentSet::from_files(files);
    let report = plan::validate_result(&doc_set, &Schema::default());

    assert_eq!(
        report.summary.errors, 0,
        "un fichero del proyecto ausente no es un error duro: {:?}",
        report.diagnostics
    );
    assert!(
        report.summary.warnings >= 1,
        "el `LINK-TARGET-MISSING` de un `.rs` ausente tiene que llegar hasta `validate_result` — \
         desde E16-H05 NINGÚN productor emitía `Severity::Warn`: {:?}",
        report.diagnostics
    );
    assert!(
        report.conformant,
        "conforme = sin errores duros, con o sin warnings"
    );
    assert!(
        !plan::can_apply(
            &report,
            &PlanPolicy {
                require_conformant_result: true,
                allow_warnings: false,
            }
        ),
        "con `allowWarnings:false` un solo warning bloquea el plan"
    );
    assert!(
        plan::can_apply(
            &report,
            &PlanPolicy {
                require_conformant_result: true,
                allow_warnings: true,
            }
        ),
        "con `allowWarnings:true` el mismo plan es aplicable"
    );
}

/// Diagnósticos de `raw` desde `raiz.md` (atajo local del criterio 4).
fn markdown_checks(raw: &str, i: &Inventory) -> Vec<Check> {
    diagnosticos(raw, "raiz.md", i)
}

// =============================================================================
// Criterio 5 — `externos_y_anchors_no_diagnostican`
// =============================================================================

/// **Dado** un enlace externo y un anchor propio, **Cuando** se validan, **Entonces** no producen
/// diagnóstico.
#[test]
fn externos_y_anchors_no_diagnostican() {
    let i = inv(&["docs/guia.md", "docs/otra.md"], &[]);

    let silencioso = concat!(
        "# Guía\n",
        "\n",
        "Web: [ejemplo](https://example.com/no/existe.md).\n",
        "Correo: [escribe](mailto:a@b.c).\n",
        "Autolink: <https://example.com/tampoco.md>.\n",
        "Anchor propio: [aquí](#instalacion).\n",
        "Anchor de otro: [allí](otra.md#seccion).\n",
        "Interno bueno: [otra](otra.md).\n",
    );
    assert!(
        diagnosticos(silencioso, "docs/guia.md", &i).is_empty(),
        "ni las URIs externas (aunque terminen en `.md` y no existan), ni los autolinks, ni los \
         anchors —propios o de otro documento que sí existe— producen diagnóstico: {:?}",
        codigos(&diagnosticos(silencioso, "docs/guia.md", &i))
    );

    // No vacuo: el MISMO cuerpo con un enlace roto de verdad produce exactamente uno. Sin este
    // control, un `diagnose` que devolviera siempre `vec![]` pasaría el criterio sin hacer nada.
    let con_roto = format!("{silencioso}Roto: [falta](no-existe.md).\n");
    assert_eq!(
        codigos(&diagnosticos(&con_roto, "docs/guia.md", &i)),
        ["LINK-TARGET-MISSING"],
        "el único diagnóstico del cuerpo es el del enlace realmente roto"
    );

    // Un anchor propio no se confunde con un documento homónimo que exista…
    let i2 = inv(
        &["docs/guia.md", "instalacion.md", "docs/instalacion.md"],
        &[],
    );
    assert!(
        diagnosticos("Ver [aquí](#instalacion).\n", "docs/guia.md", &i2).is_empty(),
        "`#instalacion` es `SelfAnchor`: no pasa por el inventario"
    );
    // …ni un anchor a un documento INEXISTENTE deja de ser un destino ausente por llevar `#`.
    let cs = diagnosticos("Ver [allí](no-existe.md#seccion).\n", "docs/guia.md", &i);
    assert_eq!(
        codigos(&cs),
        ["LINK-TARGET-MISSING"],
        "el fragmento no exime al path: `docs/no-existe.md` sigue faltando"
    );
    assert_eq!(
        cs[0].related,
        vec![rp("docs/no-existe.md")],
        "…y el destino relacionado va SIN fragmento (el `#` jamás entra en un `RelPath`)"
    );
}
