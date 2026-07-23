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
use std::time::{Duration, SystemTime};

use lodestar_core::plan;
use lodestar_core::types::{
    ChangeReceipt, ChangeSet, ChangeSetId, FileMap, FrontmatterPatch, NormalizedOperation,
    PlanHash, ReceiptId, RelPath, RiskAssessment, SemanticDiff, ValidationReport,
    WorkspaceRevision,
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

    // Change set cuyo resultado es NO conforme. MIGRADO en E16-H05: era un `Create` con `type`
    // vacío (`OKF-TYPE`), retirado del catálogo; hoy es un cuerpo con marcadores de merge sin
    // resolver (`DOC-CONFLICT-MARKER`), mismo motivo por el que rechaza
    // `create_concept_no_conforme_no_escribe`.
    let cs = change_set(
        "changeset:no-conforme",
        vec![NormalizedOperation::Create {
            path: RelPath::new("malo.md").unwrap(),
            frontmatter: patch(&[("type", "Nota"), ("title", "Malo")]),
            body: Some("# Malo\n\n<<<<<<< HEAD\nuno\n=======\ndos\n>>>>>>> rama\n".to_string()),
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

// ---------------------------------------------------------------------------
// E13-H04 — Copias de recuperación (backup de los originales).
//
// Firmas asumidas de E13-H04 (fase ROJA; el implementador debe respetarlas):
// - `Workspace::backup_originals(&self, txn_id: &str, affected: &[RelPath]) -> Result<RecoveryDir,
//   WorkspaceError>`: ANTES de sustituir el canónico, por cada `RelPath` de `affected`, si el `.md`
//   existe en el canónico copia su contenido **byte-a-byte** a
//   `.lodestar/runtime/recovery/<txnId>/<path>`; si NO existe (se va a crear), registra una marca
//   "no existía" (fichero/entrada, p. ej. un `.absent` o un manifiesto) para poder borrarlo al
//   revertir. Devuelve el `RecoveryDir` que referenciará el journal (E13-H03).
// - `RecoveryDir::path(&self) -> &std::path::Path`: raíz del directorio de recuperación de la
//   transacción, bajo `.lodestar/runtime/recovery/<txnId>/`.
// - `RecoveryDir::backup_path(&self, path: &RelPath) -> std::path::PathBuf`: ruta donde vive (o
//   viviría) la copia de recuperación de `path` bajo el directorio de la transacción. Los tests la
//   usan para comprobar existencia y para leer el contenido byte-a-byte del backup.
// - `RecoveryDir::was_absent(&self, path: &RelPath) -> bool`: `true` si `path` se marcó "no existía"
//   (no había original que copiar; se creará y habrá que borrarlo al revertir); `false` si tenía
//   original y se copió.
//
// El directorio de recuperación vive bajo `.lodestar/runtime/` (desechable, invariante #1), como el
// journal (H03) y el staging (H01), por lo que no viola «los `.md` son la única fuente de verdad».
// ---------------------------------------------------------------------------

/// **E13-H04** · Criterio `backup_originales`: dada una transacción que modifica `b.md` (existe) y
/// crea `c.md` (no existe), al preparar las copias existe el backup de `b.md` bajo
/// `.lodestar/runtime/recovery/<txnId>/` y hay una marca de que `c.md` "no existía" (para poder
/// borrarlo al revertir).
#[test]
fn backup_originales() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    // `b.md` EXISTE en el canónico con contenido conocido; `c.md` NO existe (se va a crear).
    let b = RelPath::new("b.md").unwrap();
    let c = RelPath::new("c.md").unwrap();
    std::fs::write(
        dir.path().join("b.md"),
        "---\ntype: Nota\ntitle: B\n---\n# B\n\ncuerpo previo\n",
    )
    .unwrap();
    assert!(
        !dir.path().join("c.md").exists(),
        "precondición: c.md no debe existir (se creará en la transacción)"
    );

    // Se preparan las copias de recuperación para los dos paths afectados.
    let recovery = ws
        .backup_originals("txn-h04-b-y-c", &[b.clone(), c.clone()])
        .expect("preparar las copias de recuperación de los paths afectados");

    // El directorio de recuperación vive bajo `.lodestar/runtime/recovery/<txnId>/`.
    let recovery_root: PathBuf = recovery.path().to_path_buf();
    assert!(
        recovery_root.starts_with(dir.path().join(".lodestar/runtime/recovery")),
        "la recuperación no vive bajo .lodestar/runtime/recovery: {}",
        recovery_root.display()
    );

    // El backup de `b.md` (que existía) está materializado en disco.
    let backup_b: PathBuf = recovery.backup_path(&b);
    assert!(
        backup_b.is_file(),
        "debe existir el backup del original b.md en {}",
        backup_b.display()
    );
    assert!(
        !recovery.was_absent(&b),
        "b.md existía: no debe marcarse como \"no existía\""
    );

    // `c.md` (que no existía) queda marcado "no existía" y SIN copia (no había original que copiar).
    assert!(
        recovery.was_absent(&c),
        "c.md no existía: debe marcarse \"no existía\" para poder borrarlo al revertir"
    );
    assert!(
        !recovery.backup_path(&c).is_file(),
        "c.md no tenía original: no debe existir una copia de recuperación para él"
    );
}

/// `FileMap` del conocimiento `.md` canónico (mismas claves relativas POSIX que usa el core),
/// reutilizando el recorrido de [`canonical_md`]. Es el `files` de entrada con el que el core prevé
/// el resultado del plan ([`plan::apply_normalized_ops`]) y la [`WorkspaceRevision`] resultante.
fn canonical_filemap(root: &Path) -> FileMap {
    canonical_md(root)
        .into_iter()
        .map(|(rel, content)| (RelPath::new(&rel.replace('\\', "/")).unwrap(), content))
        .collect()
}

/// **E13-H04** · Criterio `backup_fiel`: dado un path afectado con contenido X (con bytes UTF-8
/// multibyte no triviales), al hacer backup la copia de recuperación contiene X **byte-a-byte**.
#[test]
fn backup_fiel() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    // Contenido X con bytes no triviales: UTF-8 multibyte (acentos, símbolo €, kana) y saltos.
    let contenido_x: &[u8] =
        "---\ntype: Nota\ntitle: Böé\n---\n# Café ☕\n\ncuerpo con € y 日本語\n".as_bytes();
    let b = RelPath::new("b.md").unwrap();
    std::fs::write(dir.path().join("b.md"), contenido_x).unwrap();

    let recovery = ws
        .backup_originals("txn-h04-fiel", std::slice::from_ref(&b))
        .expect("preparar la copia de recuperación de b.md");

    // El backup contiene X byte-a-byte (lectura binaria y comparación exacta de bytes).
    let backup_b: PathBuf = recovery.backup_path(&b);
    let bytes_backup = std::fs::read(&backup_b).unwrap_or_else(|e| {
        panic!(
            "el backup de b.md debe existir y ser legible {}: {e}",
            backup_b.display()
        )
    });
    assert_eq!(
        bytes_backup, contenido_x,
        "el backup de b.md no es fiel byte-a-byte al original"
    );
}

// ---------------------------------------------------------------------------
// E13-H05 — Aplicación atómica por lote (único escritor).
//
// Firma asumida de E13-H05 (fase ROJA; el implementador debe respetarla):
// - `Workspace::publish(&self, change_set: &ChangeSet, journal: &mut Journal)
//     -> Result<WorkspaceRevision, WorkspaceError>`:
//   publica el resultado del `change_set` sobre el conocimiento canónico por el ÚNICO escritor.
//   Reusa `plan::apply_normalized_ops` sobre el `FileMap` canónico para obtener el `FileMap`
//   resultante y, en orden determinista, sustituye cada `.md` con `io::write_atomic` (temp + fsync
//   + rename) o lo borra con `io::delete` (paths que el resultado ya no contiene), actualizando el
//   `journal` tras cada operación (`Journal::mark_applied`). NO hay segundo escritor: el watcher
//   absorbe el lote auto-originado (gate blake3). Al terminar, deja el journal en estado `applied`
//   y devuelve la `resultWorkspaceRevision` calculada del conocimiento ya publicado — que debe
//   coincidir con la `result_rev` que el plan capturó y con la que se creó el journal (H03).
//
// El grep estructural del criterio ("la publicación usa `write_atomic`; ningún otro camino de
// escritura del canónico") es checklist de revisión, no un test de integración: se verifica leyendo
// `publish` en `src/`, no desde aquí.
// ---------------------------------------------------------------------------

/// **E13-H05** · Criterio `publica_lote`: dado un change set de 3 escrituras, al publicarlo los 3
/// `.md` CANÓNICOS (leídos de disco, no del staging) quedan con el contenido del staging (el
/// resultado que `plan::apply_normalized_ops` prevé, que es exactamente lo que
/// `materialize_staging` escribe).
#[test]
fn publica_lote() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    // Change set con 3 escrituras (create de 3 `.md` nuevos).
    let cs = change_set(
        "changeset:publica-tres",
        vec![
            create_conforme("uno.md", "Nota", "Uno"),
            create_conforme("dos.md", "Nota", "Dos"),
            create_conforme("tres.md", "Nota", "Tres"),
        ],
    );

    // El resultado que el plan prevé sobre el canónico (idéntico al contenido del staging que
    // `materialize_staging` materializaría): la referencia contra la que se compara el canónico.
    let canonico_antes = canonical_filemap(dir.path());
    let esperado = plan::apply_normalized_ops(&canonico_antes, &cs.operations)
        .expect("aplicar las ops normalizadas para prever el resultado del plan");
    // Los 3 `.md` del plan figuran en el resultado previsto (precondición del test, no vacuo).
    for f in ["uno.md", "dos.md", "tres.md"] {
        assert!(
            esperado.contains_key(&RelPath::new(f).unwrap()),
            "precondición: {f} debe estar en el resultado previsto del plan"
        );
    }

    // Journal de la transacción (H03) con la `resultWorkspaceRevision` que el plan prevé.
    let ops: Vec<RelPath> = cs
        .operations
        .iter()
        .map(|op| match op {
            NormalizedOperation::Create { path, .. } => path.clone(),
            _ => unreachable!("el change set de este test solo tiene `Create`"),
        })
        .collect();
    let base = ws.workspace_revision().unwrap();
    let result_rev = lodestar_core::types::workspace_revision(&esperado, &[]);
    let mut journal = ws
        .create_journal("txn-h05-publica-lote", &ops, &base, &result_rev)
        .expect("crear el journal de la transacción");

    // Publica el lote por el único escritor.
    ws.publish(&cs, &mut journal)
        .expect("publicar el change set sobre el canónico");

    // Los 3 `.md` CANÓNICOS (releídos de disco) quedan con el contenido del staging/plan.
    let canonico_despues = canonical_filemap(dir.path());
    for (rel, contenido) in &esperado {
        let en_disco = canonico_despues.get(rel).unwrap_or_else(|| {
            panic!(
                "tras publicar, el `.md` canónico {} debe existir en disco",
                rel.as_str()
            )
        });
        assert_eq!(
            en_disco,
            contenido,
            "el `.md` canónico {} no quedó con el contenido del staging tras publicar",
            rel.as_str()
        );
    }
    // Y el canónico es EXACTAMENTE el resultado previsto (ni ficheros de más ni de menos).
    assert_eq!(
        canonico_despues, esperado,
        "el conocimiento canónico publicado no coincide con el resultado del plan"
    );
}

/// **E13-H05** · Criterio `revision_resultante_coincide`: tras publicar, la `WorkspaceRevision`
/// calculada coincide con la `resultWorkspaceRevision` que el plan previó. El esperado se obtiene
/// aplicando el plan sobre el canónico (`plan::apply_normalized_ops`) y hasheando el resultado con
/// la misma lógica del core (`types::workspace_revision`, writableRoots por defecto = vacío en un
/// bundle recién abierto). Se comprueba tanto el valor devuelto por `publish` como el que
/// `Workspace::workspace_revision()` calcula del canónico ya publicado.
#[test]
fn revision_resultante_coincide() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    // Un concepto canónico previo, para que la publicación opere sobre una base no vacía.
    ws.create_concept(
        &RelPath::new("raiz.md").unwrap(),
        "Nota",
        Some("Raiz"),
        "# H\n\ncuerpo\n",
        false,
    )
    .unwrap();

    let cs = change_set(
        "changeset:revision-resultante",
        vec![
            create_conforme("uno.md", "Nota", "Uno"),
            create_conforme("dos.md", "Nota", "Dos"),
            create_conforme("tres.md", "Nota", "Tres"),
        ],
    );

    // `resultWorkspaceRevision` prevista por el plan: hash del resultado de aplicar las ops sobre
    // el canónico actual (writableRoots por defecto = vacío → cubre todos los `.md`).
    let canonico_antes = canonical_filemap(dir.path());
    let esperado = plan::apply_normalized_ops(&canonico_antes, &cs.operations)
        .expect("aplicar las ops normalizadas para prever el resultado del plan");
    let result_rev_prevista = lodestar_core::types::workspace_revision(&esperado, &[]);

    let ops: Vec<RelPath> = cs
        .operations
        .iter()
        .map(|op| match op {
            NormalizedOperation::Create { path, .. } => path.clone(),
            _ => unreachable!("el change set de este test solo tiene `Create`"),
        })
        .collect();
    let base = ws.workspace_revision().unwrap();
    let mut journal = ws
        .create_journal("txn-h05-revision", &ops, &base, &result_rev_prevista)
        .expect("crear el journal de la transacción");

    // `publish` devuelve la `resultWorkspaceRevision` calculada del conocimiento publicado.
    let devuelta = ws
        .publish(&cs, &mut journal)
        .expect("publicar el change set sobre el canónico");
    assert_eq!(
        devuelta, result_rev_prevista,
        "la revisión devuelta por publish no coincide con la resultWorkspaceRevision del plan"
    );

    // Y recalculada del canónico ya publicado, coincide igualmente con la prevista por el plan.
    let recalculada = ws
        .workspace_revision()
        .expect("recomputar la WorkspaceRevision del canónico publicado");
    assert_eq!(
        recalculada, result_rev_prevista,
        "la WorkspaceRevision del canónico publicado no coincide con la prevista por el plan"
    );
}

