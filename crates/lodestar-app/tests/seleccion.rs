//! **E21-H02** — Selecciones masivas por consulta (`ARCHITECTURE.md §20.11`,
//! `REFACTOR_PHASE_2 §Fase 12 (Operaciones masivas basadas en consulta)`). Fase ROJA.
//!
//! ## La forma del wire que fijan estos tests
//!
//! `change_plan` gana una forma de **selección** además del array de operaciones sueltas. Cuando el
//! valor de operaciones es un OBJETO
//!
//! ```json
//! { "selection": { "where": "<consulta E19>" | "filter": { … } },
//!   "operation":  { "<op universal>": { <parámetros> } } }
//! ```
//!
//! la consulta E19 (`§Fase 5`) selecciona documentos y la `operation` se expande a **una
//! `NormalizedOperation` por documento seleccionado** (el array suelto `[ {op…}, … ]` sigue
//! valiendo tal cual). La `operation` codifica el tipo como CLAVE (`{patch_frontmatter: {…}}`),
//! según el ejemplo de la historia; solo las ops con sentido en masa (`patch_frontmatter`,
//! `replace_text`, `delete`) — `create` no aplica a documentos existentes, así que estos tests solo
//! ejercen `patch_frontmatter`. El valor de `patch_frontmatter` es el merge-patch (RFC 7386) que se
//! aplica a cada documento que casa.
//!
//! (E21-H02 admitía una cuarta op en masa, `apply_fix`. **E23-H11 la retiró** de la superficie
//! entera —enum, contrato y normalizador— porque no hay productor de `fixes` desde E20-H03 y la
//! op siempre fallaba; hoy pedirla en `operation` es `INVALID_SCHEMA`.)
//!
//! ## Por qué son ROJOS hoy
//!
//! `App::change_plan` exige que las operaciones sean un ARRAY (`raw_ops.as_array()`), así que un
//! objeto `{selection, operation}` se rechaza hoy con `Err(InvalidSchema)`. Los tres tests fallan
//! por esa razón (el `.expect()` sobre `change_plan` entra en pánico) hasta que E21-H02 enseñe a
//! `change_plan` a interpretar la selección.
//!
//! `seleccion_captura_revisiones` fija además una clave de wire NUEVA en el plan: `capturedRevisions`
//! (objeto `path → "blake3:…"`), donde el plan registra la revisión de cada documento seleccionado
//! (`§Fase 12`: query → documentos → **snapshot de revisiones** → … → change plan). Se observa
//! serializando el `PlanResult` a JSON, de modo que el test no depende de un símbolo Rust concreto
//! del struct — pero SÍ fija que esa clave debe existir y su forma.

use std::path::Path;

use lodestar_app::App;
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::DocumentRef;
use lodestar_core::types::ErrorCode;
use lodestar_core::types::RelPath;
use serde_json::{json, Value};

/// Escribe un `.md` dentro del workspace temporal, creando los directorios intermedios.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Política permisiva: el criterio a probar es la EXPANSIÓN de la selección, no el veredicto de
/// conformidad — sin esto un plan podría fallar por una razón distinta de la que se fija.
fn policy_permisiva() -> PlanPolicy {
    PlanPolicy {
        require_valid_result: false,
        allow_warnings: true,
    }
}

/// Referencia a un documento por su path (identidad v2, `id: None`).
fn doc_ref(path: &str) -> DocumentRef {
    DocumentRef {
        path: RelPath::new(path).unwrap(),
        id: None,
    }
}

/// Workspace con 5 documentos donde la consulta `type = "decision" and status = "draft"` casa
/// EXACTAMENTE dos (`d1.md`, `d2.md`): el resto queda fuera por `type` (`index.md`, `n1.md`) o por
/// `status` (`d3.md`), de modo que la selección no puede colar ni saltarse ninguno.
fn app_con_decisiones() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Índice\n---\n\n# Índice\n",
    );
    escribe(
        dir.path(),
        "d1.md",
        "---\ntype: decision\nstatus: draft\ntitle: D1\n---\n\n# D1\n",
    );
    escribe(
        dir.path(),
        "d2.md",
        "---\ntype: decision\nstatus: draft\ntitle: D2\n---\n\n# D2\n",
    );
    escribe(
        dir.path(),
        "d3.md",
        "---\ntype: decision\nstatus: accepted\ntitle: D3\n---\n\n# D3\n",
    );
    escribe(
        dir.path(),
        "n1.md",
        "---\ntype: note\nstatus: draft\ntitle: N1\n---\n\n# N1\n",
    );
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");
    (dir, app)
}

