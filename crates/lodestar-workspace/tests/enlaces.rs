//! **Cableado** de la clasificación de enlaces desde el descubrimiento hasta el producto
//! (`ARCHITECTURE.md §20.6`, `docs/REFACTOR_PHASE_2.md §Fase 7`).
//!
//! E17-H02 implementó `LinkTarget::WorkspaceFile`, `Inventory::new(documents, other_files)` y
//! `DocumentSet::with_other_files`, y su test `crates/lodestar-core/tests/enlaces.rs::
//! enlace_a_codigo` pasa. Pero ese test **construye el `Inventory` a mano**, así que no puede ver
//! el hueco real: `Workspace::document_set()` llama a `DocumentSet::from_files`, `discover` solo
//! devuelve los `.md`, y por tanto `other_files` está **siempre vacío** en el producto. Resultado
//! observable sobre el binario: un enlace a `src/auth/token_service.rs` que existe en disco se
//! clasifica `Missing` y emite un `LINK-TARGET-MISSING` espurio.
//!
//! Este fichero es el test que lo habría cazado: mide la clasificación **a través de
//! `Workspace`**, o sea con el inventario que construye el descubrimiento de verdad. Va en un
//! fichero propio (y no en `discovery.rs` o `workspace.rs`) porque su sujeto no es la política de
//! descubrimiento ni el único escritor, sino la frontera entre ambos — el mismo motivo por el que
//! el core tiene su `tests/enlaces.rs` separado de `tests/core.rs`.
//!
//! ---
//!
//! ## La firma que fija esta historia
//!
//! Los ficheros del proyecto que **no** son documentos del inventario viajan por el propio
//! resultado del descubrimiento, no por una segunda pasada:
//!
//! ```ignore
//! pub struct Discovered {
//!     pub files: FileMap,                    // los documentos (los `.md` que pasan `include`)
//!     pub other_files: BTreeSet<RelPath>,    // NUEVO: todo lo demás que el walker VISITA
//!     pub diagnostics: Vec<Check>,
//! }
//!
//! // Workspace::document_set()
//! let d = discovery::discover(&self.root, &self.discovery_policy())?;
//! Ok(DocumentSet::with_other_files(d.files, d.other_files))
//! ```
//!
//! Por qué un campo de `Discovered` y no un método aparte: el walker **ya visita** esos ficheros
//! (desde el arreglo del bloqueante de E15, `include` es un filtro final sobre el fichero
//! superviviente, no una poda del recorrido), así que recolectar su ruta es un `insert` por
//! entrada y **cero I/O extra** — no se lee su contenido. Un método aparte pagaría un segundo
//! recorrido completo del árbol por cada `document_set()`, que es el coste que sí duele.
//!
//! El coste que queda, y que esta historia acepta sin fijarlo por test: en un repo grande sin
//! `.gitignore` útil, `other_files` puede ser un `BTreeSet` de decenas de miles de rutas que se
//! construye en cada `document_set()`. Es pequeño **al lado de lo que ya se paga** —el mismo
//! recorrido carga en memoria el contenido íntegro de todos los `.md`—, y el mando para acotarlo
//! ya existe (`discovery.exclude` y los `.gitignore` del árbol podan el directorio entero antes de
//! descender). Si algún día se mide como problema, la salida no es dejar de recolectar sino
//! cachear el `Discovered` y invalidarlo por el watcher; lo que **no** vale es resolver enlaces
//! preguntando al disco desde el core (invariante #2: el core es puro).
//!
//! `discover_files()` (el atajo que usan revisión/staging/publish/recovery) se queda como está:
//! descarta `other_files` igual que hoy descarta los diagnósticos. Ninguno de esos llamadores
//! resuelve enlaces.

use std::path::Path;

use lodestar_core::types::{Analysis, LinkTarget, RelPath};
use lodestar_workspace::Workspace;

