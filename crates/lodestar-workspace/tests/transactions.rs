//! Tests de integración de la mecánica transaccional de `lodestar-workspace` (E13:
//! publicación recuperable). Este fichero cubre **E13-H01 — Staging: materializar el resultado
//! completo + validar staging** y **E13-H02 — Lock de workspace + re-verificación de la
//! `WorkspaceRevision` base**.
//!
//! Firmas asumidas de E13-H01 (fase ROJA; el implementador debe respetarlas):
//! - `Workspace::materialize_staging(&self, change_set: &ChangeSet) -> Result<StagingDir, WorkspaceError>`
//!   escribe TODOS los ficheros resultantes del plan (reusando `plan::apply_normalized_ops` sobre
//!   el `FileMap` canónico) bajo `.lodestar/runtime/staging/<changeSetId>/`. No toca el canónico.
//! - `Workspace::validate_staging(&self, staging: &StagingDir) -> Result<(), WorkspaceError>`
//!   construye un `Bundle` desde el árbol de staging (canónico + staging), corre `analyze` y, si el
//!   resultado no cumple la política (gate estricto: `hard_fail > 0`), aborta SIN tocar el canónico
//!   y limpia el staging. El `Err` mapea al wire `NONCONFORMANT_RESULT` (`WorkspaceError::code()`).
//! - `StagingDir::path(&self) -> &Path` expone el directorio de staging materializado.
//!
//! Firmas asumidas de E13-H02 (fase ROJA; el implementador debe respetarlas):
//! - `Workspace::acquire_lock(&self) -> Result<WorkspaceLock, WorkspaceError>`: adquiere el lock
//!   exclusivo de publicación (fichero en `.lodestar/runtime/` con owner/pid/timestamp). **Modelo
//!   fail-fast**: si el lock ya está tomado (por este u otro handle sobre el mismo root) devuelve
//!   `Err` (no bloquea). El `WorkspaceLock` devuelto es un guard RAII: su `Drop` borra el fichero
//!   de lock, de modo que el lock se libera SIEMPRE (incluido en un `panic`/desenrollado de pila).
//! - `Workspace::lock_path(&self) -> PathBuf`: ruta del fichero de lock de publicación (bajo
//!   `.lodestar/runtime/`), exista o no. Determinista; los tests la usan para comprobar que el
//!   guard crea el fichero mientras vive y lo borra al dropearse.
//! - `Workspace::workspace_revision(&self) -> Result<WorkspaceRevision, WorkspaceError>`: computa la
//!   `WorkspaceRevision` actual del conocimiento escribible (misma lógica que
//!   `lodestar_core::types::workspace_revision(files, &cfg.workspace.writable_roots)`, E10-H03).
//! - `Workspace::reverify_base_revision(&self, base: &WorkspaceRevision) -> Result<(), WorkspaceError>`:
//!   re-verifica que la revisión actual sigue siendo la `base` esperada por el plan. Si coincide →
//!   `Ok(())`; si el workspace cambió entre plan y apply → `Err` cuyo `.code()` mapea al wire
//!   `WRITE_CONFLICT` (nueva variante `WorkspaceError::WriteConflict`), y NO se publica.
//!
//! `ChangeSet` (dominio de `lodestar-core`) es el argumento: `materialize_staging` solo necesita su
//! `id` (nombre del directorio de staging) y sus `operations`; los campos de análisis
//! (`risk`/`semantic_diff`/`validation`) son irrelevantes para la materialización y aquí se rellenan
//! con `Default`.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use lodestar_core::types::{
    ChangeSet, ChangeSetId, FrontmatterPatch, NormalizedOperation, PlanHash, RelPath,
    RiskAssessment, SemanticDiff, ValidationReport, WorkspaceRevision,
};
use lodestar_workspace::Workspace;