// ===========================================================================
// E13-H07 — `ChangeReceipt` + retención.
//
// Firmas asumidas de E13-H07 (fase ROJA; el implementador debe respetarlas):
// - `Workspace::write_receipt(&self, receipt: &ChangeReceipt) -> Result<(), WorkspaceError>`:
//   persiste el `ChangeReceipt` de una aplicación completada (`done`) como
//   `.lodestar/runtime/receipts/<receiptId>.json`. El wire es el de `ChangeReceipt`
//   (`serde(rename_all = "camelCase")`): `previousRevision`/`resultRevision` son strings
//   (`WorkspaceRevision` es `#[serde(transparent)]`).
// - `Workspace::gc_receipts(&self) -> Result<(), WorkspaceError>`: recolecta los recibos caducados
//   (`transactions.retainReceiptsFor`) o excedentes (`transactions.maximumReceipts`) según la config
//   del workspace (E9-H05, default `24h`/`20`), borrando además las copias de recuperación asociadas
//   (`.lodestar/runtime/recovery/<receiptId>/`).
// - `Workspace::load_receipt(&self, id: &ReceiptId) -> Result<ChangeReceipt, WorkspaceError>`:
//   (auxiliar, no ejercitada aquí) lee un receipt persistido por id.
//
// **Cómo decide el GC "el más antiguo"**: por el **mtime** del fichero `<receiptId>.json` — es el
// mismo reloj que gobierna la retención por edad (`retainReceiptsFor`), y `ChangeReceipt` no lleva
// timestamp propio (los recibos son runtime desechable, invariante #1). El test `receipt_gc` fija
// mtimes escalonados explícitos para que el orden por antigüedad sea determinista y no dependa de la
// resolución del reloj del sistema de ficheros.
// ---------------------------------------------------------------------------

