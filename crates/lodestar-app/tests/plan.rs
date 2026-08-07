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

// ---------------------------------------------------------------------------
// E28-H04 (cierre de reserva) — la red de colisiones intra-plan tiene que vivir en el lado que
// ESCRIBE, no solo en el que planifica.
//
// EL HUECO QUE ESTE MÓDULO CIERRA
//
// `App::change_plan_uncounted` llama a `plan::assert_sin_colisiones_intra_plan` sobre la secuencia ya
// normalizada (`lib.rs` ~L1788) y documenta esa llamada como «red de seguridad… deja el veredicto
// verificado sobre el plan COMPLETO, que es lo que se persiste y se aplica». Pero el plan que se
// aplica NO vuelve a pasar por ella: `App::change_apply_uncounted` (~L1972-2007) valida
// `expiresAt` (vía `load_plan`), `expectedWorkspaceRevision` y `planHash`, y con eso publica. La red
// está exclusivamente en el camino de LECTURA/planificación.
//
// Eso importa porque el plan es un artefacto PERSISTIDO y de larga vida
// (`.lodestar/runtime/plans/<hash>.json`, TTL en `expiresAt`), no un valor en memoria: un plan
// escrito por un binario anterior al guard —o por cualquier vía futura que construya
// `normalizedOperations` sin acumular ocupación— sigue siendo aplicable mientras no caduque y su
// `planHash` case con la base. Ninguna de las tres validaciones del apply mira las operaciones entre
// sí, así que `[move a→final, move b→final]` se publica: `plan::apply_one` para `Move` hace
// `files.remove(from)` + `files.insert(to)`, y el segundo `move` pisa lo que dejó el primero — el
// contenido de `a.md` desaparece del disco sin un solo diagnóstico. Es exactamente el defecto
// destructivo que H04 describe, entrando por la puerta que H04 no cerró.
//
// CÓMO SE FORJA EL PLAN (lo que el implementador debe saber)
//
// El plan persistido se reescribe a mano, respetando TODO lo que el gate del apply comprueba, para
// que el rechazo (cuando llegue) solo pueda venir del guard de colisión y nunca de un tecnicismo:
//   - `normalizedOperations` ← las dos ops colisionadas, serializadas por el MISMO `serde` que usa
//     el motor (se construyen como `NormalizedOperation` de `core::types`, no como JSON a mano);
//   - `planHash` ← recomputado con la fórmula literal de `compute_plan_hash`
//     (`blake3(baseWorkspaceRevision ‖ 0x00 ‖ serde_json::to_vec(ops))`), sobre la MISMA base que
//     el workspace tiene ahora, de modo que el paso (3) del apply lo dé por bueno;
//   - `changeSetId` ← `changeset:<hash desnudo>`, y el fichero se renombra a `<hash desnudo>.json`,
//     que es la convención de `plan_file_name`/`plan_file_path` por la que `load_plan` lo encuentra;
//   - `expiresAt` ← el que puso `change_plan` (futuro), intacto.
// Las guardas de no vacuidad del propio test verifican que el forjado es correcto: si el apply
// respondiera `PLAN_STALE`/`PLAN_EXPIRED`/`REVISION_CONFLICT`, el test FALLA en vez de darse por
// bueno, porque un rechazo por el motivo equivocado no demuestra nada.
// ---------------------------------------------------------------------------

mod colision_intra_plan_en_el_apply {
    use super::*;
    use lodestar_core::types::NormalizedOperation;