/// El wire de una selección masiva `patch_frontmatter` con la consulta `where` dada.
fn seleccion_patch(where_expr: &str) -> Value {
    json!({
        "selection": { "where": where_expr },
        "operation": { "patch_frontmatter": { "status": "review" } }
    })
}

/// `seleccion_masiva_patch` — **Dado** `{selection:{where:"type = \"decision\" and status =
/// \"draft\""}, operation:{patch_frontmatter:{status:"review"}}}`, **Cuando** se planifica,
/// **Entonces** el plan tiene una op por documento que casa la consulta (d1, d2), cada una un
/// `patch_frontmatter` que fija `status: review`.
#[test]
fn seleccion_masiva_patch() {
    let (_dir, app) = app_con_decisiones();

    let plan = app
        .change_plan(
            None,
            &seleccion_patch("type = \"decision\" and status = \"draft\""),
            policy_permisiva(),
        )
        .expect("una selección masiva válida debe producir un plan");

    // Se observa la forma normalizada por su serialización JSON (evita depender de `serde_yaml` en
    // el binario de test): una op por documento que casa, y ninguna más.
    let ops = serde_json::to_value(&plan.normalized_operations).unwrap();
    let arr = ops
        .as_array()
        .unwrap_or_else(|| panic!("normalizedOperations debe ser un array: {ops}"));
    assert_eq!(
        arr.len(),
        2,
        "la selección debe expandirse a UNA op por documento que casa (d1, d2), no {}: {ops}",
        arr.len(),
    );

    let mut paths: Vec<String> = Vec::new();
    for op in arr {
        assert_eq!(
            op["op"], "patch_frontmatter",
            "cada op de la selección debe ser un `patch_frontmatter`: {op}",
        );
        // DISCRIMINADOR: el patch fija de verdad `status: review` (una expansión vacua que no
        // llevara el patch pasaría el conteo pero fallaría aquí).
        assert_eq!(
            op["patch"]["status"], "review",
            "cada op debe aplicar el patch `status: review` de la selección: {op}",
        );
        paths.push(
            op["path"]
                .as_str()
                .unwrap_or_else(|| panic!("la op debe llevar `path`: {op}"))
                .to_string(),
        );
    }
    paths.sort();
    assert_eq!(
        paths,
        vec!["d1.md".to_string(), "d2.md".to_string()],
        "la selección debe expandirse EXACTAMENTE sobre los documentos que casan (d1, d2)",
    );
}

/// `seleccion_vacia` — **Dado** una selección que no casa ningún documento, **Cuando** se
/// planifica, **Entonces** el plan es vacío (sin cambios), SIN error.
#[test]
fn seleccion_vacia() {
    let (_dir, app) = app_con_decisiones();

    let plan = app
        .change_plan(
            None,
            // Ningún documento tiene `type = "inexistente"`.
            &seleccion_patch("type = \"inexistente\""),
            policy_permisiva(),
        )
        .expect("una selección que no casa nada debe planificar SIN error (plan vacío)");

    assert!(
        plan.normalized_operations.is_empty(),
        "una selección vacía debe dar un plan sin operaciones: {:?}",
        plan.normalized_operations,
    );
    assert_eq!(
        plan.impact.affected_count, 0,
        "una selección vacía no afecta a ningún documento: {:?}",
        plan.impact,
    );
}