/// Un `ChangeReceipt` mínimo con id y revisiones conocidas (los `changed_paths`/`semantic_diff` no
/// intervienen en la persistencia ni en el GC — van a un valor razonable/`Default`).
fn receipt(id: &str, previous: &str, result: &str) -> ChangeReceipt {
    ChangeReceipt {
        id: ReceiptId(id.to_string()),
        change_set_id: ChangeSetId(format!("changeset:{id}")),
        previous_revision: WorkspaceRevision(previous.to_string()),
        result_revision: WorkspaceRevision(result.to_string()),
        changed_paths: vec![RelPath::new("uno.md").unwrap()],
        semantic_diff: SemanticDiff::default(),
    }
}

/// Fija el mtime de `path` a `t` (abriendo el fichero con permiso de escritura). Sirve para
/// escalonar de forma determinista la "antigüedad" de los recibos en `receipt_gc`.
fn set_mtime(path: &Path, t: SystemTime) {
    let f = std::fs::OpenOptions::new()
        .write(true)
        .open(path)
        .unwrap_or_else(|e| panic!("abrir {} para fijar mtime: {e}", path.display()));
    f.set_modified(t)
        .unwrap_or_else(|e| panic!("fijar mtime de {}: {e}", path.display()));
}

/// Cuenta los ficheros `*.json` directamente bajo `dir` (los recibos persistidos).
fn contar_json(dir: &Path) -> usize {
    std::fs::read_dir(dir)
        .map(|rd| {
            rd.flatten()
                .filter(|e| e.path().extension().and_then(|x| x.to_str()) == Some("json"))
                .count()
        })
        .unwrap_or(0)
}

