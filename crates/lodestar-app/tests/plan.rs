//! Tests de integración de E12-H09: persistencia del plan en `.lodestar/runtime/plans/`.
//!
//! `change_plan` (E12-H08) YA orquesta y devuelve un `PlanResult`, pero NO lo persiste. Esta
//! historia añade dos cosas:
//!   1. Al planificar con éxito, escribir el plan completo a `.lodestar/runtime/plans/<id>.json`
//!      (runtime: gitignored, NO canónico, FUERA de `WorkspaceRevision` — E9-H06/E10-H03).
//!   2. `App::load_plan` que lee un plan persistido y **rechaza los caducados** con `PLAN_EXPIRED`.
//!
//! Fase ROJA: `App::load_plan` NO existe todavía (símbolo ausente) y `change_plan` no escribe. En
//! consecuencia este fichero de test **no compila** hasta que E12-H09 cree `App::load_plan`; ese es
//! el rojo esperado y documentado (regla 2: «símbolo inexistente esperado — pueden no compilar»).
//! Una vez exista `load_plan` como stub `todo!()`, los asserts de `plan_persistido`/
//! `plan_caducado`/`plan_fuera_de_revision` fallan por la razón correcta (no hay fichero / no hay
//! caducidad).
//!
//! API objetivo asumida (el implementador debe crearla con ESTE nombre/firma):
//!
//! ```ignore
//! // en `lodestar-app`:
//! impl App {
//!     /// Carga el plan persistido `changeSetId`; `Err(ErrorCode::PlanExpired)` si `expiresAt` ya pasó.
//!     pub fn load_plan(&self, id: &ChangeSetId) -> Result<PlanResult, ErrorCode>;
//! }
//! ```
//!
//! Formato de persistencia asumido (aseverado de forma robusta, no acoplado al struct exacto):
//!   - Un fichero JSON por plan bajo `.lodestar/runtime/plans/`.
//!   - Con una clave de wire `planHash` (string `"blake3:<hex>"`) igual a la que devolvió el plan.
//!   - Con una clave de wire `expiresAt` (string de segundos epoch, mismo formato que
//!     `PlanResult::expires_at`), sobre la que `load_plan` decide la caducidad.
//!
//! El test NO fija si el nombre del fichero conserva el `changeset:<hex>` literal o lo sanea: escanea
//! el directorio `plans/` (que en estos tests contiene un único plan) y valida su contenido.

use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use lodestar_app::{App, PlanResult};
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::{ChangeSetId, ErrorCode};

/// Escribe un `.md` (creando los directorios intermedios) dentro del workspace temporal.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Monta un `App` sobre un workspace temporal con un index raíz + un documento conforme (`alfa.md`).
/// `App::open` crea el scaffold `.lodestar/runtime/{plans,receipts,staging}` (E9-H06). El `TempDir`
/// se devuelve para mantener el directorio vivo mientras dure el test.
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

/// Una propuesta mínima pero real: un `patch_frontmatter` inocuo sobre `alfa.md` (actualiza
/// `description`). Basta para que `change_plan` produzca un plan con una `normalizedOperation`.
fn una_operacion() -> serde_json::Value {
    serde_json::json!([
        { "op": "patch_frontmatter", "ref": { "path": "alfa.md" },
          "patch": { "description": "alfa actualizada por el plan" } },
    ])
}

/// Política permisiva: no exige resultado conforme y admite warnings, de modo que el plan siempre
/// se produce con éxito (el criterio no depende del veredicto de conformidad).
fn policy_permisiva() -> PlanPolicy {
    PlanPolicy {
        require_valid_result: false,
        allow_warnings: true,
    }
}

/// Directorio de planes runtime del workspace.
fn plans_dir(root: &Path) -> PathBuf {
    root.join(".lodestar").join("runtime").join("plans")
}

