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

/// Escribe un `.md` (creando los directorios intermedios) dentro del bundle temporal.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Monta un `App` sobre un bundle temporal con un index raíz + un concept conforme (`alfa.md`).
/// `App::open` crea el scaffold `.lodestar/runtime/{plans,receipts,staging}` (E9-H06). El `TempDir`
/// se devuelve para mantener el directorio vivo mientras dure el test.
fn app_con_bundle() -> (tempfile::TempDir, App) {
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
    let app = App::open(dir.path()).expect("el bundle temporal debe abrir");
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
        require_conformant_result: false,
        allow_warnings: true,
    }
}

/// Directorio de planes runtime del bundle.
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
    let (dir, app) = app_con_bundle();
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
    let (dir, app) = app_con_bundle();
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
        matches!(resultado, Err(ErrorCode::PlanExpired)),
        "cargar un plan con `expiresAt` en el pasado debe dar Err(PlanExpired), dio {resultado:?}",
    );
}

/// Guarda contra la vacuidad de `plan_caducado`: un plan VIGENTE (recién persistido, `expiresAt`
/// futuro) se carga con éxito y su `planHash` coincide — así `load_plan` no puede limitarse a
/// devolver `PlanExpired` siempre.
#[test]
fn plan_vigente_carga() {
    let (_dir, app) = app_con_bundle();
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
    let (dir, app) = app_con_bundle();

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
    let app2 = App::open(dir.path()).expect("reabrir el bundle debe funcionar");
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