/// **E13-H07** · Criterio `receipt_persistido`: dado un apply completado, al terminar existe el
/// receipt en `.lodestar/runtime/receipts/<receiptId>.json` con `previousRevision` y `resultRevision`
/// correctos (leídos del disco, la fuente de verdad recuperable).
#[test]
fn receipt_persistido() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    let rc = receipt("receipt-persistido", "blake3:previa", "blake3:resultante");

    // Persistir el receipt de la aplicación completada.
    ws.write_receipt(&rc)
        .expect("persistir el receipt de una aplicación completada");

    // El receipt vive en `.lodestar/runtime/receipts/<receiptId>.json`.
    let receipt_path = dir
        .path()
        .join(".lodestar/runtime/receipts")
        .join("receipt-persistido.json");
    assert!(
        receipt_path.is_file(),
        "el receipt debe persistirse en {}",
        receipt_path.display()
    );

    // Releído del disco: sus revisiones (wire camelCase) coinciden con las conocidas.
    let raw = std::fs::read_to_string(&receipt_path).unwrap_or_else(|e| {
        panic!(
            "el receipt debe ser legible {}: {e}",
            receipt_path.display()
        )
    });
    let json: serde_json::Value = serde_json::from_str(&raw)
        .unwrap_or_else(|e| panic!("el receipt debe ser JSON bien formado: {e}"));
    assert_eq!(
        json["previousRevision"].as_str(),
        Some("blake3:previa"),
        "el receipt debe registrar la previousRevision correcta: {json}"
    );
    assert_eq!(
        json["resultRevision"].as_str(),
        Some("blake3:resultante"),
        "el receipt debe registrar la resultRevision correcta: {json}"
    );
}

/// **E13-H07** · Criterio `receipt_gc`: dados 21 recibos con `maximumReceipts:20`, al hacer GC queda
/// el más antiguo (`receipt-00`, por mtime) fuera —su receipt y su copia de recuperación borrados—
/// y persisten exactamente los 20 más recientes con sus copias intactas.
#[test]
fn receipt_gc() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    // Config explícita del criterio: retener como máximo 20 recibos (y 24h por edad, holgado).
    std::fs::write(
        dir.path().join(".lodestar/config.yaml"),
        "transactions:\n  retainReceiptsFor: \"24h\"\n  maximumReceipts: 20\n",
    )
    .unwrap();

    let receipts_dir = dir.path().join(".lodestar/runtime/receipts");
    let recovery_dir = dir.path().join(".lodestar/runtime/recovery");
    std::fs::create_dir_all(&recovery_dir).unwrap();

    // 21 recibos con mtimes ESCALONADOS: `receipt-00` el más antiguo … `receipt-20` el más nuevo,
    // cada uno con su copia de recuperación asociada en `recovery/<id>/` (convención: mismo id).
    let now = SystemTime::now();
    let ids: Vec<String> = (0..21).map(|i| format!("receipt-{i:02}")).collect();
    for (i, id) in ids.iter().enumerate() {
        let rc = receipt(id, "blake3:previa", "blake3:resultante");
        let path = receipts_dir.join(format!("{id}.json"));
        std::fs::write(&path, serde_json::to_vec(&rc).unwrap()).unwrap();

        let rec = recovery_dir.join(id);
        std::fs::create_dir_all(&rec).unwrap();
        std::fs::write(rec.join("uno.md"), b"backup").unwrap();

        // mtime: receipt-00 = hace 20 s (más antiguo) … receipt-20 = ahora (más nuevo). Todos MUY
        // dentro de las 24h de retención, de modo que SOLO el límite de cantidad (20) fuerza la purga.
        let t = now - Duration::from_secs((20 - i) as u64);
        set_mtime(&path, t);
    }

    // Precondición no vacua: 21 recibos antes del GC.
    assert_eq!(
        contar_json(&receipts_dir),
        21,
        "precondición: deben existir 21 recibos antes del GC"
    );

    // Recolectar los excedentes según `maximumReceipts:20`.
    ws.gc_receipts()
        .expect("recolectar los recibos que exceden maximumReceipts");

    // El más antiguo (`receipt-00`) queda fuera: su receipt y su copia de recuperación se borraron.
    assert!(
        !receipts_dir.join("receipt-00.json").exists(),
        "el receipt más antiguo debía purgarse por exceder maximumReceipts:20"
    );
    assert!(
        !recovery_dir.join("receipt-00").exists(),
        "la copia de recuperación del receipt purgado debía borrarse también"
    );

    // Quedan exactamente 20 recibos (los más recientes), con sus copias de recuperación intactas.
    assert_eq!(
        contar_json(&receipts_dir),
        20,
        "tras el GC deben quedar exactamente maximumReceipts=20 recibos"
    );
    for id in &ids[1..] {
        assert!(
            receipts_dir.join(format!("{id}.json")).exists(),
            "el receipt reciente {id} no debía purgarse"
        );
        assert!(
            recovery_dir.join(id).exists(),
            "la copia de recuperación del receipt reciente {id} no debía borrarse"
        );
    }
}