    /// Monta un workspace con `a.md` y `b.md` (los dos con contenido distinguible) y sin `final.md`.
    fn app_con_a_y_b() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        escribe(
            dir.path(),
            "a.md",
            "---\ntitle: A\n---\n\n# A\n\ncuerpo de a\n",
        );
        escribe(
            dir.path(),
            "b.md",
            "---\ntitle: B\n---\n\n# B\n\ncuerpo de b\n",
        );
        let app = App::open(dir.path()).expect("el workspace temporal debe abrir");
        (dir, app)
    }

    /// Todos los `.md` del canónico como `ruta → contenido`, para comparar el disco byte a byte
    /// antes y después del apply rechazado.
    fn canonico(root: &Path) -> std::collections::BTreeMap<String, String> {
        let mut out = std::collections::BTreeMap::new();
        for entry in std::fs::read_dir(root).unwrap().flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) == Some("md") {
                out.insert(
                    path.file_name().unwrap().to_string_lossy().to_string(),
                    std::fs::read_to_string(&path).unwrap(),
                );
            }
        }
        out
    }

    /// El `planHash` con la fórmula LITERAL de `App::compute_plan_hash` (privada): `blake3` de la
    /// revisión base, un `0x00` separador y la serialización JSON de las operaciones normalizadas.
    fn plan_hash(base: &str, ops: &[NormalizedOperation]) -> String {
        let mut hasher = blake3::Hasher::new();
        hasher.update(base.as_bytes());
        hasher.update(b"\0");
        hasher.update(&serde_json::to_vec(ops).expect("las ops normalizadas serializan"));
        format!("blake3:{}", hasher.finalize().to_hex())
    }

    /// Reescribe el plan persistido único de `root` con las `ops` dadas: recalcula `planHash` y
    /// `changeSetId`, renombra el fichero a `<hash desnudo>.json` y devuelve el `ChangeSetId` con el
    /// que hay que llamar a `change_apply`. Conserva el resto del JSON (incluido `expiresAt`).
    fn forja_plan_persistido(root: &Path, base: &str, ops: &[NormalizedOperation]) -> ChangeSetId {
        let (ruta, mut valor) = json_del_plan_unico(root);
        let hash = plan_hash(base, ops);
        let desnudo = hash.strip_prefix("blake3:").unwrap().to_string();
        valor["normalizedOperations"] = serde_json::to_value(ops).unwrap();
        valor["planHash"] = serde_json::Value::String(hash);
        valor["changeSetId"] = serde_json::Value::String(format!("changeset:{desnudo}"));
        std::fs::remove_file(&ruta).unwrap();
        let nueva = ruta.with_file_name(format!("{desnudo}.json"));
        std::fs::write(&nueva, serde_json::to_vec_pretty(&valor).unwrap()).unwrap();
        ChangeSetId(format!("changeset:{desnudo}"))
    }

    /// **E28-H04 (reserva)** — **Dado** un plan PERSISTIDO cuyas dos operaciones mueven documentos
    /// distintos al MISMO destino (`[move a→final, move b→final]`), **Cuando** se llama a
    /// `change_apply` con su `changeSetId`, **Entonces** la publicación se rechaza con
    /// `DOCUMENT_ALREADY_EXISTS` y el disco queda intacto byte a byte.
    ///
    /// Fija la red de colisión intra-plan **en el lado que ESCRIBE**. Hoy `change_apply` solo juzga
    /// caducidad, revisión esperada y `planHash`: ninguno de los tres mira las operaciones entre sí,
    /// así que un plan persistido con la colisión —el que dejaría un binario anterior al guard de
    /// H04, o cualquier vía futura que construyera `normalizedOperations` sin acumular ocupación—
    /// aplica y destruye `a.md` en silencio. Que el guard esté en `change_plan` no protege al apply:
    /// el plan es un artefacto durable con TTL propio, no un valor en memoria.
    #[test]
    fn apply_de_plan_persistido_con_colision_intra_plan_rechaza_sin_tocar_disco() {
        let (dir, app) = app_con_a_y_b();
        let root = dir.path();

        // (1) Un plan VÁLIDO cualquiera, solo para obtener el fichero persistido con su forma real
        //     (base, expiresAt, diff, impacto…). Su única op no colisiona.
        let base = app
            .change_plan(
                None,
                &serde_json::json!([
                    { "op": "move", "from": "a.md", "to": "final.md", "rewriteInboundLinks": true },
                ]),
                policy_permisiva(),
            )
            .expect("el plan de partida (un solo move a un destino libre) debe producirse")
            .base_workspace_revision;

        // (2) Forjado: las DOS ops colisionadas, con el `planHash` recomputado sobre la misma base.
        let colisionadas = vec![
            NormalizedOperation::Move {
                from: RelPath::new("a.md").unwrap(),
                to: RelPath::new("final.md").unwrap(),
                rewrite_inbound_links: false,
            },
            NormalizedOperation::Move {
                from: RelPath::new("b.md").unwrap(),
                to: RelPath::new("final.md").unwrap(),
                rewrite_inbound_links: false,
            },
        ];
        let id = forja_plan_persistido(root, &base.0, &colisionadas);

        // Guarda de no vacuidad del forjado: el plan forjado tiene que CARGAR (si no, el apply
        // fallaría por `PLAN_STALE`/`PLAN_EXPIRED` y el rechazo no probaría nada).
        let cargado = app.load_plan(&id).expect(
            "el plan forjado debe cargar: si no, el apply fallaría por el motivo equivocado",
        );
        assert_eq!(
            cargado.normalized_operations.len(),
            2,
            "el plan forjado debe llevar las DOS operaciones colisionadas"
        );

        let antes = canonico(root);
        assert!(
            antes.contains_key("a.md") && antes.contains_key("b.md"),
            "precondición: los dos documentos existen antes del apply: {antes:?}"
        );

        let resultado = app.change_apply(&id, None);

        let err = match resultado {
            Ok(aplicado) => panic!(
                "aplicar un plan con dos operaciones que ocupan el MISMO path debe rechazarse: el \
                 segundo `move` publica encima de lo que dejó el primero y `a.md` desaparece sin \
                 diagnóstico. El apply respondió applied={} sobre {:?}; el disco quedó en {:?}",
                aplicado.applied,
                aplicado.changed_paths,
                canonico(root),
            ),
            Err(e) => e,
        };
        assert_eq!(
            err.code.as_str(),
            COLISION,
            "el rechazo debe ser el MISMO código de colisión que emite `change_plan` (una sola \
             verdad de criterio, invariante #3), no un `PLAN_STALE` ni un error de IO; el error fue \
             {err}",
        );
        assert!(
            err.message.contains("final.md"),
            "y debe nombrar el path colisionado, como hace el guard del plan; fue {:?}",
            err.message,
        );
        assert_eq!(
            canonico(root),
            antes,
            "y el rechazo ocurre ANTES de la primera escritura: ni `a.md`, ni `b.md`, ni `final.md` \
             se mueven un byte"
        );
        assert!(
            !app.workspace().recovery_pending(),
            "ni queda una transacción a medio publicar: nada llegó a prepararse"
        );
    }

    /// Control anti-vacuo del test de arriba: un plan persistido **sin** colisión y forjado por la
    /// MISMA vía (mismas ops normalizadas, mismo recálculo de `planHash`, mismo renombrado) se
    /// aplica con éxito.
    ///
    /// Sin esto, un arreglo que rechazara todo plan cuyo fichero se hubiera tocado —o directamente
    /// todo plan con más de una operación— haría pasar el criterio sin cerrar el hueco.
    #[test]
    fn apply_de_plan_persistido_forjado_sin_colision_sigue_aplicando() {
        let (dir, app) = app_con_a_y_b();
        let root = dir.path();

        let base = app
            .change_plan(
                None,
                &serde_json::json!([
                    { "op": "move", "from": "a.md", "to": "final.md", "rewriteInboundLinks": true },
                ]),
                policy_permisiva(),
            )
            .expect("el plan de partida debe producirse")
            .base_workspace_revision;

        // Dos moves a destinos DISTINTOS: legítimo, y ejercita el mismo forjado.
        let sin_colision = vec![
            NormalizedOperation::Move {
                from: RelPath::new("a.md").unwrap(),
                to: RelPath::new("final.md").unwrap(),
                rewrite_inbound_links: false,
            },
            NormalizedOperation::Move {
                from: RelPath::new("b.md").unwrap(),
                to: RelPath::new("otro.md").unwrap(),
                rewrite_inbound_links: false,
            },
        ];
        let id = forja_plan_persistido(root, &base.0, &sin_colision);

        app.change_apply(&id, None).unwrap_or_else(|e| {
            panic!(
                "un plan persistido forjado por la misma vía pero SIN colisión debe seguir \
                 aplicándose (si no, el guard nuevo estaría rechazando por el forjado, no por la \
                 colisión): {e}"
            )
        });

        let despues = canonico(root);
        assert!(
            despues.contains_key("final.md") && despues.contains_key("otro.md"),
            "los dos destinos se publican: {despues:?}"
        );
        assert!(
            !despues.contains_key("a.md") && !despues.contains_key("b.md"),
            "y los orígenes desaparecen, que es lo que hace un `move`: {despues:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// E29-H07 — el veredicto `canApply` del plan VINCULA a `change_apply` (`decisiones §18`).
//
// EL DEFECTO (verificado ejecutando el motor antes de escribir estos tests, no supuesto)
//
// `can_apply` se computa en `App::change_plan` (`lib.rs` ~L1836, con `core::plan::can_apply` bajo la
// `PlanPolicy` que mandó el cliente) y viaja al cliente dentro del `PlanResult`. El camino de apply
// —`App::change_apply_uncounted`, pasos (1) a (4)— NO lo consulta: valida caducidad
// (`load_plan`), `expectedWorkspaceRevision`, `planHash` y colisiones intra-plan, y publica. El
// único filtro de validez que queda es el **gate de staging** (`rejectNewErrors`/
// `allowExistingErrors` de la sección `transactions`, E20-H04), que es una política DISTINTA de la
// `PlanPolicy` con la que se computó `canApply`. Dos políticas: una publicada y otra ejercida.
//
// Medido hoy sobre un workspace con un enlace roto preexistente y la policy por defecto:
//   change_plan  → canApply=false (diagnosticsAfter.errors=1, requireValidResult=true)
//   change_apply → Ok(applied=true) y el `.md` cambiado en disco.
// Y con `allowWarnings:false` sobre un resultado con 1 warning: canApply=false → Ok(applied=true).
// El gate de staging no muerde en ninguno de los dos casos (los errores son PREEXISTENTES, no
// nuevos; los warnings no le incumben), que es justo lo que deja el hueco abierto.
//
// QUÉ FIJAN ESTOS TESTS Y QUÉ NO
//
// Fijan el **comportamiento observable**: `change_apply` de un plan cuyo `canApply` era `false`
// devuelve `Err` con el wire `INVALID_RESULT`, el mensaje nombra la CLÁUSULA de la policy que
// bloqueó (`requireValidResult` / `allowWarnings`), el disco queda byte-idéntico y el rechazo ocurre
// ANTES del lock (sin journal, sin staging, sin recibo, sin copias de recuperación).
//
// NO fijan la representación: la spec deja al implementador elegir entre persistir el `canApply`
// computado o persistir la `PlanPolicy` y recomputar `can_apply` (recomendación de la historia).
// Por eso los tests van por la API pública (`change_plan(policy) → change_apply`) y no forjan el
// JSON del plan, a diferencia del módulo de E28-H04 de arriba.
//
// POR QUÉ `INVALID_RESULT` Y NO UN CÓDIGO NUEVO (cláusula de escape de `decisiones §18`)
//
// El catálogo NO se abre. El mensaje distingue los dos orígenes sin ambigüedad porque cada gate
// nombra sus propias cláusulas: el de staging ya dice hoy, literalmente, «1 error(es) nuevo(s), 1
// error(es) en total (rejectNewErrors=true, allowExistingErrors=true)», y el gate nuevo tiene que
// nombrar `requireValidResult`/`allowWarnings`. Son vocabularios disjuntos, y el remedio del agente
// es el mismo en ambos casos (replanificar o relajar la política), que es el criterio con el que
// `§18` justifica reusar la fila existente. `mensaje_del_rechazo_no_se_confunde_con_el_gate_de_staging`
// asevera esa disjunción como criterio propio, para que el reuso del código no se pague en claridad.
//
// DIFERENCIA CON LOS RECHAZOS YA CUBIERTOS (no se duplica nada)
//
//   - `apply_de_plan_con_colision_rechaza_sin_tocar_disco` (E28-H02, `mcp.rs`) y
//     `apply_de_plan_persistido_con_colision_intra_plan_rechaza_sin_tocar_disco` (E28-H04, arriba):
//     rechazan por COLISIÓN de paths —dos operaciones que ocupan el mismo destino, o un destino ya
//     ocupado—, con `DOCUMENT_ALREADY_EXISTS`. Es un juicio sobre las OPERACIONES entre sí.
//   - `rechaza_errores_nuevos` (E20-H04, `validacion.rs`): rechaza por el gate de STAGING, con la
//     política `transactions` y sobre errores NUEVOS.
//   - Esta historia: rechaza por el VEREDICTO del plan (`canApply:false` bajo su `PlanPolicy`), que
//     hoy no bloquea nada aunque el resultado sea no conforme por errores PREEXISTENTES o tenga
//     warnings. Ninguno de los otros dos gates cubre este caso: por eso el apply pasa hoy.
// ---------------------------------------------------------------------------

mod can_apply_vincula_al_apply {
    use super::*;

    /// El código de wire con el que `decisiones §18` decide rechazar (reuso de la fila existente,
    /// sin abrir el catálogo). Expresado por wire —lo que ve el agente— y no por variante Rust,
    /// para que el test no se acople a la forma interna del error.
    const RECHAZO: &str = "INVALID_RESULT";

    /// Workspace con un **error preexistente** (`roto.md` enlaza a un `.md` inexistente ⇒
    /// `LINK-TARGET-MISSING` de nivel `Err`, familia `danglingDocumentLinks`) y un documento limpio
    /// sobre el que operar. Es el montaje de la demo y el que reprodujo `§18`.
    ///
    /// Sin `.lodestar/config.yaml`: los defaults de `transactions` (`rejectNewErrors:true`,
    /// `allowExistingErrors:true`) son precisamente los que dejan pasar el apply hoy, y esa es la
    /// condición del defecto.
    fn app_con_error_preexistente() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        escribe(
            dir.path(),
            "roto.md",
            "# Roto\n\nEnlace a un documento que no existe: [falta](inexistente-previo.md).\n",
        );
        escribe(
            dir.path(),
            "limpio.md",
            "---\ntitle: Limpio\n---\n\n# Limpio\n\nDocumento sin problemas.\n",
        );
        let app = App::open(dir.path()).expect("el workspace temporal debe abrir");
        (dir, app)
    }

    /// Workspace **sin errores** pero con un **warning** reproducible: `nota.md` enlaza a un fichero
    /// de proyecto que no existe (`assets/logo.png`), que `§20.9` clasifica como
    /// `missingWorkspaceFiles: warning`. Es el escenario de la cláusula `allowWarnings`.
    fn app_con_warning() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        escribe(
            dir.path(),
            "nota.md",
            "---\ntitle: Nota\n---\n\n# Nota\n\nDiagrama: [logo](assets/logo.png)\n",
        );
        let app = App::open(dir.path()).expect("el workspace temporal debe abrir");
        (dir, app)
    }

    /// Un patch inocuo sobre `path`: no crea ni borra documentos, no introduce enlaces y por tanto
    /// **no puede** disparar el gate de staging. Si el apply se rechaza con este cambio, es por el
    /// veredicto del plan y por nada más.
    fn patch_inocuo(path: &str) -> serde_json::Value {
        serde_json::json!([
            { "op": "patch_frontmatter", "ref": { "path": path },
              "patch": { "status": "revisado" } },
        ])
    }

    /// Instantánea `ruta → contenido` de todos los `.md` bajo `root` (recursiva, saltando
    /// `.lodestar/`): el vehículo del «byte-idéntico».
    fn canonico_md(root: &Path) -> std::collections::BTreeMap<String, String> {
        fn recorre(base: &Path, dir: &Path, out: &mut std::collections::BTreeMap<String, String>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if e.file_name() == ".lodestar" {
                    continue;
                }
                if p.is_dir() {
                    recorre(base, &p, out);
                } else if p.extension().and_then(|x| x.to_str()) == Some("md") {
                    out.insert(
                        p.strip_prefix(base)
                            .unwrap()
                            .to_string_lossy()
                            .replace('\\', "/"),
                        std::fs::read_to_string(&p).unwrap(),
                    );
                }
            }
        }
        let mut out = std::collections::BTreeMap::new();
        recorre(root, root, &mut out);
        out
    }

    /// Rutas relativas (POSIX) de todo lo que cuelga de `.lodestar/runtime/<sub>`; vacío si el
    /// directorio no existe.
    fn runtime_sub(root: &Path, sub: &str) -> Vec<String> {
        fn recorre(base: &Path, dir: &Path, out: &mut Vec<String>) {
            if let Ok(it) = std::fs::read_dir(dir) {
                for e in it.flatten() {
                    let p = e.path();
                    if p.is_dir() {
                        recorre(base, &p, out);
                    } else {
                        out.push(
                            p.strip_prefix(base)
                                .unwrap()
                                .to_string_lossy()
                                .replace('\\', "/"),
                        );
                    }
                }
            }
        }
        let mut out = Vec::new();
        recorre(
            root,
            &root.join(".lodestar").join("runtime").join(sub),
            &mut out,
        );
        out.sort();
        out
    }

    /// **Criterio `apply_de_plan_no_aplicable_se_rechaza_sin_escribir`** (mitad de servicio) —
    /// **Dado** un workspace con un error preexistente y un `change_plan` con la policy por defecto
    /// que devuelve `canApply:false`, **Cuando** se llama a `change_apply` con ese `changeSetId`,
    /// **Entonces** falla con `INVALID_RESULT`, el mensaje nombra la cláusula que bloqueó
    /// (`requireValidResult`) y el disco queda byte-idéntico.
    ///
    /// ROJO HOY: `change_apply` devuelve `Ok(applied=true)` y `limpio.md` sale con `status:
    /// revisado` en disco.
    #[test]
    fn apply_de_plan_no_aplicable_se_rechaza_sin_escribir() {
        let (dir, app) = app_con_error_preexistente();
        let root = dir.path();

        let plan = app
            .change_plan(None, &patch_inocuo("limpio.md"), PlanPolicy::default())
            .expect("el plan se computa: `canApply:false` es un VEREDICTO, no un error de plan");

        // Precondiciones no vacuas: el escenario es el del defecto y no otro.
        assert!(
            !plan.can_apply,
            "precondición: con `requireValidResult:true` (default) y un error preexistente el plan \
             debe salir NO aplicable; diagnosticsAfter={:?}",
            plan.diagnostics_after
        );
        assert_eq!(
            plan.diagnostics_after.errors, 1,
            "precondición: el resultado simulado conserva el error preexistente de `roto.md` \
             (es lo que hace `canApply:false`): {:?}",
            plan.diagnostics_after
        );

        let antes = canonico_md(root);

        let resultado = app.change_apply(&plan.change_set_id, None);

        let err = match resultado {
            Ok(aplicado) => panic!(
                "aplicar un plan cuyo `canApply` era FALSE debe rechazarse: la superficie prometió \
                 «este plan no es aplicable bajo tu policy» y el motor lo publicó igual \
                 (applied={}, changedPaths={:?}). El gate de staging no lo frena porque el error es \
                 PREEXISTENTE, no nuevo. Disco resultante: {:?}",
                aplicado.applied,
                aplicado.changed_paths,
                canonico_md(root),
            ),
            Err(e) => e,
        };
        assert_eq!(
            err.code.as_str(),
            RECHAZO,
            "el rechazo por el veredicto del plan reusa la fila existente {RECHAZO} \
             («el resultado del plan no es aceptable»), sin abrir el catálogo (`decisiones §18`); \
             el error fue {err}",
        );
        assert!(
            err.message.contains("requireValidResult"),
            "el mensaje debe NOMBRAR la cláusula de la `PlanPolicy` que bloqueó \
             (`requireValidResult`), no ser un genérico: es lo único que le dice al agente si \
             replanificar o relajar la política. Fue: {:?}",
            err.message,
        );

        assert_eq!(
            canonico_md(root),
            antes,
            "y el rechazo no escribe un solo byte del conocimiento canónico (invariante #1)"
        );
    }

    /// **Criterio `apply_rechaza_tambien_por_allow_warnings`** — **Dado** un plan con
    /// `allowWarnings:false` sobre un resultado con warnings (`canApply:false` por la OTRA
    /// cláusula), **Cuando** se aplica, **Entonces** también se rechaza y el mensaje nombra
    /// `allowWarnings`.
    ///
    /// Este caso es el que demuestra que el gate juzga la `PlanPolicy` y no la conformidad: aquí el
    /// resultado simulado es **válido** (`errors == 0`), así que ninguna política de staging tiene
    /// nada que objetar. Hoy el apply pasa.
    #[test]
    fn apply_rechaza_tambien_por_allow_warnings() {
        let (dir, app) = app_con_warning();
        let root = dir.path();

        let policy = PlanPolicy {
            require_valid_result: true,
            allow_warnings: false,
        };
        let plan = app
            .change_plan(None, &patch_inocuo("nota.md"), policy)
            .expect("el plan se computa aunque no sea aplicable");

        assert!(
            !plan.can_apply,
            "precondición: `allowWarnings:false` sobre un resultado con warnings da `canApply:false`"
        );
        assert_eq!(
            plan.diagnostics_after.errors, 0,
            "precondición: el resultado es VÁLIDO (0 errores) — lo único que bloquea es el warning, \
             de modo que ninguna política de staging puede reclamar el mérito del rechazo: {:?}",
            plan.diagnostics_after
        );
        assert!(
            plan.diagnostics_after.warnings >= 1,
            "precondición: el enlace a `assets/logo.png` aporta al menos un warning \
             (`missingWorkspaceFiles`): {:?}",
            plan.diagnostics_after
        );

        let antes = canonico_md(root);

        let err = match app.change_apply(&plan.change_set_id, None) {
            Ok(aplicado) => panic!(
                "aplicar un plan bloqueado por `allowWarnings:false` debe rechazarse; el motor \
                 publicó igual (applied={}, changedPaths={:?})",
                aplicado.applied, aplicado.changed_paths,
            ),
            Err(e) => e,
        };
        assert_eq!(
            err.code.as_str(),
            RECHAZO,
            "las dos cláusulas de la policy rechazan con el MISMO código; el error fue {err}",
        );
        assert!(
            err.message.contains("allowWarnings"),
            "el mensaje debe nombrar la cláusula CONCRETA que bloqueó (`allowWarnings`), no la otra \
             ni un genérico: fue {:?}",
            err.message,
        );
        assert_eq!(canonico_md(root), antes, "y tampoco aquí se escribe nada");
    }

    /// **Criterio `el_rechazo_por_can_apply_no_deja_rastro_transaccional`** — **Dado** un plan
    /// rechazado por este gate, **Cuando** se inspecciona `.lodestar/runtime/`, **Entonces** no hay
    /// journal, ni staging, ni recibo, ni copias de recuperación de esa transacción: el rechazo es
    /// un veredicto sobre el PLAN y ocurre antes de tomar el lock.
    ///
    /// El plan persistido (`runtime/plans/`) y la línea de auditoría (`runtime/audit.jsonl`) quedan
    /// FUERA del criterio a propósito: el primero es el artefacto que se está juzgando (caducará por
    /// TTL) y la segunda se anexa en todo intento, con éxito o sin él (E13-H10).
    #[test]
    fn el_rechazo_por_can_apply_no_deja_rastro_transaccional() {
        let (dir, app) = app_con_error_preexistente();
        let root = dir.path();

        let plan = app
            .change_plan(None, &patch_inocuo("limpio.md"), PlanPolicy::default())
            .expect("el plan se computa");
        assert!(!plan.can_apply, "precondición: el plan no es aplicable");

        let err = app
            .change_apply(&plan.change_set_id, None)
            .err()
            .unwrap_or_else(|| {
                panic!("precondición de este criterio: el apply debe RECHAZARSE (hoy publica)")
            });
        assert_eq!(err.code.as_str(), RECHAZO, "el rechazo esperado; fue {err}");

        for sub in ["journal", "staging", "receipts", "recovery"] {
            let residuos = runtime_sub(root, sub);
            assert!(
                residuos.is_empty(),
                "un rechazo por `canApply:false` ocurre ANTES del lock: `.lodestar/runtime/{sub}/` \
                 no puede contener nada de esa transacción, y contiene {residuos:?}"
            );
        }
        assert!(
            !app.workspace().recovery_pending(),
            "ni queda una transacción a medio publicar: nunca se llegó a preparar ninguna"
        );
    }

    /// **Criterio `apply_con_policy_permisiva_sigue_aplicando`** (control anti-vacuo) — **Dado** el
    /// MISMO workspace con el error preexistente, **Cuando** se planifica con
    /// `policy: {requireValidResult:false}` y se aplica, **Entonces** el apply funciona como hoy.
    ///
    /// Es el control que impide que el arreglo degenere en «todo plan sobre un workspace con
    /// errores se rechaza»: el gate solo puede morder donde el plan dijo que mordería. Verde hoy y
    /// tiene que seguir verde después.
    #[test]
    fn apply_con_policy_permisiva_sigue_aplicando() {
        let (dir, app) = app_con_error_preexistente();
        let root = dir.path();

        let plan = app
            .change_plan(None, &patch_inocuo("limpio.md"), policy_permisiva())
            .expect("el plan permisivo debe producirse");
        assert!(
            plan.can_apply,
            "precondición del control: con `requireValidResult:false` el mismo plan SÍ es aplicable \
             pese al error preexistente (diagnosticsAfter={:?})",
            plan.diagnostics_after
        );

        let aplicado = app
            .change_apply(&plan.change_set_id, None)
            .unwrap_or_else(|e| {
                panic!(
                "un plan con `canApply:true` debe seguir aplicándose exactamente como antes de la \
                 historia; el gate nuevo lo rechazó con {e}"
            )
            });
        assert!(aplicado.applied, "el apply permitido reporta applied:true");
        assert!(
            std::fs::read_to_string(root.join("limpio.md"))
                .unwrap()
                .contains("status: revisado"),
            "y el cambio está de verdad en disco"
        );
        assert!(
            std::fs::read_to_string(root.join("roto.md"))
                .unwrap()
                .contains("inexistente-previo.md"),
            "el error preexistente sigue ahí: el apply lo TOLERA, no lo repara (y por eso el plan \
             por defecto lo declaraba no aplicable)"
        );
    }

    /// **Criterio propio del autor de tests (carga de la prueba de `decisiones §18`)** — el mensaje
    /// del rechazo por el veredicto del plan es DISTINGUIBLE del `INVALID_RESULT` que emite el gate
    /// de staging, aunque compartan código de wire.
    ///
    /// Es la evidencia que sostiene no abrir la fila 18 del catálogo: cada gate nombra sus propias
    /// cláusulas, que son vocabularios disjuntos —`requireValidResult`/`allowWarnings` (política del
    /// PLAN) frente a `rejectNewErrors`/`allowExistingErrors` (política de STAGING)—, así que un
    /// agente puede saber cuál de los dos le habló leyendo el mensaje. Si esta aserción no pudiera
    /// satisfacerse, la cláusula de escape de `§18` se activaría y habría que abrir
    /// `PLAN_NOT_APPLICABLE`.
    #[test]
    fn mensaje_del_rechazo_no_se_confunde_con_el_gate_de_staging() {
        // (a) Rechazo por el VEREDICTO DEL PLAN: policy por defecto, error preexistente, patch
        //     inocuo (el gate de staging no tiene nada que objetar: no hay errores NUEVOS).
        let (_dir_plan, app_plan) = app_con_error_preexistente();
        let plan = app_plan
            .change_plan(None, &patch_inocuo("limpio.md"), PlanPolicy::default())
            .expect("el plan se computa");
        assert!(!plan.can_apply, "precondición: plan no aplicable");
        let err_plan = app_plan
            .change_apply(&plan.change_set_id, None)
            .err()
            .unwrap_or_else(|| panic!("precondición: el apply debe rechazarse (hoy publica)"));

        // (b) Rechazo por el GATE DE STAGING: policy permisiva (el plan SÍ es aplicable) pero el
        //     cambio introduce un error NUEVO. Este camino ya existe y no lo toca la historia.
        let (_dir_stg, app_stg) = app_con_error_preexistente();
        let plan_stg = app_stg
            .change_plan(
                None,
                &serde_json::json!([
                    { "op": "create", "path": "nuevo.md",
                      "body": "# Nuevo\n\n[roto nuevo](inexistente-nuevo.md)\n" },
                ]),
                policy_permisiva(),
            )
            .expect("el plan permisivo se computa");
        assert!(
            plan_stg.can_apply,
            "precondición: bajo la policy permisiva este plan SÍ es aplicable, de modo que quien \
             lo rechace solo puede ser el gate de staging"
        );
        let err_stg = app_stg
            .change_apply(&plan_stg.change_set_id, None)
            .expect_err("el gate de staging rechaza los errores nuevos desde E20-H04");

        // Mismo código de wire (la fila no se duplica)…
        assert_eq!(err_plan.code.as_str(), RECHAZO, "(a) fue {err_plan}");
        assert_eq!(err_stg.code.as_str(), RECHAZO, "(b) fue {err_stg}");

        // …y sin embargo cada mensaje nombra SU política, sin invadir la del otro.
        assert!(
            err_plan.message.contains("requireValidResult")
                && !err_plan.message.contains("rejectNewErrors")
                && !err_plan.message.contains("allowExistingErrors"),
            "el rechazo por el veredicto del plan debe hablar de la `PlanPolicy` y solo de ella; \
             fue: {:?}",
            err_plan.message,
        );
        assert!(
            err_stg.message.contains("rejectNewErrors")
                && !err_stg.message.contains("requireValidResult")
                && !err_stg.message.contains("allowWarnings"),
            "y el del gate de staging debe seguir hablando de `transactions`, sin contaminarse con \
             el vocabulario del plan; fue: {:?}",
            err_stg.message,
        );
        assert_ne!(
            err_plan.message, err_stg.message,
            "dos orígenes distintos bajo el mismo código exigen, como mínimo, mensajes distintos"
        );
    }

    // -----------------------------------------------------------------------
    // PINES SOBRE EL PLAN PERSISTIDO (remates del juez ciego de E29-H07).
    //
    // Los cinco tests de arriba van por la API pública (`change_plan(policy) → change_apply`), que es
    // lo correcto para los criterios de la historia: no acoplan el test a la representación. El
    // precio es que dejan dos mutantes vivos, porque por esa vía el plan persistido SIEMPRE lleva
    // `policy` y su `canApply` SIEMPRE es congruente con ella:
    //
    //   (1) hacer la default de `#[serde(default)] policy` PERMISIVA —un plan sin la clave, el que
    //       dejó cualquier binario anterior a E29-H07, pasaría el gate—;
    //   (2) sustituir la recomputación por `if !plan.can_apply` —leer el booleano persistido en vez
    //       de re-derivarlo—.
    //
    // Los dos sobreviven a la suite entera si no se forja el fichero, así que estos dos pines lo
    // forjan. Nacen **VERDES**: fijan el comportamiento que la implementación ya tiene, no piden uno
    // nuevo. Y, a diferencia de los cinco de arriba, **sí fijan la representación (ii)** —policy
    // persistida + recomputación— que es la que el rustdoc de `PlanResult::policy` y el delta de
    // contrato declaran: si alguien migrase a la (i) (congelar el booleano), el pin (2) tiene que
    // romperse y obligar a re-ratificar la decisión, no pasar en silencio.
    //
    // FORJADO: se reescriben **solo** campos que NO entran en el `planHash` (`policy`, `canApply`).
    // `compute_plan_hash` mezcla `baseWorkspaceRevision` y `normalizedOperations` y nada más, así que
    // el paso (3) del apply sigue dando el hash por bueno y el fichero conserva su nombre — el
    // rechazo, cuando llega, solo puede venir del gate del veredicto. Las guardas de no vacuidad de
    // cada test lo verifican: si el apply respondiera `PLAN_STALE`/`PLAN_EXPIRED`, el test FALLA.
    // -----------------------------------------------------------------------

    /// Reescribe **in situ** el plan persistido único de `root` aplicando `mutacion` sobre su JSON.
    /// No toca `normalizedOperations` ni `baseWorkspaceRevision`, así que el `planHash` y el nombre
    /// del fichero siguen siendo válidos y el `changeSetId` que devolvió `change_plan` sigue
    /// sirviendo para el apply.
    fn reescribe_plan_persistido(root: &Path, mutacion: impl FnOnce(&mut serde_json::Value)) {
        let (ruta, mut valor) = json_del_plan_unico(root);
        mutacion(&mut valor);
        std::fs::write(&ruta, serde_json::to_vec_pretty(&valor).unwrap()).unwrap();
    }

    /// **Pin de la DEFAULT ESTRICTA** — **Dado** un plan persistido **sin** la clave `policy` (el que
    /// dejó un binario anterior a E29-H07) sobre un workspace donde la policy por defecto rechaza,
    /// **Cuando** se aplica, **Entonces** se rechaza con `INVALID_RESULT` nombrando
    /// `requireValidResult` y no se escribe nada.
    ///
    /// Es decir: el `#[serde(default)]` de `PlanResult::policy` completa con la policy **más
    /// estricta** de las dos que el wire admite (`requireValidResult:true`), nunca con una permisiva.
    /// Al revés —default laxa— un plan viejo se colaría por el gate justo en el caso que `§18`
    /// vino a cerrar, y ningún test de la API pública lo notaría porque por esa vía la clave siempre
    /// está.
    ///
    /// VERDE hoy: fija el comportamiento implementado, y mata el mutante «default permisiva».
    #[test]
    fn plan_persistido_sin_policy_cae_a_la_default_estricta() {
        let (dir, app) = app_con_error_preexistente();
        let root = dir.path();

        // Se planifica con policy PERMISIVA a propósito: así el plan nace aplicable y, al borrar la
        // clave, lo único que puede rechazarlo después es la default con la que serde la complete.
        let plan = app
            .change_plan(None, &patch_inocuo("limpio.md"), policy_permisiva())
            .expect("el plan permisivo se computa");
        assert!(
            plan.can_apply,
            "precondición: con la policy permisiva el plan nace APLICABLE (el rechazo que se espera \
             abajo solo puede venir de la default que sustituya a la clave borrada)"
        );

        // Forjado: el plan de un binario anterior a E29-H07 no tiene el campo.
        reescribe_plan_persistido(root, |v| {
            v.as_object_mut().unwrap().remove("policy");
        });
        let recargado = app.load_plan(&plan.change_set_id).expect(
            "el plan forjado debe CARGAR: si no, el apply fallaría por el motivo equivocado",
        );
        assert_eq!(
            recargado.policy,
            PlanPolicy::default(),
            "un plan sin la clave `policy` debe deserializar a la default, que es la ESTRICTA \
             (`requireValidResult:true`): si aquí saliera permisiva, el gate sería más laxo de lo \
             que el cliente pidió"
        );

        let antes = canonico_md(root);
        let err = match app.change_apply(&plan.change_set_id, None) {
            Ok(aplicado) => panic!(
                "un plan persistido SIN `policy` —el que dejó un binario anterior a E29-H07— debe \
                 juzgarse con la default ESTRICTA y rechazarse sobre este workspace (el resultado \
                 conserva el error preexistente). Se publicó igual: applied={}, changedPaths={:?}",
                aplicado.applied, aplicado.changed_paths,
            ),
            Err(e) => e,
        };
        assert_eq!(
            err.code.as_str(),
            RECHAZO,
            "el rechazo del plan sin policy es el mismo {RECHAZO} del veredicto, no un PLAN_STALE ni \
             un error de deserialización; fue {err}",
        );
        assert!(
            err.message.contains("requireValidResult"),
            "y nombra la cláusula de la DEFAULT que bloqueó: fue {:?}",
            err.message,
        );
        assert_eq!(canonico_md(root), antes, "y no escribe un solo byte");
    }

    /// **Contraprueba del pin anterior** (anti-vacuo): un plan persistido **sin** `policy` cuyo
    /// resultado **sí** es válido bajo la default se aplica con normalidad.
    ///
    /// Sin esto, «default estricta» podría implementarse como «todo plan sin policy se rechaza», que
    /// convertiría el TTL de los planes en curso de una actualización de binario en una pérdida
    /// gratuita de trabajo. La default es estricta, no prohibitiva.
    #[test]
    fn plan_persistido_sin_policy_con_resultado_valido_sigue_aplicando() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        // Workspace LIMPIO: sin errores ni warnings, de modo que la default (`requireValidResult:
        // true`, `allowWarnings: true`) no tiene nada que objetar.
        escribe(
            root,
            "limpio.md",
            "---\ntitle: Limpio\n---\n\n# Limpio\n\nDocumento sin problemas.\n",
        );
        let app = App::open(root).expect("el workspace temporal debe abrir");

        let plan = app
            .change_plan(None, &patch_inocuo("limpio.md"), policy_permisiva())
            .expect("el plan se computa");
        assert_eq!(
            plan.diagnostics_after.errors, 0,
            "precondición: el resultado es válido bajo la default: {:?}",
            plan.diagnostics_after
        );

        reescribe_plan_persistido(root, |v| {
            v.as_object_mut().unwrap().remove("policy");
        });

        let aplicado = app
            .change_apply(&plan.change_set_id, None)
            .unwrap_or_else(|e| {
                panic!(
                "un plan sin `policy` cuyo resultado SÍ pasa la default debe aplicarse: la default \
                 es estricta, no prohibitiva. Fue rechazado con {e}"
            )
            });
        assert!(aplicado.applied, "y publica de verdad: {aplicado:?}");
        assert!(
            std::fs::read_to_string(root.join("limpio.md"))
                .unwrap()
                .contains("status: revisado"),
            "el cambio está en disco"
        );
    }

    /// **Pin de la RECOMPUTACIÓN** — **Dado** un plan persistido cuyo `canApply` dice `true` pero es
    /// **incongruente** con su propia `policy` (estricta) y con su resultado (que conserva un error),
    /// **Cuando** se aplica, **Entonces** se rechaza igualmente.
    ///
    /// Fija la representación **(ii)** de la historia —persistir la `PlanPolicy` y **recomputar**
    /// `plan::can_apply` en el apply— frente a la (i) —congelar el booleano—. La recomendación de la
    /// spec la eligió por una razón concreta: el apply re-verifica el `planHash` sobre la base actual
    /// y la revisión bajo el lock, así que un `canApply` congelado sería el **único** veredicto del
    /// apply que se acepta sin re-derivar. Un booleano persistido es, además, un campo de un fichero
    /// de runtime editable: leerlo tal cual convierte el gate en una casilla que basta con marcar.
    ///
    /// VERDE hoy (`change_apply` llama a `plan::can_apply(&after_report, &plan.policy)`, no mira
    /// `plan.can_apply`), y mata el mutante `if !plan.can_apply`. **Si alguien migra a la (i), este
    /// test debe romperse**: es el pin que obliga a re-ratificar la decisión en vez de cambiarla en
    /// silencio.
    #[test]
    fn apply_recomputa_el_veredicto_y_no_se_fia_del_can_apply_persistido() {
        let (dir, app) = app_con_error_preexistente();
        let root = dir.path();

        let plan = app
            .change_plan(None, &patch_inocuo("limpio.md"), PlanPolicy::default())
            .expect("el plan se computa");
        assert!(
            !plan.can_apply,
            "precondición: bajo la default el plan NO es aplicable (el error preexistente sobrevive)"
        );

        // Forjado: el booleano MIENTE (`canApply: true`) mientras la policy persistida sigue siendo
        // la estricta y el resultado sigue teniendo el error. Un apply que leyera el booleano
        // publicaría; uno que recompute, rechaza.
        reescribe_plan_persistido(root, |v| {
            v["canApply"] = serde_json::Value::Bool(true);
        });
        let recargado = app
            .load_plan(&plan.change_set_id)
            .expect("el plan forjado debe CARGAR");
        assert!(
            recargado.can_apply,
            "precondición del forjado: el `canApply` persistido dice ahora `true`…"
        );
        assert!(
            recargado.policy.require_valid_result,
            "…mientras su `policy` persistida sigue siendo la estricta: esa es la incongruencia que \
             el test explota"
        );

        let antes = canonico_md(root);
        let err = match app.change_apply(&plan.change_set_id, None) {
            Ok(aplicado) => panic!(
                "el apply debe RECOMPUTAR `can_apply` sobre el resultado hipotético con la `policy` \
                 persistida, no fiarse del booleano del fichero: un `canApply:true` forjado en \
                 `.lodestar/runtime/plans/` no puede ser la llave que abre la publicación. Se \
                 publicó: applied={}, changedPaths={:?}",
                aplicado.applied, aplicado.changed_paths,
            ),
            Err(e) => e,
        };
        assert_eq!(
            err.code.as_str(),
            RECHAZO,
            "el rechazo recomputado es el mismo {RECHAZO}; fue {err}",
        );
        assert!(
            err.message.contains("requireValidResult"),
            "y nombra la cláusula que lo bloquea al recomputarlo: fue {:?}",
            err.message,
        );
        assert_eq!(canonico_md(root), antes, "y no escribe un solo byte");
    }
}