/// `seleccion_captura_revisiones` — **Dado** una selección masiva, **Cuando** se planifica,
/// **Entonces** cada documento del plan lleva su `DocumentRevision` capturada.
///
/// La captura se fija como una clave de wire del plan: `capturedRevisions`, un objeto `path →
/// "blake3:…"` con una entrada por documento seleccionado, igual a su revisión ACTUAL en disco (la
/// misma que reporta `knowledge_get`). Se lee de la serialización JSON del `PlanResult`.
#[test]
fn seleccion_captura_revisiones() {
    let (_dir, app) = app_con_decisiones();

    let plan = app
        .change_plan(
            None,
            &seleccion_patch("type = \"decision\" and status = \"draft\""),
            policy_permisiva(),
        )
        .expect("una selección masiva válida debe producir un plan");

    let plan_json = serde_json::to_value(&plan).unwrap();
    let captured = plan_json
        .get("capturedRevisions")
        .and_then(Value::as_object)
        .unwrap_or_else(|| {
            panic!(
                "el plan de una selección masiva debe llevar `capturedRevisions` (objeto \
                 path→revisión): {plan_json}"
            )
        });

    for p in ["d1.md", "d2.md"] {
        let esperada = app
            .knowledge_get(&doc_ref(p), &[], None)
            .expect("el documento seleccionado debe existir")
            .revision;
        let capturada = captured.get(p).and_then(Value::as_str).unwrap_or_else(|| {
            panic!("`capturedRevisions` debe tener entrada para {p}: {plan_json}")
        });
        assert_eq!(
            capturada, esperada.0,
            "la revisión capturada de {p} debe ser su DocumentRevision actual",
        );
    }
    assert_eq!(
        captured.len(),
        2,
        "`capturedRevisions` debe tener exactamente una entrada por documento seleccionado (d1, d2): \
         {plan_json}",
    );
}

// ---------------------------------------------------------------------------
// E26-H08 — Una selección masiva no salta documentos en silencio
//
// Los tres tests de E21-H02 de arriba siguen VERDES tal cual: `Ok(false)` sigue siendo exclusión
// (`seleccion_vacia` lo prueba con una consulta perfectamente tipada que no casa nada), y lo único
// que cambia de tratamiento es `Err(TypeError)`.
//
// Lo que E19-H04/E21-H02 decidieron —y esta historia REVISA— está en el rustdoc de
// `expand_selection`: *«Un `TypeError` sobre ESTE documento lo excluye (no casa), sin propagarse al
// plan entero»*. El criterio era el de E19: el corpus es heterogéneo y un tipo incompatible no debe
// tumbar la consulta sobre los demás. E24-H07 ya revisó ese mismo criterio para el PARSEO («una
// respuesta silenciosamente equivocada es peor que un error») y E26-H08 lo lleva a la EVALUACIÓN:
// una selección masiva que se salta documentos produce un plan que afecta a menos ficheros de los
// que el agente cree haber seleccionado, y nada en la respuesta lo delata.
// ---------------------------------------------------------------------------

/// La consulta del defecto: orden entre un campo numérico y un literal string
/// (`TypeError::OrderNotDefined`, E19-H01).
const ORDEN_CRUZADO: &str = "priority >= \"high\"";

/// Workspace con `priority` de tipos MEZCLADOS. El reparto es discriminante para el criterio de
/// determinismo: `alfa.md` va primero en el orden total pero NO yerra (string vs string), `bravo.md`
/// es el primero que yerra —el que debe salir nombrado— y `zulu.md` yerra el último.
/// (`primer_type_error_en_el_orden_total`, en `lodestar-core/tests/consulta.rs`, clava esa premisa
/// sobre el mismo fixture.)
fn app_con_prioridades_mixtas() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "alfa.md",
        "---\npriority: high\nstatus: draft\n---\n\n# Alfa\n",
    );
    escribe(
        dir.path(),
        "bravo.md",
        "---\npriority: 2\nstatus: draft\n---\n\n# Bravo\n",
    );
    escribe(
        dir.path(),
        "charlie.md",
        "---\npriority: urgent\nstatus: draft\n---\n\n# Charlie\n",
    );
    escribe(
        dir.path(),
        "zulu.md",
        "---\npriority: 9\nstatus: draft\n---\n\n# Zulu\n",
    );
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");
    (dir, app)
}