// ---------------------------------------------------------------------------
// E13-H03 — Write-ahead journal.
//
// Firmas asumidas de E13-H03 (fase ROJA; el implementador debe respetarlas):
// - `Workspace::create_journal(&self, txn_id: &str, ops: &[RelPath], base_rev: &WorkspaceRevision,
//   result_rev: &WorkspaceRevision) -> Result<Journal, WorkspaceError>`: escribe el write-ahead
//   journal de la transacción en `.lodestar/runtime/journal/<txnId>.json` en estado `prepared`
//   ANTES de la primera sustitución del canónico, con las N operaciones registradas (una por
//   `RelPath`), la `baseWorkspaceRevision` y la `resultWorkspaceRevision` esperadas, y lo **fsyncea**
//   a disco (el fsync no es directamente testeable a nivel unitario — el test solo comprueba que el
//   fichero quedó en disco y bien formado; el `Journal` devuelto es un handle vivo para marcar los
//   renames a medida que se completan).
// - `Journal::path(&self) -> &std::path::Path`: ruta del fichero de journal materializado (bajo
//   `.lodestar/runtime/journal/`).
// - `Journal::mark_applied(&mut self, path: &RelPath) -> Result<(), WorkspaceError>`: marca la
//   operación de `path` como aplicada (rename completado) y **persiste** el journal actualizado a
//   disco; la primera marca transiciona el estado global del journal de `prepared` a `applying`.
// - `Journal::state(&self) -> JournalState` (asumida disponible; los tests leen el estado del JSON
//   en disco, que es la fuente de verdad recuperable, por lo que no la invocan directamente).
//
// Forma del JSON del journal que asumen los tests (`.lodestar/runtime/journal/<txnId>.json`):
//   {
//     "txnId": "txn-h03-tres-ops",
//     "state": "prepared",            // prepared -> applying -> applied -> done
//     "baseWorkspaceRevision": "blake3:...",
//     "resultWorkspaceRevision": "blake3:...",
//     "operations": [
//       { "path": "uno.md",  "state": "pending" },   // pending -> applied (por rename)
//       { "path": "dos.md",  "state": "pending" },
//       { "path": "tres.md", "state": "pending" }
//     ]
//   }
// Los tests solo dependen de: `state` (string a nivel raíz), `operations` (array con un `path` por
// entrada y un `state` por entrada). Los nombres exactos de campo (`state`/`operations`/`path`) son
// parte del contrato de recuperación (H06 releerá este mismo JSON).
// ---------------------------------------------------------------------------

/// Lee y parsea el JSON del journal desde disco (la fuente de verdad recuperable). Falla si el
/// fichero no existe o no es JSON válido — así el ROJO por journal ausente es inequívoco.
fn leer_journal(path: &Path) -> serde_json::Value {
    let raw = std::fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("el journal debe existir en disco {}: {e}", path.display()));
    serde_json::from_str(&raw).unwrap_or_else(|e| {
        panic!(
            "el journal debe ser JSON bien formado {}: {e}",
            path.display()
        )
    })
}

/// Estado (campo `state`) de la operación cuyo `path` coincide, leído del JSON del journal.
fn estado_op<'a>(journal: &'a serde_json::Value, path: &str) -> &'a str {
    let ops = journal["operations"]
        .as_array()
        .expect("el journal debe listar `operations` como array");
    ops.iter()
        .find(|op| op["path"].as_str() == Some(path))
        .unwrap_or_else(|| panic!("el journal no registra la operación {path}"))["state"]
        .as_str()
        .unwrap_or_else(|| panic!("la operación {path} debe tener un `state` string"))
}

/// Un `FrontmatterPatch` a partir de pares `(clave, valor_string)`.
fn patch(pares: &[(&str, &str)]) -> FrontmatterPatch {
    let mut map = BTreeMap::new();
    for (k, v) in pares {
        map.insert(
            (*k).to_string(),
            Some(serde_yaml::Value::String((*v).to_string())),
        );
    }
    FrontmatterPatch(map)
}

/// Envuelve un conjunto de `NormalizedOperation` en un `ChangeSet` mínimo con el `id` dado. Los
/// campos de análisis van a `Default` porque `materialize_staging` no los consume.
fn change_set(id: &str, operations: Vec<NormalizedOperation>) -> ChangeSet {
    ChangeSet {
        id: ChangeSetId(id.to_string()),
        base_revision: WorkspaceRevision("blake3:test".to_string()),
        operations,
        plan_hash: PlanHash("blake3:test".to_string()),
        risk: RiskAssessment::default(),
        semantic_diff: SemanticDiff::default(),
        validation: ValidationReport::default(),
        expires_at: "0".to_string(),
    }
}

/// Un `Create` conforme (con `type` y `title`) que resuelve al `.md` `path`.
fn create_conforme(path: &str, ty: &str, title: &str) -> NormalizedOperation {
    NormalizedOperation::Create {
        path: RelPath::new(path).unwrap(),
        frontmatter: patch(&[("type", ty), ("title", title)]),
        body: Some(format!("# {title}\n\ncuerpo\n")),
    }
}