// ===========================================================================
// E29-H08 — La TABLA DE CAMPOS LEGALES por operación (condición de entrada)
// `requirements/epica-29-honestidad-superficie.md §E29-H08` (L820-840) · `decisiones §15`.
// ===========================================================================
//
// `§15` fija esto como **primer criterio de aceptación Y condición de entrada**: la tabla de campos
// legales por operación se materializa en tests **VERDES antes de tocar el rechazo**, y sigue verde
// después. La razón es concreta y está medida: `operacion_item_schema()` declara 17 propiedades
// planas **a propósito** —sin `oneOf` por op— porque `path`/`ref` son intercambiables salvo en
// `create` y `body` pertenece a DOS ops (`create` y `replace_body`). Activar la validación sin
// haber fijado antes qué es legal rompería `create` con campos de otra op, o los lotes en los que un
// agente reutiliza la misma plantilla de objeto.
//
// Estos tests nacen **VERDES** (no son la fase roja: fijan lo que HOY funciona) y su valor está en
// que cualquier rechazo que se implemente encima tenga que seguir pasándolos. La mitad ROJA de la
// historia vive por el wire, en `crates/lodestar-mcp/tests/mcp.rs` y `tests/descubribilidad.rs`.
//
// La tabla (fuente: `normalize_raw_op` en `lodestar-app`, vía `decisiones §15`):
//
//   | Campo                                  | Ops                                    |
//   |----------------------------------------|----------------------------------------|
//   | `op`                                   | todas (discriminador, obligatorio)     |
//   | `path`                                 | todas (obligatoria en `create`)        |
//   | `ref`                                  | todas menos `create`                   |
//   | `expectedRevision`                     | todas                                  |
//   | `frontmatter`                          | `create`                               |
//   | `body`                                 | `create`, `replace_body` (COMPARTIDO)  |
//   | `patch`                                | `patch_frontmatter`                    |
//   | `find`,`replace`,`expectedOccurrences` | `replace_text`                         |
//   | `headingPath`,`mode`,`content`         | `edit_section`                         |
//   | `from`,`to`,`rewriteInboundLinks`      | `move`                                 |
//   | `inboundLinksPolicy`                   | `delete`                               |
mod campos_legales_por_operacion {
    use super::*;
    use lodestar_core::types::RelPath;