// ===========================================================================
// E13-H06 — Crash-recovery determinista (journal incompleto al abrir).
//
// Gateado tras la feature `test-failpoints`: `cargo test -p lodestar-workspace
// --features test-failpoints`. En el build por defecto (`cargo test --workspace`) este módulo NO se
// compila, de modo que la suite verde de H01–H05 no se ve afectada por los ROJOS de H06.
//
// ---------------------------------------------------------------------------
// API de `FailPoint` y punto de entrada transaccional ASUMIDOS (fase ROJA; el implementador de
// H06/H08 debe respetarlos):
//
// - **Sonda de fallo (`FailPoint`)** — taxonomía de puntos de caída de la publicación transaccional.
//   El *contrato de producción* que el implementador cableará es una sonda de test global
//   (thread-local) tras la feature `test-failpoints`, consultada por el orquestador transaccional
//   (`Workspace::apply_transaction`, E13-H08) en cada paso etiquetado para ABORTAR ahí y modelar un
//   crash a mitad. En ESTA fase ROJA (recuperación al abrir, H06) no dependemos de que ese seam de
//   producción exista todavía: `simular_caida` reproduce el mismo estado en disco COMPONIENDO las
//   primitivas ya construidas (H01 `materialize_staging` · H03 `create_journal` · H04
//   `backup_originals` · H05 renames + `mark_applied`/`mark_all_applied`) hasta el punto de fallo y
//   deteniéndose — deja exactamente lo que dejaría el crash real: journal no-`done` + renames
//   parciales + copias de recuperación + staging. El enum `FailPoint` de este fichero ES esa
//   taxonomía; el implementador la re-usará como etiquetas de sus `#[cfg(feature="test-failpoints")]`
//   en el orquestador.
//
// - **Punto de entrada de recuperación** — `Workspace::recover(&self) -> Result<(), WorkspaceError>`:
//   al reabrir un `Workspace` NUEVO sobre el mismo directorio, ejecuta la recuperación determinista
//   leyendo el/los journal(s) no-`done` de `.lodestar/runtime/journal/`. Por el estado GLOBAL del
//   journal: `applied` → COMPLETAR (canónico ya renombrado; limpiar staging/backup y sellar `done`);
//   `applying`/`prepared` → RESTAURAR (deshacer renames parciales desde los backups de H04; borrar
//   los creados que marca `.absent`). Es explícito (no un efecto colateral del constructor): mientras
//   la recuperación esté PENDIENTE, las escrituras se bloquean con `WORKSPACE_RECOVERY_REQUIRED`.
//   (Se asume que `Workspace::open` DETECTA el journal no-`done` y marca el workspace como
//   "recuperación pendiente"; `App::workspace_status().recovery.pendingTransaction` lo refleja,
//   E10-H08 — probado en la capa `App`, fuera de este crate.)
//
// - **Convención de id de transacción** — un MISMO id nombra el journal (`<id>.json`), el staging
//   (`staging/<id>/`) y las copias de recuperación (`recovery/<id>/`), de modo que `recover` localiza
//   staging y backups a partir del `txnId` del journal. Por eso los ids de aquí van SIN prefijo
//   `changeset:` (así `staging_dir_name`, `recovery_dir_name` y el stem del journal coinciden).
// ---------------------------------------------------------------------------

#[cfg(feature = "test-failpoints")]
mod recuperacion {
    use std::collections::BTreeSet;
    use std::path::{Path, PathBuf};

    use lodestar_core::plan;
    use lodestar_core::types::{ChangeSet, FileMap, NormalizedOperation, RelPath};
    use lodestar_workspace::Workspace;

    use super::{canonical_filemap, change_set, create_conforme};