/// Mapa `ruta relativa -> contenido` de todos los `.md` canónicos (excluye `.lodestar/` y `.git/`).
fn canonical_md(root: &Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = entry.file_name();
            if name == ".lodestar" || name == ".git" {
                continue;
            }
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|e| e == "md") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .to_string();
                let content = std::fs::read_to_string(&path).unwrap();
                out.insert(rel, content);
            }
        }
    }
    walk(root, root, &mut out);
    out
}

/// **E13-H01** · Criterio: dado un change set de 3 escrituras, al materializarlo en staging los 3
/// ficheros existen bajo `.lodestar/runtime/staging/<id>/` y el canónico NO cambió.
#[test]
fn staging_no_toca_canonico() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    // Concepto canónico previo, para comprobar que la materialización no lo altera.
    ws.create_concept(
        &RelPath::new("raiz.md").unwrap(),
        "Nota",
        Some("Raiz"),
        "# H\n\ncuerpo\n",
        false,
    )
    .unwrap();

    let antes = canonical_md(dir.path());

    // Change set con 3 escrituras nuevas.
    let cs = change_set(
        "changeset:tres-escrituras",
        vec![
            create_conforme("uno.md", "Nota", "Uno"),
            create_conforme("dos.md", "Nota", "Dos"),
            create_conforme("tres.md", "Nota", "Tres"),
        ],
    );

    let staging = ws.materialize_staging(&cs).unwrap();

    // El directorio de staging vive bajo `.lodestar/runtime/staging/`.
    let staging_path: PathBuf = staging.path().to_path_buf();
    assert!(
        staging_path.starts_with(dir.path().join(".lodestar/runtime/staging")),
        "el staging no vive bajo .lodestar/runtime/staging: {}",
        staging_path.display()
    );

    // Los 3 ficheros del plan existen materializados en staging.
    for f in ["uno.md", "dos.md", "tres.md"] {
        assert!(
            staging_path.join(f).is_file(),
            "falta {f} en el staging {}",
            staging_path.display()
        );
    }

    // El canónico NO cambió (mismos `.md`, mismo contenido; ningún fichero nuevo en el canónico).
    let despues = canonical_md(dir.path());
    assert_eq!(
        antes, despues,
        "la materialización en staging alteró el conocimiento canónico"
    );
    assert!(
        !dir.path().join("uno.md").exists(),
        "un fichero del plan se filtró al canónico"
    );
}

/// **E13-H01** · Criterio: dado un staging que resultaría NO conforme (política estricta), al
/// validarlo aborta con `NONCONFORMANT_RESULT` y limpia el staging.
#[test]
fn staging_no_conforme_aborta() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    let antes = canonical_md(dir.path());

    // Change set cuyo resultado es NO conforme: un `Create` con `type` vacío (hard-fail de esquema,
    // mismo motivo que `create_concept("")` rechaza en `create_concept_no_conforme_no_escribe`).
    let cs = change_set(
        "changeset:no-conforme",
        vec![NormalizedOperation::Create {
            path: RelPath::new("malo.md").unwrap(),
            frontmatter: patch(&[("type", ""), ("title", "Malo")]),
            body: Some("# Malo\n".to_string()),
        }],
    );

    let staging = ws.materialize_staging(&cs).unwrap();
    let staging_path: PathBuf = staging.path().to_path_buf();

    // La validación bajo gate estricto rechaza el resultado no conforme.
    let err = ws
        .validate_staging(&staging)
        .expect_err("un staging no conforme debe abortar la validación");
    assert_eq!(
        err.code(),
        "NONCONFORMANT_RESULT",
        "el error de validación no mapea a NONCONFORMANT_RESULT: {err:?}"
    );

    // El staging quedó limpio (el directorio del changeSetId no persiste).
    assert!(
        !staging_path.exists(),
        "el staging no se limpió tras abortar: {}",
        staging_path.display()
    );

    // El canónico nunca se tocó.
    let despues = canonical_md(dir.path());
    assert_eq!(
        antes, despues,
        "un staging abortado alteró el conocimiento canónico"
    );
}

// ---------------------------------------------------------------------------
// E13-H02 — Lock de workspace + re-verificación de la `WorkspaceRevision` base.
// ---------------------------------------------------------------------------