    /// Workspace con lo justo para ejercer las 7 ops: un documento existente con frontmatter,
    /// cuerpo con un texto sustituible y una sección con heading (para `edit_section`), más un
    /// `enlazado.md` con un backlink hacia él (para que `delete` tenga que decidir su
    /// `inboundLinksPolicy`) y un path libre para `create`.
    fn app_para_las_siete_ops() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        escribe(
            dir.path(),
            "notas/alfa.md",
            "---\nstatus: accepted\nowner: ana\n---\n\n# Alfa\n\nCuerpo con lodestar dentro.\n\n\
             ## Seguridad\n\nContenido de la sección.\n",
        );
        escribe(
            dir.path(),
            "notas/enlazado.md",
            "---\nstatus: draft\n---\n\n# Enlazado\n\n[hacia alfa](alfa.md)\n",
        );
        let app = App::open(dir.path()).expect("el workspace temporal debe abrir");
        (dir, app)
    }

    /// La revisión vigente de un documento, para poder mandar `expectedRevision` **con un valor
    /// real** en cada op: el campo es legal en las 7 y solo se ejercita de verdad si el control
    /// optimista pasa (con un valor inventado, la op fallaría por `REVISION_CONFLICT` y el test
    /// mediría otra cosa).
    fn revision_de(app: &App, path: &str) -> String {
        let rel = RelPath::new(path).expect("path relativo válido");
        let doc = app
            .knowledge_get(
                &lodestar_core::types::DocumentRef {
                    path: rel,
                    id: None,
                },
                &["revision".to_string()],
                None,
            )
            .expect("el documento debe existir");
        serde_json::to_value(&doc)
            .expect("el `DocumentView` serializa")
            .get("revision")
            .and_then(|v| v.as_str())
            .expect("se pidió `revision` en el include, así que debe venir poblada")
            .to_string()
    }

    /// **E29-H08 · Criterio `los_campos_legales_de_cada_operacion_se_aceptan`** (CONDICIÓN DE
    /// ENTRADA — verde antes de activar el rechazo, y verde después):
    /// **Dado** un `change_plan` con **una operación de cada uno de los 7 tipos**, cada una con
    /// **todos** sus campos legales de la tabla, **Cuando** se planifica, **Entonces** las 7 se
    /// normalizan sin error.
    ///
    /// Se planifica cada op **por separado** (7 llamadas, no un lote de 7): las ops de estructura
    /// interfieren entre sí sobre los mismos paths —un `move` de `alfa.md` dejaría sin objetivo al
    /// `delete` que le sigue—, y lo que este criterio fija es la LEGALIDAD DE LOS CAMPOS, no la
    /// composición de un lote. Cada llamada usa el `App` recién montado, así que el estado de
    /// partida es idéntico para las 7.
    #[test]
    fn los_campos_legales_de_cada_operacion_se_aceptan() {
        // (op, campos legales de ESA op según la tabla). `expectedRevision` se inyecta abajo con
        // la revisión real del documento objetivo, porque su valor depende del workspace.
        let casos: Vec<(&str, serde_json::Value)> = vec![
            // `create`: `path` OBLIGATORIA (no admite `ref`), `frontmatter` y `body` suyos.
            (
                "create",
                serde_json::json!({
                    "op": "create", "path": "notas/nuevo.md",
                    "frontmatter": { "status": "draft" },
                    "body": "# Nuevo\n\nCuerpo del documento nuevo.\n"
                }),
            ),
            // `patch_frontmatter`: forma LARGA del objetivo (`ref`), que es legal en todas menos
            // `create`.
            (
                "patch_frontmatter",
                serde_json::json!({
                    "op": "patch_frontmatter", "ref": { "path": "notas/alfa.md" },
                    "patch": { "status": "review" }
                }),
            ),
            // `replace_body`: la SEGUNDA op de `body`, el campo compartido de la tabla. Aquí con
            // la forma CORTA (`path`), para que el par (`ref`,`path`) quede ejercitado en las dos.
            (
                "replace_body",
                serde_json::json!({
                    "op": "replace_body", "path": "notas/alfa.md",
                    "body": "# Alfa\n\nCuerpo reemplazado entero.\n"
                }),
            ),
            (
                "replace_text",
                serde_json::json!({
                    "op": "replace_text", "path": "notas/alfa.md",
                    "find": "lodestar", "replace": "Lodestar", "expectedOccurrences": 1
                }),
            ),
            (
                "edit_section",
                serde_json::json!({
                    "op": "edit_section", "path": "notas/alfa.md",
                    "headingPath": ["Seguridad"], "mode": "replace",
                    "content": "Contenido nuevo de la sección.\n"
                }),
            ),
            // `move`: `from`/`to` propios + `rewriteInboundLinks`.
            (
                "move",
                serde_json::json!({
                    "op": "move", "from": "notas/alfa.md", "to": "notas/alfa-movida.md",
                    "rewriteInboundLinks": true
                }),
            ),
            // `delete`: `inboundLinksPolicy` es OBLIGATORIA aquí porque `alfa.md` tiene un backlink
            // desde `enlazado.md` (`§20.11` prohíbe elegir en silencio).
            (
                "delete",
                serde_json::json!({
                    "op": "delete", "path": "notas/alfa.md",
                    "inboundLinksPolicy": "remove_links"
                }),
            ),
        ];

        for (nombre, mut op) in casos {
            let (_dir, app) = app_para_las_siete_ops();

            // `expectedRevision` es legal en las 7: se añade con la revisión REAL del documento
            // objetivo (`create` no tiene documento previo, así que ahí se omite — pedir la
            // revisión de un documento que aún no existe no es un campo legal, es un sinsentido).
            // Las seis ops restantes operan sobre `notas/alfa.md`, incluido el `move` (su `from`).
            if nombre != "create" {
                op["expectedRevision"] = serde_json::json!(revision_de(&app, "notas/alfa.md"));
            }

            let resultado =
                app.change_plan(None, &serde_json::json!([op.clone()]), policy_permisiva());
            let plan = resultado.unwrap_or_else(|e| {
                panic!(
                    "la op «{nombre}» con TODOS sus campos legales de la tabla de `decisiones §15` \
                     debe normalizarse sin error (condición de entrada de E29-H08: esto funciona \
                     HOY y el rechazo de campos desconocidos no puede romperlo). Op: {op}\nError: {e}"
                )
            });
            assert_eq!(
                plan.normalized_operations.len().min(1),
                1,
                "la op «{nombre}» debe producir al menos una `NormalizedOperation`: {op}"
            );
        }
    }

    /// **E29-H08 · Control anti-vacuo del campo COMPARTIDO y de las dos formas del objetivo**: la
    /// tabla dice que `body` pertenece a **dos** ops y que `path`/`ref` son intercambiables salvo en
    /// `create`. Si el rechazo se escribiera como una partición limpia por op, ESTE es el test que
    /// cae — y es el riesgo que `decisiones §15` señala como «no teórico».
    ///
    /// Nace VERDE y debe seguir verde: fija las dos propiedades que hacen que la validación tenga
    /// que ser por unión.
    #[test]
    fn body_es_legal_en_dos_ops_y_path_y_ref_son_intercambiables() {
        // (a) `body` en `create` y en `replace_body`: el MISMO campo, dos ops.
        for (op_kind, op) in [
            (
                "create",
                serde_json::json!({ "op": "create", "path": "notas/otro.md",
                                    "body": "# Otro\n\ncuerpo\n" }),
            ),
            (
                "replace_body",
                serde_json::json!({ "op": "replace_body", "path": "notas/alfa.md",
                                    "body": "# Alfa\n\ncuerpo nuevo\n" }),
            ),
        ] {
            let (_dir, app) = app_para_las_siete_ops();
            app.change_plan(None, &serde_json::json!([op.clone()]), policy_permisiva())
                .unwrap_or_else(|e| {
                    panic!(
                        "`body` es legal en «{op_kind}» (la tabla lo declara COMPARTIDO entre dos \
                         ops): {op}\nError: {e}"
                    )
                });
        }

        // (b) `path` y `ref.path` designan el mismo objetivo en una op que no es `create`.
        for (forma, op) in [
            (
                "path (forma corta)",
                serde_json::json!({ "op": "patch_frontmatter", "path": "notas/alfa.md",
                                    "patch": { "status": "review" } }),
            ),
            (
                "ref.path (forma larga)",
                serde_json::json!({ "op": "patch_frontmatter", "ref": { "path": "notas/alfa.md" },
                                    "patch": { "status": "review" } }),
            ),
        ] {
            let (_dir, app) = app_para_las_siete_ops();
            app.change_plan(None, &serde_json::json!([op.clone()]), policy_permisiva())
                .unwrap_or_else(|e| {
                    panic!(
                        "el objetivo por «{forma}» debe seguir aceptándose: la tabla los declara \
                         INTERCAMBIABLES salvo en `create`: {op}\nError: {e}"
                    )
                });
        }
    }
}