    /// Punto de caída de la publicación transaccional (E13-H06). Describe HASTA DÓNDE progresa la
    /// transacción antes de "caer"; `simular_caida` compone las primitivas de H01/H03/H04/H05 hasta
    /// ese punto y se detiene, dejando en disco lo que dejaría un crash real (journal no-`done` +
    /// renames parciales + copias de recuperación + staging).
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum FailPoint {
        /// Journal `prepared`, aún sin copias de recuperación ni renames (0 renames).
        TrasJournalPrepared,
        /// Journal `prepared`, copias de recuperación listas, aún 0 renames.
        TrasBackup,
        /// Journal `applying`, **1** rename hecho (entre el rename 1 y el 2 de 3).
        EntreRenames,
        /// Journal `applying`, **2** renames hechos.
        TrasSegundoRename,
        /// Journal `applying`, **3** renames hechos, pero SIN `mark_all_applied` (borde: el journal
        /// nunca registró durablemente que el lote estaba completo → recuperación conservadora).
        TrasUltimoRenameSinApplied,
        /// Journal `applied`, 3 renames hechos, SIN sellar `done` (tras el último rename, antes de
        /// `done`).
        TrasAppliedSinDone,
        /// Journal `applied`, 3 renames hechos, antes de escribir el receipt (E13-H07).
        AntesDelReceipt,
    }

    /// Todos los puntos de caída, para el property test `recovery_sin_parciales`.
    pub const TODOS_LOS_FAILPOINTS: &[FailPoint] = &[
        FailPoint::TrasJournalPrepared,
        FailPoint::TrasBackup,
        FailPoint::EntreRenames,
        FailPoint::TrasSegundoRename,
        FailPoint::TrasUltimoRenameSinApplied,
        FailPoint::TrasAppliedSinDone,
        FailPoint::AntesDelReceipt,
    ];

    /// Desde este punto de caída, la recuperación determinista debe **restaurar** al estado original
    /// (`true`) o **completar** hasta el resultado (`false`). Decisión por el estado GLOBAL del
    /// journal: solo `applied` (fijado atómicamente por `mark_all_applied` TRAS el último rename)
    /// autoriza a completar; cualquier estado anterior (`prepared`/`applying`) restaura, incluido el
    /// borde en que los 3 renames ocurrieron pero el journal no llegó a `applied`.
    pub fn debe_restaurar(fp: FailPoint) -> bool {
        !matches!(
            fp,
            FailPoint::TrasAppliedSinDone | FailPoint::AntesDelReceipt
        )
    }

    /// Cuántos renames deja hechos el punto de caída (0..=3).
    fn renames_hechos(fp: FailPoint, total: usize) -> usize {
        match fp {
            FailPoint::TrasJournalPrepared | FailPoint::TrasBackup => 0,
            FailPoint::EntreRenames => 1,
            FailPoint::TrasSegundoRename => 2,
            _ => total,
        }
    }

    /// Abre un bundle con 3 conceptos canónicos conocidos (`uno/dos/tres.md`) y devuelve el
    /// workspace + el `FileMap` canónico ORIGINAL (el estado "antes de la transacción").
    fn bundle_con_tres(root: &Path) -> (Workspace, FileMap) {
        let ws = Workspace::open(root).unwrap();
        for (p, t) in [("uno.md", "Uno"), ("dos.md", "Dos"), ("tres.md", "Tres")] {
            ws.create_concept(
                &RelPath::new(p).unwrap(),
                "Nota",
                Some(t),
                &format!("# {t}\n\ncuerpo original\n"),
                false,
            )
            .unwrap();
        }
        let original = canonical_filemap(root);
        (ws, original)
    }

    /// Change set de 3 **modificaciones** (`ReplaceBody`) de los conceptos existentes.
    fn cs_modifica_tres(id: &str) -> ChangeSet {
        change_set(
            id,
            vec![
                NormalizedOperation::ReplaceBody {
                    path: RelPath::new("uno.md").unwrap(),
                    body: "# Uno\n\nCUERPO MODIFICADO uno\n".to_string(),
                },
                NormalizedOperation::ReplaceBody {
                    path: RelPath::new("dos.md").unwrap(),
                    body: "# Dos\n\nCUERPO MODIFICADO dos\n".to_string(),
                },
                NormalizedOperation::ReplaceBody {
                    path: RelPath::new("tres.md").unwrap(),
                    body: "# Tres\n\nCUERPO MODIFICADO tres\n".to_string(),
                },
            ],
        )
    }

    /// Change set de 3 **creaciones** de conceptos nuevos (`a/b/c.md`): ejercita la ruta de
    /// recuperación por `.absent` (borrar los creados al restaurar).
    fn cs_crea_tres(id: &str) -> ChangeSet {
        change_set(
            id,
            vec![
                create_conforme("a.md", "Nota", "A"),
                create_conforme("b.md", "Nota", "B"),
                create_conforme("c.md", "Nota", "C"),
            ],
        )
    }

    /// Conjunto de paths afectados por el plan, en el MISMO orden determinista que
    /// `Workspace::publish` (BTreeSet por `RelPath`): creados/modificados (el resultado difiere del
    /// canónico) + borrados (el canónico los tenía y el resultado ya no).
    fn afectados(original: &FileMap, result: &FileMap) -> Vec<RelPath> {
        let mut set: BTreeSet<RelPath> = BTreeSet::new();
        for (rel, content) in result {
            if original.get(rel) != Some(content) {
                set.insert(rel.clone());
            }
        }
        for rel in original.keys() {
            if !result.contains_key(rel) {
                set.insert(rel.clone());
            }
        }
        set.into_iter().collect()
    }