/// Localiza EL fichero de plan persistido y lo devuelve (path + JSON parseado). Asevera que el
/// directorio `plans/` contiene exactamente un `.json` (estos tests generan un único plan), sin
/// acoplarse al nombre exacto del fichero.
fn json_del_plan_unico(root: &Path) -> (PathBuf, serde_json::Value) {
    let dir = plans_dir(root);
    let jsons: Vec<PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| {
            panic!(
                "el directorio de planes {} debe existir: {e}",
                dir.display()
            )
        })
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|x| x.to_str()) == Some("json"))
        .collect();
    assert_eq!(
        jsons.len(),
        1,
        "tras un `change_plan` exitoso `plans/` debe contener exactamente un plan .json, hay {}: {jsons:?}",
        jsons.len(),
    );
    let ruta = jsons.into_iter().next().unwrap();
    let contenido = std::fs::read_to_string(&ruta).unwrap();
    let valor: serde_json::Value = serde_json::from_str(&contenido)
        .unwrap_or_else(|e| panic!("el plan persistido debe ser JSON válido: {e}\n{contenido}"));
    (ruta, valor)
}

/// El hash desnudo (sin el prefijo `changeset:`) de un `ChangeSetId`, para aseverar que el nombre
/// del fichero referencia al plan sin fijar el saneo del `:`.
fn hash_desnudo(id: &ChangeSetId) -> String {
    id.0.strip_prefix("changeset:").unwrap_or(&id.0).to_string()
}

/// `plan_persistido` — Dado un `change_plan` exitoso, Cuando termina, Entonces existe
/// `.lodestar/runtime/plans/<id>.json` y su contenido lleva el `planHash` que devolvió el plan.
#[test]
fn plan_persistido() {
    let (dir, app) = app_con_workspace();
    let plan = app
        .change_plan(None, &una_operacion(), policy_permisiva())
        .expect("el `change_plan` debe tener éxito y producir un plan");

    let (ruta, valor) = json_del_plan_unico(dir.path());

    // El fichero referencia al plan por su changeSetId (con o sin saneo del `:`).
    let nombre = ruta.file_name().unwrap().to_string_lossy();
    assert!(
        nombre.contains(&hash_desnudo(&plan.change_set_id)),
        "el nombre del plan persistido ({nombre}) debe referenciar el changeSetId {:?}",
        plan.change_set_id,
    );

    // El contenido persistido lleva el mismo `planHash` (clave de wire camelCase).
    let hash_persistido = valor
        .get("planHash")
        .and_then(serde_json::Value::as_str)
        .unwrap_or_else(|| panic!("el plan persistido debe llevar `planHash` (string): {valor}"));
    assert_eq!(
        hash_persistido, plan.plan_hash.0,
        "el `planHash` persistido debe coincidir con el que devolvió `change_plan`",
    );
}

/// `plan_caducado` — Dado un plan persistido con `expiresAt` en el pasado, Cuando se carga con
/// `App::load_plan`, Entonces devuelve `Err(ErrorCode::PlanExpired)` (wire `PLAN_EXPIRED`).
///
/// Montaje del plan caducado: se genera un plan real (por `change_plan`, que lo persiste con un
/// `expiresAt` futuro) y se REESCRIBE en su sitio el mismo JSON con `expiresAt` en el pasado — así
/// el fichero sigue siendo por lo demás plenamente válido y solo la caducidad cambia (no se induce
/// un error de deserialización distinto).
#[test]
fn plan_caducado() {
    let (dir, app) = app_con_workspace();
    let plan = app
        .change_plan(None, &una_operacion(), policy_permisiva())
        .expect("el `change_plan` debe tener éxito y producir un plan");

    // Reescribe el plan persistido con un `expiresAt` claramente pasado (epoch de hace una hora,
    // mismo formato de segundos epoch string que `PlanResult::expires_at`).
    let (ruta, mut valor) = json_del_plan_unico(dir.path());
    let pasado = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
        - 3600;
    valor["expiresAt"] = serde_json::Value::String(pasado.to_string());
    std::fs::write(&ruta, serde_json::to_vec(&valor).unwrap()).unwrap();

    let resultado = app.load_plan(&plan.change_set_id);
    assert!(
        matches!(&resultado, Err(e) if e.code == ErrorCode::PlanExpired),
        "cargar un plan con `expiresAt` en el pasado debe dar Err(PlanExpired), dio {resultado:?}",
    );
}

/// Guarda contra la vacuidad de `plan_caducado`: un plan VIGENTE (recién persistido, `expiresAt`
/// futuro) se carga con éxito y su `planHash` coincide — así `load_plan` no puede limitarse a
/// devolver `PlanExpired` siempre.
#[test]
fn plan_vigente_carga() {
    let (_dir, app) = app_con_workspace();
    let plan = app
        .change_plan(None, &una_operacion(), policy_permisiva())
        .expect("el `change_plan` debe tener éxito y producir un plan");

    let cargado: PlanResult = app
        .load_plan(&plan.change_set_id)
        .expect("un plan vigente recién persistido debe cargar con éxito");
    assert_eq!(
        cargado.plan_hash.0, plan.plan_hash.0,
        "el plan cargado debe conservar el `planHash` del plan persistido",
    );
}