// ===========================================================================
// E30-H03 — seguimiento A-06: `replace_text` sin ocurrencias en forma-array es un no-op silencioso
// (`requirements/epica-30-higiene-escoba.md` E30-H03 punto A-06, `decisiones §23`).
//
// `docs/user/safe-changes.md` ya documenta el vacío-sin-error para SELECCIONES MASIVAS ("a
// selection that matches nothing produces an empty plan, not an error"), pero no fija el mismo
// comportamiento para la forma-array (`operations: [{...}]`) de operaciones sueltas, que es un
// camino de código distinto (no pasa por `select`/`captured_revisions`). Este test de guardia FIJA
// el comportamiento actual: un `replace_text` cuyo `find` no aparece en el documento, sin
// `expectedOccurrences` (que sería lo que convertiría un recuento distinto de 0 en error), produce
// un plan `canApply: true` con diff vacío para ese documento — ninguna operación normalizada lo
// modifica y `semantic_diff` no lo lista como `modified`/`body_changes`.
//
// GUARDA (nace VERDE, `E30-H03` lo declara documental salvo el test): no se ha encontrado en el
// repo un test previo (E28/E29) que ejerza exactamente esta combinación (forma-array +
// `replace_text` + `find` sin match + sin `expectedOccurrences`); los tests de `replace_text`
// existentes (`los_campos_legales_de_cada_operacion_se_aceptan`, arriba) usan un `find` que SÍ
// aparece. Congela el comportamiento para que un cambio futuro que lo convierta en error o en un
// plan con contenido lo note.
// ===========================================================================