/// `seleccion_con_type_error_aborta_el_plan` — **Dado** una selección masiva cuya consulta choca de
/// tipo con algunos documentos, **Cuando** se planifica, **Entonces** falla con `INVALID_SCHEMA` y
/// un mensaje que nombra el campo, los dos tipos y el primer documento del orden total que yerra,
/// **y** ese error es el MISMO que da `knowledge_search` con la misma consulta.
///
/// Gemelo por la API de `misma_consulta_mismo_error_en_search_y_en_plan` y
/// `el_type_error_reportado_es_determinista` (por el wire, en `lodestar-mcp/tests/mcp.rs`). Se prueba
/// aquí además porque el `AppError` es observable entero —código y mensaje como campos, no como
/// texto concatenado— y porque esta es la superficie donde el defecto es más caro: el plan.
#[test]
fn seleccion_con_type_error_aborta_el_plan() {
    let (_dir, app) = app_con_prioridades_mixtas();

    let err = app
        .change_plan(None, &seleccion_patch(ORDEN_CRUZADO), policy_permisiva())
        .expect_err(
            "una selección masiva cuya consulta yerra de tipo sobre parte del corpus debe FALLAR: \
             hasta v0.4.0 `expand_selection` se saltaba en silencio los documentos que erraban \
             (`alfa.md`/`charlie.md` entraban, `bravo.md`/`zulu.md` desaparecían) y devolvía un plan \
             de aspecto legítimo que afectaba a menos ficheros de los seleccionados",
        );

    assert_eq!(
        err.code,
        ErrorCode::InvalidSchema,
        "el código es el del catálogo para «tu entrada no es respondible sobre estos datos»: {err}"
    );
    let bajo = err.message.to_lowercase();
    assert!(
        bajo.contains("priority"),
        "el mensaje debe nombrar el campo que choca: {err}"
    );
    assert!(
        ["number", "numero", "número", "numérico"]
            .iter()
            .any(|t| bajo.contains(t)),
        "…el tipo que tiene el campo en el documento: {err}"
    );
    assert!(
        ["string", "cadena", "texto"]
            .iter()
            .any(|t| bajo.contains(t)),
        "…y el tipo del literal con el que se comparó: {err}"
    );
    assert!(
        err.message.contains("bravo.md"),
        "…y el documento sobre el que se produjo, que debe ser el PRIMERO del orden total de \
         `Analysis::documents` que yerra (no `alfa.md`, que va antes y no yerra): {err}"
    );
    assert!(
        !err.message.contains("zulu.md"),
        "…y solo ese: nombrar el último que yerra haría depender el mensaje de dónde paró el motor: \
         {err}"
    );

    // Equivalencia de superficies (`§20.10`): la misma consulta, el mismo veredicto y la misma
    // redacción por `knowledge_search`. Si divergieran, el agente aprendería a corregir con una y
    // se quedaría a ciegas con la otra.
    let e_search = app
        .knowledge_search("", Some(ORDEN_CRUZADO), None, &[], None, None)
        .expect_err("`knowledge_search` con la misma consulta también debe fallar");
    assert_eq!(
        e_search, err,
        "`knowledge_search` y `change_plan.selection` comparten el lenguaje, así que comparten el \
         error: mismo código y mismo mensaje"
    );
}