/// `plan_fuera_de_revision` — Dado el plan persistido, Cuando se calcula `WorkspaceRevision`,
/// Entonces el plan no la afecta (es runtime, `.lodestar/` queda excluido).
///
/// Se compara la `baseWorkspaceRevision` que computa `change_plan` ANTES de persistir (R1) contra
/// la que computa un `App` reabierto DESPUÉS de que el plan quedó en disco (R2). Si la persistencia
/// del plan runtime entrara en la identidad del workspace, R2 diferiría de R1. La aserción previa
/// de que el fichero de plan existe blinda el test contra la vacuidad (si no se persistiera nada,
/// R1==R2 trivialmente).
#[test]
fn plan_fuera_de_revision() {
    let (dir, app) = app_con_workspace();

    // R1: revisión base que computa `change_plan` (sobre el disco PRE-persistencia). Esta llamada
    // persiste el plan en `.lodestar/runtime/plans/`.
    let plan = app
        .change_plan(None, &una_operacion(), policy_permisiva())
        .expect("el `change_plan` debe tener éxito y producir un plan");
    let r1 = plan.base_workspace_revision.clone();

    // El plan se persistió de verdad (si no, el test sería vacuo).
    let _ = json_del_plan_unico(dir.path());

    // R2: revisión base que computa un `App` reabierto (lectura fresca del disco, que ya incluye el
    // plan runtime en `.lodestar/`).
    let app2 = App::open(dir.path()).expect("reabrir el workspace debe funcionar");
    let plan2 = app2
        .change_plan(None, &una_operacion(), policy_permisiva())
        .expect("el segundo `change_plan` debe tener éxito");
    let r2 = plan2.base_workspace_revision;

    assert_eq!(
        r1, r2,
        "persistir el plan runtime NO debe alterar la WorkspaceRevision (el plan es runtime, \
         excluido de la identidad del workspace)",
    );
}

// ---------------------------------------------------------------------------
// E24-H03 — `change_plan` planifica sobre el estado YA RECUPERADO
//
// Defecto reproducido matando el servidor con SIGKILL a mitad de `change_apply`: la primera pareja
// `change_plan` + `change_apply` posterior fallaba SIEMPRE con `WRITE_CONFLICT` (10 de 11
// reproducciones), y el segundo intento funcionaba. La cadena era:
//
//   1. `change_plan` leía el disco SIN recuperar (renames parciales visibles) y fijaba ahí
//      `base_revision`.
//   2. `change_apply` recomputaba el `planHash` sobre ese mismo estado → coincidía, no saltaba
//      `PLAN_STALE`.
//   3. `apply_transaction` paso (2) llamaba a `recover()`, que restauraba los originales.
//   4. Paso (7) `reverify_base_revision` comparaba la base pre-recuperación contra el estado
//      post-recuperación → `WriteConflict`.
//
// El código además MENTÍA: `WRITE_CONFLICT` significa «otro escritor lo modificó entre el plan y el
// apply», y aquí quien lo modificó fue la recuperación del propio Lodestar.
//
// El montaje del estado «transacción a medias» usa las mismas primitivas públicas y durables que
// `simular_caida` (`backup_originals` de E13-H04 + `create_journal`/`mark_applied` de E13-H03),
// deteniéndose en el equivalente a `FailPoint::EntreRenames`. Es lo que un crash real deja en disco.
// ---------------------------------------------------------------------------

use lodestar_core::types::RelPath;
use lodestar_workspace::Workspace;

