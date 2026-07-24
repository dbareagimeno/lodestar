//! **E21-H03** — `delete_document` con política de enlaces entrantes, POR LA FACHADA
//! (`ARCHITECTURE.md §20.11`, `REFACTOR_PHASE_2 §Fase 12 (Eliminación)`).
//!
//! `delete_document` exige una política explícita ante los backlinks (`InboundLinksPolicy`, ya en
//! `types.rs`). Estos dos tests fijan, a través de `App::change_plan`, las dos políticas que la
//! historia nombra:
//!   · `reject` → se rechaza con `INBOUND_LINKS_EXIST` (no se borra un documento referenciado);
//!   · `remove_links` → se borra Y se quitan los enlaces entrantes, y el plan lo refleja.
//!
//! Nota de fase: la mecánica de `reject`/`remove_links` ya existe en el core desde E12-H06
//! (`plan::normalize_delete`) y la fachada la despacha (`op: "delete"`), así que estos tests son
//! COBERTURA de los criterios de aceptación de E21-H03 por la fachada — verifican que la capacidad
//! sigue en pie por el camino MCP/CLI, no un comportamiento nuevo de esta historia. Las políticas
//! `retarget`/`create_stub` quedan para el implementador (la historia no las fija).

use std::path::Path;

use lodestar_app::App;
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::{ErrorCode, RelPath};
use serde_json::{json, Value};

/// Escribe un `.md` dentro del workspace temporal, creando los directorios intermedios.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Política permisiva: el criterio a probar es la política de enlaces entrantes, no el veredicto de
/// conformidad del resultado.
fn policy_permisiva() -> PlanPolicy {
    PlanPolicy {
        require_conformant_result: false,
        allow_warnings: true,
    }
}

/// Workspace con `objetivo.md` referenciado desde `a.md` y `b.md` (2 backlinks).
fn app_con_referenciado() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Índice\n---\n\n# Índice\n\n* [A](a.md)\n* [B](b.md)\n",
    );
    escribe(
        dir.path(),
        "objetivo.md",
        "---\ntype: doc\ntitle: Objetivo\n---\n\n# Objetivo\n\ncuerpo\n",
    );
    for slug in ["a", "b"] {
        escribe(
            dir.path(),
            &format!("{slug}.md"),
            &format!("---\ntype: doc\ntitle: {slug}\n---\n\n# {slug}\n\n[Objetivo](objetivo.md)\n"),
        );
    }
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");
    (dir, app)
}

/// `delete_rechaza_con_backlinks` — **Dado** un `delete_document` sobre un documento con backlinks y
/// política `reject`, **Cuando** se planifica, **Entonces** se rechaza con `INBOUND_LINKS_EXIST`.
#[test]
fn delete_rechaza_con_backlinks() {
    let (_dir, app) = app_con_referenciado();

    let resultado = app.change_plan(
        None,
        &json!([
            { "op": "delete", "ref": { "path": "objetivo.md" }, "inboundLinksPolicy": "reject" }
        ]),
        policy_permisiva(),
    );

    assert!(
        matches!(resultado, Err(ErrorCode::InboundLinksExist)),
        "borrar un documento referenciado con política `reject` debe dar Err(InboundLinksExist) \
         (wire INBOUND_LINKS_EXIST); dio {resultado:?}",
    );
    // Guarda de coherencia con el wire.
    assert_eq!(ErrorCode::InboundLinksExist.as_str(), "INBOUND_LINKS_EXIST");
}