    /// Simula una **caída** de la publicación transaccional en el punto `fp`: compone las primitivas
    /// ya construidas (staging H01 → journal H03 → backup H04 → renames H05) hasta ese punto y se
    /// detiene, dejando en disco el journal no-`done`, los renames parciales, las copias de
    /// recuperación y el staging — tal cual los dejaría un crash real. Devuelve la ruta del
    /// directorio de staging (para comprobar que la recuperación lo limpia).
    ///
    /// El `id` nombra a la vez el change set (staging), el journal y las copias de recuperación
    /// (convención documentada: `recover` los localiza por el `txnId` del journal).
    fn simular_caida(
        ws: &Workspace,
        root: &Path,
        id: &str,
        cs: &ChangeSet,
        fp: FailPoint,
    ) -> PathBuf {
        let original = canonical_filemap(root);
        let result = plan::apply_normalized_ops(&original, &cs.operations)
            .expect("prever el resultado del plan");
        let affected = afectados(&original, &result);
        assert_eq!(
            affected.len(),
            3,
            "precondición del arnés: la transacción debe afectar a 3 paths (fp {fp:?})"
        );

        let base = ws.workspace_revision().unwrap();
        let result_rev = lodestar_core::types::workspace_revision(&result, &[]);

        // Paso 1 (H01): materializa el resultado en staging (aún sin tocar el canónico).
        let staging = ws
            .materialize_staging(cs)
            .expect("materializar el staging de la transacción");
        let staging_path = staging.path().to_path_buf();

        // Paso 2 (H03): write-ahead journal `prepared`.
        let mut journal = ws
            .create_journal(id, &affected, &base, &result_rev)
            .expect("crear el write-ahead journal");
        if fp == FailPoint::TrasJournalPrepared {
            return staging_path;
        }

        // Paso 3 (H04): copias de recuperación de los originales afectados.
        ws.backup_originals(id, &affected)
            .expect("preparar las copias de recuperación");
        if fp == FailPoint::TrasBackup {
            return staging_path;
        }

        // Paso 4 (H05): renames parciales, marcando el journal tras cada uno (igual que `publish`).
        let k = renames_hechos(fp, affected.len());
        for rel in affected.iter().take(k) {
            match result.get(rel) {
                Some(content) => std::fs::write(root.join(rel.as_str()), content).unwrap(),
                None => {
                    let _ = std::fs::remove_file(root.join(rel.as_str()));
                }
            }
            journal
                .mark_applied(rel)
                .expect("marcar el rename en el journal");
        }

        // Los puntos de caída que COMPLETAN sellaron `applied` (todos los renames + mark_all_applied)
        // antes de caer; los demás quedan en `applying`/`prepared`.
        if matches!(
            fp,
            FailPoint::TrasAppliedSinDone | FailPoint::AntesDelReceipt
        ) {
            journal
                .mark_all_applied()
                .expect("sellar el journal a `applied`");
        }

        // Se "cae": el journal NO llega a `done`; el handle se dropea aquí.
        staging_path
    }

    /// **E13-H06** · Criterio `recovery_restaura_desde_medio`: un fallo inyectado ENTRE el rename 1 y
    /// el 2 de 3 → al reabrir y recuperar, el estado queda COMO ANTES de la transacción (los 3
    /// originales), sin `.md` a medias.
    #[test]
    fn recovery_restaura_desde_medio() {
        let dir = tempfile::tempdir().unwrap();
        let (ws, original) = bundle_con_tres(dir.path());

        let cs = cs_modifica_tres("recovery-restaura-desde-medio");
        let result = plan::apply_normalized_ops(&original, &cs.operations).unwrap();
        // Precondición no vacua: original y resultado difieren (hay algo que restaurar).
        assert_ne!(original, result, "el plan debe cambiar el canónico");

        // Caída entre el rename 1 y el 2 (journal `applying`, 1 rename hecho).
        simular_caida(
            &ws,
            dir.path(),
            "recovery-restaura-desde-medio",
            &cs,
            FailPoint::EntreRenames,
        );
        drop(ws);

        // Se REABRE un workspace nuevo sobre el mismo directorio y se recupera.
        let ws2 = Workspace::open(dir.path()).unwrap();
        ws2.recover()
            .expect("la recuperación debe restaurar sin error");

        // El canónico quedó EXACTAMENTE como antes de la transacción (los 3 originales, byte-a-byte).
        let despues = canonical_filemap(dir.path());
        assert_eq!(
            despues, original,
            "tras un fallo a mitad, la recuperación debía restaurar el estado original íntegro"
        );
    }

    /// **E13-H06** · Criterio `recovery_completa`: un fallo inyectado TRAS el último rename pero ANTES
    /// de marcar `done` → al reabrir y recuperar, la transacción se COMPLETA (resultado final,
    /// staging limpio).
    #[test]
    fn recovery_completa() {
        let dir = tempfile::tempdir().unwrap();
        let (ws, original) = bundle_con_tres(dir.path());

        let cs = cs_modifica_tres("recovery-completa");
        let result = plan::apply_normalized_ops(&original, &cs.operations).unwrap();
        assert_ne!(original, result, "el plan debe cambiar el canónico");

        // Caída tras el último rename, con el journal ya en `applied` pero SIN sellar `done`.
        let staging_path = simular_caida(
            &ws,
            dir.path(),
            "recovery-completa",
            &cs,
            FailPoint::TrasAppliedSinDone,
        );
        drop(ws);

        let ws2 = Workspace::open(dir.path()).unwrap();
        ws2.recover()
            .expect("la recuperación debe completar sin error");

        // El canónico quedó con el RESULTADO final del plan.
        let despues = canonical_filemap(dir.path());
        assert_eq!(
            despues, result,
            "tras un fallo con el journal `applied`, la recuperación debía completar al resultado"
        );

        // Y el staging de la transacción quedó limpio (el directorio del txn ya no existe).
        assert!(
            !staging_path.exists(),
            "la recuperación al completar debía limpiar el staging: {}",
            staging_path.display()
        );
    }