/// Monta un workspace con una transacción interrumpida durable: copias de recuperación de los dos
/// documentos afectados, journal `applying` con el primer rename marcado, y el canónico reflejando
/// solo ese primer rename. Nada sella `done`, así que `recovery_pending()` queda en `true`.
///
/// Devuelve el tempdir y el contenido ORIGINAL de los dos documentos, para poder aseverar que la
/// recuperación restaura (la transacción quedó en `applying`, que restaura, no completa).
fn workspace_con_transaccion_a_medias() -> (tempfile::TempDir, String, String) {
    const TXN: &str = "txn-e24-h03-a-medias";
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let uno_original = "# Uno\n\ncuerpo original de uno\n".to_string();
    let dos_original = "# Dos\n\ncuerpo original de dos\n".to_string();
    escribe(root, "notas/uno.md", &uno_original);
    escribe(root, "notas/dos.md", &dos_original);

    let ws = Workspace::open(root).expect("el workspace de prueba debe abrir");
    let uno = RelPath::new("notas/uno.md").unwrap();
    let dos = RelPath::new("notas/dos.md").unwrap();
    let afectados = [uno.clone(), dos];
    let base = ws.workspace_revision().expect("revisión base");

    ws.backup_originals(TXN, &afectados)
        .expect("copias de recuperación");
    let mut journal = ws
        .create_journal(TXN, &afectados, &base, &base)
        .expect("write-ahead journal");
    // Primer rename «hecho»: el canónico ya refleja el cambio de uno.md; dos.md sigue original.
    std::fs::write(
        root.join("notas/uno.md"),
        "# Uno\n\ncuerpo a MEDIO publicar\n",
    )
    .unwrap();
    journal.mark_applied(&uno).expect("marcar el primer rename");
    // Se «cae» aquí: el journal nunca llega a `done`.
    drop(journal);
    drop(ws);

    (dir, uno_original, dos_original)
}

/// **E24-H03** — tras un crash, `change_plan` + `change_apply` funciona **al primer intento**.
#[test]
fn apply_tras_crash_no_da_write_conflict() {
    let (dir, uno_original, _dos_original) = workspace_con_transaccion_a_medias();
    let app = App::open(dir.path()).expect("el workspace debe abrir");

    // Guarda de no vacuidad: el montaje tiene que dejar recuperación pendiente de verdad. Sin
    // esto, un montaje roto haría pasar el test por la razón equivocada.
    assert!(
        app.workspace().recovery_pending(),
        "precondición: el montaje debe dejar una transacción a medias (journal no-`done`)"
    );

    let plan = app
        .change_plan(
            None,
            &serde_json::json!([{ "op": "create", "path": "testigo.md", "body": "# Testigo\n" }]),
            PlanPolicy {
                require_valid_result: false,
                allow_warnings: true,
            },
        )
        .expect("el plan debe producirse sobre el estado recuperado");

    let receipt = app.change_apply(&plan.change_set_id, None);
    assert!(
        receipt.is_ok(),
        "tras un crash, el PRIMER `change_apply` debe funcionar: hasta E24-H03 fallaba siempre \
         con WRITE_CONFLICT porque el plan capturaba la base de un estado que `apply_transaction` \
         deshacía después. Error: {:?}",
        receipt.err()
    );

    assert!(
        dir.path().join("testigo.md").exists(),
        "la transacción del agente debe haberse publicado de verdad"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("notas/uno.md")).unwrap(),
        uno_original,
        "la transacción interrumpida quedó en `applying`, así que la recuperación RESTAURA: \
         `notas/uno.md` vuelve a su contenido original"
    );
    assert!(
        !app.workspace().recovery_pending(),
        "tras recuperar y publicar, no puede quedar recuperación pendiente"
    );
}

/// **E24-H03** — el plan parte del estado recuperado, no del estado parcial.
///
/// Es la mitad que explica POR QUÉ desaparece el `WRITE_CONFLICT`: si la `baseWorkspaceRevision`
/// del plan fuera la del estado a medias, seguiría sin casar con la del canónico recuperado.
#[test]
fn plan_tras_crash_parte_del_estado_recuperado() {
    let (dir, _uno, _dos) = workspace_con_transaccion_a_medias();
    let app = App::open(dir.path()).expect("el workspace debe abrir");
    assert!(
        app.workspace().recovery_pending(),
        "precondición: debe haber recuperación pendiente"
    );

    // La revisión del estado PARCIAL, leída antes de planificar.
    let parcial = app
        .workspace()
        .workspace_revision()
        .expect("revisión del estado parcial");

    let plan = app
        .change_plan(
            None,
            &serde_json::json!([{ "op": "create", "path": "t.md", "body": "# T\n" }]),
            PlanPolicy {
                require_valid_result: false,
                allow_warnings: true,
            },
        )
        .expect("el plan debe producirse");

    assert_ne!(
        plan.base_workspace_revision, parcial,
        "la base del plan NO puede ser la del estado a medias: `change_plan` recupera antes de \
         leer, así que parte del canónico ya restaurado"
    );
    assert_eq!(
        plan.base_workspace_revision,
        app.workspace()
            .workspace_revision()
            .expect("revisión tras recuperar"),
        "la base del plan es la revisión del canónico YA recuperado"
    );
}

