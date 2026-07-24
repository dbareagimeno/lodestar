//! Tests de integración de E10-H04: resolución de `DocumentRef` contra un workspace abierto.
//!
//! Criterio de aceptación `ref_inexistente`: un `DocumentRef` a un path que NO existe en el workspace,
//! Cuando se resuelve con `App::resolve_ref`, Entonces devuelve `Err(ErrorCode::DocumentNotFound)`
//! (wire `DOCUMENT_NOT_FOUND`). La deserialización de `DocumentRef` (path válido / traversal) se prueba
//! en el core (`crates/lodestar-core/tests/core.rs`), aquí probamos la RESOLUCIÓN, que exige un
//! `Workspace` abierto (el core es puro y no toca el filesystem).
//!
//! Fase ROJA: NI el struct `DocumentRef` (core) NI el método `App::resolve_ref` existen todavía. Este
//! test hace ROJO por símbolos ausentes hasta que E10-H04 los implemente.
//!
//! API objetivo asumida (el implementador debe crearla con ESTE nombre/firma):
//!
//! ```ignore
//! // en `lodestar-core::types`:
//! pub struct DocumentRef { pub path: RelPath, pub id: Option<DocumentId> }  // deser: { "path": … }
//! // en `lodestar-app`:
//! impl App {
//!     pub fn resolve_ref(&self, r: &DocumentRef) -> Result<RelPath, ErrorCode>;
//! }
//! ```
//!
//! `DocumentRef` se construye por deserialización (no por literal de struct) para NO acoplar el test
//! a la visibilidad de sus campos ni al nombre del newtype `DocumentId` — el contrato aseverado es el
//! `ErrorCode`/`RelPath` resultante, no la mecánica interna del struct.

use std::path::Path;

use lodestar_app::App;
use lodestar_core::types::{DocumentRef, ErrorCode, RelPath};

/// Escribe un `.md` (creando los directorios intermedios) dentro del workspace temporal.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Monta un `App` sobre un workspace temporal con un index raíz + un documento conforme (`alfa.md`).
/// Se apoya en `App::open` (que delega en `Workspace::open`, sin exigir git). El `TempDir` se
/// devuelve para mantener el directorio vivo mientras dure el test.
fn app_con_workspace() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Alfa](alfa.md)\n",
    );
    escribe(
        dir.path(),
        "alfa.md",
        "---\ntype: Concept\ntitle: Alfa\ndescription: Primer concept\n---\n\n# Resumen\n\ncuerpo\n",
    );
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");
    (dir, app)
}

/// `ref_inexistente` — Dado un `DocumentRef` a un path que no existe en el workspace, Cuando se resuelve,
/// Entonces `App::resolve_ref` devuelve `Err(ErrorCode::DocumentNotFound)`.
#[test]
fn ref_inexistente() {
    let (_dir, app) = app_con_workspace();
    let referencia: DocumentRef = serde_json::from_str(r#"{"path":"no-existe.md"}"#)
        .expect("un path válido pero inexistente debe deserializar");
    let resultado = app.resolve_ref(&referencia);
    assert!(
        matches!(resultado, Err(ErrorCode::DocumentNotFound)),
        "un DocumentRef a un path inexistente debe dar Err(DocumentNotFound), dio {resultado:?}",
    );
}

/// `error_code_documento` (E16-H06) — **Dado** una tool que recibe un documento inexistente,
/// **Cuando** falla, **Entonces** el código de error **de wire** es `DOCUMENT_NOT_FOUND`.
///
/// Es el único criterio de comportamiento del renombre `Concept` → `Document` (`§20.3`): no basta
/// con que la variante de Rust se llame `DocumentNotFound`, tiene que **serializar** así. v0.3 es
/// incompatible con v0.2 sin alias, así que el test fija además que `CONCEPT_NOT_FOUND` ya no
/// aparece en el wire.
///
/// Se ejerce a través de una tool real ([`App::knowledge_get`], superficie congelada de `§19.6`),
/// no de la enum a pelo, porque lo que el contrato promete es lo que un agente ve por el protocolo.
#[test]
fn error_code_documento() {
    let (_dir, app) = app_con_workspace();
    let referencia: DocumentRef = serde_json::from_str(r#"{"path":"no-existe.md"}"#)
        .expect("un path válido pero inexistente debe deserializar");

    let code = app
        .knowledge_get(&referencia, &[], None)
        .expect_err("`knowledge_get` sobre un documento inexistente debe fallar");

    // El wire: `ErrorCode` serializa a la cadena SCREAMING_SNAKE del catálogo.
    let wire = serde_json::to_value(code).expect("`ErrorCode` debe serializar");
    assert_eq!(
        wire,
        serde_json::json!("DOCUMENT_NOT_FOUND"),
        "el código de wire de «documento inexistente» debe ser DOCUMENT_NOT_FOUND, es {wire:?}",
    );
    // Y por la vía sin serde (`ErrorCode::as_str`), que es la que usan CLI/grep.
    assert_eq!(
        code.as_str(),
        "DOCUMENT_NOT_FOUND",
        "`ErrorCode::as_str` debe coincidir con el wire de serde",
    );

    // Sin alias: `CONCEPT_NOT_FOUND` no deserializa a ninguna variante (v0.3 es incompatible).
    assert!(
        serde_json::from_value::<ErrorCode>(serde_json::json!("CONCEPT_NOT_FOUND")).is_err(),
        "CONCEPT_NOT_FOUND se retiró del catálogo sin alias de compatibilidad",
    );
}

/// Caso positivo (para no ser vacuo): un `DocumentRef` a un path que SÍ existe resuelve `Ok(path)`
/// con el `RelPath` esperado.
#[test]
fn ref_existente_resuelve() {
    let (_dir, app) = app_con_workspace();
    let referencia: DocumentRef =
        serde_json::from_str(r#"{"path":"alfa.md"}"#).expect("`alfa.md` debe deserializar");
    let resuelto = app
        .resolve_ref(&referencia)
        .expect("un DocumentRef a un path existente debe resolver a Ok");
    assert_eq!(
        resuelto,
        RelPath::new("alfa.md").unwrap(),
        "la resolución debe devolver el RelPath `alfa.md`",
    );
}