/// Escribe un fichero dentro del workspace temporal, creando los directorios intermedios.
fn write(dir: &Path, rel: &str, content: &str) {
    let p = dir.join(rel);
    std::fs::create_dir_all(p.parent().expect("ruta con padre")).expect("crear directorios");
    std::fs::write(p, content).expect("escribir fichero");
}

fn rp(s: &str) -> RelPath {
    RelPath::new(s).expect("ruta relativa válida")
}

/// El destino ya clasificado del enlace cuyo href **crudo** es `href`, entre los salientes de
/// `origen`. Busca por href (no por índice) para que el test no dependa del orden de extracción.
fn destino(a: &Analysis, origen: &str, href: &str) -> LinkTarget {
    let salientes = a
        .outgoing
        .get(&rp(origen))
        .unwrap_or_else(|| panic!("«{origen}» debe estar entre los documentos analizados"));
    salientes
        .iter()
        .find(|l| l.href == href)
        .unwrap_or_else(|| {
            let vistos: Vec<&str> = salientes.iter().map(|l| l.href.as_str()).collect();
            panic!("«{origen}» debe tener un saliente con href «{href}»; tiene {vistos:?}")
        })
        .target
        .clone()
}

/// Los `related` de los diagnósticos `LINK-TARGET-MISSING` de `origen` (el destino perdido viaja
/// ahí, no en `targets`, que es siempre el documento que hay que editar).
fn destinos_reportados_como_perdidos(a: &Analysis, origen: &str) -> Vec<String> {
    a.diagnostics
        .get(&rp(origen))
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|c| c.code.as_str() == "LINK-TARGET-MISSING")
        .flat_map(|c| c.related.iter().map(|r| r.as_str().to_string()))
        .collect()
}