    /// **E13-H06** · Criterio `recovery_bloquea_escritura`: con una recuperación PENDIENTE (journal
    /// no-`done` al reabrir, aún sin `recover`), una escritura → `WORKSPACE_RECOVERY_REQUIRED`.
    #[test]
    fn recovery_bloquea_escritura() {
        let dir = tempfile::tempdir().unwrap();
        let (ws, original) = bundle_con_tres(dir.path());

        let cs = cs_modifica_tres("recovery-bloquea-escritura");
        // Deja una transacción a medias (journal `applying`): la recuperación queda PENDIENTE.
        simular_caida(
            &ws,
            dir.path(),
            "recovery-bloquea-escritura",
            &cs,
            FailPoint::EntreRenames,
        );
        drop(ws);

        // Se reabre pero NO se llama a `recover`: la recuperación sigue pendiente.
        let ws2 = Workspace::open(dir.path()).unwrap();

        // Una escritura con recuperación pendiente debe rechazarse con WORKSPACE_RECOVERY_REQUIRED.
        match ws2.create_concept(
            &RelPath::new("nuevo.md").unwrap(),
            "Nota",
            Some("Nuevo"),
            "# Nuevo\n\ncuerpo\n",
            false,
        ) {
            Err(e) => assert_eq!(
                e.code(),
                "WORKSPACE_RECOVERY_REQUIRED",
                "una escritura con recuperación pendiente debe mapear a WORKSPACE_RECOVERY_REQUIRED: {e:?}"
            ),
            Ok(outcome) => panic!(
                "una escritura con recuperación pendiente debía fallar con \
                 WORKSPACE_RECOVERY_REQUIRED, pero create_concept escribió (written={})",
                outcome.written
            ),
        }

        // Y el canónico no se tocó por esa escritura bloqueada (`nuevo.md` no se creó).
        assert!(
            !dir.path().join("nuevo.md").exists(),
            "la escritura bloqueada no debía tocar el canónico"
        );
        // El bundle sigue conteniendo los 3 originales (no se perdió nada del canónico previo).
        let _ = original;
    }

    /// **E13-H06** · Criterio `recovery_sin_parciales`: property test sobre TODOS los `FailPoint` (×
    /// dos formas de change set: modificaciones y creaciones). Para cada combinación: se simula la
    /// caída, se reabre, se recupera, y se asevera que NINGÚN `.md` canónico queda en estado parcial
    /// — el conocimiento converge de forma determinista a UNO de los dos bordes de la transacción
    /// (todo el original íntegro, o todo el resultado íntegro), nunca una mezcla.
    #[test]
    fn recovery_sin_parciales() {
        // Cada forma de change set se fabrica desde el bundle base (3 conceptos existentes).
        type FormaCs = (&'static str, fn(&str) -> ChangeSet);
        let formas: &[FormaCs] = &[("modifica", cs_modifica_tres), ("crea", cs_crea_tres)];

        for (forma, build_cs) in formas {
            for &fp in TODOS_LOS_FAILPOINTS {
                let dir = tempfile::tempdir().unwrap();
                let (ws, original) = bundle_con_tres(dir.path());

                let id = format!("recovery-sin-parciales-{forma}-{fp:?}");
                let cs = build_cs(&id);
                let result = plan::apply_normalized_ops(&original, &cs.operations).unwrap();
                assert_ne!(
                    original, result,
                    "[{forma}/{fp:?}] el plan debe cambiar el canónico (test no vacuo)"
                );

                simular_caida(&ws, dir.path(), &id, &cs, fp);
                drop(ws);

                let ws2 = Workspace::open(dir.path()).unwrap();
                ws2.recover()
                    .unwrap_or_else(|e| panic!("[{forma}/{fp:?}] la recuperación falló: {e:?}"));

                let despues = canonical_filemap(dir.path());

                // (1) Convergencia determinista: el conjunto canónico ES o bien el original íntegro
                //     o bien el resultado íntegro — nunca un estado intermedio.
                let esperado = if debe_restaurar(fp) {
                    &original
                } else {
                    &result
                };
                assert_eq!(
                    &despues, esperado,
                    "[{forma}/{fp:?}] la recuperación no convergió al borde determinista esperado"
                );

                // (2) Ningún fichero con contenido parcial: cada `.md` canónico es byte-a-byte O su
                //     original íntegro O su resultado íntegro (jamás truncado/mezclado/foráneo).
                for (rel, contenido) in &despues {
                    let es_original = original.get(rel) == Some(contenido);
                    let es_resultado = result.get(rel) == Some(contenido);
                    assert!(
                        es_original || es_resultado,
                        "[{forma}/{fp:?}] el `.md` {} quedó con contenido parcial/foráneo",
                        rel.as_str()
                    );
                }
            }
        }
    }
}