/// **E24-H03** — control anti-vacuo: sin recuperación pendiente, `change_plan` no toca el canónico.
///
/// El arreglo no puede convertir el plan en un escritor habitual: recuperar es reparar, y solo
/// ocurre cuando hay algo que reparar.
#[test]
fn plan_sin_recuperacion_pendiente_no_escribe() {
    let dir = tempfile::tempdir().unwrap();
    escribe(dir.path(), "a.md", "# A\n\ncuerpo\n");
    let app = App::open(dir.path()).expect("el workspace debe abrir");
    assert!(
        !app.workspace().recovery_pending(),
        "precondición: un workspace limpio no tiene recuperación pendiente"
    );

    let antes = std::fs::read_to_string(dir.path().join("a.md")).unwrap();
    let rev_antes = app.workspace().workspace_revision().unwrap();

    app.change_plan(
        None,
        &serde_json::json!([{ "op": "replace_body", "path": "a.md", "body": "# A\n\notro\n" }]),
        PlanPolicy {
            require_valid_result: false,
            allow_warnings: true,
        },
    )
    .expect("el plan debe producirse");

    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.md")).unwrap(),
        antes,
        "`change_plan` no publica: el canónico queda idéntico"
    );
    assert_eq!(
        app.workspace().workspace_revision().unwrap(),
        rev_antes,
        "la revisión del workspace no se mueve al planificar"
    );
}

// ---------------------------------------------------------------------------
// E28-H02 — el error de colisión de `create`/`move` llega al envelope como
// `DOCUMENT_ALREADY_EXISTS` (defecto A-05 del testbench homelab,
// `docs/qa/testbench/batches/verify_G1-11.json`).
//
// El guard vive en la normalización pura (`core::plan::normalize_create`/`normalize_move`, cubierto
// por `crates/lodestar-core/tests/core.rs`); lo que estos dos tests fijan es el MAPEO de esa
// condición al código estable del protocolo en la capa de servicios, igual que
// `NormalizeTargetNotFound → DOCUMENT_NOT_FOUND` para la dirección contraria.
//
// EXPRESADO POR WIRE, NO POR VARIANTE: `ErrorCode::DocumentAlreadyExists` todavía no existe (la
// historia lo añade como fila 17 del catálogo), así que la aserción compara `err.code.as_str()`
// contra la cadena `"DOCUMENT_ALREADY_EXISTS"` —que es exactamente lo que ve el agente— en vez de
// nombrar la variante Rust. Así el test compila hoy y falla por ASERCIÓN.
//
// ROJO esperado HOY: `change_plan` devuelve `Ok(PlanResult)` (plan aplicable que pisaría el
// documento), no `Err`.
// ---------------------------------------------------------------------------

/// El código de wire (`ErrorCode::as_str`) que corresponde a una colisión de destino en E28-H02.
const COLISION: &str = "DOCUMENT_ALREADY_EXISTS";

/// Monta un `App` sobre un workspace temporal con DOS documentos existentes: `notas/existente.md`
/// (el destino ocupado) y `notas/origen.md` (el documento a mover).
fn app_con_dos_notas() -> (tempfile::TempDir, App) {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "notas/existente.md",
        "---\ntitle: Existente\n---\n\n# Existente\n\ncontenido que no se debe pisar\n",
    );
    escribe(
        dir.path(),
        "notas/origen.md",
        "---\ntitle: Origen\n---\n\n# Origen\n\ncuerpo del origen\n",
    );
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");
    (dir, app)
}

