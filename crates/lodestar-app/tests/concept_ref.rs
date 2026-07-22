//! Tests de integración de E10-H04: resolución de `ConceptRef` contra un bundle abierto.
//!
//! Criterio de aceptación `ref_inexistente`: un `ConceptRef` a un path que NO existe en el bundle,
//! Cuando se resuelve con `App::resolve_ref`, Entonces devuelve `Err(ErrorCode::ConceptNotFound)`
//! (wire `CONCEPT_NOT_FOUND`). La deserialización de `ConceptRef` (path válido / traversal) se prueba
//! en el core (`crates/lodestar-core/tests/core.rs`), aquí probamos la RESOLUCIÓN, que exige un
//! `Workspace` abierto (el core es puro y no toca el filesystem).
//!
//! Fase ROJA: NI el struct `ConceptRef` (core) NI el método `App::resolve_ref` existen todavía. Este
//! test hace ROJO por símbolos ausentes hasta que E10-H04 los implemente.
//!
//! API objetivo asumida (el implementador debe crearla con ESTE nombre/firma):
//!
//! ```ignore
//! // en `lodestar-core::types`:
//! pub struct ConceptRef { pub path: RelPath, pub id: Option<ConceptId> }  // deser: { "path": … }
//! // en `lodestar-app`:
//! impl App {
//!     pub fn resolve_ref(&self, r: &ConceptRef) -> Result<RelPath, ErrorCode>;
//! }
//! ```
//!
//! `ConceptRef` se construye por deserialización (no por literal de struct) para NO acoplar el test
//! a la visibilidad de sus campos ni al nombre del newtype `ConceptId` — el contrato aseverado es el
//! `ErrorCode`/`RelPath` resultante, no la mecánica interna del struct.

use std::path::Path;

use lodestar_app::App;
use lodestar_core::types::{ConceptRef, ErrorCode, RelPath};

/// Escribe un `.md` (creando los directorios intermedios) dentro del bundle temporal.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Monta un `App` sobre un bundle temporal con un index raíz + un concept conforme (`alfa.md`).
/// Se apoya en `App::open` (que delega en `Workspace::open`, sin exigir git). El `TempDir` se
/// devuelve para mantener el directorio vivo mientras dure el test.
fn app_con_bundle() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "index.md",
        "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Alfa](alfa.md)\n",
    );
    escribe(
        dir.path(),
        "alfa.md",
        "---\ntype: Concept\ntitle: Alfa\ndescription: Primer concept\n---\n\n# Resumen\n\ncuerpo\n",
    );
    let app = App::open(dir.path()).expect("el bundle temporal debe abrir");
    (dir, app)
}

/// `ref_inexistente` — Dado un `ConceptRef` a un path que no existe en el bundle, Cuando se resuelve,
/// Entonces `App::resolve_ref` devuelve `Err(ErrorCode::ConceptNotFound)`.
#[test]
fn ref_inexistente() {
    let (_dir, app) = app_con_bundle();
    let referencia: ConceptRef = serde_json::from_str(r#"{"path":"no-existe.md"}"#)
        .expect("un path válido pero inexistente debe deserializar");
    let resultado = app.resolve_ref(&referencia);
    assert!(
        matches!(resultado, Err(ErrorCode::ConceptNotFound)),
        "un ConceptRef a un path inexistente debe dar Err(ConceptNotFound), dio {resultado:?}",
    );
}

/// Caso positivo (para no ser vacuo): un `ConceptRef` a un path que SÍ existe resuelve `Ok(path)`
/// con el `RelPath` esperado.
#[test]
fn ref_existente_resuelve() {
    let (_dir, app) = app_con_bundle();
    let referencia: ConceptRef =
        serde_json::from_str(r#"{"path":"alfa.md"}"#).expect("`alfa.md` debe deserializar");
    let resuelto = app
        .resolve_ref(&referencia)
        .expect("un ConceptRef a un path existente debe resolver a Ok");
    assert_eq!(
        resuelto,
        RelPath::new("alfa.md").unwrap(),
        "la resolución debe devolver el RelPath `alfa.md`",
    );
}