/// `seleccion_con_campo_ausente_sigue_expandiendo` — control anti-vacuo del criterio anterior: un
/// campo AUSENTE excluye su documento sin error, como hasta ahora.
///
/// El arreglo no puede consistir en abortar ante todo lo que no sea `Ok(true)`: la ausencia
/// cortocircuita antes de comprobar tipos (`campo_inexistente`, E19-H01) y sigue siendo exclusión.
/// El documento sin `priority` va primero en el orden total a propósito.
#[test]
fn seleccion_con_campo_ausente_sigue_expandiendo() {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "a-sin-priority.md",
        "---\nstatus: draft\n---\n\n# Sin prioridad\n",
    );
    escribe(
        dir.path(),
        "b-con-priority.md",
        "---\nstatus: draft\npriority: 5\n---\n\n# Con prioridad\n",
    );
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");

    let plan = app
        .change_plan(None, &seleccion_patch("priority >= 3"), policy_permisiva())
        .expect("preguntar por una clave que un documento no tiene es legítimo, no un error");

    let paths: Vec<String> = serde_json::to_value(&plan.normalized_operations)
        .unwrap()
        .as_array()
        .expect("normalizedOperations debe ser un array")
        .iter()
        .map(|op| op["path"].as_str().unwrap_or_default().to_string())
        .collect();
    assert_eq!(
        paths,
        vec!["b-con-priority.md".to_string()],
        "la selección se expande sobre el documento que tiene la clave con el tipo correcto, y el \
         que no la tiene queda fuera SIN ruido"
    );
}

// ---------------------------------------------------------------------------
// E28-H04 — criterio de selección masiva que E28-H02 dejó pendiente (control ANTI-VACUO).
//
// H02 declaró en su «Alcance» que el guard de colisión de `create`/`move` no tiene superficie que
// tocar en la selección masiva, porque `create`/`move` NO son ops admitidas en esa vía (el contrato
// ya lo dice: *«create/move no aplican a una selección de documentos existentes»*, y
// `single_operation` lo hace cumplir). Ese criterio se declaró «se confirma con un criterio
// anti-vacuo, no con lógica nueva» y se quedó sin test; la adenda de H04 lo cierra aquí.
//
// H04 refuerza la razón de ser de este test: su acumulado de ocupación queda deliberadamente FUERA
// de la vía de selección (cada documento seleccionado genera como mucho una operación, y no hay
// secuencia intra-selección que acumular). Esa afirmación solo se sostiene mientras `create`/`move`
// sigan sin poder entrar por aquí — que es exactamente lo que este test vigila.
//
// **NACE VERDE, a propósito**: es un control del comportamiento YA implementado
// (`crates/lodestar-app/src/lib.rs`, `single_operation`), no una fase roja. Su valor es que si
// alguien admitiera `create` o `move` en masa —o cambiara el mensaje que le dice al agente por qué
// no—, el hueco quedaría visible en lugar de abrirse en silencio.
// ---------------------------------------------------------------------------

/// `seleccion_masiva_rechaza_create_y_move` — **Dado** una selección masiva cuya `operation` pide
/// `{"create": {…}}` o `{"move": {…}}`, **Cuando** se planifica, **Entonces** se rechaza con
/// `INVALID_SCHEMA` y el mensaje de `single_operation`, que explica el porqué de cada una: «create»
/// no aplica a documentos existentes y «move» necesita un destino por documento.
#[test]
fn seleccion_masiva_rechaza_create_y_move() {
    let (_dir, app) = app_con_decisiones();

    for (kind, params) in [
        ("create", json!({ "path": "nuevo.md", "body": "# Nuevo\n" })),
        ("move", json!({ "to": "archivo/destino.md" })),
    ] {
        let err = app
            .change_plan(
                None,
                &json!({
                    "selection": { "where": "type = \"decision\" and status = \"draft\"" },
                    "operation": { kind: params }
                }),
                policy_permisiva(),
            )
            .expect_err(
                "«create»/«move» no son ops admitidas en una selección masiva: pedirlas debe \
                 rechazarse, no expandirse sobre los documentos que casan la consulta",
            );

        assert_eq!(
            err.code,
            ErrorCode::InvalidSchema,
            "una op no admitida en masa es un error de FORMA de la operación, no de estado del \
             workspace: {err}"
        );
        assert!(
            err.message.contains(kind),
            "el mensaje debe NOMBRAR la op que el agente pidió («{kind}»), para que sepa cuál de \
             las dos claves retirar: {err}"
        );
        assert!(
            err.message.contains("patch_frontmatter")
                && err.message.contains("replace_text")
                && err.message.contains("delete"),
            "…y debe enumerar las que SÍ tienen sentido en masa (patch_frontmatter, replace_text, \
             delete), que es lo que convierte el rechazo en accionable: {err}"
        );
    }
}
