//! Tests de integración de E10-H01: forma del envelope de protocolo.
//!
//! Criterio de aceptación `envelope_shape`: un `Envelope<Value>` serializado lleva EXACTAMENTE
//! las 7 claves wire camelCase `ok`/`workspaceRevision`/`summary`/`data`/`diagnostics`/`warnings`/
//! `resourceLinks`.

use lodestar_app::Envelope;
use lodestar_core::types::WorkspaceRevision;
use serde_json::{json, Value};

/// `envelope_shape` — Dado un `Envelope<Value>` serializado, Cuando se inspecciona, Entonces lleva
/// las 7 claves wire camelCase exactas y nada más.
#[test]
fn envelope_shape() {
    // Construimos un envelope mínimo. Los `Vec` van vacíos: el foco del criterio es la forma wire,
    // no el contenido. `resource_links`/`diagnostics` infieren su tipo del campo del struct, así que
    // no necesitamos nombrar `ResourceLink`/`Check` aquí.
    let envelope: Envelope<Value> = Envelope {
        ok: true,
        workspace_revision: WorkspaceRevision(String::from("blake3:0000")),
        summary: String::from("ok"),
        data: json!({ "hello": "world" }),
        diagnostics: Vec::new(),
        warnings: Vec::new(),
        resource_links: Vec::new(),
    };

    let serialized = serde_json::to_value(&envelope).expect("el envelope debe serializar");
    let object = serialized
        .as_object()
        .expect("el envelope serializado debe ser un objeto JSON");

    // Las 7 claves esperadas, en camelCase de wire.
    let esperadas = [
        "ok",
        "workspaceRevision",
        "summary",
        "data",
        "diagnostics",
        "warnings",
        "resourceLinks",
    ];

    for clave in esperadas {
        assert!(
            object.contains_key(clave),
            "falta la clave wire `{clave}` en el envelope serializado: {object:?}"
        );
    }

    // Exactamente 7 claves: ni una de más (p. ej. un snake_case filtrado) ni de menos.
    assert_eq!(
        object.len(),
        esperadas.len(),
        "el envelope debe tener exactamente 7 claves wire, tiene {}: {:?}",
        object.len(),
        object.keys().collect::<Vec<_>>()
    );
}