/// **A-06** · `replace_text_sin_ocurrencias_en_forma_array_es_noop`: **Dado** un documento sin
/// ninguna ocurrencia de un `find` dado, **Cuando** se llama a `change_plan` con `replace_text` en
/// forma-array sin `expectedOccurrences`, **Entonces** el plan resultante tiene `canApply: true` y
/// un diff vacío para ese documento (sin error).
#[test]
fn replace_text_sin_ocurrencias_en_forma_array_es_noop() {
    let (_dir, app) = app_con_workspace();

    // `alfa.md` (ver `app_con_workspace`) no contiene la cadena «esta-cadena-no-existe-en-el-doc».
    let ops = serde_json::json!([
        { "op": "replace_text", "path": "alfa.md",
          "find": "esta-cadena-no-existe-en-el-doc", "replace": "sustituto" },
    ]);

    let plan = app.change_plan(None, &ops, policy_permisiva()).expect(
        "un `replace_text` cuyo `find` no aparece no debe fallar al planificar: es un no-op",
    );

    assert!(
        plan.can_apply,
        "un plan no-op (find sin ocurrencias, sin expectedOccurrences) debe ser `canApply: true`: {plan:?}"
    );
    let alfa = lodestar_core::types::RelPath::new("alfa.md").unwrap();
    assert!(
        !plan.semantic_diff.modified.contains(&alfa),
        "el diff no debe listar `alfa.md` como modificado si `find` no tuvo ninguna ocurrencia: \
         {:?}",
        plan.semantic_diff
    );
    assert!(
        !plan.semantic_diff.body_changes.contains(&alfa),
        "el diff no debe listar cambios de cuerpo en `alfa.md` si `find` no tuvo ninguna ocurrencia: \
         {:?}",
        plan.semantic_diff
    );
    assert!(
        plan.semantic_diff.created.is_empty()
            && plan.semantic_diff.deleted.is_empty()
            && plan.semantic_diff.moved.is_empty(),
        "un `replace_text` no-op no debe producir ningún otro tipo de cambio en el diff: {:?}",
        plan.semantic_diff
    );
}