/// **Dado** un workspace con un `README.md` que enlaza a un fichero de código que **existe** y a
/// otro que **no**, **Cuando** se abre con `Workspace` y se analiza, **Entonces** el primero se
/// clasifica `WorkspaceFile` y solo el segundo es `Missing`.
///
/// Es el criterio de aceptación final de `REFACTOR_PHASE_2` («los enlaces a archivos no Markdown
/// se clasifican correctamente») medido **donde el producto lo entrega**, no donde el core lo sabe
/// hacer. Verificado sobre el binario real antes de escribirlo: hoy el fichero que existe también
/// sale `Missing`.
///
/// La segunda mitad del test cubre `§20.6` precisión 2 —un `.md` que existe en disco pero está
/// **excluido del descubrimiento** es `WorkspaceFile`, no `Missing`—, que es el caso caro: un
/// destino terminado en `.md` que «falta» es `Err` (`danglingDocumentLinks: error`), o sea que el
/// hueco no solo miente sobre el disco, tumba la puerta de CI.
#[test]
fn enlace_a_codigo_llega_al_producto() {
    // --- (1) fichero de código que existe vs. fichero de código que no ---------------------
    let dir = tempfile::tempdir().expect("tempdir");
    write(
        dir.path(),
        "README.md",
        "# Proyecto\n\nLa rotación la implementa el [servicio de tokens]\
         (src/auth/token_service.rs), no el [servicio viejo](src/auth/no_existe.rs).\n",
    );
    // El fichero de código EXISTE en el proyecto. No es un documento (no es `.md`), así que no es
    // nodo del grafo — pero está ahí, y el enlace que lo apunta no está roto.
    write(
        dir.path(),
        "src/auth/token_service.rs",
        "// Destino de un enlace WorkspaceFile.\npub struct TokenService;\n",
    );

    let ws = Workspace::open(dir.path()).expect("abrir el workspace");
    let a = ws.analyze().expect("analizar el workspace");

    assert_eq!(
        destino(&a, "README.md", "src/auth/token_service.rs"),
        LinkTarget::WorkspaceFile(rp("src/auth/token_service.rs")),
        "el fichero de código EXISTE en disco: el análisis del workspace tiene que clasificarlo \
         `WorkspaceFile`. Si sale `Missing`, `other_files` llegó vacío al `DocumentSet` y la \
         capacidad de E17-H02 no está cableada al producto"
    );
    // El contraste que impide clasificar por extensión (y que hace el test no vacuo): un `.rs`
    // que NO está en disco sigue siendo `Missing`.
    assert_eq!(
        destino(&a, "README.md", "src/auth/no_existe.rs"),
        LinkTarget::Missing(rp("src/auth/no_existe.rs")),
        "`WorkspaceFile` afirma que el fichero existe: un fichero de código ausente sigue siendo \
         `Missing` (no se clasifica por extensión, se clasifica por el inventario)"
    );

    // Consecuencia en la superficie de diagnósticos: el enlace vivo no puede producir un
    // `LINK-TARGET-MISSING` espurio, y el roto sí tiene que producirlo.
    let perdidos = destinos_reportados_como_perdidos(&a, "README.md");
    assert!(
        !perdidos.contains(&"src/auth/token_service.rs".to_string()),
        "un fichero del proyecto que está en disco no puede reportarse como destino perdido; \
         reportados: {perdidos:?}"
    );
    assert!(
        perdidos.contains(&"src/auth/no_existe.rs".to_string()),
        "…pero el que no está sí: {perdidos:?}"
    );

    // Y sigue sin ser nodo del grafo: `WorkspaceFile` dice «existe», no «es un documento».
    assert!(
        !a.documents.iter().any(|p| p.as_str().ends_with(".rs")),
        "un fichero de código nunca entra en `Analysis::documents`: {:?}",
        a.documents
    );
    assert!(
        !a.dangling
            .iter()
            .any(|d| d.target.as_str() == "src/auth/token_service.rs"),
        "…ni cuenta como enlace colgante: los colgantes son solo los `Missing`"
    );

    // --- (2) `§20.6` precisión 2: un `.md` EXCLUIDO del descubrimiento --------------------
    // `include` acota el inventario a `docs/`, así que `notas/nota.md` existe en disco pero NO es
    // documento. Un enlace a él no «falta»: es `WorkspaceFile`. Decir `Missing` sería mentir
    // sobre el disco — y, por terminar en `.md`, sería un `Err` que bloquea la puerta de CI.
    let acotado = tempfile::tempdir().expect("tempdir");
    write(
        acotado.path(),
        ".lodestar/config.yaml",
        "discovery:\n  include: [\"docs/**/*.md\"]\n",
    );
    write(
        acotado.path(),
        "docs/guia.md",
        "# Guía\n\nEl detalle está en la [nota archivada](../notas/nota.md).\n",
    );
    write(
        acotado.path(),
        "notas/nota.md",
        "# Nota archivada\n\nExisto, pero no soy del inventario.\n",
    );

    let ws = Workspace::open(acotado.path()).expect("abrir el workspace acotado");
    let a = ws.analyze().expect("analizar el workspace acotado");

    assert!(
        !a.documents.iter().any(|p| p.as_str() == "notas/nota.md"),
        "el `include` deja `notas/nota.md` fuera del inventario (premisa del caso): {:?}",
        a.documents
    );
    assert_eq!(
        destino(&a, "docs/guia.md", "../notas/nota.md"),
        LinkTarget::WorkspaceFile(rp("notas/nota.md")),
        "un `.md` que existe en disco pero está excluido del descubrimiento es `WorkspaceFile`, \
         no `Missing` (`§20.6` precisión 2): los `.md` excluidos van a `other_files`, no a \
         `documents`"
    );
    assert!(
        destinos_reportados_como_perdidos(&a, "docs/guia.md").is_empty(),
        "…y por tanto no dispara `LINK-TARGET-MISSING`, que sobre un destino `.md` sería un `Err` \
         y tumbaría la puerta de CI por un fichero que está ahí"
    );
}