/// **E13-H02** · Criterio `lock_exclusivo`: dado un lock tomado, cuando otro publicador intenta
/// adquirirlo, entonces falla (modelo fail-fast: no dos escritores). Al liberar el primero, un
/// nuevo intento vuelve a adquirirlo.
#[test]
fn lock_exclusivo() {
    let dir = tempfile::tempdir().unwrap();
    // Dos handles sobre el MISMO root: modelan dos publicadores concurrentes.
    let ws = Workspace::open(dir.path()).unwrap();
    let otro = Workspace::open(dir.path()).unwrap();

    // El primer publicador adquiere el lock; el guard vive mientras esté en alcance.
    let guard = ws
        .acquire_lock()
        .expect("el primer publicador debe adquirir el lock");

    // El segundo publicador NO puede adquirirlo con el lock ya tomado (fail-fast, no bloqueante).
    assert!(
        otro.acquire_lock().is_err(),
        "un segundo publicador no debe poder adquirir un lock ya tomado (no dos escritores)"
    );

    // Al soltar el primero, el guard se dropea y el lock queda libre...
    drop(guard);

    // ...y un nuevo intento SÍ lo obtiene.
    let _tercero = otro
        .acquire_lock()
        .expect("tras liberar el lock, un nuevo publicador debe poder adquirirlo");
}

/// **E13-H02** · Criterio `revision_base_cambiada`: si el workspace cambió entre plan y apply, al
/// re-verificar la revisión base → `WRITE_CONFLICT` y no se publica.
#[test]
fn revision_base_cambiada() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    // Estado inicial sobre el que se "planificó".
    ws.create_concept(
        &RelPath::new("base.md").unwrap(),
        "Nota",
        Some("Base"),
        "# H\n\ncuerpo\n",
        false,
    )
    .unwrap();

    // R1: la `baseWorkspaceRevision` del plan.
    let r1 = ws
        .workspace_revision()
        .expect("computa la revisión base del workspace");

    // Sin cambios, la re-verificación contra R1 es coherente (no es un test vacuo al revés).
    ws.reverify_base_revision(&r1)
        .expect("sin cambios, re-verificar la revisión base debe ser Ok");

    // El workspace cambia ENTRE plan y apply: otro escritor introduce un concepto.
    ws.create_concept(
        &RelPath::new("intruso.md").unwrap(),
        "Nota",
        Some("Intruso"),
        "# H\n\notro\n",
        false,
    )
    .unwrap();

    // Re-verificar contra R1 detecta que la base ya no es la misma → conflicto de escritura.
    let err = ws
        .reverify_base_revision(&r1)
        .expect_err("la base cambió entre plan y apply: la re-verificación debe abortar");
    assert_eq!(
        err.code(),
        "WRITE_CONFLICT",
        "el conflicto de revisión base debe mapear al wire WRITE_CONFLICT: {err:?}"
    );
}

/// **E13-H02** · Criterio `lock_se_libera_en_panic`: un panic durante la publicación → el guard se
/// dropea → el lock se libera (no queda fichero huérfano ni bloqueo lógico).
#[test]
fn lock_se_libera_en_panic() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    let lock_path: PathBuf = ws.lock_path();
    assert!(
        !lock_path.exists(),
        "no debe existir fichero de lock antes de adquirirlo"
    );

    // Publicación que paniquea con el guard vivo. `catch_unwind` recoge el desenrollado; durante él
    // el `Drop` del guard debe ejecutarse y liberar el lock.
    let resultado = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = ws
            .acquire_lock()
            .expect("adquiere el lock antes de publicar");
        assert!(
            lock_path.exists(),
            "el fichero de lock debe existir mientras el guard vive"
        );
        panic!("fallo simulado durante la publicación");
    }));

    assert!(
        resultado.is_err(),
        "el panic debe propagarse fuera del catch_unwind"
    );

    // El Drop del guard liberó el lock: ni fichero huérfano...
    assert!(
        !lock_path.exists(),
        "el Drop del guard debe borrar el fichero de lock tras el panic (no queda huérfano)"
    );

    // ...ni bloqueo lógico: un nuevo publicador vuelve a adquirirlo.
    let _nuevo = ws
        .acquire_lock()
        .expect("tras el panic y la liberación, el lock debe poder re-adquirirse");
}