/// `delete_exige_politica_explicita` — **Dado** un `delete_document` SIN `inboundLinksPolicy` sobre
/// un documento CON backlinks, **Cuando** se planifica, **Entonces** se rechaza por FALTA DE
/// POLÍTICA (esquema incompleto), no eligiendo `reject` en silencio.
///
/// `§Fase 12`: «`delete_document` … requiere una política explícita … no elegir una política
/// automáticamente». Hoy `normalize_raw_op` defaultea a `reject` (`_ => InboundLinksPolicy::Reject`),
/// así que un delete sin política y con backlinks YA da error — pero por la razón EQUIVOCADA
/// (`INBOUND_LINKS_EXIST`, «la política reject encontró backlinks»), no por «falta declarar la
/// política». El rojo lo distingue por el CÓDIGO, no por el mero hecho de que haya error:
///   · aquí debe ser `INVALID_SCHEMA` (campo requerido ausente),
///   · NUNCA `INBOUND_LINKS_EXIST` — ese es el camino de `delete_rechaza_con_backlinks`, donde la
///     política SÍ se declaró (`reject`).
/// Elijo fijar `INVALID_SCHEMA` (el código que la propia fachada ya usa para una op mal formada):
/// un `delete` al que le falta su campo ahora obligatorio es, exactamente, una op mal formada.
///
/// Contraste que lo blinda: el MISMO delete DECLARANDO la política (`remove_links`) debe **funcionar**
/// — lo que faltaba era la declaración, no que el documento fuera intocable.
#[test]
fn delete_exige_politica_explicita() {
    let (_dir, app) = app_con_referenciado();

    // (1) SIN `inboundLinksPolicy`: no puede colarse un `reject` implícito.
    let sin_politica = app.change_plan(
        None,
        &json!([ { "op": "delete", "ref": { "path": "objetivo.md" } } ]),
        policy_permisiva(),
    );
    assert!(
        matches!(sin_politica, Err(ErrorCode::InvalidSchema)),
        "un `delete` SIN `inboundLinksPolicy` debe rechazarse pidiendo una política explícita \
         (INVALID_SCHEMA: campo requerido ausente), NO elegir `reject` en silencio —que daría \
         INBOUND_LINKS_EXIST por la razón equivocada, el mismo camino que \
         `delete_rechaza_con_backlinks`—; dio {sin_politica:?}",
    );

    // (2) DECLARANDO la política, el mismo delete funciona: lo que faltaba era la declaración.
    let con_politica = app.change_plan(
        None,
        &json!([
            { "op": "delete", "ref": { "path": "objetivo.md" },
              "inboundLinksPolicy": "remove_links" }
        ]),
        policy_permisiva(),
    );
    assert!(
        con_politica.is_ok(),
        "declarar `inboundLinksPolicy: remove_links` debe permitir el plan (la declaración era lo \
         que faltaba, no que el documento fuera intocable); dio {con_politica:?}",
    );
}

/// `delete_remove_links` — **Dado** el mismo delete con política `remove_links`, **Cuando** se
/// planifica, **Entonces** los enlaces entrantes se eliminan (el texto queda, el destino se va) y el
/// plan lo refleja (borra `objetivo.md` y modifica los dos orígenes).
#[test]
fn delete_remove_links() {
    let (_dir, app) = app_con_referenciado();

    let plan = app
        .change_plan(
            None,
            &json!([
                { "op": "delete", "ref": { "path": "objetivo.md" },
                  "inboundLinksPolicy": "remove_links" }
            ]),
            policy_permisiva(),
        )
        .expect("borrar con `remove_links` sobre un documento referenciado no debe fallar");

    let ops = serde_json::to_value(&plan.normalized_operations).unwrap();
    let arr = ops
        .as_array()
        .unwrap_or_else(|| panic!("normalizedOperations debe ser un array: {ops}"));

    // 1) Exactamente un `delete` sobre `objetivo.md`.
    let deletes: Vec<&Value> = arr.iter().filter(|op| op["op"] == "delete").collect();
    assert_eq!(deletes.len(), 1, "debe haber un único `delete`: {ops}");
    assert_eq!(
        deletes[0]["path"], "objetivo.md",
        "el `delete` debe borrar `objetivo.md`: {ops}",
    );

    // 2) Por cada origen (a.md, b.md), una reescritura que QUITA el enlace: el destino desaparece,
    //    el texto del enlace se conserva.
    for src in ["a.md", "b.md"] {
        let reescritura = arr
            .iter()
            .find(|op| op["op"] == "replace_body" && op["path"] == src)
            .unwrap_or_else(|| panic!("debe haber un `replace_body` que reescriba {src}: {ops}"));
        let body = reescritura["body"].as_str().unwrap_or("");
        assert!(
            !body.contains("objetivo.md"),
            "tras `remove_links` el enlace a `objetivo.md` debe desaparecer de {src}: {body:?}",
        );
        assert!(
            body.contains("Objetivo"),
            "`remove_links` debe conservar el TEXTO del enlace en {src}: {body:?}",
        );
    }

    // 3) El plan lo refleja: `objetivo.md` borrado y los dos orígenes modificados.
    let objetivo = RelPath::new("objetivo.md").unwrap();
    assert!(
        plan.semantic_diff.deleted.contains(&objetivo),
        "el semanticDiff debe marcar `objetivo.md` como borrado: {:?}",
        plan.semantic_diff.deleted,
    );
    for src in ["a.md", "b.md"] {
        let p = RelPath::new(src).unwrap();
        assert!(
            plan.semantic_diff.modified.contains(&p),
            "el semanticDiff debe marcar {src} como modificado (enlace quitado): {:?}",
            plan.semantic_diff.modified,
        );
    }
}