/// **E28-H02** — Dado un `create` sobre `notas/existente.md` (ya ocupado), Cuando se planifica,
/// Entonces el servicio devuelve `Err` con el código estable `DOCUMENT_ALREADY_EXISTS` y un mensaje
/// que nombra el path colisionado (mismo estilo que `DOCUMENT_NOT_FOUND`).
#[test]
fn create_sobre_path_ocupado_mapea_a_document_already_exists() {
    let (_dir, app) = app_con_dos_notas();

    let resultado = app.change_plan(
        None,
        &serde_json::json!([
            { "op": "create", "path": "notas/existente.md",
              "frontmatter": { "title": "Pisado" }, "body": "x\n" },
        ]),
        policy_permisiva(),
    );

    let err = match resultado {
        Ok(plan) => panic!(
            "planificar un `create` sobre un path YA OCUPADO debe fallar con {COLISION}; devolvió un \
             plan aplicable (canApply={}) que pisaría el documento existente: {:?}",
            plan.can_apply, plan.normalized_operations,
        ),
        Err(e) => e,
    };
    assert_eq!(
        err.code.as_str(),
        COLISION,
        "la colisión de un `create` debe mapear al código estable {COLISION} (fila 17 del catálogo, \
         simétrica de DOCUMENT_NOT_FOUND); el error fue {err}",
    );
    assert!(
        err.message.contains("notas/existente.md"),
        "el mensaje debe nombrar el path colisionado para que el agente sepa qué reparar; fue {:?}",
        err.message,
    );
}

/// **E28-H02** — Dado un `move` cuyo `to` ya está ocupado, Cuando se planifica, Entonces el mismo
/// código `DOCUMENT_ALREADY_EXISTS`, nombrando el destino (no el origen, que sí puede moverse).
#[test]
fn move_a_destino_ocupado_mapea_a_document_already_exists() {
    let (_dir, app) = app_con_dos_notas();

    let resultado = app.change_plan(
        None,
        &serde_json::json!([
            { "op": "move", "from": "notas/origen.md", "to": "notas/existente.md",
              "rewriteInboundLinks": true },
        ]),
        policy_permisiva(),
    );

    let err = match resultado {
        Ok(plan) => panic!(
            "planificar un `move` hacia un destino YA OCUPADO debe fallar con {COLISION}; devolvió un \
             plan aplicable (canApply={}) que publicaría encima del documento existente: {:?}",
            plan.can_apply, plan.normalized_operations,
        ),
        Err(e) => e,
    };
    assert_eq!(
        err.code.as_str(),
        COLISION,
        "la colisión del destino de un `move` debe mapear al mismo código {COLISION}; el error fue {err}",
    );
    assert!(
        err.message.contains("notas/existente.md"),
        "el mensaje debe nombrar el DESTINO ocupado (`notas/existente.md`), no el origen; fue {:?}",
        err.message,
    );
}

/// **E28-H02** · control anti-vacuo del mapeo: la dirección CONTRARIA no cambia de código. Un `move`
/// cuyo `from` no existe sigue siendo `DOCUMENT_NOT_FOUND`, y un `create`/`move` hacia un destino
/// libre sigue produciendo plan. Así el guard nuevo no puede degenerar en «todo es colisión».
#[test]
fn destino_libre_y_origen_inexistente_conservan_su_codigo() {
    let (_dir, app) = app_con_dos_notas();

    // (a) `create` sobre un path LIBRE: sigue habiendo plan.
    app.change_plan(
        None,
        &serde_json::json!([
            { "op": "create", "path": "notas/nueva.md", "body": "# Nueva\n" },
        ]),
        policy_permisiva(),
    )
    .expect("un `create` sobre un path libre debe seguir produciendo plan");

    // (b) `move` hacia un destino LIBRE: sigue habiendo plan.
    app.change_plan(
        None,
        &serde_json::json!([
            { "op": "move", "from": "notas/origen.md", "to": "notas/destino.md",
              "rewriteInboundLinks": true },
        ]),
        policy_permisiva(),
    )
    .expect("un `move` hacia un destino libre debe seguir produciendo plan");

    // (c) `move` cuyo ORIGEN no existe: `DOCUMENT_NOT_FOUND`, no el código de colisión.
    let err = app
        .change_plan(
            None,
            &serde_json::json!([
                { "op": "move", "from": "notas/fantasma.md", "to": "notas/destino.md" },
            ]),
            policy_permisiva(),
        )
        .expect_err("un `move` desde un path inexistente debe seguir fallando");
    assert_eq!(
        err.code,
        ErrorCode::DocumentNotFound,
        "un origen inexistente es `DOCUMENT_NOT_FOUND`, no una colisión de destino; el error fue {err}",
    );
}