// ---------------------------------------------------------------------------
// E13-H03 — Write-ahead journal (tests).
// ---------------------------------------------------------------------------

/// **E13-H03** · Criterio `journal_prepared_antes_de_publicar`: dada una transacción a punto de
/// publicar con N operaciones, al prepararla existe el journal en estado `prepared` con las N
/// operaciones (fsynced — no directamente testeable a nivel unitario; se comprueba que el fichero
/// quedó en disco y bien formado).
#[test]
fn journal_prepared_antes_de_publicar() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    // Las N (=3) operaciones que la transacción va a sustituir en el canónico.
    let ops = [
        RelPath::new("uno.md").unwrap(),
        RelPath::new("dos.md").unwrap(),
        RelPath::new("tres.md").unwrap(),
    ];
    let base = WorkspaceRevision("blake3:base".to_string());
    let result = WorkspaceRevision("blake3:result".to_string());

    // Se prepara el write-ahead journal ANTES de la primera sustitución del canónico.
    let journal = ws
        .create_journal("txn-h03-tres-ops", &ops, &base, &result)
        .expect("crear el journal en estado prepared");

    // El fichero vive bajo `.lodestar/runtime/journal/<txnId>.json`.
    let journal_path: PathBuf = journal.path().to_path_buf();
    assert!(
        journal_path.starts_with(dir.path().join(".lodestar/runtime/journal")),
        "el journal no vive bajo .lodestar/runtime/journal: {}",
        journal_path.display()
    );
    assert_eq!(
        journal_path.file_name().and_then(|n| n.to_str()),
        Some("txn-h03-tres-ops.json"),
        "el journal debe nombrarse <txnId>.json: {}",
        journal_path.display()
    );
    assert!(
        journal_path.is_file(),
        "el journal debe estar materializado en disco (fsynced) antes de publicar: {}",
        journal_path.display()
    );

    // Releído del disco: estado `prepared` y las 3 operaciones registradas.
    let json = leer_journal(&journal_path);
    assert_eq!(
        json["state"].as_str(),
        Some("prepared"),
        "el journal recién creado debe estar en estado `prepared`: {json}"
    );
    let listadas = json["operations"]
        .as_array()
        .expect("el journal debe listar `operations` como array");
    assert_eq!(
        listadas.len(),
        3,
        "el journal debe registrar las N=3 operaciones de la transacción: {json}"
    );
    for f in ["uno.md", "dos.md", "tres.md"] {
        assert_eq!(
            estado_op(&json, f),
            "pending",
            "toda operación de un journal `prepared` debe estar `pending`: {json}"
        );
    }
}

/// **E13-H03** · Criterio `journal_registra_cada_rename`: dada una sustitución completada, al
/// registrarla el journal la marca aplicada (y el estado global transiciona a `applying`),
/// persistido a disco.
#[test]
fn journal_registra_cada_rename() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    let ops = [
        RelPath::new("uno.md").unwrap(),
        RelPath::new("dos.md").unwrap(),
        RelPath::new("tres.md").unwrap(),
    ];
    let base = WorkspaceRevision("blake3:base".to_string());
    let result = WorkspaceRevision("blake3:result".to_string());

    let mut journal = ws
        .create_journal("txn-h03-un-rename", &ops, &base, &result)
        .expect("crear el journal en estado prepared");
    let journal_path: PathBuf = journal.path().to_path_buf();

    // El primer rename se completa: se registra en el journal.
    journal
        .mark_applied(&RelPath::new("dos.md").unwrap())
        .expect("marcar `dos.md` como aplicada tras completar su rename");

    // Releído del disco (la fuente de verdad recuperable): la op figura aplicada y el estado global
    // transicionó a `applying`; las demás siguen pendientes.
    let json = leer_journal(&journal_path);
    assert_eq!(
        estado_op(&json, "dos.md"),
        "applied",
        "la operación cuyo rename se completó debe figurar `applied`: {json}"
    );
    assert_eq!(
        json["state"].as_str(),
        Some("applying"),
        "tras el primer rename el journal debe transicionar a `applying`: {json}"
    );
    assert_eq!(
        estado_op(&json, "uno.md"),
        "pending",
        "una operación aún no aplicada debe seguir `pending`: {json}"
    );
    assert_eq!(
        estado_op(&json, "tres.md"),
        "pending",
        "una operación aún no aplicada debe seguir `pending`: {json}"
    );
}
