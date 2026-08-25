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
//!   construye un `DocumentSet` desde el árbol de staging (canónico + staging), corre `analyze` y, si el
//!   resultado no cumple la política (gate estricto: `hard_fail > 0`), aborta SIN tocar el canónico
//!   y limpia el staging. El `Err` mapea al wire `INVALID_RESULT` (`WorkspaceError::code()`).
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

/// Un `Create` válido que resuelve al `.md` `path`, con `type`/`title` como frontmatter
/// **arbitrario** (E23-H02: el motor no privilegia ninguna clave; aquí son solo dos claves más).
fn create_conforme(path: &str, ty: &str, title: &str) -> NormalizedOperation {
    NormalizedOperation::Create {
        path: RelPath::new(path).unwrap(),
        frontmatter: Some(patch(&[("type", ty), ("title", title)])),
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
                let rel = lodestar_workspace::discovery::rel_path_from(
                    path.strip_prefix(root).unwrap_or_else(|e| {
                        panic!(
                            "el documento {} debe estar bajo la raíz {}: {e}",
                            path.display(),
                            root.display()
                        )
                    }),
                )
                .unwrap_or_else(|check| {
                    panic!(
                        "la ruta canónica {} debe ser representable como RelPath: {check:?}",
                        path.display()
                    )
                })
                .as_str()
                .to_owned();
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

    // Documento canónico previo, para comprobar que la materialización no lo altera.
    ws.create_document(
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
/// validarlo aborta con `INVALID_RESULT` y limpia el staging.
#[test]
fn staging_no_conforme_aborta() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    let antes = canonical_md(dir.path());

    // Change set cuyo resultado es NO conforme. MIGRADO en E16-H05: era un `Create` con `type`
    // vacío (`OKF-TYPE`), retirado del catálogo; hoy es un cuerpo con marcadores de merge sin
    // resolver (`DOC-CONFLICT-MARKER`), mismo motivo por el que rechaza
    // `create_document_no_conforme_no_escribe`.
    let cs = change_set(
        "changeset:no-conforme",
        vec![NormalizedOperation::Create {
            path: RelPath::new("malo.md").unwrap(),
            frontmatter: Some(patch(&[("type", "Nota"), ("title", "Malo")])),
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
        "INVALID_RESULT",
        "el error de validación no mapea a INVALID_RESULT: {err:?}"
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
    ws.create_document(
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

    // El workspace cambia ENTRE plan y apply: otro escritor introduce un documento.
    ws.create_document(
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
/// workspace recién abierto). Se comprueba tanto el valor devuelto por `publish` como el que
/// `Workspace::workspace_revision()` calcula del canónico ya publicado.
#[test]
fn revision_resultante_coincide() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    // Un documento canónico previo, para que la publicación opere sobre una base no vacía.
    ws.create_document(
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

/// Escribe `<root>/.lodestar/config.yaml` con la sección `transactions` dada, creando el directorio
/// si hace falta.
///
/// El `create_dir_all` no es ceremonia: desde **E23-H12** abrir un workspace no monta ningún
/// scaffold (leer un proyecto ajeno no puede modificarlo), así que el directorio lo crea **quien
/// escribe** — y quien escribe la config del test es el test.
fn escribe_config_transacciones(root: &Path, transactions_yaml: &str) {
    std::fs::create_dir_all(root.join(".lodestar")).unwrap();
    std::fs::write(
        root.join(".lodestar/config.yaml"),
        format!("transactions:\n{transactions_yaml}"),
    )
    .unwrap();
}

/// **E13-H07** · Criterio `receipt_gc`: dados 21 recibos con `maximumReceipts:20`, al hacer GC queda
/// el más antiguo (`receipt-00`, por mtime) fuera —su receipt y su copia de recuperación borrados—
/// y persisten exactamente los 20 más recientes con sus copias intactas.
///
/// ## Dos arreglos de E23-H12 (el test se apoyaba en un efecto secundario y en una config muerta)
///
/// 1. **Los `create_dir_all` que le faltaban.** Plantaba ficheros bajo `.lodestar/` sin crear
///    `.lodestar/` ni `runtime/receipts/`, y le funcionaba porque `Workspace::open` le montaba el
///    scaffold de runtime al abrir. E23-H12 retira ese efecto (abrir no modifica el proyecto) y el
///    directorio pasa a crearlo quien escribe: en producción `write_receipt`, aquí el test —que
///    planta los recibos a mano justamente para controlar sus mtimes.
/// 2. **La config se escribe ANTES de abrir.** `WorkspaceConfig` se lee **una sola vez, al abrir**
///    (`ARCHITECTURE.md §20.5`), así que el `config.yaml` que este test escribía DESPUÉS de
///    `Workspace::open` no llegaba nunca al GC: se estaba juzgando con los defaults —que casualmente
///    son los mismos, `24h`/`20`— y el test aseveraba sobre una config que jamás se aplicó. Ahora la
///    precondición comprueba explícitamente que la sesión la leyó.
///
/// ## Fase 2 — retención por EDAD, y la prueba de que manda la config
///
/// Con la fase 1 sola, `maximumReceipts:20` es indistinguible del default: el criterio lo cumpliría
/// igual un `gc_receipts` que ignorase la config entera. La fase 2 cierra dos huecos de un tiro: una
/// config **no-default** (`retainReceiptsFor: "1h"`) que solo puede purgar si de verdad se lee, y la
/// retención por edad del alcance de E13-H07, que hasta ahora no ejercitaba ningún test (los mtimes
/// de la fase 1 están todos dentro de las 24h a propósito, para que solo muerda el límite de
/// cantidad).
///
/// Las edades de la fase 2 son **bimodales** (2 h vs. ahora) contra un TTL de 1 h: el veredicto
/// queda a una hora de cualquier jitter del reloj, así que no puede volverse flaky. Los mtimes
/// escalonados por segundos de la fase 1 solo ORDENAN, no deciden caducidad.
#[test]
fn receipt_gc() {
    let dir = tempfile::tempdir().unwrap();

    // Config explícita del criterio (retener como máximo 20 recibos, 24h por edad = holgado),
    // escrita ANTES de abrir: es lo único que la hace efectiva.
    escribe_config_transacciones(
        dir.path(),
        "  retainReceiptsFor: \"24h\"\n  maximumReceipts: 20\n",
    );
    let ws = Workspace::open(dir.path()).unwrap();
    assert_eq!(
        ws.config().transactions.maximum_receipts,
        20,
        "precondición: la sesión tiene que haber LEÍDO la config declarada (se lee una sola vez, al \
         abrir); si no, el GC estaría juzgando con defaults y el criterio sería vacuo"
    );

    let receipts_dir = dir.path().join(".lodestar/runtime/receipts");
    let recovery_dir = dir.path().join(".lodestar/runtime/recovery");
    // El runtime lo crea QUIEN ESCRIBE (E23-H12: la apertura ya no monta scaffold). Aquí los recibos
    // los planta el test, así que los directorios son suyos.
    std::fs::create_dir_all(&receipts_dir).unwrap();
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

    // === Fase 2 — retención por EDAD (`retainReceiptsFor`), con una config NO-default ===========
    // Sesión NUEVA: la config se lee al abrir, así que reescribirla exige reabrir para que aplique.
    escribe_config_transacciones(
        dir.path(),
        "  retainReceiptsFor: \"1h\"\n  maximumReceipts: 20\n",
    );
    let ws = Workspace::open(dir.path()).unwrap();
    assert_eq!(
        ws.config().transactions.retain_receipts_for,
        "1h",
        "precondición: la sesión nueva debe haber leído la retención declarada (con el default de \
         24h no se purgaría nada y la fase 2 sería vacua)"
    );

    // De los 20 supervivientes, los 5 más antiguos pasan a tener 2 h: CADUCADOS con el TTL de 1 h
    // declarado, y VIGENTES con el default de 24h — que es justo lo que hace discriminante esta
    // fase. El resto se re-sella a «ahora». Siguen siendo 20, así que `maximumReceipts:20` no purga
    // a nadie: lo único que puede mover ficheros aquí es la edad.
    let supervivientes = &ids[1..];
    let (caducados, vigentes) = supervivientes.split_at(5);
    for id in caducados {
        set_mtime(
            &receipts_dir.join(format!("{id}.json")),
            now - Duration::from_secs(2 * 3600),
        );
    }
    for id in vigentes {
        set_mtime(&receipts_dir.join(format!("{id}.json")), now);
    }

    ws.gc_receipts()
        .expect("recolectar los recibos caducados por retainReceiptsFor");

    for id in caducados {
        assert!(
            !receipts_dir.join(format!("{id}.json")).exists(),
            "el receipt {id} (2 h de antigüedad) debía purgarse con retainReceiptsFor:1h; si sigue \
             ahí, el GC está usando el default de 24h en vez de la config del workspace"
        );
        assert!(
            !recovery_dir.join(id).exists(),
            "la copia de recuperación del receipt caducado {id} debía borrarse con él"
        );
    }
    for id in vigentes {
        assert!(
            receipts_dir.join(format!("{id}.json")).exists(),
            "el receipt {id} está dentro de la retención: no debía purgarse"
        );
        assert!(
            recovery_dir.join(id).exists(),
            "la copia de recuperación del receipt vigente {id} no debía borrarse"
        );
    }
    assert_eq!(
        contar_json(&receipts_dir),
        vigentes.len(),
        "tras el GC por edad deben quedar exactamente los recibos dentro de la retención"
    );
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

    /// Abre un workspace con 3 documentos canónicos conocidos (`uno/dos/tres.md`) y devuelve el
    /// workspace + el `FileMap` canónico ORIGINAL (el estado "antes de la transacción").
    fn workspace_con_tres(root: &Path) -> (Workspace, FileMap) {
        let ws = Workspace::open(root).unwrap();
        for (p, t) in [("uno.md", "Uno"), ("dos.md", "Dos"), ("tres.md", "Tres")] {
            ws.create_document(
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

    /// Change set de 3 **modificaciones** (`ReplaceBody`) de los documentos existentes.
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

    /// Change set de 3 **creaciones** de documentos nuevos (`a/b/c.md`): ejercita la ruta de
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
        let (ws, original) = workspace_con_tres(dir.path());

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
        let (ws, original) = workspace_con_tres(dir.path());

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
        let (ws, original) = workspace_con_tres(dir.path());

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
        match ws2.create_document(
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
                 WORKSPACE_RECOVERY_REQUIRED, pero create_document escribió (written={})",
                outcome.written
            ),
        }

        // Y el canónico no se tocó por esa escritura bloqueada (`nuevo.md` no se creó).
        assert!(
            !dir.path().join("nuevo.md").exists(),
            "la escritura bloqueada no debía tocar el canónico"
        );
        // El workspace sigue conteniendo los 3 originales (no se perdió nada del canónico previo).
        let _ = original;
    }

    /// **E13-H06** · Criterio `recovery_sin_parciales`: property test sobre TODOS los `FailPoint` (×
    /// dos formas de change set: modificaciones y creaciones). Para cada combinación: se simula la
    /// caída, se reabre, se recupera, y se asevera que NINGÚN `.md` canónico queda en estado parcial
    /// — el conocimiento converge de forma determinista a UNO de los dos bordes de la transacción
    /// (todo el original íntegro, o todo el resultado íntegro), nunca una mezcla.
    #[test]
    fn recovery_sin_parciales() {
        // Cada forma de change set se fabrica desde el workspace base (3 documentos existentes).
        type FormaCs = (&'static str, fn(&str) -> ChangeSet);
        let formas: &[FormaCs] = &[("modifica", cs_modifica_tres), ("crea", cs_crea_tres)];

        for (forma, build_cs) in formas {
            for &fp in TODOS_LOS_FAILPOINTS {
                let dir = tempfile::tempdir().unwrap();
                let (ws, original) = workspace_con_tres(dir.path());

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

// ---------------------------------------------------------------------------
// E24-H05 — Una transacción FALLIDA no deja staging
//
// `StagingDir` no tenía `Drop`, y los pasos (7)–(10) de `apply_transaction` salen por `?`
// saltándose el `remove_dir_all` del paso (11). Cualquier transacción que fallara el control
// optimista, el guard de escritura, el journal o la publicación dejaba en disco el árbol `.md`
// COMPLETO de su resultado, sin que nada volviera a recogerlo. El caso que lo destapó —el
// WRITE_CONFLICT sistemático tras un crash (E24-H03)— dejaba 121 ficheros por intento, y el GC
// nunca los veía porque solo itera `receipts/` y solo corre en el camino de éxito.
// ---------------------------------------------------------------------------

/// Ficheros bajo `.lodestar/runtime/staging/`, o vacío si el directorio no existe.
fn stagings_en_disco(root: &Path) -> Vec<String> {
    let base = root.join(".lodestar").join("runtime").join("staging");
    let Ok(entries) = std::fs::read_dir(&base) else {
        return Vec::new();
    };
    let mut out: Vec<String> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.file_name().to_string_lossy().to_string())
        .collect();
    out.sort();
    out
}

/// **E24-H05** — una transacción que falla el control optimista no deja su staging en disco.
#[test]
fn staging_no_sobrevive_a_una_transaccion_fallida() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    ws.create_document(
        &RelPath::new("base.md").unwrap(),
        "Nota",
        Some("Base"),
        "# H\n\ncuerpo\n",
        false,
    )
    .unwrap();

    // Un change set cuya `base_revision` es de un estado que ya no existe: el paso (7)
    // `reverify_base_revision` lo rechazará DESPUÉS de materializar el staging (paso 6).
    let mut cs = change_set(
        "e24-h05-fallida",
        vec![create_conforme("nuevo.md", "Nota", "Nuevo")],
    );
    cs.base_revision = WorkspaceRevision("blake3:0000000000000000".to_string());

    let err = ws
        .apply_transaction(&cs)
        .expect_err("la base del change set no es la actual: la transacción debe abortar");
    assert_eq!(
        err.code(),
        "WRITE_CONFLICT",
        "precondición del test: el fallo debe ser el del control optimista (paso 7), que ocurre \
         DESPUÉS de materializar el staging (paso 6). Si fallara antes, el test sería vacuo: {err:?}"
    );

    assert!(
        stagings_en_disco(dir.path()).is_empty(),
        "una transacción fallida no puede dejar su árbol de staging en disco: {:?}",
        stagings_en_disco(dir.path())
    );
    assert!(
        !dir.path().join("nuevo.md").exists(),
        "y desde luego no puede haber publicado nada en el canónico"
    );
}

/// **E24-H05** — control anti-vacuo: el camino feliz también limpia, y publica lo que debe.
///
/// Sin esto, un `Drop` que borrase el staging demasiado pronto (antes de publicar) pasaría el test
/// de arriba y rompería la publicación entera.
#[test]
fn staging_se_limpia_tambien_en_el_camino_feliz() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    ws.create_document(
        &RelPath::new("base.md").unwrap(),
        "Nota",
        Some("Base"),
        "# H\n\ncuerpo\n",
        false,
    )
    .unwrap();

    let base = ws.workspace_revision().unwrap();
    let mut cs = change_set(
        "e24-h05-feliz",
        vec![create_conforme("nuevo.md", "Nota", "Nuevo")],
    );
    cs.base_revision = base;

    ws.apply_transaction(&cs)
        .expect("la transacción debe publicarse");

    assert!(
        dir.path().join("nuevo.md").exists(),
        "el camino feliz debe publicar de verdad: si el `Drop` limpiase el staging antes de \
         publicar, esto fallaría"
    );
    assert!(
        stagings_en_disco(dir.path()).is_empty(),
        "tras publicar tampoco puede quedar staging: {:?}",
        stagings_en_disco(dir.path())
    );
}

// ---------------------------------------------------------------------------
// E24-H06 — El GC recoge huérfanos del plano de control
//
// `gc_receipts` iteraba SOLO `receipts/`, así que un `staging/<txn>/` cuya transacción nunca llegó
// a producir recibo le era invisible: no hay entrada con ese stem. Y solo se disparaba desde el
// camino de éxito de `change_apply`/`change_revert`, o sea que el flujo que produce la basura era
// justo el que no la recogía.
//
// El barrido va al revés: recorre `staging/` y `recovery/` y purga lo que no tiene ni journal vivo
// (transacción en curso o pendiente de recuperar) ni recibo vigente (revertible).
// ---------------------------------------------------------------------------

/// Crea un directorio con un fichero dentro, bajo `.lodestar/runtime/<sub>/<nombre>/`.
fn siembra_runtime_dir(root: &Path, sub: &str, nombre: &str) {
    let d = root
        .join(".lodestar")
        .join("runtime")
        .join(sub)
        .join(nombre);
    std::fs::create_dir_all(&d).unwrap();
    std::fs::write(d.join("resto.md"), "# resto\n").unwrap();
}

fn existe_runtime_dir(root: &Path, sub: &str, nombre: &str) -> bool {
    root.join(".lodestar")
        .join("runtime")
        .join(sub)
        .join(nombre)
        .exists()
}

/// **E24-H06** — un staging y un recovery sin journal ni recibo se purgan.
#[test]
fn gc_purga_huerfanos_sin_recibo() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    ws.create_document(
        &RelPath::new("a.md").unwrap(),
        "Nota",
        Some("A"),
        "# A\n\ncuerpo\n",
        false,
    )
    .unwrap();

    siembra_runtime_dir(dir.path(), "staging", "txn-huerfano");
    siembra_runtime_dir(dir.path(), "recovery", "txn-huerfano");

    ws.gc_receipts().expect("el GC debe correr");

    assert!(
        !existe_runtime_dir(dir.path(), "staging", "txn-huerfano"),
        "un `staging/<txn>/` sin journal ni recibo es basura y el GC debe recogerlo"
    );
    assert!(
        !existe_runtime_dir(dir.path(), "recovery", "txn-huerfano"),
        "lo mismo para su `recovery/<txn>/`"
    );
}

/// **E24-H06** — control anti-vacuo: una transacción VIVA (con journal) no se toca.
///
/// Sin esto, un barrido que borrase todo pasaría el test de arriba y destruiría exactamente los
/// datos que la recuperación necesita.
#[test]
fn gc_no_toca_transacciones_vivas() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    ws.create_document(
        &RelPath::new("a.md").unwrap(),
        "Nota",
        Some("A"),
        "# A\n\ncuerpo\n",
        false,
    )
    .unwrap();

    // Transacción EN CURSO: journal `prepared` + su staging y sus copias.
    let a = RelPath::new("a.md").unwrap();
    let base = ws.workspace_revision().unwrap();
    ws.backup_originals("txn-viva", std::slice::from_ref(&a))
        .expect("copias de recuperación");
    let _journal = ws
        .create_journal("txn-viva", &[a], &base, &base)
        .expect("journal");
    siembra_runtime_dir(dir.path(), "staging", "txn-viva");

    ws.gc_receipts().expect("el GC debe correr");

    assert!(
        existe_runtime_dir(dir.path(), "staging", "txn-viva"),
        "una transacción con journal vivo está a medio publicar o a medio recuperar: su staging es \
         justo lo que la recuperación necesita, el GC NO puede tocarlo"
    );
    assert!(
        existe_runtime_dir(dir.path(), "recovery", "txn-viva"),
        "ni sus copias de recuperación"
    );
}

/// **E24-H06** — los temporales `*.lodestar-tmp` abandonados por un crash se recogen.
#[test]
fn gc_purga_temporales_huerfanos() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();
    let journal_dir = dir.path().join(".lodestar").join("runtime").join("journal");
    std::fs::create_dir_all(&journal_dir).unwrap();
    let tmp = journal_dir.join("txn.json.12345-0.lodestar-tmp");
    std::fs::write(&tmp, "{}").unwrap();

    ws.gc_receipts().expect("el GC debe correr");

    assert!(
        !tmp.exists(),
        "un temporal de la escritura atómica abandonado por un crash entre el `create` y el \
         `rename` no rompe nada, pero se acumula sin límite: el GC debe recogerlo"
    );
}

// ---------------------------------------------------------------------------
// E24-H13 — Seam REAL de failpoints: la caída se inyecta DENTRO de `apply_transaction`
//
// Los cuatro tests de E13-H06 componen el estado post-crash a mano con `simular_caida`, y en un
// orden que NO es el del orquestador (journal antes que backup, cuando producción hace backup antes
// que journal). Consecuencia medida: `FailPoint::TrasJournalPrepared` de aquella taxonomía describe
// un estado que el código real no puede producir, y pasa vacuamente porque sin directorio de
// recuperación `restore_from_recovery` devuelve Ok() de inmediato.
//
// Estos tests recorren el orquestador de verdad, así que la taxonomía ya no puede divergir del
// orden de producción: si alguien reordena los pasos, cambia el comportamiento observado aquí.
// ---------------------------------------------------------------------------

#[cfg(feature = "test-failpoints")]
mod seam_real {
    use super::*;
    use lodestar_workspace::failpoints::{self, FailPoint};

    /// Workspace con tres documentos y un change set que los toca todos.
    fn tres_documentos() -> (tempfile::TempDir, Workspace, BTreeMap<String, String>) {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        for n in ["uno", "dos", "tres"] {
            ws.create_document(
                &RelPath::new(&format!("{n}.md")).unwrap(),
                "Nota",
                Some(n),
                &format!("# {n}\n\ncuerpo original\n"),
                false,
            )
            .unwrap();
        }
        let original = canonical_md(dir.path());
        (dir, ws, original)
    }

    /// Un change set que modifica los tres documentos.
    fn cs_modifica_los_tres(ws: &Workspace, id: &str) -> ChangeSet {
        let mut cs = change_set(
            id,
            ["uno", "dos", "tres"]
                .iter()
                .map(|n| NormalizedOperation::ReplaceBody {
                    path: RelPath::new(&format!("{n}.md")).unwrap(),
                    body: format!("# {n}\n\ncuerpo NUEVO\n"),
                })
                .collect(),
        );
        cs.base_revision = ws.workspace_revision().unwrap();
        cs
    }

    /// **E24-H13** — property test sobre el orquestador REAL: desde cualquier punto de caída, tras
    /// recuperar el canónico converge a uno de los dos bordes. Nunca a un estado parcial.
    #[test]
    fn recovery_sin_parciales_por_el_orquestador_real() {
        let puntos = [
            FailPoint::AlEntrar,
            FailPoint::TrasBackupSinJournal,
            FailPoint::TrasJournalPrepared,
            FailPoint::EntreRenames,
            FailPoint::TrasPublicarSinSellar,
            FailPoint::AntesDeSellar,
        ];

        for (i, fp) in puntos.iter().enumerate() {
            let (dir, ws, original) = tres_documentos();
            let cs = cs_modifica_los_tres(&ws, &format!("e24-h13-{i}"));
            // `ReplaceBody` CONSERVA el frontmatter (E23-H03), así que el borde «resultado» es la
            // CABECERA original —byte a byte, incluido su separador (E31-H02, `decisiones §26`)—
            // seguida del cuerpo que pide la operación, que es `"# {n}\n\ncuerpo NUEVO\n"` SIN
            // salto inicial. Hasta v0.5.0 el esperado se construía sustituyendo texto sobre el
            // original, y casaba porque la reconstrucción inyectaba un `\n` tras el `---` de
            // cierre; con la cabecera preservada el motor escribe exactamente lo que se le pidió,
            // así que el borde se compone igual que lo compone el motor.
            let resultado_esperado: BTreeMap<String, String> = original
                .iter()
                .map(|(k, v)| {
                    let n = k.trim_end_matches(".md");
                    let corte = lodestar_core::model::split_front(v).body_offset(v);
                    (k.clone(), format!("{}# {n}\n\ncuerpo NUEVO\n", &v[..corte]))
                })
                .collect();

            failpoints::armar(*fp);
            let r = ws.apply_transaction(&cs);
            failpoints::desarmar();
            assert!(
                r.is_err(),
                "el failpoint {fp:?} debe abortar la transacción (si no, el test es vacuo)"
            );

            // Se «reabre» el workspace, como haría el proceso siguiente, y se recupera.
            drop(ws);
            let ws2 = Workspace::open(dir.path()).unwrap();
            if ws2.recovery_pending() {
                ws2.recover().expect("la recuperación debe completarse");
            }

            let final_ = canonical_md(dir.path());
            let es_original = final_ == original;
            let es_resultado = final_ == resultado_esperado;
            assert!(
                es_original || es_resultado,
                "desde {fp:?}, el canónico debe converger a UNO de los dos bordes de la \
                 transacción, jamás a un estado parcial.\nfinal:     {final_:?}\noriginal:  \
                 {original:?}\nresultado: {resultado_esperado:?}"
            );
            assert!(
                !ws2.recovery_pending(),
                "tras recuperar desde {fp:?} no puede quedar recuperación pendiente"
            );
        }
    }

    /// **E24-H13** — el estado que la simulación de E13-H06 NO modelaba: copias escritas, journal
    /// aún ausente.
    ///
    /// Producción hace backup ANTES que journal; la simulación lo hacía al revés, así que este
    /// estado —el único que el código real produce entre esos dos pasos— no lo cubría nadie. Al
    /// reabrir, `recovery_pending()` es `false` (solo mira journals): hay que comprobar que el
    /// canónico está intacto, porque ningún rename ha ocurrido todavía.
    ///
    /// **E25-H03 — por qué el GC puede barrer aquí**: la razón es de **PROPIEDAD**, no de ausencia de
    /// journal. La transacción de este escenario **terminó** (el failpoint la abortó y su lock se
    /// soltó por RAII), así que su material ya no tiene dueño y es basura. La ausencia de journal por
    /// sí sola NO autoriza a barrer: entre `backup_originals` y `create_journal` una transacción
    /// **viva** —de este proceso o de otro— está exactamente en este mismo estado durable, y
    /// destruirle las copias la deja publicando sin plano de recuperación. Esa dimensión, la de
    /// vida, la fijan `gc_y_transacciones_vivas::gc_no_destruye_una_transaccion_en_curso_de_otro_proceso`
    /// (el material de una transacción EN CURSO sobrevive al GC de otro handle) y
    /// `gc_y_transacciones_vivas::la_marca_no_sobrevive_a_la_transaccion` (la señal de propiedad muere
    /// con la transacción, también cuando esta acaba en `Err` — que es lo que mantiene verde el
    /// barrido de aquí abajo sin tocar ni una aserción de este test).
    #[test]
    fn caida_entre_backup_y_journal() {
        let (dir, ws, original) = tres_documentos();
        let cs = cs_modifica_los_tres(&ws, "e24-h13-backup-sin-journal");

        failpoints::armar(FailPoint::TrasBackupSinJournal);
        let r = ws.apply_transaction(&cs);
        failpoints::desarmar();
        assert!(r.is_err(), "el failpoint debe abortar");

        drop(ws);
        let ws2 = Workspace::open(dir.path()).unwrap();
        assert!(
            !ws2.recovery_pending(),
            "sin journal no hay recuperación pendiente que detectar: `pending_journals` solo mira \
             journals. Es justamente por eso que el canónico tiene que estar ya intacto"
        );
        assert_eq!(
            canonical_md(dir.path()),
            original,
            "la caída ocurrió ANTES del primer rename, así que el canónico no puede haberse \
             movido ni un byte"
        );

        // El árbol de recuperación queda SIN DUEÑO —la transacción abortó y soltó su lock, así que
        // nadie va a publicar con esas copias— y por eso lo recoge el GC (E25-H03: el criterio es de
        // propiedad; que no haya journal ni recibo describe el estado, no lo autoriza a barrer — una
        // transacción viva en la ventana `[backup, journal)` presenta ese mismo estado y su material
        // es intocable).
        ws2.gc_receipts().expect("el GC debe correr");
        let recovery = dir
            .path()
            .join(".lodestar")
            .join("runtime")
            .join("recovery");
        let huerfanos = std::fs::read_dir(&recovery)
            .map(|rd| rd.flatten().count())
            .unwrap_or(0);
        assert_eq!(
            huerfanos, 0,
            "las copias de una transacción que nunca llegó a tener journal ni recibo son basura: \
             el GC de E24-H06 debe recogerlas"
        );
    }

    /// **E24-H13** — control anti-vacuo del seam: sin armar nada, la transacción publica.
    #[test]
    fn sin_failpoint_armado_la_transaccion_publica() {
        let (dir, ws, _original) = tres_documentos();
        let cs = cs_modifica_los_tres(&ws, "e24-h13-sin-armar");
        ws.apply_transaction(&cs)
            .expect("sin failpoint armado la transacción debe publicar");
        let final_ = canonical_md(dir.path());
        assert!(
            final_.values().all(|c| c.contains("cuerpo NUEVO")),
            "el seam no puede alterar el camino normal: {final_:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// E25-H01 — La publicación no escribe fuera de lo que respaldó
// (`requirements/epica-25-endurecimiento-escritura.md`, bloque A). Fase ROJA.
//
// EL DEFECTO
//
// `apply_transaction` computa el canónico, el resultado y el conjunto AFECTADO en **T1**
// (`src/transaction.rs:127-129`) y sobre ESE conjunto ejerce `assert_writable` (:133), el backup
// (:156) y el journal (:161). Pero `publish_result` **vuelve a leer el canónico** en **T3**
// (`src/publish.rs:104`) y **recomputa** `affected` contra el `result` de T1 (`publish.rs:114-124`),
// escribiendo o borrando todo lo que difiera (`publish.rs:127-134`) — sin `assert_writable`, sin
// copia de recuperación y sin entrada de journal. Cualquier cosa que aparezca o cambie en la
// ventana `[T1, T3)` cae dentro de esa diferencia recomputada.
//
// EL CONTRATO QUE FIJAN ESTOS TESTS
//
// El conjunto que la publicación sustituye es **exactamente** el que pasó por el guard, por el
// backup y por el journal — o la transacción aborta con `WRITE_CONFLICT` **antes del primer
// rename**. Un `WRITE_CONFLICT` sigue siendo terminal (modelo fail-fast, `§19.5`): no hay reintento.
//
// EL SEAM QUE HACE FALTA (lo añade el implementador; aquí solo se USA)
//
// El `failpoint!` de `src/lib.rs:38` solo sabe **abortar**, así que no sirve para inyectar una
// edición externa: hace falta un punto que ejecute un gancho del test y **continúe**. API mínima
// esperada, en `crates/lodestar-workspace/src/failpoints.rs`, bajo `#[cfg(feature =
// "test-failpoints")]` (en compilación normal no genera ni una instrucción):
//
// ```rust
// /// Punto del orquestador donde se ejecuta un gancho del test y la transacción CONTINÚA.
// #[derive(Debug, Clone, Copy, PartialEq, Eq)]
// pub enum PuntoDeGancho {
//     /// Dentro de la ventana `[T1, T3)`, en su último instante: tras `create_journal` e
//     /// INMEDIATAMENTE ANTES de `publish_result` (`transaction.rs:167`). Es donde el defecto vive:
//     /// copias y journal ya cubren el conjunto de T1, y el bucle de publicación aún no ha
//     /// sustituido nada.
//     AntesDePublicar,
// }
//
// /// Arma un gancho para el HILO ACTUAL (`thread_local`, igual que `failpoints::armar`: los tests
// /// corren en paralelo en el mismo proceso). Se dispara **una sola vez** y se desarma solo.
// pub fn armar_gancho(punto: PuntoDeGancho, gancho: impl Fn() + 'static);
//
// /// Desarma cualquier gancho del hilo actual (higiene: el gancho puede no haberse disparado).
// pub fn desarmar_ganchos();
// ```
//
// El gancho vive en el orquestador REAL (`apply_transaction`), no en una reconstrucción del flujo
// (lección de E24-H13).
//
// ROJO ESPERADO HOY
// - Los tres tests del módulo `ventana_de_publicacion`: **no compilan** (`armar_gancho`,
//   `desarmar_ganchos` y `PuntoDeGancho` no existen). Es rojo válido: el error nombra exactamente
//   los símbolos del seam que la historia encarga.
// - `apply_sin_interferencia_publica_igual` y `publicado_igual_a_respaldado` son los **controles
//   anti-vacuos**: no usan el gancho, están FUERA del gate de la feature y deben pasar **ya en
//   main** (el arreglo no puede consistir en abortar más a menudo ni en publicar menos).
// ---------------------------------------------------------------------------

/// Abre un workspace temporal con un documento por nombre (`<n>.md`, cuerpo «cuerpo original»).
fn siembra_documentos(root: &Path, nombres: &[&str]) -> Workspace {
    let ws = Workspace::open(root).unwrap();
    for n in nombres {
        ws.create_document(
            &RelPath::new(&format!("{n}.md")).unwrap(),
            "Nota",
            Some(n),
            &format!("# {n}\n\ncuerpo original\n"),
            false,
        )
        .unwrap();
    }
    ws
}

/// Change set que sustituye el cuerpo de cada `<n>.md`, con la `base_revision` ACTUAL del
/// workspace (si no, la transacción moriría en el control optimista del paso 7 y no llegaría a la
/// ventana que se está probando).
fn cs_modifica(ws: &Workspace, id: &str, paths: &[&str]) -> ChangeSet {
    let mut cs = change_set(
        id,
        paths
            .iter()
            .map(|p| NormalizedOperation::ReplaceBody {
                path: RelPath::new(p).unwrap(),
                body: format!("# {p}\n\ncuerpo NUEVO\n"),
            })
            .collect(),
    );
    cs.base_revision = ws.workspace_revision().unwrap();
    cs
}

/// Un `FileMap` visto como `ruta -> contenido` (mismas claves que [`canonical_md`]).
fn como_md(files: &FileMap) -> BTreeMap<String, String> {
    files
        .iter()
        .map(|(rel, c)| (rel.as_str().to_string(), c.clone()))
        .collect()
}

/// Conjunto de rutas que difieren entre dos estados del canónico: creadas/modificadas + borradas.
/// Es la definición observable de «lo publicado» (lo que el disco sustituyó de verdad).
fn diferencia(
    antes: &BTreeMap<String, String>,
    despues: &BTreeMap<String, String>,
) -> std::collections::BTreeSet<String> {
    let mut out = std::collections::BTreeSet::new();
    for (rel, contenido) in despues {
        if antes.get(rel) != Some(contenido) {
            out.insert(rel.clone());
        }
    }
    for rel in antes.keys() {
        if !despues.contains_key(rel) {
            out.insert(rel.clone());
        }
    }
    out
}

/// Conjunto de rutas **respaldadas** por una transacción: las copias byte-a-byte que hay bajo
/// `.lodestar/runtime/recovery/<txnId>/` MÁS las que el manifiesto `.absent` marcó «no existía»
/// (afectadas que se iban a crear, sin original que copiar).
fn respaldado_en_recovery(root: &Path, txn_id: &str) -> std::collections::BTreeSet<String> {
    let base = root
        .join(".lodestar")
        .join("runtime")
        .join("recovery")
        .join(txn_id);
    let mut out = std::collections::BTreeSet::new();
    fn walk(dir: &Path, base: &Path, out: &mut std::collections::BTreeSet<String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, out);
                continue;
            }
            let rel = path
                .strip_prefix(base)
                .unwrap()
                .to_string_lossy()
                .to_string();
            if rel == ".absent" {
                for linea in std::fs::read_to_string(&path).unwrap_or_default().lines() {
                    let l = linea.trim();
                    if !l.is_empty() {
                        out.insert(l.to_string());
                    }
                }
            } else {
                out.insert(rel);
            }
        }
    }
    walk(&base, &base, &mut out);
    out
}

/// **E25-H01** · Criterio `apply_sin_interferencia_publica_igual` (**control anti-vacuo**, pasa ya
/// en main) — **Dado** un apply SIN interferencia, **Cuando** se aplica, **Entonces** publica
/// exactamente los mismos paths y contenidos que hoy, con el mismo `resultWorkspaceRevision` y el
/// mismo `changedPaths`.
///
/// El oráculo no es un golden: se computa aparte con la única lógica del core
/// (`plan::apply_normalized_ops` + `types::workspace_revision`), que es lo que la transacción tiene
/// que reproducir. El change set mezcla las tres formas de afectación —modificar, crear y borrar—
/// para que el control cubra las tres ramas del bucle de publicación.
#[test]
fn apply_sin_interferencia_publica_igual() {
    let dir = tempfile::tempdir().unwrap();
    let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);

    let canonico_antes = canonical_filemap(dir.path());
    let mut cs = change_set(
        "e25-h01-sin-interferencia",
        vec![
            NormalizedOperation::ReplaceBody {
                path: RelPath::new("uno.md").unwrap(),
                body: "# uno\n\ncuerpo NUEVO\n".to_string(),
            },
            create_conforme("cuatro.md", "Nota", "cuatro"),
            NormalizedOperation::Delete {
                path: RelPath::new("tres.md").unwrap(),
                inbound_links_policy: lodestar_core::types::InboundLinksPolicy::Reject,
            },
        ],
    );
    let previous_esperada = ws.workspace_revision().unwrap();
    cs.base_revision = previous_esperada.clone();

    // Oráculo independiente: el resultado que el core prevé, su revisión y su conjunto afectado.
    let esperado = plan::apply_normalized_ops(&canonico_antes, &cs.operations)
        .expect("prever el resultado del plan");
    let esperado_md = como_md(&esperado);
    let antes_md = como_md(&canonico_antes);
    let afectados_esperados = diferencia(&antes_md, &esperado_md);
    assert_eq!(
        afectados_esperados.len(),
        3,
        "precondición: el change set debe afectar a 3 paths (modificado, creado y borrado)"
    );
    let revision_esperada = lodestar_core::types::workspace_revision(&esperado, &[]);

    let (previous, result, changed) = ws
        .apply_transaction(&cs)
        .expect("sin interferencia, la transacción debe publicar");

    assert_eq!(
        previous, previous_esperada,
        "la `previousRevision` debe ser la revisión sobre la que se publicó"
    );
    assert_eq!(
        result, revision_esperada,
        "la `resultWorkspaceRevision` publicada debe ser la que el core prevé para el resultado"
    );
    let changed_set: std::collections::BTreeSet<String> = changed
        .iter()
        .map(|p| p.as_str().to_string())
        .collect::<std::collections::BTreeSet<String>>();
    assert_eq!(
        changed_set, afectados_esperados,
        "`changedPaths` debe ser exactamente el conjunto afectado por el plan"
    );
    assert_eq!(
        canonical_md(dir.path()),
        esperado_md,
        "el canónico publicado debe ser, path a path y byte a byte, el resultado del plan"
    );
}

/// **E25-H01** · Criterio `publicado_igual_a_respaldado` (propiedad; **control anti-vacuo** en su
/// mitad de éxito) — **Dado** el conjunto publicado y el conjunto respaldado, **Cuando** termina
/// cualquier apply con éxito, **Entonces** son idénticos.
///
/// «Publicado» se mide en el disco (diferencia real entre el canónico de antes y el de después), no
/// en lo que la transacción dice haber cambiado: si el bucle de publicación tocara un path que
/// nadie respaldó, la igualdad se rompería aunque `changedPaths` siguiera pareciendo correcto. Se
/// recorren las mismas formas de change set que el arnés de recuperación (modificar, crear, borrar,
/// mover) más una mixta.
#[test]
fn publicado_igual_a_respaldado() {
    fn cs_mod(ws: &Workspace, id: &str) -> ChangeSet {
        cs_modifica(ws, id, &["uno.md", "dos.md", "tres.md"])
    }
    fn cs_crea(ws: &Workspace, id: &str) -> ChangeSet {
        let mut cs = change_set(
            id,
            vec![
                create_conforme("a.md", "Nota", "a"),
                create_conforme("b.md", "Nota", "b"),
            ],
        );
        cs.base_revision = ws.workspace_revision().unwrap();
        cs
    }
    fn cs_borra(ws: &Workspace, id: &str) -> ChangeSet {
        let mut cs = change_set(
            id,
            vec![NormalizedOperation::Delete {
                path: RelPath::new("tres.md").unwrap(),
                inbound_links_policy: lodestar_core::types::InboundLinksPolicy::Reject,
            }],
        );
        cs.base_revision = ws.workspace_revision().unwrap();
        cs
    }
    fn cs_mueve(ws: &Workspace, id: &str) -> ChangeSet {
        let mut cs = change_set(
            id,
            vec![NormalizedOperation::Move {
                from: RelPath::new("uno.md").unwrap(),
                to: RelPath::new("movido.md").unwrap(),
                rewrite_inbound_links: false,
            }],
        );
        cs.base_revision = ws.workspace_revision().unwrap();
        cs
    }
    fn cs_mixto(ws: &Workspace, id: &str) -> ChangeSet {
        let mut cs = change_set(
            id,
            vec![
                NormalizedOperation::ReplaceBody {
                    path: RelPath::new("dos.md").unwrap(),
                    body: "# dos\n\ncuerpo NUEVO\n".to_string(),
                },
                create_conforme("nuevo.md", "Nota", "nuevo"),
                NormalizedOperation::Delete {
                    path: RelPath::new("tres.md").unwrap(),
                    inbound_links_policy: lodestar_core::types::InboundLinksPolicy::Reject,
                },
            ],
        );
        cs.base_revision = ws.workspace_revision().unwrap();
        cs
    }

    type Forma = (&'static str, fn(&Workspace, &str) -> ChangeSet);
    let formas: &[Forma] = &[
        ("modifica", cs_mod),
        ("crea", cs_crea),
        ("borra", cs_borra),
        ("mueve", cs_mueve),
        ("mixto", cs_mixto),
    ];

    for (forma, build) in formas {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);
        let antes = canonical_md(dir.path());

        let id = format!("e25-h01-respaldado-{forma}");
        let cs = build(&ws, &id);
        let (_, _, changed) = ws
            .apply_transaction(&cs)
            .unwrap_or_else(|e| panic!("[{forma}] la transacción debe publicar: {e:?}"));

        let despues = canonical_md(dir.path());
        let publicado = diferencia(&antes, &despues);
        assert!(
            !publicado.is_empty(),
            "[{forma}] precondición no vacua: el change set debe cambiar algo del canónico"
        );

        let respaldado = respaldado_en_recovery(dir.path(), &id);
        assert_eq!(
            publicado, respaldado,
            "[{forma}] lo que la publicación sustituyó en disco debe ser EXACTAMENTE lo que se \
             respaldó en `recovery/{id}/` (copias + manifiesto `.absent`): todo lo publicado tiene \
             que ser recuperable"
        );

        let changed_set: std::collections::BTreeSet<String> =
            changed.iter().map(|p| p.as_str().to_string()).collect();
        assert_eq!(
            changed_set, publicado,
            "[{forma}] y el `changedPaths` del recibo debe declarar ese mismo conjunto"
        );
    }
}

#[cfg(feature = "test-failpoints")]
mod ventana_de_publicacion {
    use super::*;
    use lodestar_workspace::failpoints::{self, PuntoDeGancho};

    /// Ruta del árbol de copias de recuperación de una transacción.
    fn recovery_de(root: &Path, txn_id: &str) -> PathBuf {
        root.join(".lodestar")
            .join("runtime")
            .join("recovery")
            .join(txn_id)
    }

    /// **E25-H01** · Criterio 1 — **Dado** un apply en curso con el gancho armado en la ventana
    /// `[T1, T3)`, **Cuando** el gancho modifica un `.md` AFECTADO, **Entonces** la transacción
    /// falla con `WRITE_CONFLICT` y **ni uno** de los `.md` canónicos ha cambiado (incluido el
    /// editado, que conserva la edición externa).
    ///
    /// Hoy `publish_result` relee el canónico en T3 y sustituye `dos.md` por el contenido del plan:
    /// la edición externa se pierde y el backup guarda una versión de T1 que ya no era el estado
    /// real, así que un `change_revert` restauraría un estado que nunca existió.
    #[test]
    fn edicion_externa_en_la_ventana_aborta_sin_publicar() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);
        let antes = canonical_md(dir.path());
        let dos_en_t1 = antes.get("dos.md").expect("dos.md sembrado").clone();

        let id = "e25-h01-edicion-externa";
        let cs = cs_modifica(&ws, id, &["uno.md", "dos.md", "tres.md"]);

        // El gancho hace de «otro proceso»: edita un `.md` que la transacción va a sustituir,
        // cuando el guard, las copias y el journal ya se ejercieron sobre el estado de T1.
        //
        // La **precondición del escenario** —que el gancho se dispara DENTRO de la ventana y
        // DESPUÉS de las copias, que es donde vive el defecto— se asevera desde DENTRO del gancho:
        // es el único instante en que ese estado existe. Antes se comprobaba tras el fallo, pero
        // E25-H02 sella el journal y el árbol del aborto de ventana (ajuste declarado en su spec),
        // así que a la vuelta ya no queda ninguno de los dos que mirar.
        let ruta_dos = dir.path().join("dos.md");
        let edicion_externa = "---\ntype: Nota\ntitle: dos\n---\n\n# dos\n\nEDICIÓN EXTERNA\n";
        {
            let root = dir.path().to_path_buf();
            let dos_t1 = dos_en_t1.clone();
            failpoints::armar_gancho(PuntoDeGancho::AntesDePublicar, move || {
                let recovery = recovery_de(&root, id);
                assert!(
                    recovery.is_dir(),
                    "el gancho debe dispararse tras `backup_originals` (último instante de la \
                     ventana): sin árbol de recuperación, el aborto ocurriría antes de que el \
                     defecto sea posible"
                );
                assert_eq!(
                    std::fs::read_to_string(recovery.join("dos.md")).unwrap_or_default(),
                    dos_t1,
                    "y la copia de recuperación guarda la versión de T1: exactamente por eso \
                     publicar sobre un canónico que ya no es el de T1 dejaría un backup que no \
                     restaura nada real"
                );
                std::fs::write(&ruta_dos, edicion_externa)
                    .expect("el gancho debe poder editar dos.md");
            });
        }
        let resultado = ws.apply_transaction(&cs);
        failpoints::desarmar_ganchos();

        let err = match resultado {
            Err(e) => e,
            Ok((_, _, changed)) => panic!(
                "una edición externa dentro de la ventana `[T1, T3)` debe abortar la transacción \
                 ANTES del primer rename, pero publicó: changedPaths={changed:?}"
            ),
        };
        assert_eq!(
            err.code(),
            "WRITE_CONFLICT",
            "el aborto debe llevar el código estable WRITE_CONFLICT (terminal: el agente \
             replanifica, no se reintenta); era: {err:?}"
        );

        // E25-H02 (**ajuste declarado en su spec**, aserción INVERTIDA): el aborto de ventana sella
        // su propia transacción bajo el mismo lock —sabe por control de flujo que no ha entrado en
        // el bucle de renames, así que cero renames significa que no hay NADA que restaurar—, de
        // modo que tras el `WRITE_CONFLICT` no puede quedar ni journal ni árbol de recuperación.
        // Hasta E25-H01 esta misma aserción exigía justo lo contrario (que el árbol siguiera ahí,
        // como precondición del escenario); la precondición se garantiza ahora desde dentro del
        // gancho, que es donde ese estado existe de verdad. Si el material sobreviviera, la
        // siguiente operación restauraría las copias de T1 encima de la edición externa que este
        // aborto existe para no pisar.
        assert!(
            !recovery_de(dir.path(), id).exists(),
            "el aborto de ventana no puede dejar su árbol de recuperación en disco: {}",
            recovery_de(dir.path(), id).display()
        );
        assert!(
            !journal_de(dir.path(), id).exists(),
            "ni su fichero de journal: {}",
            journal_de(dir.path(), id).display()
        );
        assert!(
            !ws.recovery_pending(),
            "y por tanto no queda recuperación pendiente que la siguiente operación vaya a ejecutar"
        );

        // El criterio: ni un `.md` canónico sustituido. El único cambio es el del propio gancho.
        let mut esperado = antes.clone();
        esperado.insert("dos.md".to_string(), edicion_externa.to_string());
        assert_eq!(
            canonical_md(dir.path()),
            esperado,
            "tras el aborto, ningún `.md` canónico puede haber sido sustituido — y la edición \
             externa de `dos.md` sigue intacta (no la pisó la publicación)"
        );
    }

    /// **E25-H01** · Criterio 2 — **Dado** ese mismo apply, **Cuando** el gancho **crea** un `.md`
    /// que el plan no menciona, **Entonces** ese fichero sigue existiendo con su contenido intacto.
    ///
    /// Hoy desaparece: no está en el `result` de T1, así que el bucle de `publish.rs:120-124` lo
    /// mete en `affected` y `publish.rs:130` lo **borra**, sin backup (nunca estuvo en el conjunto
    /// de T1) y sin entrada de journal. El borrado es irrecuperable y el recibo ni lo menciona.
    #[test]
    fn fichero_nuevo_en_la_ventana_no_se_borra() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);
        let antes = canonical_md(dir.path());

        let cs = cs_modifica(
            &ws,
            "e25-h01-fichero-nuevo",
            &["uno.md", "dos.md", "tres.md"],
        );

        let ruta_intruso = dir.path().join("intruso.md");
        let contenido_intruso =
            "---\ntype: Nota\ntitle: intruso\n---\n\n# intruso\n\ncreado por otro proceso\n";
        {
            let ruta = ruta_intruso.clone();
            failpoints::armar_gancho(PuntoDeGancho::AntesDePublicar, move || {
                std::fs::write(&ruta, contenido_intruso)
                    .expect("el gancho debe poder crear el .md");
            });
        }
        let resultado = ws.apply_transaction(&cs);
        failpoints::desarmar_ganchos();

        // El fichero que nadie respaldó tiene que seguir ahí, byte a byte. Es el criterio.
        assert!(
            ruta_intruso.is_file(),
            "un `.md` creado por otro proceso dentro de la ventana NO puede desaparecer: la \
             publicación solo puede tocar lo que respaldó y anotó en el journal"
        );
        assert_eq!(
            std::fs::read_to_string(&ruta_intruso).unwrap(),
            contenido_intruso,
            "y su contenido debe estar intacto"
        );

        let err = match resultado {
            Err(e) => e,
            Ok((_, _, changed)) => panic!(
                "un canónico que cambió dentro de la ventana debe abortar la transacción antes del \
                 primer rename, pero publicó: changedPaths={changed:?}"
            ),
        };
        assert_eq!(
            err.code(),
            "WRITE_CONFLICT",
            "el aborto debe llevar el código estable WRITE_CONFLICT; era: {err:?}"
        );

        // Y no se publicó nada: el canónico es el de T1 más el intruso.
        let mut esperado = antes.clone();
        esperado.insert("intruso.md".to_string(), contenido_intruso.to_string());
        assert_eq!(
            canonical_md(dir.path()),
            esperado,
            "el aborto ocurre ANTES del primer rename: ningún `.md` del plan puede haberse escrito"
        );
    }

    /// **E25-H01** · Criterio 3 — **Dado** un workspace con `referenceRoots`, **Cuando** el gancho
    /// crea un `.md` bajo un `referenceRoot` dentro de la ventana, **Entonces** ese fichero no se
    /// toca.
    ///
    /// Es el escenario que el control optimista **no puede** ver ni en principio:
    /// `lodestar_core::types::workspace_revision` excluye lo que queda fuera de `writableRoots`
    /// (`core/types.rs:1247-1249`, fijado por `core.rs::revision_excluye_reference_roots`), así que
    /// `reverify_base_revision` es ciego a él. Solo una comparación del canónico completo entre T1 y
    /// T3 lo detecta — y mientras tanto `publish_result` lo borra sin haberlo respaldado, violando
    /// de paso la inmutabilidad que `assert_writable` promete para los `referenceRoots`.
    #[test]
    fn reference_root_no_se_borra_en_la_ventana() {
        let dir = tempfile::tempdir().unwrap();
        let lodestar = dir.path().join(".lodestar");
        std::fs::create_dir_all(&lodestar).unwrap();
        std::fs::write(
            lodestar.join("config.yaml"),
            "workspace:\n  writableRoots: [conocimiento]\n  referenceRoots: [referencia]\n",
        )
        .unwrap();

        let ws = siembra_documentos(
            dir.path(),
            &["conocimiento/uno", "conocimiento/dos", "referencia/manual"],
        );
        let antes = canonical_md(dir.path());
        assert!(
            antes.contains_key("referencia/manual.md"),
            "precondición: el `referenceRoot` es visible para el descubrimiento (por eso el bucle \
             de publicación puede llegar a borrarlo)"
        );

        let cs = cs_modifica(
            &ws,
            "e25-h01-reference-root",
            &["conocimiento/uno.md", "conocimiento/dos.md"],
        );

        let ruta_intruso = dir.path().join("referencia").join("intruso.md");
        let contenido_intruso = "# intruso\n\nmaterial de solo lectura aparecido en la ventana\n";
        {
            let ruta = ruta_intruso.clone();
            failpoints::armar_gancho(PuntoDeGancho::AntesDePublicar, move || {
                std::fs::write(&ruta, contenido_intruso)
                    .expect("el gancho debe poder crear el .md");
            });
        }
        let resultado = ws.apply_transaction(&cs);
        failpoints::desarmar_ganchos();

        // El criterio: lo que hay bajo un `referenceRoot` es intocable, aparezca cuando aparezca.
        assert!(
            ruta_intruso.is_file(),
            "un `.md` aparecido bajo un `referenceRoot` (inmutable por `assert_writable`) dentro de \
             la ventana NO puede borrarlo la publicación: hoy lo borra porque recomputa `affected` \
             contra el canónico de T3 sin volver a pasar por el guard"
        );
        assert_eq!(
            std::fs::read_to_string(&ruta_intruso).unwrap(),
            contenido_intruso,
            "y su contenido debe estar intacto"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("referencia").join("manual.md")).unwrap(),
            antes["referencia/manual.md"],
            "tampoco puede tocarse el material de referencia que ya estaba"
        );

        let err = match resultado {
            Err(e) => e,
            Ok((_, _, changed)) => panic!(
                "el canónico cambió dentro de la ventana (aunque fuera de `writableRoots`, donde la \
                 revisión no mira): la transacción debe abortar, pero publicó: \
                 changedPaths={changed:?}"
            ),
        };
        assert_eq!(
            err.code(),
            "WRITE_CONFLICT",
            "el aborto debe llevar el código estable WRITE_CONFLICT; era: {err:?}"
        );
        assert_eq!(
            canonical_md(dir.path())
                .into_iter()
                .filter(|(k, _)| k.starts_with("conocimiento/"))
                .collect::<BTreeMap<String, String>>(),
            antes
                .iter()
                .filter(|(k, _)| k.starts_with("conocimiento/"))
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<BTreeMap<String, String>>(),
            "y nada del conocimiento escribible se publicó: el aborto es antes del primer rename"
        );
    }
}

// ---------------------------------------------------------------------------
// E25-H02 — Las copias de recuperación son durables, verificadas y nunca encallan el workspace
// (`requirements/epica-25-endurecimiento-escritura.md`, bloque B). Fase ROJA.
//
// LOS DEFECTOS
//
// 1. **Copias no durables**: `backup_originals` copia con `std::fs::copy`
//    (`src/recovery.rs:140`) y escribe el manifiesto `.absent` con `std::fs::write` (`:155-158`),
//    ninguno con volcado; el journal, en cambio, SÍ se fsynca antes del primer rename. Tras un
//    corte de energía puede quedar un journal durable apuntando a una copia truncada o ausente.
// 2. **Restauración verbatim**: `restore_backups` (`:358-364`) lee la copia y la escribe tal cual
//    sobre el canónico, sin verificar nada. Una copia truncada se PUBLICA como si fuera el original.
// 3. **Workspace encallado para siempre**: si la copia es ilegible, `restore_backups` devuelve `Err`
//    y `recover()` lo propaga con `?` en sus tres brazos (`:271`, `:275-276`, `:297-298`). No hay
//    cuarentena ni descarte: `pending_journals` sigue viendo el journal, `recovery_pending()` sigue
//    en `true` y TODA escritura futura muere en el paso (2) de `apply_transaction`.
// 4. **`.absent` perdido → estado híbrido**: `read_absent_manifest` (`:179-188`) trata cualquier
//    fallo de lectura como conjunto vacío, así que la restauración no borra los ficheros que la
//    transacción creó: el canónico queda con los originales MÁS los creados. Ni un borde ni el otro.
// 5. **La recuperación deshace lo que el aborto de ventana acababa de proteger** (estado nuevo, lo
//    destapó E25-H01): tras el `WRITE_CONFLICT` de ventana quedan en disco el journal `prepared`
//    (creado en `transaction.rs:164`, ANTES de la comprobación) y su árbol de recuperación con las
//    copias de T1, con CERO renames aplicados. La siguiente operación clasifica ese journal como
//    `prepared` → RESTAURAR y escribe las copias de T1 encima de la edición externa que el aborto
//    existía para no pisar; y lo que el usuario CREÓ en la ventana está marcado `.absent`, así que
//    la restauración lo borra. Sin esta enmienda, las tres garantías de E25-H01 duran exactamente
//    hasta la siguiente operación.
//
// EL CONTRATO QUE FIJAN ESTOS TESTS
//
// - Nada se escribe sobre el canónico a partir de una copia que no verifica.
// - Un journal que no se puede restaurar manda su material a
//   `.lodestar/runtime/journal/quarantine/<txnId>/` —donde NADA se borra: es material forense—,
//   `recover()` sigue con los demás, la operación que lo disparó falla UNA vez con
//   `RECOVERY_FAILED` nombrando esa ruta, y la siguiente procede.
// - El aborto de ventana sella su propia transacción bajo el mismo lock (journal primero, árbol
//   después), así que no deja recuperación pendiente que pise nada.
//
// EL SEAM QUE HACE FALTA (lo añade el implementador; aquí solo se USA)
//
// Para el criterio `el_sellado_del_aborto_es_seguro_a_mitad` hace falta poder interrumpir el sellado
// del aborto ENTRE el borrado del journal y el del árbol. La taxonomía de `FailPoint`
// (`src/failpoints.rs:45-64`) gana un punto —solo bajo `--features test-failpoints`, sin generar ni
// una instrucción en compilación normal—:
//
// ```rust
// pub enum FailPoint {
//     …
//     /// En medio del sellado del **aborto de ventana** (E25-H02): el fichero de journal ya se ha
//     /// borrado y el árbol de recuperación TODAVÍA no. Modela el proceso que muere entre los dos
//     /// borrados; al reabrir no debe haber recuperación pendiente y el árbol huérfano lo recoge
//     /// el GC (E24-H06).
//     EnMedioDelSelladoDelAborto,
// }
// ```
//
// Se ejerce con el `failpoint!` que ya existe (aborta con `Err`), colocado en el camino de aborto
// por divergencia de ventana, entre `remove_file(journal)` y la limpieza del árbol. Cuando está
// armado, el error que sale de `apply_transaction` es el del failpoint y NO `WRITE_CONFLICT`: el
// test solo exige `is_err()`.
//
// Además, `RECOVERY_FAILED` gana su primer emisor real: `WorkspaceError` necesita una variante cuyo
// `.code()` sea `ErrorCode::RecoveryFailed.as_str()` («RECOVERY_FAILED») y cuyo `Display` nombre la
// ruta de cuarentena. Los tests NO nombran la variante —solo `code()` y el mensaje—, así que la
// forma exacta la elige el implementador.
//
// ROJO ESPERADO HOY
// - `copia_truncada_no_se_restaura_verbatim`: la copia truncada se escribe encima del canónico.
// - `journal_irrecuperable_no_encalla_el_workspace`: la primera operación falla con `IO`
//   (no `RECOVERY_FAILED`), no hay cuarentena, y la segunda falla igual — para siempre.
// - `un_journal_roto_no_arrastra_a_los_demas`: `recover()` sale por `?` en el primer journal roto.
// - `absent_perdido_no_deja_estado_hibrido`: `recover()` devuelve `Ok` y los ficheros creados
//   sobreviven junto a los originales.
// - `la_cuarentena_no_borra_nada`: no existe cuarentena alguna.
// - Los cuatro del aborto de ventana: la recuperación de la siguiente operación pisa la edición
//   externa, borra el fichero nuevo y deja journal + árbol en disco.
// - `cero_applied_no_significa_cero_renames` es el **control anti-vacuo declarado**: pasa ya hoy y
//   tiene que seguir pasando. Protege contra la generalización prohibida por la spec
//   («un journal `prepared` con cero `applied` no hay que restaurarlo»), que sellaría publicaciones
//   parciales: «cero `applied` durables» describe también la caída ENTRE el primer rename y su
//   anotación.
// ---------------------------------------------------------------------------

/// Ruta del árbol de copias de recuperación de una transacción (`recovery/<txnId>/`), exista o no.
fn recovery_de(root: &Path, txn_id: &str) -> PathBuf {
    root.join(".lodestar")
        .join("runtime")
        .join("recovery")
        .join(txn_id)
}

/// Ruta del write-ahead journal de una transacción (`journal/<txnId>.json`), exista o no.
fn journal_de(root: &Path, txn_id: &str) -> PathBuf {
    root.join(".lodestar")
        .join("runtime")
        .join("journal")
        .join(format!("{txn_id}.json"))
}

/// Ruta del directorio de **cuarentena** de una transacción cuya recuperación falló (E25-H02):
/// `.lodestar/runtime/journal/quarantine/<txnId>/`. Nada se borra ahí dentro: es material forense.
fn cuarentena_de(root: &Path, txn_id: &str) -> PathBuf {
    root.join(".lodestar")
        .join("runtime")
        .join("journal")
        .join("quarantine")
        .join(txn_id)
}

/// Todos los ficheros bajo `dir` (recursivo) como `ruta relativa POSIX -> bytes`. Compara **bytes**,
/// no texto: una copia de recuperación corrupta no es UTF-8 y el criterio «la cuarentena no borra
/// nada» es byte-a-byte.
fn ficheros_bajo(dir: &Path) -> BTreeMap<String, Vec<u8>> {
    fn walk(d: &Path, base: &Path, out: &mut BTreeMap<String, Vec<u8>>) {
        let Ok(entries) = std::fs::read_dir(d) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, base, out);
                continue;
            }
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .replace('\\', "/");
            out.insert(rel, std::fs::read(&path).unwrap_or_default());
        }
    }
    let mut out = BTreeMap::new();
    walk(dir, dir, &mut out);
    out
}

/// Conjunto de paths afectados por llevar `canonico` a `resultado`, con el MISMO criterio y el mismo
/// orden determinista que el orquestador (`transaction.rs::affected_paths`): así el arnés respalda y
/// registra exactamente el lote que producción respaldaría y registraría.
fn afectados_por(canonico: &FileMap, resultado: &FileMap) -> Vec<RelPath> {
    let mut set: std::collections::BTreeSet<RelPath> = std::collections::BTreeSet::new();
    for (rel, contenido) in resultado {
        if canonico.get(rel) != Some(contenido) {
            set.insert(rel.clone());
        }
    }
    for rel in canonico.keys() {
        if !resultado.contains_key(rel) {
            set.insert(rel.clone());
        }
    }
    set.into_iter().collect()
}

/// Deja en disco el estado durable EXACTO de una transacción **interrumpida**, en el orden real del
/// orquestador (copias → journal → renames, `transaction.rs:159-180`): `backup_originals` (E13-H04)
/// → `create_journal` (E13-H03) → los `renames` primeros renames del lote.
///
/// - `renames`: cuántas sustituciones del canónico llegaron a completarse, en el orden determinista
///   del lote (el mismo que publica `publish_result`).
/// - `anotar`: si cada rename se marcó en el journal (`mark_applied`, que lo lleva a `applying`) o
///   no. `anotar = false` con `renames = 1` es la caída ENTRE el primer rename y su anotación:
///   journal `prepared`, cero entradas `applied` **y un rename ya hecho en disco**.
///
/// Devuelve `(afectados, resultado)`: el lote respaldado/registrado y el `FileMap` que la
/// transacción habría dejado si hubiera terminado.
fn transaccion_interrumpida(
    ws: &Workspace,
    root: &Path,
    txn_id: &str,
    cs: &ChangeSet,
    renames: usize,
    anotar: bool,
) -> (Vec<RelPath>, FileMap) {
    let canonico = canonical_filemap(root);
    let resultado = plan::apply_normalized_ops(&canonico, &cs.operations)
        .expect("prever el resultado del plan");
    let afectados = afectados_por(&canonico, &resultado);
    assert!(
        !afectados.is_empty(),
        "precondición del arnés: la transacción debe afectar a algún path"
    );
    assert!(
        renames <= afectados.len(),
        "precondición del arnés: no se pueden simular más renames ({renames}) que paths afectados \
         ({})",
        afectados.len()
    );

    let base = ws.workspace_revision().unwrap();
    let writable = &[] as &[RelPath];
    let result_rev = lodestar_core::types::workspace_revision(&resultado, writable);

    // (8) Copias de recuperación de los originales afectados — ANTES del journal, como producción.
    ws.backup_originals(txn_id, &afectados)
        .expect("preparar las copias de recuperación");

    // (9) Write-ahead journal `prepared`, fsynced antes del primer rename.
    let mut journal = ws
        .create_journal(txn_id, &afectados, &base, &result_rev)
        .expect("crear el write-ahead journal");

    // (10) Los `renames` primeros renames del lote, en el orden determinista de la publicación.
    for rel in afectados.iter().take(renames) {
        match resultado.get(rel) {
            Some(contenido) => std::fs::write(root.join(rel.as_str()), contenido).unwrap(),
            None => {
                let _ = std::fs::remove_file(root.join(rel.as_str()));
            }
        }
        if anotar {
            journal
                .mark_applied(rel)
                .expect("anotar el rename en el journal");
        }
    }

    // Se «cae»: el journal no llega a `applied` ni se sella.
    (afectados, resultado)
}

/// `true` si `mensaje` nombra la cuarentena de `txn_id` (la ruta que la spec exige citar para que
/// quien lea el error sepa dónde quedó el material forense).
fn menciona_la_cuarentena(mensaje: &str, txn_id: &str) -> bool {
    mensaje.contains("quarantine") && mensaje.contains(txn_id)
}

mod durabilidad_de_la_recuperacion {
    use super::*;

    /// **E25-H02** · Criterio 1 — **Dado** un journal `prepared` cuya copia de recuperación está
    /// **truncada**, **Cuando** se reabre el workspace y se recupera, **Entonces** el canónico NO se
    /// sobrescribe con la copia rota.
    ///
    /// Hoy `restore_backups` (`recovery.rs:358-364`) lee la copia y la escribe tal cual: el `.md`
    /// canónico queda con los bytes truncados —frontmatter partido incluido— y la recuperación lo
    /// declara un éxito. Una copia que no verifica no es un original: no se restaura.
    #[test]
    fn copia_truncada_no_se_restaura_verbatim() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);
        let original = canonical_md(dir.path());

        let id = "e25-h02-copia-truncada";
        let cs = cs_modifica(&ws, id, &["uno.md", "dos.md", "tres.md"]);
        // Un rename ya hecho y anotado (journal `applying`): la restauración es NECESARIA, así que
        // ninguna implementación puede saltarse la copia sin más y pasar el test vacuamente.
        let (afectados, resultado) = transaccion_interrumpida(&ws, dir.path(), id, &cs, 1, true);
        let publicado = afectados[0].as_str().to_string();
        let esperado = como_md(&resultado);
        assert_ne!(
            original[&publicado], esperado[&publicado],
            "precondición: el path renombrado debe cambiar de contenido (si no, no hay nada que \
             restaurar y el test sería vacuo)"
        );

        // La copia de recuperación de ESE path queda truncada a la mitad, como la dejaría un corte
        // de energía sobre un `std::fs::copy` sin volcado.
        let copia = recovery_de(dir.path(), id).join(&publicado);
        let completa = std::fs::read(&copia).expect("la copia de recuperación debe existir");
        let truncada = completa[..completa.len() / 2].to_vec();
        assert!(
            !truncada.is_empty() && truncada != completa,
            "precondición: la copia truncada debe diferir de la íntegra"
        );
        std::fs::write(&copia, &truncada).unwrap();
        drop(ws);

        let ws2 = Workspace::open(dir.path()).unwrap();
        // La recuperación puede fallar (copia que no verifica → cuarentena) o no, pero si falla lo
        // hace con su código propio, no con un IO genérico.
        if let Err(e) = ws2.recover() {
            assert_eq!(
                e.code(),
                "RECOVERY_FAILED",
                "una copia que no verifica es un fallo de recuperación, con su código propio del \
                 catálogo; era: {e:?}"
            );
        }

        let despues = canonical_md(dir.path());
        let contenido = despues
            .get(&publicado)
            .unwrap_or_else(|| panic!("`{publicado}` debe seguir existiendo en el canónico"));
        assert_ne!(
            contenido.as_bytes(),
            truncada.as_slice(),
            "el canónico NO puede quedar con los bytes de la copia truncada: restaurar sin \
             verificar publica basura firmada como «el original»"
        );
        assert!(
            contenido == &original[&publicado] || contenido == &esperado[&publicado],
            "y tiene que ser uno de los dos bordes íntegros de la transacción (original o \
             resultado), nunca una copia a medias.\nen disco: {contenido:?}\noriginal: {:?}\n\
             resultado: {:?}",
            original[&publicado],
            esperado[&publicado]
        );
        assert!(
            !ws2.recovery_pending(),
            "y el workspace vuelve a ser escribible: tras recuperar (aunque sea a cuarentena) no \
             puede quedar recuperación pendiente"
        );
    }

    /// **E25-H02** · Criterio 2 — **Dado** un journal `prepared` cuya copia es **ilegible**,
    /// **Cuando** se recupera y luego se intenta una transacción nueva, **Entonces** la primera
    /// falla con `RECOVERY_FAILED` nombrando la cuarentena y **la segunda tiene éxito**.
    ///
    /// Hoy no tiene éxito ninguna de las dos, nunca: `restore_backups` devuelve `Err`, `recover()`
    /// lo propaga con `?`, el journal sigue pendiente y toda escritura futura muere en el paso (2)
    /// de `apply_transaction`. Un solo fichero ilegible cierra el workspace para siempre.
    ///
    /// La copia se hace ilegible escribiéndole bytes que **no son UTF-8**: `read_to_string` falla
    /// exactamente igual que con un permiso denegado, y sin depender de `chmod` ni del usuario que
    /// corre los tests (en CI podría ser root, para quien no hay fichero ilegible).
    #[test]
    fn journal_irrecuperable_no_encalla_el_workspace() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);

        let id = "e25-h02-journal-irrecuperable";
        let cs = cs_modifica(&ws, id, &["uno.md", "dos.md", "tres.md"]);
        let (afectados, _) = transaccion_interrumpida(&ws, dir.path(), id, &cs, 1, true);
        let publicado = afectados[0].as_str().to_string();
        std::fs::write(
            recovery_de(dir.path(), id).join(&publicado),
            [0xff, 0xfe, 0x00, 0x01],
        )
        .unwrap();
        drop(ws);

        let ws2 = Workspace::open(dir.path()).unwrap();
        assert!(
            ws2.recovery_pending(),
            "precondición: al reabrir hay una recuperación pendiente que la primera operación \
             tendrá que atender"
        );

        // La PRIMERA operación real dispara la recuperación en su paso (2) y hereda su fallo.
        let cs2 = cs_modifica(&ws2, "e25-h02-siguiente", &["tres.md"]);
        let err = match ws2.apply_transaction(&cs2) {
            Err(e) => e,
            Ok((_, _, changed)) => panic!(
                "la operación que arrastra una recuperación irrecuperable no puede reportar éxito: \
                 changedPaths={changed:?}"
            ),
        };
        assert_eq!(
            err.code(),
            "RECOVERY_FAILED",
            "el fallo de recuperación tiene su propio código del catálogo (E25-H02 le da su primer \
             emisor real): no es un IO genérico ni un WORKSPACE_RECOVERY_REQUIRED; era: {err:?}"
        );
        let mensaje = err.to_string();
        assert!(
            menciona_la_cuarentena(&mensaje, id),
            "y el mensaje debe NOMBRAR la ruta de cuarentena \
             (.lodestar/runtime/journal/quarantine/{id}/): es el único sitio donde el operador se \
             entera de que hay material forense esperando. Mensaje: {mensaje}"
        );
        assert!(
            cuarentena_de(dir.path(), id).is_dir(),
            "y la cuarentena debe existir de verdad en {}",
            cuarentena_de(dir.path(), id).display()
        );
        assert!(
            !ws2.recovery_pending(),
            "tras la cuarentena ya no queda recuperación pendiente: es lo que desencalla el \
             workspace"
        );

        // La SEGUNDA operación tiene éxito: el workspace volvió a ser escribible.
        ws2.apply_transaction(&cs2).unwrap_or_else(|e| {
            panic!(
                "la segunda operación debe publicar con normalidad (el journal irrecuperable ya \
                 está en cuarentena): {e:?}"
            )
        });
        assert!(
            canonical_md(dir.path())["tres.md"].contains("cuerpo NUEVO"),
            "y su resultado tiene que estar en el canónico"
        );
    }

    /// **E25-H02** · Criterio 3 — **Dado** dos journals pendientes, uno sano y uno irrecuperable,
    /// **Cuando** se recupera, **Entonces** el sano se recupera igualmente.
    ///
    /// Hoy `recover()` itera `pending_journals` y sale por `?` en el primero que falla, así que el
    /// destino del journal sano depende del orden de `read_dir`. El test muerde en los dos órdenes:
    /// si el roto va primero, el sano no se recupera; si va segundo, el sano se recupera pero el
    /// error que sale es un `IO` genérico y sin cuarentena.
    #[test]
    fn un_journal_roto_no_arrastra_a_los_demas() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres", "cuatro"]);
        let original = canonical_md(dir.path());

        // Transacción SANA interrumpida sobre `uno.md` (copia íntegra, 1 rename hecho).
        let sano = "e25-h02-sano";
        let cs_sano = cs_modifica(&ws, sano, &["uno.md"]);
        let (af_sano, _) = transaccion_interrumpida(&ws, dir.path(), sano, &cs_sano, 1, true);
        assert_eq!(af_sano.len(), 1, "precondición: la sana afecta a un path");

        // Transacción ROTA interrumpida sobre `tres.md` (copia ilegible, 1 rename hecho).
        let roto = "e25-h02-roto";
        let cs_roto = cs_modifica(&ws, roto, &["tres.md"]);
        let (af_roto, _) = transaccion_interrumpida(&ws, dir.path(), roto, &cs_roto, 1, true);
        assert_eq!(af_roto.len(), 1, "precondición: la rota afecta a un path");
        std::fs::write(
            recovery_de(dir.path(), roto).join(af_roto[0].as_str()),
            [0xff, 0xfe, 0x00, 0x01],
        )
        .unwrap();
        drop(ws);

        let ws2 = Workspace::open(dir.path()).unwrap();
        let err = match ws2.recover() {
            Err(e) => e,
            Ok(()) => panic!(
                "una recuperación con un journal irrecuperable debe reportarlo una vez, no callarlo"
            ),
        };
        assert_eq!(
            err.code(),
            "RECOVERY_FAILED",
            "el fallo del journal roto se reporta con su código propio; era: {err:?}"
        );

        // El journal SANO se recuperó igualmente: su path volvió al original y su plano de control
        // quedó sellado.
        let despues = canonical_md(dir.path());
        assert_eq!(
            despues["uno.md"], original["uno.md"],
            "el journal sano se restaura igual: que otro journal esté roto no puede dejar a medias \
             una transacción que sí se podía deshacer"
        );
        assert!(
            !journal_de(dir.path(), sano).exists(),
            "y su journal queda sellado (borrado), no pendiente"
        );
        assert!(
            !recovery_de(dir.path(), sano).exists(),
            "ni su árbol de recuperación"
        );

        // El roto quedó en cuarentena y el workspace desencallado.
        assert!(
            cuarentena_de(dir.path(), roto).is_dir(),
            "el journal roto va a cuarentena: {}",
            cuarentena_de(dir.path(), roto).display()
        );
        assert!(
            !ws2.recovery_pending(),
            "y tras recuperar no queda NINGÚN journal pendiente (ni el sano, ni el roto)"
        );
    }

    /// **E25-H02** · Criterio 4 — **Dado** una transacción que **crea** ficheros y cuyo manifiesto
    /// `.absent` se pierde antes de la caída, **Cuando** se recupera, **Entonces** el canónico
    /// converge al borde «original» —los creados no sobreviven— o la recuperación va a cuarentena;
    /// **nunca** al híbrido.
    ///
    /// Hoy `read_absent_manifest` (`recovery.rs:179-188`) trata la ausencia del manifiesto como
    /// conjunto vacío, así que `recover()` devuelve `Ok(())` y deja los ficheros creados en su
    /// sitio: el canónico queda con los originales MÁS los creados, que es exactamente lo que el
    /// rustdoc de `recover` promete que no puede pasar. Lo prohibido es el híbrido **silencioso**:
    /// un `Ok(())` con los creados vivos.
    #[test]
    fn absent_perdido_no_deja_estado_hibrido() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos"]);
        let original = canonical_md(dir.path());

        let id = "e25-h02-absent-perdido";
        let mut cs = change_set(
            id,
            vec![
                create_conforme("a.md", "Nota", "a"),
                create_conforme("b.md", "Nota", "b"),
            ],
        );
        cs.base_revision = ws.workspace_revision().unwrap();
        // Un fichero creado ya en disco (el primer rename, anotado): hay algo que deshacer.
        let (afectados, _) = transaccion_interrumpida(&ws, dir.path(), id, &cs, 1, true);
        let creado = afectados[0].as_str().to_string();
        assert!(
            dir.path().join(&creado).is_file(),
            "precondición: la transacción llegó a crear `{creado}` en el canónico"
        );

        // El manifiesto `.absent` no llegó a disco (se escribía con `std::fs::write`, sin volcado).
        let manifiesto = recovery_de(dir.path(), id).join(".absent");
        assert!(
            manifiesto.is_file(),
            "precondición: una transacción que solo CREA escribe el manifiesto `.absent` con los \
             paths que no existían"
        );
        std::fs::remove_file(&manifiesto).unwrap();
        drop(ws);

        let ws2 = Workspace::open(dir.path()).unwrap();
        match ws2.recover() {
            Ok(()) => assert_eq!(
                canonical_md(dir.path()),
                original,
                "si la recuperación se declara exitosa, el canónico tiene que estar en el borde \
                 «original»: los ficheros que la transacción creó NO pueden sobrevivir. Un `Ok` con \
                 los creados vivos es el estado híbrido que la recuperación existe para impedir"
            ),
            Err(e) => {
                assert_eq!(
                    e.code(),
                    "RECOVERY_FAILED",
                    "si el manifiesto perdido hace la restauración indecidible, se reporta como \
                     fallo de recuperación (y el material va a cuarentena); era: {e:?}"
                );
                assert!(
                    cuarentena_de(dir.path(), id).is_dir(),
                    "y con su material en cuarentena: {}",
                    cuarentena_de(dir.path(), id).display()
                );
            }
        }
        assert!(
            !ws2.recovery_pending(),
            "en los dos bordes admisibles, el workspace vuelve a ser escribible"
        );
    }

    /// **E25-H02** · Criterio 5 — **Dado** el material en cuarentena, **Cuando** se inspecciona,
    /// **Entonces** el journal y el árbol de recuperación siguen ahí **completos**.
    ///
    /// La cuarentena es material forense: se **mueve**, no se depura. Lo que había en
    /// `journal/<txnId>.json` y en `recovery/<txnId>/` —incluida la copia corrupta que causó el
    /// fallo y las copias sanas de los demás paths— tiene que seguir byte a byte dentro de
    /// `journal/quarantine/<txnId>/`.
    #[test]
    fn la_cuarentena_no_borra_nada() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);

        let id = "e25-h02-cuarentena-completa";
        let cs = cs_modifica(&ws, id, &["uno.md", "dos.md", "tres.md"]);
        let (afectados, _) = transaccion_interrumpida(&ws, dir.path(), id, &cs, 1, true);
        std::fs::write(
            recovery_de(dir.path(), id).join(afectados[0].as_str()),
            [0xff, 0xfe, 0x00, 0x01],
        )
        .unwrap();

        // Foto exacta del material ANTES de recuperar: el árbol entero y el JSON del journal.
        let arbol_antes = ficheros_bajo(&recovery_de(dir.path(), id));
        assert!(
            arbol_antes.len() >= 3,
            "precondición: el árbol debe tener una copia por path afectado; tiene {:?}",
            arbol_antes.keys().collect::<Vec<_>>()
        );
        let journal_antes = std::fs::read(journal_de(dir.path(), id)).expect("el journal en disco");
        drop(ws);

        let ws2 = Workspace::open(dir.path()).unwrap();
        ws2.recover()
            .expect_err("la copia ilegible debe hacer fallar la recuperación de esa transacción");

        // Se movió: en su sitio original ya no queda nada (por eso la siguiente operación procede).
        assert!(
            !journal_de(dir.path(), id).exists(),
            "el journal en cuarentena sale de `journal/<txnId>.json`: si se quedara, el gate de \
             `recovery_pending` seguiría cerrado"
        );
        assert!(
            !recovery_de(dir.path(), id).exists(),
            "y su árbol sale de `recovery/<txnId>/`"
        );

        // Y está completo dentro de la cuarentena, byte a byte.
        let cuarentena = cuarentena_de(dir.path(), id);
        assert!(
            cuarentena.is_dir(),
            "la cuarentena debe existir en {}",
            cuarentena.display()
        );
        let en_cuarentena = ficheros_bajo(&cuarentena);
        for (rel, bytes) in &arbol_antes {
            assert!(
                en_cuarentena
                    .iter()
                    .any(|(k, v)| k.ends_with(rel) && v == bytes),
                "la copia `{rel}` ({} bytes) debe seguir íntegra dentro de la cuarentena: nada se \
                 borra ahí, es material forense. Contenido de la cuarentena: {:?}",
                bytes.len(),
                en_cuarentena.keys().collect::<Vec<_>>()
            );
        }
        assert!(
            en_cuarentena.values().any(|v| v == &journal_antes),
            "y el JSON del journal, tal cual estaba, también: sin él no se puede saber qué \
             pretendía la transacción. Contenido de la cuarentena: {:?}",
            en_cuarentena.keys().collect::<Vec<_>>()
        );
    }

    /// **E25-H02** · Criterio 10 (**control anti-vacuo declarado; pasa YA HOY y debe seguir
    /// pasando**) — **Dado** una caída **entre el primer rename y su anotación en el journal**
    /// (journal `prepared`, cero entradas `applied`, **un rename ya hecho en disco**), **Cuando** se
    /// recupera, **Entonces** SÍ se restaura.
    ///
    /// Es el guardia de la generalización que la spec prohíbe explícitamente: *«`recover()` no
    /// restaura un journal `prepared` con cero entradas `applied`»*. `mark_applied` re-persiste el
    /// journal **después** de cada rename (`publish.rs:206`), así que «cero `applied` durables»
    /// describe también este estado — sellar por esa inferencia daría por buena una publicación
    /// parcial, que es justo lo que la recuperación existe para impedir. El sellado del aborto de
    /// ventana tiene que decidirlo el camino que **sabe** que no publicó, no una lectura del journal
    /// a posteriori.
    #[test]
    fn cero_applied_no_significa_cero_renames() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);
        let original = canonical_md(dir.path());

        let id = "e25-h02-cero-applied";
        let cs = cs_modifica(&ws, id, &["uno.md", "dos.md", "tres.md"]);
        // 1 rename hecho y NO anotado: el journal se queda en `prepared` con todo `pending`.
        let (afectados, resultado) = transaccion_interrumpida(&ws, dir.path(), id, &cs, 1, false);
        let publicado = afectados[0].as_str().to_string();

        let journal = leer_journal(&journal_de(dir.path(), id));
        assert_eq!(
            journal["state"].as_str(),
            Some("prepared"),
            "precondición: el journal quedó `prepared` (la anotación del rename no llegó a disco)"
        );
        assert_eq!(
            estado_op(&journal, &publicado),
            "pending",
            "precondición: cero entradas `applied` en el journal…"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join(&publicado)).unwrap(),
            como_md(&resultado)[&publicado],
            "…y sin embargo el rename YA está hecho en disco: es la caída entre el rename y su \
             anotación"
        );
        drop(ws);

        let ws2 = Workspace::open(dir.path()).unwrap();
        ws2.recover().expect("la recuperación debe restaurar");

        assert_eq!(
            canonical_md(dir.path()),
            original,
            "un journal `prepared` con cero `applied` PUEDE tener renames hechos: hay que \
             restaurarlo. Sellarlo por «no publicó nada» daría por buena una publicación parcial"
        );
        assert!(!ws2.recovery_pending(), "y tras restaurar queda sellado");
    }

    /// Ruta del **sidecar de huellas** de una transacción (E25-H02):
    /// `.lodestar/runtime/recovery/<txnId>.digests.json`, hermano del árbol de copias.
    fn sidecar_de(root: &Path, txn_id: &str) -> PathBuf {
        root.join(".lodestar")
            .join("runtime")
            .join("recovery")
            .join(format!("{txn_id}.digests.json"))
    }

    /// **E25-H02** · Criterio 1, segunda mitad (**cobertura pedida por el juez ciego**) — **Dado** un
    /// journal `prepared` cuya copia de recuperación está **corrupta pero del mismo tamaño**,
    /// **Cuando** se recupera, **Entonces** el canónico no se sobrescribe con ella y el fallo se
    /// reporta con `RECOVERY_FAILED`.
    ///
    /// `copia_truncada_no_se_restaura_verbatim` se contenta con una verificación de **tamaño**: una
    /// implementación que solo compare bytes-en-disco contra bytes-respaldados lo pasa. Este
    /// escenario cierra ese hueco — la copia tiene EXACTAMENTE los mismos bytes de longitud y sigue
    /// siendo UTF-8 válido, así que solo la **revisión blake3** del contenido la distingue del
    /// original. Es el caso realista de una corrupción en el medio físico (un bit/byte cambiado sin
    /// cambiar el tamaño del fichero), que es justo lo que un `std::fs::copy` sin volcado no
    /// protege.
    ///
    /// Mata la mutación «verificar solo el tamaño»: neutralizando la comparación de `revision` en
    /// `restore_backups`, la copia corrupta pasa la verificación, se restaura, `recover()` devuelve
    /// `Ok` y este test se pone rojo.
    #[test]
    fn copia_corrupta_del_mismo_tamano_no_se_restaura() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);
        let original = canonical_md(dir.path());

        let id = "e25-h02-copia-corrupta-mismo-tamano";
        let cs = cs_modifica(&ws, id, &["uno.md", "dos.md", "tres.md"]);
        // Un rename hecho y anotado (journal `applying`): la restauración es NECESARIA.
        let (afectados, resultado) = transaccion_interrumpida(&ws, dir.path(), id, &cs, 1, true);
        let publicado = afectados[0].as_str().to_string();
        let esperado = como_md(&resultado);

        // La copia se corrompe SIN cambiar su tamaño y siguiendo siendo texto válido: un byte del
        // cuerpo cambia («original» → «0riginal»). Ni el tamaño ni la legibilidad delatan nada.
        let copia = recovery_de(dir.path(), id).join(&publicado);
        let intacta = std::fs::read_to_string(&copia).expect("la copia debe existir y ser texto");
        let corrupta = intacta.replacen("cuerpo original", "cuerpo 0riginal", 1);
        assert_eq!(
            corrupta.len(),
            intacta.len(),
            "precondición del escenario: la corrupción NO cambia el tamaño (si lo cambiara, una \
             verificación de solo tamaño ya la vería y el test no probaría nada nuevo)"
        );
        assert_ne!(
            corrupta, intacta,
            "precondición: y sí cambia el contenido (si no, no hay corrupción que detectar)"
        );
        std::fs::write(&copia, &corrupta).unwrap();
        drop(ws);

        let ws2 = Workspace::open(dir.path()).unwrap();
        let err = match ws2.recover() {
            Err(e) => e,
            Ok(()) => panic!(
                "una copia corrupta del mismo tamaño NO puede pasar por original: comparar solo el \
                 tamaño (o no comparar nada) publica contenido alterado firmándolo como «el estado \
                 anterior». El canónico de `{publicado}` quedó: {:?}",
                canonical_md(dir.path()).get(&publicado)
            ),
        };
        assert_eq!(
            err.code(),
            "RECOVERY_FAILED",
            "y el fallo lleva su código propio del catálogo; era: {err:?}"
        );

        let despues = canonical_md(dir.path());
        let contenido = despues
            .get(&publicado)
            .unwrap_or_else(|| panic!("`{publicado}` debe seguir existiendo en el canónico"));
        assert_ne!(
            contenido, &corrupta,
            "el canónico no puede quedar con el contenido corrupto de la copia"
        );
        assert!(
            contenido == &original[&publicado] || contenido == &esperado[&publicado],
            "y tiene que ser uno de los dos bordes íntegros de la transacción.\nen disco: \
             {contenido:?}\noriginal: {:?}\nresultado: {:?}",
            original[&publicado],
            esperado[&publicado]
        );
        assert!(
            cuarentena_de(dir.path(), id).is_dir(),
            "el material de la transacción irrecuperable va a cuarentena: {}",
            cuarentena_de(dir.path(), id).display()
        );
        assert!(
            !ws2.recovery_pending(),
            "y el workspace vuelve a ser escribible"
        );
    }

    /// **E25-H02** · Camino de migración (**cobertura pedida por el juez ciego**) — **Dado** un árbol
    /// de recuperación válido **sin sidecar de huellas** (una transacción respaldada por ≤ v0.3.1),
    /// **Cuando** se recupera, **Entonces** se **restaura** como en v0.3.1: el canónico converge al
    /// borde «original», sin cuarentena, y el workspace queda escribible.
    ///
    /// Es el estado que un journal escrito por una versión anterior deja en disco tras actualizar el
    /// binario, y hoy no lo ejercía nadie: si la restauración sin huellas fuera un no-op —o si el
    /// camino sin sidecar mandara a cuarentena por no poder verificar—, una actualización de
    /// Lodestar convertiría una transacción interrumpida perfectamente recuperable en un estado
    /// parcial sellado (o en material forense inútil), justo al reabrir.
    ///
    /// Mata la mutación «`restore_backups_legacy` como no-op»: sin restaurar nada, el canónico se
    /// queda con el rename publicado y la comparación contra el borde original falla.
    #[test]
    fn arbol_sin_sidecar_restaura_como_v031() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);
        let original = canonical_md(dir.path());

        let id = "e25-h02-sin-sidecar";
        let cs = cs_modifica(&ws, id, &["uno.md", "dos.md", "tres.md"]);
        // Copias íntegras y un rename ya hecho y anotado: hay algo que deshacer y se puede deshacer.
        let (afectados, resultado) = transaccion_interrumpida(&ws, dir.path(), id, &cs, 1, true);
        let publicado = afectados[0].as_str().to_string();
        assert_eq!(
            std::fs::read_to_string(dir.path().join(&publicado)).unwrap(),
            como_md(&resultado)[&publicado],
            "precondición: el rename se publicó, así que la restauración tiene trabajo que hacer"
        );

        // Se simula el árbol de una versión anterior a E25-H02: existe el árbol de copias, pero no
        // hay sidecar de huellas con el que verificar.
        let sidecar = sidecar_de(dir.path(), id);
        assert!(
            sidecar.is_file(),
            "precondición: la transacción actual sí escribe su sidecar de huellas en {}",
            sidecar.display()
        );
        std::fs::remove_file(&sidecar).unwrap();
        assert!(
            recovery_de(dir.path(), id).is_dir(),
            "y el árbol de copias sigue en su sitio: es lo único que dejaba v0.3.1"
        );
        drop(ws);

        let ws2 = Workspace::open(dir.path()).unwrap();
        ws2.recover().unwrap_or_else(|e| {
            panic!(
                "un árbol legítimo sin huellas NO es un fallo de recuperación: se restaura como en \
                 v0.3.1 (nunca peor que el comportamiento que E25-H02 endurece). Era: {e:?}"
            )
        });

        assert_eq!(
            canonical_md(dir.path()),
            original,
            "el canónico tiene que converger al borde «original»: las copias estaban íntegras y el \
             único motivo para no restaurarlas sería no poder verificarlas, que es exactamente lo \
             que v0.3.1 nunca hizo"
        );
        assert!(
            !cuarentena_de(dir.path(), id).exists(),
            "y no hay nada que poner en cuarentena: la transacción se recuperó, no falló"
        );
        assert!(
            !journal_de(dir.path(), id).exists(),
            "el journal queda sellado (borrado)"
        );
        assert!(
            !recovery_de(dir.path(), id).exists(),
            "y su árbol de copias limpio"
        );
        assert!(
            !ws2.recovery_pending(),
            "así que el workspace vuelve a ser escribible sin intervención"
        );
    }
}

#[cfg(feature = "test-failpoints")]
mod aborto_de_ventana {
    use super::*;
    use lodestar_workspace::failpoints::{self, FailPoint, PuntoDeGancho};

    /// El escenario común de los cuatro: un apply que va a modificar los tres documentos y un
    /// gancho que, en el último instante de la ventana `[T1, T3)`, hace de «otro proceso». Devuelve
    /// `(tempdir, workspace, canónico de T1, contenido de la edición externa)`.
    fn aborto_por_edicion_externa(
        id: &'static str,
    ) -> (
        tempfile::TempDir,
        Workspace,
        BTreeMap<String, String>,
        &'static str,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);
        let antes = canonical_md(dir.path());
        let cs = cs_modifica(&ws, id, &["uno.md", "dos.md", "tres.md"]);

        let edicion_externa = "---\ntype: Nota\ntitle: dos\n---\n\n# dos\n\nEDICIÓN EXTERNA\n";
        {
            let ruta = dir.path().join("dos.md");
            failpoints::armar_gancho(PuntoDeGancho::AntesDePublicar, move || {
                std::fs::write(&ruta, edicion_externa).expect("el gancho debe poder editar dos.md");
            });
        }
        let resultado = ws.apply_transaction(&cs);
        failpoints::desarmar_ganchos();
        let err = match resultado {
            Err(e) => e,
            Ok((_, _, changed)) => panic!(
                "una edición externa en la ventana `[T1, T3)` debe abortar la transacción: \
                 changedPaths={changed:?}"
            ),
        };
        assert_eq!(
            err.code(),
            "WRITE_CONFLICT",
            "el aborto de ventana es un WRITE_CONFLICT (E25-H01); era: {err:?}"
        );
        (dir, ws, antes, edicion_externa)
    }

    /// **E25-H02** · Criterio 6 — **Dado** un `WRITE_CONFLICT` de ventana (con la edición externa ya
    /// en disco), **Cuando** la siguiente operación abre el workspace y recupera, **Entonces** la
    /// edición externa sigue **intacta** byte a byte.
    ///
    /// Hoy la recuperación la sobrescribe con la copia de T1: el journal `prepared` y su árbol
    /// sobreviven al aborto, `recover()` los clasifica como «renames parciales» y restaura. Es el
    /// defecto de E25-H01 con un rodeo — en vez de pisar la edición al publicar, la pisa al
    /// recuperar, una operación más tarde y sin que nadie lo pida.
    #[test]
    fn el_aborto_de_ventana_no_deja_recuperacion_que_pise_la_edicion() {
        let id = "e25-h02-aborto-no-pisa";
        let (dir, ws, antes, edicion_externa) = aborto_por_edicion_externa(id);
        drop(ws);

        // La siguiente operación: reabre y recupera (el paso (2) de toda transacción).
        let ws2 = Workspace::open(dir.path()).unwrap();
        ws2.recover()
            .expect("la recuperación de la siguiente operación no puede fallar");

        assert_eq!(
            std::fs::read_to_string(dir.path().join("dos.md")).unwrap(),
            edicion_externa,
            "la edición externa que el aborto protegió tiene que seguir intacta byte a byte: si la \
             recuperación escribe la copia de T1 encima, el aborto no protegió nada — solo retrasó \
             la pérdida una operación"
        );
        let mut esperado = antes.clone();
        esperado.insert("dos.md".to_string(), edicion_externa.to_string());
        assert_eq!(
            canonical_md(dir.path()),
            esperado,
            "y ningún otro `.md` puede haberse movido: no hubo un solo rename que deshacer"
        );
    }

    /// **E25-H02** · Criterio 7 — **Dado** ese mismo `WRITE_CONFLICT` de ventana con un `.md`
    /// **creado por el usuario** dentro de la ventana, **Cuando** la siguiente operación recupera,
    /// **Entonces** ese fichero **sigue existiendo**.
    ///
    /// El plan iba a crear `x.md`, así que el manifiesto `.absent` lo marca «no existía»; el usuario
    /// crea ese mismo path en la ventana y el aborto lo respeta (E25-H01), pero la restauración de
    /// la siguiente operación lo **borra** (`recovery.rs:331-333`): la garantía de
    /// `fichero_nuevo_en_la_ventana_no_se_borra`, deshecha una operación más tarde.
    #[test]
    fn el_aborto_de_ventana_no_borra_el_fichero_nuevo_al_recuperar() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos"]);
        let antes = canonical_md(dir.path());

        let id = "e25-h02-aborto-fichero-nuevo";
        let mut cs = change_set(id, vec![create_conforme("x.md", "Nota", "x")]);
        cs.base_revision = ws.workspace_revision().unwrap();

        let del_usuario = "---\ntype: Nota\ntitle: x\n---\n\n# x\n\nESTO LO ESCRIBIÓ EL USUARIO\n";
        {
            let root = dir.path().to_path_buf();
            let ruta = dir.path().join("x.md");
            failpoints::armar_gancho(PuntoDeGancho::AntesDePublicar, move || {
                // Precondición del escenario, aseverada en el único instante en que ese estado
                // existe: el plan marcó `x.md` como «no existía» en el manifiesto `.absent`, que es
                // lo que hace peligrosa la restauración posterior (la marca dice «bórralo»).
                let manifiesto = recovery_de(&root, id).join(".absent");
                assert!(
                    std::fs::read_to_string(&manifiesto)
                        .unwrap_or_default()
                        .lines()
                        .any(|l| l.trim() == "x.md"),
                    "el manifiesto `.absent` de la transacción debe marcar `x.md` como «no \
                     existía»: {}",
                    manifiesto.display()
                );
                assert!(
                    !ruta.exists(),
                    "y el path no puede existir aún: lo crea el usuario DENTRO de la ventana"
                );
                std::fs::write(&ruta, del_usuario).expect("el gancho debe poder crear x.md");
            });
        }
        let resultado = ws.apply_transaction(&cs);
        failpoints::desarmar_ganchos();
        let err = resultado.expect_err("el canónico cambió en la ventana: debe abortar");
        assert_eq!(
            err.code(),
            "WRITE_CONFLICT",
            "el aborto de ventana es un WRITE_CONFLICT (E25-H01); era: {err:?}"
        );
        drop(ws);

        let ws2 = Workspace::open(dir.path()).unwrap();
        ws2.recover()
            .expect("la recuperación de la siguiente operación no puede fallar");

        assert!(
            dir.path().join("x.md").is_file(),
            "el `.md` que creó el usuario dentro de la ventana NO puede desaparecer al recuperar: \
             el manifiesto `.absent` dice «no existía en T1», pero la transacción que lo escribió \
             fue abortada y nunca creó nada — borrarlo es destruir un fichero ajeno sin copia"
        );
        assert_eq!(
            std::fs::read_to_string(dir.path().join("x.md")).unwrap(),
            del_usuario,
            "y con su contenido intacto"
        );
        let mut esperado = antes.clone();
        esperado.insert("x.md".to_string(), del_usuario.to_string());
        assert_eq!(
            canonical_md(dir.path()),
            esperado,
            "y sin tocar nada más del canónico"
        );
    }

    /// **E25-H02** · Criterio 8 — **Dado** ese mismo aborto, **Cuando** termina, **Entonces**
    /// `recovery_pending()` es `false` y no queda ni journal ni árbol de recuperación de esa
    /// transacción.
    ///
    /// El aborto sabe por control de flujo que **no ha entrado en el bucle de renames**, así que
    /// sellar es exacto, no una amnistía: cero renames significa que el canónico nunca se movió. El
    /// journal se borra **primero** (es lo que levanta el gate de `recovery_pending`) y el árbol
    /// después.
    #[test]
    fn el_aborto_de_ventana_no_deja_recuperacion_pendiente() {
        let id = "e25-h02-aborto-sin-pendiente";
        let (dir, ws, _antes, _edicion) = aborto_por_edicion_externa(id);

        assert!(
            !ws.recovery_pending(),
            "el aborto sella su propia transacción bajo el mismo lock: al devolver el \
             WRITE_CONFLICT no puede quedar recuperación pendiente (si queda, la siguiente \
             operación restaurará copias de un estado que nunca se publicó)"
        );
        assert!(
            !journal_de(dir.path(), id).exists(),
            "no puede quedar el fichero de journal en {}",
            journal_de(dir.path(), id).display()
        );
        assert!(
            !recovery_de(dir.path(), id).exists(),
            "ni el árbol de recuperación en {}",
            recovery_de(dir.path(), id).display()
        );

        drop(ws);
        let ws2 = Workspace::open(dir.path()).unwrap();
        assert!(
            !ws2.recovery_pending(),
            "y al reabrir tampoco (el sellado es durable, no un estado en memoria)"
        );
    }

    /// **E25-H02** · Criterio 9 — **Dado** un aborto de ventana interrumpido **entre** el borrado
    /// del journal y el del árbol, **Cuando** se reabre y corre el GC, **Entonces** no hay
    /// recuperación pendiente y el huérfano se purga.
    ///
    /// Es la razón por la que el journal va **primero**: si el proceso muere entre los dos borrados
    /// queda un árbol de recuperación **sin journal**, que es un huérfano legítimo y lo recoge el GC
    /// (E24-H06). Al revés —árbol primero— quedaría un journal apuntando a copias que ya no están, y
    /// la recuperación sellaría un estado parcial en silencio.
    ///
    /// Usa el punto de caída **nuevo** que esta historia añade a la taxonomía
    /// (`FailPoint::EnMedioDelSelladoDelAborto`, ver la cabecera de la sección). Con él armado, el
    /// error que sale de `apply_transaction` es el del failpoint y no el `WRITE_CONFLICT`: el test
    /// solo exige `is_err()`.
    #[test]
    fn el_sellado_del_aborto_es_seguro_a_mitad() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);
        let antes = canonical_md(dir.path());

        let id = "e25-h02-sellado-a-mitad";
        let cs = cs_modifica(&ws, id, &["uno.md", "dos.md", "tres.md"]);

        let edicion_externa = "---\ntype: Nota\ntitle: dos\n---\n\n# dos\n\nEDICIÓN EXTERNA\n";
        {
            let ruta = dir.path().join("dos.md");
            failpoints::armar_gancho(PuntoDeGancho::AntesDePublicar, move || {
                std::fs::write(&ruta, edicion_externa).expect("el gancho debe poder editar dos.md");
            });
        }
        failpoints::armar(FailPoint::EnMedioDelSelladoDelAborto);
        let resultado = ws.apply_transaction(&cs);
        failpoints::desarmar();
        failpoints::desarmar_ganchos();
        assert!(
            resultado.is_err(),
            "la transacción abortada por la ventana no puede reportar éxito"
        );

        // El punto de caída está ENTRE los dos borrados: journal ya fuera, árbol todavía dentro.
        assert!(
            !journal_de(dir.path(), id).exists(),
            "el sellado del aborto borra el journal PRIMERO (es lo que levanta el gate de \
             `recovery_pending`): en {} no puede quedar nada",
            journal_de(dir.path(), id).display()
        );
        assert!(
            recovery_de(dir.path(), id).is_dir(),
            "y el failpoint interrumpe justo después, con el árbol aún en disco: si ya no está, el \
             punto se colocó en el sitio equivocado y el test no ejerce el borde que le importa"
        );

        drop(ws);
        let ws2 = Workspace::open(dir.path()).unwrap();
        assert!(
            !ws2.recovery_pending(),
            "un árbol de recuperación sin journal NO es una recuperación pendiente: por eso el \
             journal se borra primero"
        );
        ws2.gc_receipts().expect("el GC debe correr");
        assert!(
            !recovery_de(dir.path(), id).exists(),
            "y el árbol huérfano (sin journal ni recibo) lo purga el GC de E24-H06"
        );

        let mut esperado = antes.clone();
        esperado.insert("dos.md".to_string(), edicion_externa.to_string());
        assert_eq!(
            canonical_md(dir.path()),
            esperado,
            "y el canónico sigue con la edición externa y sin un solo rename publicado"
        );
    }
}

// ---------------------------------------------------------------------------
// E25-H03 — El GC no destruye el plano de recuperación de una transacción viva
// (`requirements/epica-25-endurecimiento-escritura.md`, bloque E25-H03). Fase ROJA.
//
// EL DEFECTO (S3)
//
// `gc_receipts` se invoca DESPUÉS de que la transacción suelte el lock (`lodestar-app/src/lib.rs`,
// tras `apply_transaction`/`revert_transaction_con_recibo`), y `gc_runtime_huerfanos`
// (`src/receipts.rs:314-360`) purga TODO directorio de `staging/`/`recovery/` —y todo sidecar
// `recovery/<txn>.digests.json`— cuyo stem no aparezca ni en `journal/` ni en `receipts/`. Ese
// criterio es correcto con un solo proceso y FALSO con dos: entre `backup_originals`
// (`transaction.rs:162`) y `create_journal` (`:167`) hay una ventana —la que modela
// `FailPoint::TrasBackupSinJournal`— en la que la transacción tiene copias y NO tiene ni journal ni
// recibo. Un `change_apply` de otro proceso que termine en ese instante lanza su GC y **borra el
// árbol de recuperación de la transacción viva**: esa publica sin copias y, si cae,
// `restore_from_recovery` no encuentra directorio, devuelve `Ok(())` de inmediato
// (`recovery.rs:323-325`) y la recuperación **sella un estado parcial en silencio**.
//
// EL CONTRATO QUE FIJAN ESTOS TESTS (sin fijar el mecanismo)
//
// La spec deja al implementador elegir entre (1) GC bajo el lock de publicación —fail-fast: si no lo
// consigue, no barre y devuelve `Ok(())`— y (2) marca durable de «transacción en curso» creada antes
// del backup y respetada por el GC como tercer conjunto de vivos, con dueño (pid/host) y criterio de
// rancidez. Por eso estos tests aseveran **solo efectos en disco**:
//
// - con una transacción VIVA en la ventana, su árbol de recuperación y su sidecar sobreviven a un GC
//   lanzado desde otro handle sobre la misma raíz (criterio 1);
// - con el dueño MUERTO, el mismo material se purga (criterio 2, control anti-vacuo: el arreglo no
//   puede consistir en dejar de barrer, ni en dejar basura inmortal);
// - una transacción terminada —con éxito o con `Err`— no deja NINGÚN rastro de «en curso» bajo
//   `.lodestar/runtime/` (criterio 3): la marca muere con la transacción, así que el huérfano que
//   deja un aborto en la ventana sigue siendo basura recogible (es lo que mantiene verde
//   `seam_real::caida_entre_backup_y_journal` sin tocarlo);
// - un GC que no puede barrer (lock tomado, señal de propiedad ilegible) devuelve `Ok(())` y no
//   altera lo publicado (criterio 4).
//
// EL SEAM QUE HACE FALTA (declarado en la salida de la fase roja; aquí solo se USA)
//
// `FailPoint::TrasBackupSinJournal` modela ese punto **abortando**, y para reproducir el defecto hace
// falta lo contrario: que la transacción se quede ahí CONGELADA Y VIVA mientras otro handle barre.
// Se reusa por tanto el gancho *ejecuta-y-espera* de E25-H01 con un punto nuevo:
//
// ```rust
// pub enum PuntoDeGancho {
//     AntesDePublicar,   // E25-H01
//     /// Dentro de la ventana `[backup, journal)`: tras `backup_originals` y ANTES de
//     /// `create_journal`. El gancho del test bloquea en un canal y la transacción CONTINÚA al
//     /// liberarlo.
//     TrasElBackup,      // E25-H03
// }
// ```
//
// El gancho es `thread_local` (a propósito: los tests corren en paralelo en el mismo proceso), así
// que la transacción congelada corre en un **hilo propio** con el gancho armado EN ESE HILO — es lo
// que hace `congelar_en_la_ventana`.
//
// ROJO ESPERADO HOY
// - `gc_no_destruye_una_transaccion_en_curso_de_otro_proceso`: ROJO. El GC del segundo handle borra
//   el árbol de recuperación y el sidecar de la transacción viva.
// - `gc_sigue_purgando_huerfanos_de_dueno_muerto`, `la_marca_no_sobrevive_a_la_transaccion` y
//   `el_gc_nunca_tumba_a_quien_lo_llama`: **controles anti-vacuos**, pasan ya hoy y tienen que
//   seguir pasando. Hoy pasan trivialmente (el GC barre siempre y no mira ningún lock, y no existe
//   marca alguna que pueda sobrevivir); lo que prohíben es que el arreglo del criterio 1 se pague
//   con basura inmortal, con un GC que devuelva `Err` cuando el lock está tomado, o con una marca
//   que sobreviva a su transacción.
// ---------------------------------------------------------------------------

#[cfg(feature = "test-failpoints")]
mod gc_y_transacciones_vivas {
    use super::*;
    use lodestar_workspace::failpoints::{self, FailPoint, PuntoDeGancho};
    use std::sync::mpsc;

    /// Límite de cualquier rendez-vous con el hilo de la transacción congelada, y del propio GC.
    /// Solo tiene que distinguir «colgado» (infinito) de «lento», así que se elige **muy** holgado:
    /// un runner de CI cargado puede tardar segundos en lo que en local tarda milisegundos, y un
    /// valor ajustado convertiría una aserción de robustez en un test frágil.
    const LIMITE: Duration = Duration::from_secs(120);

    /// El plano de control de la sesión (`.lodestar/runtime/`), exista o no.
    fn runtime_de(root: &Path) -> PathBuf {
        root.join(".lodestar").join("runtime")
    }

    /// El sidecar de huellas de las copias de una transacción (E25-H02),
    /// `recovery/<txnId>.digests.json`: **hermano** del árbol, nunca dentro de él. Vive y muere con
    /// él, así que el GC lo juzga con el mismo criterio de propiedad.
    fn sidecar_de(root: &Path, txn_id: &str) -> PathBuf {
        runtime_de(root)
            .join("recovery")
            .join(format!("{txn_id}.digests.json"))
    }

    /// El árbol de staging de una transacción (`staging/<txnId>/`), exista o no.
    fn staging_de(root: &Path, txn_id: &str) -> PathBuf {
        runtime_de(root).join("staging").join(txn_id)
    }

    /// Todo lo que queda bajo `.lodestar/runtime/` que **no** es el material de recuperación de
    /// `txn_id` (su árbol `recovery/<txn>/…` y su sidecar `recovery/<txn>.digests.json`, que el
    /// sellado conserva a propósito porque `change_revert` los necesita).
    ///
    /// Cualquier otra cosa que sobreviva a la transacción —un fichero de lock, una marca de «en
    /// curso», un staging sin limpiar— es un rastro que no puede quedar: el GC de otro proceso lo
    /// leería como «hay alguien publicando» y el material quedaría **inmortal**, que es cambiar un
    /// defecto por otro.
    fn resto_del_plano_de_control(root: &Path, txn_id: &str) -> BTreeMap<String, Vec<u8>> {
        let arbol = format!("recovery/{txn_id}/");
        let sidecar = format!("recovery/{txn_id}.digests.json");
        ficheros_bajo(&runtime_de(root))
            .into_iter()
            .filter(|(rel, _)| !rel.starts_with(&arbol) && *rel != sidecar)
            .collect()
    }

    /// Un pid que con certeza **no** corresponde a ningún proceso vivo: se arranca el propio binario
    /// de test con `--list` (enumera los tests y sale 0 sin ejecutar ninguno) y se espera a que
    /// muera. Solo se usa en Unix, donde la prueba de vida por pid forma parte del criterio del
    /// lock; Windows debe ejercer el camino portable por TTL que cubre el caso siguiente.
    #[cfg(unix)]
    fn pid_inexistente() -> u32 {
        let exe = std::env::current_exe().expect("ruta del binario de test");
        let mut hijo = std::process::Command::new(exe)
            .arg("--list")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("arrancar el proceso sonda");
        let pid = hijo.id();
        hijo.wait().expect("esperar al proceso sonda");
        pid
    }

    /// Ejecuta `gc_receipts()` de `ws` con **límite de tiempo**: el modelo es fail-fast (`§19.5`),
    /// así que un GC que se quedara esperando un lock es un defecto, y el test tiene que decirlo en
    /// vez de dejar el CI colgado. Devuelve el resultado tal cual (el GC es best-effort: se espera
    /// `Ok(())` incluso cuando no puede barrer).
    fn gc_con_limite(ws: Workspace) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let r = ws.gc_receipts().map_err(|e| format!("{}: {e:?}", e.code()));
            let _ = tx.send(r);
        });
        match rx.recv_timeout(LIMITE) {
            Ok(r) => r,
            Err(_) => panic!(
                "el GC del plano de control no puede BLOQUEAR esperando el lock de publicación: el \
                 modelo es fail-fast; si no consigue barrer, no barre y devuelve Ok(())"
            ),
        }
    }

    /// Un `apply_transaction` **congelado y vivo** dentro de la ventana `[backup, journal)`.
    ///
    /// Corre en un **hilo propio** porque el gancho es `thread_local`: armarlo en el hilo del test no
    /// afectaría a la transacción. Mientras este valor existe, la transacción está detenida en el
    /// punto donde tiene copias de recuperación y todavía no tiene journal — el estado exacto que el
    /// GC de otro proceso confunde hoy con basura.
    struct VentanaCongelada {
        hilo: Option<std::thread::JoinHandle<Result<Vec<RelPath>, String>>>,
        liberar: Option<mpsc::Sender<()>>,
    }

    impl VentanaCongelada {
        /// Deja que la transacción continúe y espera su resultado (`changedPaths` si publicó).
        fn liberar(mut self) -> Result<Vec<RelPath>, String> {
            self.liberar
                .take()
                .expect("la ventana solo se libera una vez")
                .send(())
                .expect("el hilo de la transacción debe seguir esperando en la ventana");
            self.hilo
                .take()
                .expect("el hilo de la transacción solo se espera una vez")
                .join()
                .expect("el hilo de la transacción no puede paniquear")
        }
    }

    impl Drop for VentanaCongelada {
        /// Si una aserción del test falla **antes** de liberar la ventana, el hilo de la transacción
        /// se queda esperando en el canal: al dropear el emisor recibiría `Disconnected` y paniquearía,
        /// sepultando el fallo real bajo un segundo panic en un hilo secundario. Aquí se libera y se
        /// espera, siempre en modo best-effort (un `Drop` que paniquea durante un desenrollado aborta
        /// el proceso).
        fn drop(&mut self) {
            if let Some(tx) = self.liberar.take() {
                let _ = tx.send(());
            }
            if let Some(hilo) = self.hilo.take() {
                let _ = hilo.join();
            }
        }
    }

    /// Arranca `ws.apply_transaction(&cs)` en un hilo propio y **espera a que quede congelado**
    /// dentro de la ventana `[backup, journal)`. Al volver, la transacción está viva y detenida ahí.
    fn congelar_en_la_ventana(ws: Workspace, cs: ChangeSet) -> VentanaCongelada {
        let (tx_dentro, rx_dentro) = mpsc::channel::<()>();
        let (tx_liberar, rx_liberar) = mpsc::channel::<()>();

        let hilo = std::thread::spawn(move || {
            failpoints::armar_gancho(PuntoDeGancho::TrasElBackup, move || {
                tx_dentro
                    .send(())
                    .expect("avisar al test de que la ventana está abierta");
                rx_liberar
                    .recv_timeout(LIMITE)
                    .expect("el test debe liberar la ventana");
            });
            let r = ws.apply_transaction(&cs);
            failpoints::desarmar_ganchos();
            r.map(|(_, _, changed)| changed)
                .map_err(|e| format!("{}: {e:?}", e.code()))
        });

        if let Err(e) = rx_dentro.recv_timeout(LIMITE) {
            panic!(
                "la transacción debe alcanzar la ventana `[backup, journal)` y quedarse ahí: el \
                 gancho `PuntoDeGancho::TrasElBackup` —tras `backup_originals` y ANTES de \
                 `create_journal`— no se disparó ({e})"
            );
        }
        VentanaCongelada {
            hilo: Some(hilo),
            liberar: Some(tx_liberar),
        }
    }

    /// **E25-H03** · Criterio 1 — **Dado** un proceso A detenido en la ventana `[backup, journal)`,
    /// **Cuando** otro handle sobre la MISMA raíz ejecuta el GC, **Entonces** el árbol de
    /// recuperación de A (y su sidecar) **sigue intacto**.
    ///
    /// Hoy lo borra: `gc_runtime_huerfanos` decide «vivo» por presencia en `journal/` ∪ `receipts/`,
    /// y en esa ventana la transacción no está en ninguno de los dos aunque esté publicando. A
    /// publica entonces sin copias, y si cae, la recuperación no encuentra directorio, devuelve
    /// `Ok(())` y sella un estado parcial en silencio.
    ///
    /// El GC se lanza desde un **segundo `Workspace` sobre la misma raíz**, que es el «otro
    /// proceso» del escenario: por eso la señal que protege a A tiene que ser **durable** (un
    /// fichero de lock o una marca en disco), no un estado en memoria del handle que publica.
    #[test]
    fn gc_no_destruye_una_transaccion_en_curso_de_otro_proceso() {
        let dir = tempfile::tempdir().unwrap();
        let ws_a = siembra_documentos(dir.path(), &["uno", "dos", "tres"]);
        let antes = canonical_md(dir.path());

        let id = "e25-h03-en-curso";
        let cs = cs_modifica(&ws_a, id, &["uno.md", "dos.md", "tres.md"]);
        let ventana = congelar_en_la_ventana(ws_a, cs);

        // Precondición del escenario: A está DENTRO de la ventana — copias listas, journal ausente.
        // Si el gancho se disparara en otro punto, el defecto no sería ni posible (con journal en
        // disco el GC ya considera viva la transacción) y el test sería vacuo.
        assert!(
            recovery_de(dir.path(), id).is_dir(),
            "el gancho debe dispararse DESPUÉS de `backup_originals`: falta el árbol {}",
            recovery_de(dir.path(), id).display()
        );
        assert!(
            sidecar_de(dir.path(), id).is_file(),
            "y con él el sidecar de huellas de E25-H02: falta {}",
            sidecar_de(dir.path(), id).display()
        );
        assert!(
            !journal_de(dir.path(), id).exists(),
            "y ANTES de `create_journal`: con journal en disco el GC ya vería la transacción como \
             viva y el escenario no reproduciría nada ({})",
            journal_de(dir.path(), id).display()
        );

        let respaldo_antes = ficheros_bajo(&recovery_de(dir.path(), id));
        assert!(
            !respaldo_antes.is_empty(),
            "precondición: las copias de recuperación de A no pueden estar vacías"
        );
        let sidecar_antes = std::fs::read(sidecar_de(dir.path(), id)).expect("leer el sidecar");

        // El «otro proceso»: un segundo handle sobre la misma raíz que barre el plano de control
        // mientras A publica (es lo que hace `App::change_apply` al terminar su propia transacción).
        let ws_b = Workspace::open(dir.path()).unwrap();
        gc_con_limite(ws_b).expect(
            "el GC es best-effort: nunca falla, ni cuando no puede barrer (criterio 4 de la \
             historia)",
        );

        // EL CRITERIO.
        assert_eq!(
            ficheros_bajo(&recovery_de(dir.path(), id)),
            respaldo_antes,
            "el GC de otro proceso NO puede tocar el árbol de recuperación de una transacción EN \
             CURSO: sin esas copias, A publica sin plano de recuperación y una caída posterior \
             sella un estado parcial en silencio (`restore_from_recovery` sin directorio devuelve \
             Ok de inmediato)"
        );
        assert_eq!(
            std::fs::read(sidecar_de(dir.path(), id)).unwrap_or_default(),
            sidecar_antes,
            "ni su sidecar de huellas: sin él, las copias que sí sobrevivan se restauran sin \
             verificar (E25-H02)"
        );
        assert!(
            staging_de(dir.path(), id).is_dir(),
            "ni el staging de la transacción viva, que es material del mismo lote en vuelo: {}",
            staging_de(dir.path(), id).display()
        );

        // A continúa y publica: el GC de otro proceso no puede alterar su resultado.
        let changed = ventana
            .liberar()
            .expect("la transacción congelada debe poder publicar al liberarla");
        assert_eq!(
            changed.len(),
            3,
            "A publica su lote completo: changedPaths={changed:?}"
        );

        let despues = canonical_md(dir.path());
        assert_ne!(
            despues, antes,
            "A publicó: el canónico tiene que haber cambiado"
        );
        assert!(
            despues.values().all(|c| c.contains("cuerpo NUEVO")),
            "y con el resultado del plan: {despues:?}"
        );
        // Corolario del criterio, medido al final: las copias que `change_revert` necesita siguen
        // completas. Son las mismas que el GC intruso se llevó a mitad de vuelo.
        assert_eq!(
            ficheros_bajo(&recovery_de(dir.path(), id)),
            respaldo_antes,
            "tras publicar, las copias de recuperación de la transacción siguen completas: son las \
             que hacen reversible lo que acaba de publicarse"
        );
    }

    /// **E25-H03** · Criterio 2 (**control anti-vacuo**) — **Dado** el mismo estado de la ventana
    /// pero con el dueño **muerto**, **Cuando** corre el GC, **Entonces** el material se purga.
    ///
    /// El arreglo del criterio 1 no puede consistir en dejar de barrer: si la señal de propiedad
    /// sobrevive a un crash y nadie la caduca, el material de esa ventana queda **inmortal** y se
    /// cambia un defecto por otro (la spec lo dice explícitamente: la marca se considera rancia con
    /// el mismo criterio de propiedad que el lock, `reclamar_si_huerfano`).
    ///
    /// Dos estados, los dos con dueño que ya no está:
    /// - **(a)** el que deja un aborto en la ventana: la transacción TERMINÓ (con `Err`), así que su
    ///   material es basura y no hay dueño que lo reclame;
    /// - **(b)** el que deja un **crash real** en la ventana: el mismo material MÁS la señal durable
    ///   de propiedad que el mecanismo escriba, con el pid de este proceso sustituido por uno
    ///   **inexistente**. La señal se **captura de una ventana de verdad** (no se fabrica a mano),
    ///   precisamente para no fijar el mecanismo: sea un fichero de lock o una marca de «en curso»,
    ///   se repone tal cual con el dueño cambiado.
    #[test]
    fn gc_sigue_purgando_huerfanos_de_dueno_muerto() {
        // ---- (a) la transacción terminó: su material es un huérfano sin dueño ----
        {
            let dir = tempfile::tempdir().unwrap();
            let ws = siembra_documentos(dir.path(), &["uno", "dos"]);
            let id = "e25-h03-huerfano-sin-dueno";
            let cs = cs_modifica(&ws, id, &["uno.md", "dos.md"]);

            failpoints::armar(FailPoint::TrasBackupSinJournal);
            let r = ws.apply_transaction(&cs);
            failpoints::desarmar();
            assert!(
                r.is_err(),
                "el failpoint de la ventana debe abortar la transacción (si no, el test es vacuo)"
            );
            assert!(
                recovery_de(dir.path(), id).is_dir(),
                "precondición: el aborto deja el árbol de recuperación en disco"
            );

            let ws_b = Workspace::open(dir.path()).unwrap();
            gc_con_limite(ws_b).expect("el GC nunca falla");

            assert!(
                !recovery_de(dir.path(), id).exists(),
                "la transacción TERMINÓ: nadie va a publicar con esas copias, no hay journal ni \
                 recibo, y su dueño no existe. Es basura y el GC tiene que recogerla — si el \
                 mecanismo de protección la hace inmortal, se cambia un defecto por otro"
            );
            assert!(
                !sidecar_de(dir.path(), id).exists(),
                "y con ella su sidecar de huellas"
            );
        }

        // ---- (b) crash REAL en la ventana: mismo material + señal de propiedad de un pid muerto --
        // La prueba de vida por pid no existe en Windows; allí este escenario pasaría por la razón
        // equivocada (el TTL), así que solo se ejerce en Unix. El caso portable de abajo fija
        // explícitamente la señal rancia y deja que el TTL sea el discriminante en todas partes.
        #[cfg(unix)]
        {
            let dir = tempfile::tempdir().unwrap();
            let ws_a = siembra_documentos(dir.path(), &["uno", "dos"]);
            let id = "e25-h03-dueno-muerto";
            let cs = cs_modifica(&ws_a, id, &["uno.md", "dos.md"]);

            let ventana = congelar_en_la_ventana(ws_a, cs);
            // Foto del plano de control con la transacción VIVA dentro de la ventana: contiene el árbol,
            // el sidecar, el staging y —sea cual sea el mecanismo— la señal durable de propiedad.
            let foto = ficheros_bajo(&runtime_de(dir.path()));
            assert!(
            foto.keys()
                .any(|r| r.starts_with(&format!("recovery/{id}"))),
            "precondición: la foto debe incluir el material de recuperación de la ventana: {:?}",
            foto.keys().collect::<Vec<_>>()
        );
            let señales: Vec<&String> = foto
                .keys()
                .filter(|r| !r.starts_with("recovery/") && !r.starts_with("staging/"))
                .collect();
            assert!(
            !señales.is_empty(),
            "precondición del escenario: mientras la transacción está viva en la ventana tiene que \
             existir en disco ALGUNA señal durable de propiedad (el fichero de lock, o la marca de \
             «en curso» que el mecanismo elija) — es lo que este caso convierte en «dueño muerto». \
             Sin ninguna, este caso no se distinguiría del (a): {:?}",
            foto.keys().collect::<Vec<_>>()
        );
            ventana
                .liberar()
                .expect("la transacción congelada publica al liberarla");

            // Se repone el estado que dejaría un crash en esa ventana: todo lo que la foto tenía y el
            // sellado se llevó, con el pid de ESTE proceso (vivo) sustituido por uno inexistente. El
            // journal NO se repone: el crash ocurrió antes de crearlo, que es lo que hace del material
            // un huérfano invisible para el criterio `journal/` ∪ `receipts/`.
            let vivo = std::process::id().to_string();
            let muerto = pid_inexistente().to_string();
            for (rel, bytes) in &foto {
                let destino = runtime_de(dir.path()).join(rel);
                if destino.exists() {
                    continue;
                }
                if let Some(padre) = destino.parent() {
                    std::fs::create_dir_all(padre).unwrap();
                }
                let contenido = match std::str::from_utf8(bytes) {
                    Ok(texto) => texto.replace(&vivo, &muerto).into_bytes(),
                    Err(_) => bytes.clone(),
                };
                std::fs::write(&destino, contenido).unwrap();
            }
            assert!(
                !journal_de(dir.path(), id).exists(),
                "precondición: el crash es ANTERIOR al journal, así que no puede haber ninguno"
            );

            let ws_b = Workspace::open(dir.path()).unwrap();
            gc_con_limite(ws_b).expect("el GC nunca falla");

            assert!(
            !recovery_de(dir.path(), id).exists(),
            "el dueño de esta ventana está MUERTO (pid inexistente): su material es basura y el GC \
             tiene que recogerlo. Una señal de propiedad sin criterio de rancidez deja basura \
             inmortal en cada crash — es el defecto que E23-H23 ya cerró para el lock"
        );
            assert!(
                !sidecar_de(dir.path(), id).exists(),
                "y su sidecar de huellas con él"
            );
            assert!(
                !staging_de(dir.path(), id).exists(),
                "y su staging, que es el huérfano que motivó el barrido de E24-H06"
            );
        }

        // ---- (c) señal rancia por TTL: el camino portable que también debe cubrir Windows ----
        // Se captura el mismo estado real de la ventana, pero se modifica únicamente el timestamp
        // normativo del lock JSON. El host remoto hace que Unix tampoco consulte la vida del pid:
        // en ambos sistemas el único criterio disponible es el TTL, sin dormir 15 minutos.
        {
            let dir = tempfile::tempdir().unwrap();
            let ws_a = siembra_documentos(dir.path(), &["uno", "dos"]);
            let id = "e25-h03-senal-rancia-ttl";
            let cs = cs_modifica(&ws_a, id, &["uno.md", "dos.md"]);

            let ventana = congelar_en_la_ventana(ws_a, cs);
            let runtime = runtime_de(dir.path());
            let foto = ficheros_bajo(&runtime);
            let lock_rel = "lock.json";
            let lock_bytes = foto
                .get(lock_rel)
                .expect("precondición: la ventana debe dejar el lock durable en runtime/");
            let mut lock: serde_json::Value = serde_json::from_slice(lock_bytes)
                .expect("precondición: la señal/lock real debe ser JSON interpretable");
            assert!(
                lock.get("timestamp")
                    .and_then(serde_json::Value::as_u64)
                    .is_some(),
                "precondición: el lock real debe declarar el timestamp normativo que usa el TTL: {lock}"
            );

            assert!(
                recovery_de(dir.path(), id).is_dir(),
                "precondición: la ventana debe tener recovery materializado: {}",
                recovery_de(dir.path(), id).display()
            );
            assert!(
                sidecar_de(dir.path(), id).is_file(),
                "precondición: la ventana debe tener sidecar materializado: {}",
                sidecar_de(dir.path(), id).display()
            );
            assert!(
                staging_de(dir.path(), id).is_dir(),
                "precondición: la ventana debe tener staging materializado: {}",
                staging_de(dir.path(), id).display()
            );
            assert!(
                !journal_de(dir.path(), id).exists(),
                "precondición: la señal se captura antes de create_journal: {}",
                journal_de(dir.path(), id).display()
            );

            // La forma y el campo vienen del lock real; solo lo convertimos en una marca rancia.
            // Un host ajeno evita que Unix convierta el pid actual del test en una señal viva.
            lock["host"] = serde_json::json!("host-remoto-del-test");
            lock["timestamp"] = serde_json::json!(0_u64);
            let lock_rancio = serde_json::to_vec(&lock).expect("serializar el lock rancio");

            ventana
                .liberar()
                .expect("la transacción congelada debe poder publicar");

            // El crash se modela restaurando exclusivamente el estado durable capturado antes del
            // journal. El recibo no existe porque este arnés llama a apply_transaction sin recibo.
            for (rel, bytes) in &foto {
                let destino = runtime.join(rel);
                if destino.exists() {
                    continue;
                }
                if let Some(padre) = destino.parent() {
                    std::fs::create_dir_all(padre).unwrap();
                }
                let bytes = if rel == lock_rel { &lock_rancio } else { bytes };
                std::fs::write(&destino, bytes).unwrap();
            }

            assert!(
                recovery_de(dir.path(), id).is_dir(),
                "precondición restaurada: recovery debe estar presente antes del GC"
            );
            assert!(
                sidecar_de(dir.path(), id).is_file(),
                "precondición restaurada: sidecar debe estar presente antes del GC"
            );
            assert!(
                staging_de(dir.path(), id).is_dir(),
                "precondición restaurada: staging debe estar presente antes del GC"
            );
            assert!(
                !journal_de(dir.path(), id).exists(),
                "precondición restaurada: el crash ocurre antes del journal"
            );
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(
                    &std::fs::read_to_string(runtime.join(lock_rel)).unwrap()
                )
                .unwrap()["timestamp"],
                serde_json::json!(0_u64),
                "la señal restaurada debe ser rancia por el timestamp normativo"
            );

            let ws_b = Workspace::open(dir.path()).unwrap();
            gc_con_limite(ws_b).expect("el GC nunca falla");

            assert!(
                !recovery_de(dir.path(), id).exists(),
                "el GC portable debe purgar por TTL el recovery de la señal rancia"
            );
            assert!(
                !sidecar_de(dir.path(), id).exists(),
                "el GC portable debe purgar por TTL el sidecar junto al recovery"
            );
            assert!(
                !staging_de(dir.path(), id).exists(),
                "el GC portable debe purgar por TTL el staging huérfano"
            );
            assert!(
                !journal_de(dir.path(), id).exists(),
                "el GC portable no debe inventar un journal al purgar"
            );
            assert!(
                !runtime.join(lock_rel).exists(),
                "el lock rancio reclamado por TTL debe quedar liberado al terminar el GC"
            );
        }
    }

    /// **E25-H03** · Criterio 3 — **Dado** una transacción que termina, **Cuando** ha terminado,
    /// **Entonces** no queda **ninguna** marca de «en curso» bajo `.lodestar/runtime/`.
    ///
    /// La spec lo pide para el camino de éxito; el test cubre también el camino de `Err`, porque la
    /// invariante es la misma —**la marca muere con la transacción**— y de ella depende que un
    /// aborto en la ventana siga dejando un huérfano *recogible*: es exactamente lo que mantiene
    /// verde `seam_real::caida_entre_backup_y_journal` (que aborta en la ventana **en este mismo
    /// proceso**, con este pid VIVO) sin tener que tocarlo. Si la marca sobreviviera al `Err`, el GC
    /// vería un dueño vivo para siempre.
    ///
    /// Lo único que el sellado conserva a propósito es el material de recuperación de la transacción
    /// (árbol + sidecar), que `change_revert` necesita. Cualquier otro rastro —lock sin liberar,
    /// marca de «en curso», staging sin limpiar— es lo que este test prohíbe.
    #[test]
    fn la_marca_no_sobrevive_a_la_transaccion() {
        // ---- (a) camino feliz ----
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos"]);
        let id = "e25-h03-marca-exito";
        let cs = cs_modifica(&ws, id, &["uno.md", "dos.md"]);
        ws.apply_transaction(&cs)
            .expect("la transacción debe publicar");

        let resto = resto_del_plano_de_control(dir.path(), id);
        assert!(
            resto.is_empty(),
            "tras publicar con éxito no puede quedar ninguna marca de «en curso» bajo \
             .lodestar/runtime/ (solo el material de recuperación de la transacción, que \
             `change_revert` necesita); quedó: {:?}",
            resto.keys().collect::<Vec<_>>()
        );

        // ---- (b) camino de `Err` dentro de la ventana: la transacción también TERMINÓ ----
        let dir2 = tempfile::tempdir().unwrap();
        let ws2 = siembra_documentos(dir2.path(), &["uno", "dos"]);
        let id2 = "e25-h03-marca-abortada";
        let cs2 = cs_modifica(&ws2, id2, &["uno.md", "dos.md"]);

        failpoints::armar(FailPoint::TrasBackupSinJournal);
        let r = ws2.apply_transaction(&cs2);
        failpoints::desarmar();
        assert!(r.is_err(), "el failpoint de la ventana debe abortar");

        let resto2 = resto_del_plano_de_control(dir2.path(), id2);
        assert!(
            resto2.is_empty(),
            "una transacción que muere en la ventana también ha TERMINADO: su marca no puede \
             sobrevivirla. Si sobreviviera, el GC vería un dueño vivo con este pid y el huérfano \
             sería inmortal — y `caida_entre_backup_y_journal` pasaría a fallar por la razón \
             equivocada; quedó: {:?}",
            resto2.keys().collect::<Vec<_>>()
        );
    }

    /// **E25-H03** · Criterio 4 (**control anti-vacuo**) — **Dado** un GC que no consigue barrer
    /// (lock tomado, señal de propiedad ilegible), **Cuando** se invoca, **Entonces** devuelve
    /// `Ok(())` y la operación que lo llamó no se ve afectada.
    ///
    /// El GC es best-effort por definición y corre **después** de que la transacción haya publicado:
    /// un `Err` suyo convertiría un apply publicado en un fallo sin recibo (el defecto que cierra
    /// E25-H04). Por eso «GC bajo el lock» tiene que ser fail-fast y **silencioso**: si no consigue
    /// el lock, no barre y devuelve `Ok(())`; jamás propaga el `WRITE_CONFLICT` de `acquire_lock`.
    #[test]
    fn el_gc_nunca_tumba_a_quien_lo_llama() {
        let dir = tempfile::tempdir().unwrap();
        let ws = siembra_documentos(dir.path(), &["uno", "dos"]);
        let id = "e25-h03-gc-no-tumba";
        let cs = cs_modifica(&ws, id, &["uno.md", "dos.md"]);
        ws.apply_transaction(&cs)
            .expect("la transacción debe publicar");
        let publicado = canonical_md(dir.path());
        assert!(
            publicado.values().all(|c| c.contains("cuerpo NUEVO")),
            "precondición: la transacción publicó su lote: {publicado:?}"
        );

        // (a) El lock de publicación está TOMADO por otro: el GC no puede barrer, y no pasa nada.
        let guardia = ws
            .acquire_lock()
            .expect("tomar el lock de publicación en el test");
        let ws_b = Workspace::open(dir.path()).unwrap();
        assert_eq!(
            gc_con_limite(ws_b),
            Ok(()),
            "un GC que no consigue barrer devuelve Ok(()): es best-effort y corre DESPUÉS de \
             publicar, así que un Err suyo convertiría un apply publicado en un fallo sin recibo"
        );
        drop(guardia);

        // (b) La señal de propiedad del plano de control es ILEGIBLE (bytes que no son UTF-8 ni
        //     JSON). Tampoco puede tumbar el GC: ante la duda no se barre, pero se devuelve Ok.
        std::fs::write(ws.lock_path(), [0xff, 0xfe, 0x00, 0x01]).expect("plantar un lock ilegible");
        std::fs::write(
            runtime_de(dir.path()).join("marca.ilegible"),
            [0x00, 0xff, 0x00],
        )
        .expect("plantar una marca ilegible en el plano de control");
        let ws_c = Workspace::open(dir.path()).unwrap();
        assert_eq!(
            gc_con_limite(ws_c),
            Ok(()),
            "una señal de propiedad ilegible deja al GC sin criterio para barrer, no con un error \
             que propagar a quien acaba de publicar"
        );

        // Y lo que la operación llamante publicó sigue publicado, byte a byte.
        assert_eq!(
            canonical_md(dir.path()),
            publicado,
            "el GC no toca el conocimiento canónico: solo barre `.lodestar/runtime/`"
        );
    }
}

// ===========================================================================
// E25-H05 — dónde viven sus tests (nota para la próxima auditoría)
//
// La historia lista este fichero entre sus «Pruebas», pero ninguno de sus criterios se puede fijar
// aquí sin atarlo a una firma que la propia historia va a cambiar o a un módulo que no es visible:
//
// - **Ventana del revert y recibo de la inversa** (criterios 1, 2, 4 y 5) →
//   `crates/lodestar-app/tests/escritura.rs`, módulo `reversion_re_verificada`. La ventana que
//   describen empieza en la FACHADA (`App::change_revert` mira `receipt.result_revision` **antes**
//   de que `revert_transaction_con_recibo` tome el lock), así que solo se reproduce entera desde ahí; y
//   `revert_transaction_con_recibo` gana en esta historia un parámetro (la revisión observada), de modo que un
//   test que lo llamara directamente obligaría al implementador a tocar los tests del autor.
// - **Crash real durante la reversión** → `crates/lodestar-mcp/tests/crash_senal.rs`
//   (`crash_durante_revert_deja_inversa_reversible`), donde ya vive el arnés de `SIGKILL`.
// - **Fsync de directorio visible** (criterio 3) → `crates/lodestar-workspace/src/io.rs`, módulo
//   unitario `durabilidad_del_directorio`: `mod io` es privado y la única inyección portable del
//   fallo (un directorio que no se puede abrir) es incompatible con el descubrimiento, que necesita
//   el mismo bit de lectura. Está declarado como límite estructural en la fase roja.
// ===========================================================================

// ===========================================================================
// E25-H06 — El lock tiene dueño demostrable
// (`requirements/epica-25-endurecimiento-escritura.md`, defecto (a)). Fase ROJA.
//
// Los tres criterios de lock de la historia miran el MISMO objeto: el cuerpo del fichero
// `.lodestar/runtime/lock.json` y quién tiene derecho a borrarlo o a reclamarlo. Hoy:
//
//   1. `Drop for WorkspaceLock` borra **por ruta** (`lock.rs:51`), sin comprobar que el fichero
//      siga siendo el suyo → si otro proceso lo reclamó por huérfano y lo recreó, el `Drop` del
//      dueño original libera el lock del NUEVO dueño, y a partir de ahí encadena.
//   2. `reclamar_si_huerfano` reclama con `dueño_muerto || caducado` (`lock.rs:197`): el TTL
//      **manda sobre** la prueba de vida, así que un dueño vivo pero suspendido (portátil dormido,
//      breakpoint, reloj movido) pierde su lock.
//   3. El metadata no declara host (`lock.rs:255`), y `proceso_muerto` pregunta por el pid en la
//      máquina **local**: sobre un workspace en red, el pid de otra máquina se juzga como propio.
//
// CONTRATO OBSERVABLE QUE ASUMEN ESTOS TESTS (mínimo, para el implementador)
//
// - El cuerpo del lock sigue siendo un **objeto JSON** legible desde fuera (ya lo es, y
//   `concurrencia.rs::lock_huerfano` depende de ello desde E23-H23).
// - Ese objeto declara la **identidad de máquina** en un campo `host` de tipo string no vacío. Es
//   el único nombre que los tests fijan, porque fabricar «un lock de OTRO host» exige poder
//   escribirlo; el resto del layout —en particular el **token de propiedad**— queda a elección del
//   implementador y NINGÚN test lo nombra: los criterios se aseveran por su EFECTO (el fichero del
//   segundo dueño sobrevive al `Drop` del primero).
// - Los cuerpos que estos tests plantan se derivan del cuerpo REAL que escribe una adquisición
//   (`acquire_lock` → leer el fichero → mutar solo `pid`/`timestamp`/`host`), de modo que un
//   campo nuevo (token, boot-id, versión…) viaja en ellos sin que haya que tocar los tests.
// - Un lock con `timestamp: 0` es reclamable en cualquier plataforma (TTL vencido por tres órdenes
//   de magnitud): es la palanca portable que usan los tests para provocar un reclamo legítimo.
//
// PLATAFORMA: los criterios 2 y 3 hablan de la **prueba de vida por pid**, que solo existe en Unix
// (`proceso_muerto` devuelve `false` fuera de Unix por diseño, `lock.rs:234`, y ahí el TTL es la
// única red). Van gateados con `#[cfg(unix)]`: en Windows no habría nada que afirmar y el test
// pasaría por la razón equivocada. El criterio 1 (Drop ajeno) es portable y no se gatea.
// ===========================================================================

mod propiedad_del_lock {
    use super::*;

    /// Segundos desde la época. Los tests fabrican `timestamp`s relativos a este reloj, que es el
    /// mismo que lee `reclamar_si_huerfano` (`lock.rs:187-192`).
    ///
    /// `#[cfg(unix)]` porque sus únicos consumidores son los dos tests unix-only de este módulo (la
    /// prueba de vida por pid no existe fuera de Unix): en Windows quedaría huérfano y el
    /// `-D warnings` del CI lo rechaza como `dead_code`. El `cfg` es la verdad —es un helper de
    /// escenarios unix-only—, no un `allow` que tape el aviso.
    #[cfg(unix)]
    fn ahora_epoch() -> u64 {
        SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("el reloj del sistema debe ir por delante de la época")
            .as_secs()
    }

    /// Un `timestamp` con el TTL del lock (15 min, `lock.rs:148`) **vencido de sobra**.
    ///
    /// `#[cfg(unix)]` por el mismo motivo que [`ahora_epoch`]: solo lo consumen los dos tests
    /// unix-only del módulo.
    #[cfg(unix)]
    fn hace_una_hora() -> u64 {
        ahora_epoch().saturating_sub(3600)
    }

    /// Un PID que con certeza **no** corresponde a ningún proceso vivo: se relanza el propio
    /// binario de test con `--list` (libtest lista los tests y sale de inmediato, sin ejecutar
    /// ninguno) y se espera a que muera. Portable —no depende de rangos de PID del sistema— y
    /// realista: es exactamente el hueco que deja un escritor que se fue. Mismo truco que
    /// `crates/lodestar-mcp/tests/concurrencia.rs::pid_muerto`, que allí puede usar el binario del
    /// servidor y aquí no existe.
    fn pid_muerto() -> u64 {
        let exe = std::env::current_exe().expect("ruta del propio binario de test");
        let mut hijo = std::process::Command::new(exe)
            .arg("--list")
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("arrancar el proceso sonda");
        let pid = u64::from(hijo.id());
        hijo.wait().expect("esperar al proceso sonda");
        pid
    }

    /// Cuerpo REAL del lock, tal y como lo escribe una adquisición de verdad: se toma el lock, se
    /// lee el fichero y se suelta. Es la base sobre la que los tests fabrican sus escenarios, de
    /// modo que **todos** los campos que el implementador añada (token, host, boot-id…) viajen en
    /// los cuerpos plantados sin que los tests tengan que conocerlos.
    ///
    /// `#[cfg(unix)]`: los dos escenarios que derivan su cuerpo del real —lock rancio de pid vivo y
    /// lock de otro host— son unix-only, porque los dos hablan de la prueba de vida por pid. El
    /// escenario portable (`drop_no_borra_un_lock_ajeno`) planta su cuerpo a mano con `timestamp: 0`
    /// y no necesita este helper.
    #[cfg(unix)]
    fn metadata_real(ws: &Workspace) -> serde_json::Value {
        let guard = ws
            .acquire_lock()
            .expect("tomar el lock para leer el metadata que escribe la implementación");
        let raw = std::fs::read_to_string(ws.lock_path()).expect("leer el fichero de lock");
        drop(guard);
        serde_json::from_str(&raw).unwrap_or_else(|e| {
            panic!("el cuerpo del lock debe ser JSON legible desde fuera (E23-H23 ya lo lee de vuelta): {e}; cuerpo: {raw:?}")
        })
    }

    /// Planta `cuerpo` como fichero de lock del workspace (creando `runtime/` si falta) y devuelve
    /// el texto exacto escrito, para poder aseverar supervivencia **byte a byte**.
    fn plantar_lock(ws: &Workspace, cuerpo: &serde_json::Value) -> String {
        let path = ws.lock_path();
        let runtime = path
            .parent()
            .expect("el lock cuelga de `.lodestar/runtime/`");
        std::fs::create_dir_all(runtime).expect("crear `.lodestar/runtime/`");
        let texto = format!("{cuerpo}\n");
        std::fs::write(&path, &texto).expect("plantar el fichero de lock del escenario");
        texto
    }

    /// **E25-H06 · Criterio 1** (`drop_no_borra_un_lock_ajeno`) — **Dado** un lock reclamado por
    /// huérfano y recreado por un segundo dueño, **Cuando** el guard del **primer** dueño se
    /// dropea, **Entonces** el fichero de lock del segundo **sigue existiendo**.
    ///
    /// ROJO HOY: `Drop for WorkspaceLock` (`lock.rs:46-53`) hace `remove_file(&self.path)` a secas.
    /// El guard de A no sabe que el fichero que hay en esa ruta ya no es el que él creó, así que
    /// borra el de B — y B sigue publicando creyéndose el único escritor, con el lock libre para
    /// cualquiera. Peor: la cascada. Cada `Drop` posterior libera el lock del siguiente dueño.
    ///
    /// El escenario es el que la historia describe, montado con la API pública y sin tocar reloj ni
    /// procesos: A toma el lock; el cuerpo del lock pasa a **parecer** el de un muerto rancio
    /// (`timestamp: 0`, pid inexistente) —que es justo lo que ve un tercero cuando A quedó
    /// suspendido o su marca envejeció—; B lo reclama y lo recrea legítimamente
    /// (`reclamar_si_huerfano` → `Reclamo::Reclamado`); y solo entonces A termina y suelta su guard.
    ///
    /// ANTI-VACUO (última aserción): cuando el dueño **de verdad** se va, el lock SÍ se libera. El
    /// arreglo no puede consistir en que `Drop` deje de borrar: eso convertiría cada transacción en
    /// un lock huérfano y devolvería el defecto que cerró E23-H23.
    #[test]
    fn drop_no_borra_un_lock_ajeno() {
        let dir = tempfile::tempdir().unwrap();
        // Dos handles sobre el mismo root: los dos «procesos» del escenario.
        let ws_a = Workspace::open(dir.path()).unwrap();
        let ws_b = Workspace::open(dir.path()).unwrap();
        let lock_path = ws_a.lock_path();

        // (1) A es el dueño inicial.
        let guard_a = ws_a
            .acquire_lock()
            .expect("el primer publicador debe adquirir el lock");
        assert!(
            lock_path.exists(),
            "precondición: mientras el guard de A vive, su fichero de lock existe"
        );

        // (2) El lock de A pasa a parecer huérfano y rancio: dueño inexistente y `timestamp: 0`
        //     (TTL vencido en cualquier plataforma). Es exactamente lo que ve un tercero cuando el
        //     dueño quedó suspendido, murió por SIGKILL o el reloj se movió.
        let cuerpo_rancio = serde_json::json!({
            "owner": "a-fantasma",
            "pid": pid_muerto(),
            "timestamp": 0,
        });
        plantar_lock(&ws_a, &cuerpo_rancio);

        // (3) B lo reclama por huérfano y lo recrea: a partir de aquí el lock es SUYO.
        let guard_b = ws_b
            .acquire_lock()
            .expect("un lock rancio de dueño inexistente debe reclamarse (E23-H23)");
        let cuerpo_de_b =
            std::fs::read_to_string(&lock_path).expect("B debe haber recreado el fichero de lock");

        // (4) A termina su trabajo y suelta su guard. El fichero que hay en la ruta YA NO ES SUYO.
        drop(guard_a);

        assert!(
            lock_path.exists(),
            "el `Drop` del primer dueño NO puede borrar el lock que otro proceso reclamó y recreó: \
             hoy `Drop for WorkspaceLock` borra por RUTA (`lock.rs:51`), sin comprobar que el \
             fichero siga siendo el suyo, y a partir de ahí encadena (cada Drop libera el lock del \
             siguiente). Un token de propiedad en el cuerpo lo resuelve: si el del fichero no es el \
             mío, no es mi lock"
        );
        assert_eq!(
            std::fs::read_to_string(&lock_path).expect("el lock del segundo dueño debe seguir ahí"),
            cuerpo_de_b,
            "y sigue siendo el lock de B **byte a byte**: el Drop ajeno no lo toca de ninguna forma"
        );

        // (5) Y B conserva la exclusión: un tercero no lo obtiene mientras B vive (el fichero de B
        //     es reciente y su dueño está vivo, así que no hay reclamo posible).
        let ws_c = Workspace::open(dir.path()).unwrap();
        assert!(
            ws_c.acquire_lock().is_err(),
            "tras el `Drop` ajeno el lock debe seguir CERRADO a terceros: si no, el escritor único \
             se rompió aunque el fichero siguiera en disco"
        );

        // (6) ANTI-VACUO: el dueño legítimo sí libera al irse.
        drop(guard_b);
        assert!(
            !lock_path.exists(),
            "el `Drop` del dueño REAL debe seguir borrando su lock (RAII, E13-H02): el arreglo no \
             puede consistir en dejar de borrar nunca, o cada transacción dejaría un huérfano"
        );
    }

    /// **E25-H06 · Criterio 2** (`no_se_reclama_el_lock_de_un_pid_vivo`) — **Dado** un lock cuyo
    /// `timestamp` es más viejo que el TTL pero cuyo pid está **vivo** en esta máquina, **Cuando**
    /// otro proceso intenta adquirirlo, **Entonces** falla con `WRITE_CONFLICT` y el lock no se
    /// reclama.
    ///
    /// ROJO HOY: `reclamar_si_huerfano` reclama con `dueño_muerto || caducado` (`lock.rs:197`), así
    /// que el TTL **manda sobre** la prueba de vida: un dueño vivo pero suspendido —portátil
    /// dormido, proceso parado en un breakpoint, máquina con el reloj movido— pierde su lock y el
    /// invariante de escritor único se rompe en silencio. El arreglo lo invierte: un pid vivo
    /// **local** impide el reclamo aunque el TTL haya vencido; el TTL sigue siendo la red portable
    /// para cuando no se puede afirmar nada.
    ///
    /// POR QUÉ NO LO CUBRE YA `concurrencia.rs::lock_huerfano` (comprobado): su parte (5) planta un
    /// lock de proceso vivo con `timestamp` **de ahora**, de modo que ni el pid ni el TTL permiten
    /// reclamarlo — pasa hoy y seguirá pasando. La combinación que reproduce el defecto es la otra:
    /// **TTL vencido + pid vivo**, que allí no se ejerce.
    ///
    /// El cuerpo se deriva del que escribe una adquisición REAL, así que declara este host (y el
    /// token, y lo que el implementador añada); solo se mutan `pid` y `timestamp`.
    ///
    /// ANTI-VACUO (parte (b)): el mismo cuerpo rancio con el dueño **muerto** sí se reclama. El
    /// arreglo no puede consistir en dejar de reclamar.
    ///
    /// UNIX-ONLY: la prueba de vida por pid solo existe en Unix (`lock.rs:233-236`); en Windows el
    /// TTL es el único criterio admisible y no habría nada que afirmar.
    #[cfg(unix)]
    #[test]
    fn no_se_reclama_el_lock_de_un_pid_vivo() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let otro = Workspace::open(dir.path()).unwrap();
        let lock_path = ws.lock_path();

        // (a) Lock RANCIO (TTL vencido de sobra) cuyo dueño es un proceso VIVO de esta máquina:
        //     este mismo proceso de test, que por definición existe.
        let mut meta = metadata_real(&ws);
        meta["pid"] = serde_json::json!(std::process::id());
        meta["timestamp"] = serde_json::json!(hace_una_hora());
        let cuerpo_vivo = plantar_lock(&ws, &meta);

        let err = otro.acquire_lock().err().unwrap_or_else(|| {
            panic!(
                "un lock cuyo dueño sigue VIVO en esta máquina NO puede reclamarse aunque su \
                 `timestamp` haya vencido el TTL: hoy `reclamar_si_huerfano` reclama con \
                 `dueño_muerto || caducado` (`lock.rs:197`), así que un escritor suspendido \
                 (portátil dormido, breakpoint, reloj movido) pierde su lock y dos procesos \
                 publican a la vez. La prueba de vida debe MANDAR sobre el TTL"
            )
        });
        assert_eq!(
            err.code(),
            "WRITE_CONFLICT",
            "el intento contra un lock vivo mapea al wire `WRITE_CONFLICT`: {err:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&lock_path).expect("el lock vivo debe seguir en disco"),
            cuerpo_vivo,
            "y el lock del proceso vivo sobrevive **byte a byte** al intento: no se reclama, no se \
             reescribe, no se toca"
        );

        // (b) ANTI-VACUO: el MISMO cuerpo rancio, pero con el dueño realmente muerto, sí se
        //     reclama. La única diferencia entre (a) y (b) es la vida del pid, que es justo el
        //     discriminante que la historia introduce.
        meta["pid"] = serde_json::json!(pid_muerto());
        plantar_lock(&ws, &meta);
        let guard = otro.acquire_lock().expect(
            "un lock rancio de dueño MUERTO se sigue reclamando (E23-H23): el arreglo no puede \
             consistir en dejar de reclamar nunca, o un SIGKILL volvería a cerrar el workspace a la \
             escritura para siempre",
        );
        drop(guard);
    }

    /// **E25-H06 · Criterio 3** (`pid_de_otro_host_no_decide`) — **Dado** un lock cuyo metadata
    /// declara **otro host**, **Cuando** se examina, **Entonces** el pid no se usa como criterio y
    /// solo decide el TTL.
    ///
    /// ROJO HOY, por dos razones encadenadas: (i) `lock_metadata` (`lock.rs:245-256`) no escribe
    /// ninguna identidad de máquina, así que la primera aserción —el contrato observable— falla ya;
    /// y (ii) aunque se plantara el campo, `reclamar_si_huerfano` consulta `proceso_muerto` sin
    /// mirarlo (`lock.rs:194`), de modo que un pid de OTRA máquina se juzga contra la tabla de
    /// procesos de ÉSTA: sobre un workspace en red (o entre namespaces de PID) el lock de un
    /// escritor vivo se reclama porque «su» pid no existe aquí.
    ///
    /// CONTRATO OBSERVABLE MÍNIMO que fija este test (lo demás queda abierto): el cuerpo del lock
    /// declara un campo `host` de tipo string no vacío. Es el único nombre necesario para poder
    /// **fabricar** un lock de otra máquina; el token de propiedad del criterio 1 no se nombra en
    /// ninguna parte porque allí basta con aseverar su efecto.
    ///
    /// Las dos mitades son el criterio entero: con host ajeno, un pid muerto **no** reclama (i) y
    /// el TTL vencido **sí** (ii). La segunda es además el control anti-vacuo: «otro host» no puede
    /// significar «intocable», o un workspace compartido se quedaría bloqueado para siempre tras el
    /// primer crash remoto.
    ///
    /// UNIX-ONLY: fuera de Unix `proceso_muerto` ya devuelve `false` siempre, así que la mitad (i)
    /// pasaría por la razón equivocada.
    #[cfg(unix)]
    #[test]
    fn pid_de_otro_host_no_decide() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let otro = Workspace::open(dir.path()).unwrap();
        let lock_path = ws.lock_path();

        // (0) CONTRATO: una adquisición real declara la máquina que tomó el lock.
        let mut meta = metadata_real(&ws);
        let host_local = meta
            .get("host")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .unwrap_or_else(|| {
                panic!(
                    "el cuerpo del lock debe declarar la identidad de máquina en un campo `host` \
                     (string no vacío). Sin él, `proceso_muerto` pregunta por un pid ajeno en la \
                     tabla de procesos LOCAL y un lock de otra máquina se juzga como propio — el \
                     defecto (a) de E25-H06. Cuerpo actual: {meta}"
                )
            });
        assert!(
            !host_local.trim().is_empty(),
            "el `host` del lock no puede ser la cadena vacía: sería una identidad que no \
             identifica, y `host ajeno` dejaría de poder distinguirse de `mismo host`"
        );

        // (i) Lock de OTRO host, con un pid que aquí está muerto y un `timestamp` reciente.
        //     El pid no dice nada: ese número es de la tabla de procesos de otra máquina.
        let host_ajeno = format!("{host_local}-otra-maquina");
        assert_ne!(
            host_ajeno, host_local,
            "precondición: el host fabricado tiene que ser distinto del local"
        );
        meta["host"] = serde_json::json!(host_ajeno);
        meta["pid"] = serde_json::json!(pid_muerto());
        meta["timestamp"] = serde_json::json!(ahora_epoch());
        let cuerpo_remoto = plantar_lock(&ws, &meta);

        let err = otro.acquire_lock().err().unwrap_or_else(|| {
            panic!(
                "con un lock que declara OTRO host, el pid NO puede decidir: `proceso_muerto` \
                 responde por la tabla de procesos local, y ahí ese número o no existe o es de un \
                 proceso distinto. Con el TTL sin vencer, el único veredicto admisible es \
                 `WRITE_CONFLICT` — hoy se reclama el lock de un escritor remoto vivo y dos \
                 máquinas publican a la vez sobre el mismo workspace"
            )
        });
        assert_eq!(
            err.code(),
            "WRITE_CONFLICT",
            "el intento contra un lock remoto no caducado mapea al wire `WRITE_CONFLICT`: {err:?}"
        );
        assert_eq!(
            std::fs::read_to_string(&lock_path).expect("el lock remoto debe seguir en disco"),
            cuerpo_remoto,
            "y el lock remoto sobrevive byte a byte: no se reclama ni se reescribe"
        );

        // (ii) El MISMO lock remoto, ya caducado: el TTL es la red portable y sí decide. Control
        //      anti-vacuo — «otro host» no puede significar «intocable», o un crash remoto dejaría
        //      el workspace bloqueado para siempre.
        meta["timestamp"] = serde_json::json!(hace_una_hora());
        plantar_lock(&ws, &meta);
        let guard = otro.acquire_lock().expect(
            "con el host ajeno el TTL es el ÚNICO criterio, y aquí ha vencido: el lock se reclama. \
             Si no, un lock remoto se volvería inmortal y el workspace quedaría cerrado a la \
             escritura",
        );
        drop(guard);
    }
}

// ===========================================================================
// E28-H01 — Deshacer el *undo*: la identidad de una reversión no puede colisionar
// (`requirements/epica-28-defectos-destructivos-testbench.md`, M-01). Fase ROJA.
//
// EL DEFECTO, VISTO DESDE ESTA CAPA
//
// La derivación del `txnId` de la inversa vive en la fachada (`App::change_revert_uncounted`
// ~L2168), que la calcula sobre el `changeSetId` que el recibo lleva dentro: como un recibo
// `X-revert` HEREDA el `changeSetId` original, revertirlo vuelve a producir `orig_txn_id = X` y
// `new_txn_id = X-revert`, o sea el id de la transacción que se está deshaciendo. Esta capa recibe
// los dos ids ya calculados y NO se defiende: `backup_originals`/`create_journal`/
// `write_pending_receipt` escriben sobre `recovery/<new_txn_id>/` y `receipts/<new_txn_id>.json`
// aunque esa transacción ya exista con contenido vigente, destruyendo el estado **redo** que ese
// árbol guardaba.
//
// EL CONTRATO QUE FIJAN ESTOS TESTS
//
// 1. Encadenar reversiones con identidades DISTINTAS —lo que la fachada hará tras el arreglo— ya
//    tiene que funcionar aquí: la segunda reversión restaura el estado que la primera dejó atrás.
//    Es el control de que la mecánica compone; hoy pasa, y tiene que seguir pasando.
// 2. Una reversión cuyo `new_txn_id` coincide con el de una transacción **con material vigente**
//    (recibo persistido y/o copias de recuperación) debe fallar **ruidosamente antes de escribir
//    nada** — no degradar en silencio sobrescribiendo su propio `recovery/`/`receipts/`. Es el
//    subpunto «nunca sobrescribir» del alcance de la historia, y es la red que impide que la clase
//    entera del defecto vuelva por otra vía de derivación de ids.
// ===========================================================================

mod reversion_componible {
    use super::*;

    /// El estado **A** de `uno.md`: lo que la semilla escribe (cuerpo original).
    fn estado_a(root: &Path) -> String {
        std::fs::read_to_string(root.join("uno.md")).expect("uno.md debe existir")
    }

    /// Un `SemanticDiff` de préstamo para el recibo (esta capa no lo interpreta: lo copia).
    fn diff() -> SemanticDiff {
        SemanticDiff::default()
    }

    /// Aplica un change set que lleva `uno.md` de A a B, con recibo, y devuelve el `txnId` usado.
    fn aplica(ws: &Workspace, id: &str) -> String {
        let cs = cs_modifica(ws, id, &["uno.md"]);
        let d = diff();
        ws.apply_transaction_con_recibo(&cs, Some(&d))
            .expect("aplicar la transacción de partida");
        id.to_string()
    }

    /// **Criterio de composición (mecánica)** — dos reversiones encadenadas con ids DISTINTOS
    /// restauran cada una el estado que la anterior dejó atrás.
    ///
    /// Control de que el arreglo de la fachada tiene dónde apoyarse: la mecánica ya compone cuando
    /// los ids no colisionan, así que lo único que hay que arreglar es la derivación de identidad.
    #[test]
    fn dos_reversiones_encadenadas_con_ids_distintos_componen() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = siembra_documentos(root, &["uno"]);
        let a = estado_a(root);

        aplica(&ws, "e28-h01-apply");
        let b = std::fs::read_to_string(root.join("uno.md")).unwrap();
        assert_ne!(a, b, "precondición: el apply publica el estado B");

        let d = diff();
        let cs_id = ChangeSetId("e28-h01-apply".to_string());
        let observada = ws.workspace_revision().unwrap();
        ws.revert_transaction_con_recibo(
            "e28-h01-apply",
            "e28-h01-apply-revert",
            &observada,
            Some((&cs_id, &d)),
        )
        .expect("la primera reversión debe publicar");
        assert_eq!(
            std::fs::read_to_string(root.join("uno.md")).unwrap(),
            a,
            "precondición: la primera reversión devuelve `uno.md` al estado A"
        );

        // La segunda reversión deshace la primera: su árbol de origen es el de la primera inversa
        // (que respalda B) y su identidad es propia.
        let observada2 = ws.workspace_revision().unwrap();
        ws.revert_transaction_con_recibo(
            "e28-h01-apply-revert",
            "e28-h01-apply-revert-2",
            &observada2,
            Some((&cs_id, &d)),
        )
        .expect("la reversión de la reversión debe publicar");
        assert_eq!(
            std::fs::read_to_string(root.join("uno.md")).unwrap(),
            b,
            "deshacer el *undo* devuelve `uno.md` al estado B, que es lo que la primera reversión \
             dejó atrás y respaldó en su árbol de recuperación"
        );

        // Y cada transacción conserva su propio material: nadie pisó a nadie.
        for txn in [
            "e28-h01-apply",
            "e28-h01-apply-revert",
            "e28-h01-apply-revert-2",
        ] {
            assert!(
                recovery_de(root, txn).exists(),
                "cada transacción de la cadena conserva su árbol `recovery/{txn}/`"
            );
        }
    }

    /// **Criterio «nunca sobrescribir»** — una reversión cuyo `new_txn_id` ya identifica a una
    /// transacción **con material vigente** falla ruidosamente **antes de escribir nada**.
    ///
    /// Es el escenario exacto que la fachada produce hoy al revertir un `-revert` (`new_txn_id ==
    /// orig_txn_id` recalculado sobre el `changeSetId` heredado): en vez de degradar a un no-op que
    /// destruye el redo, esta capa tiene que negarse. El test asevera las dos mitades: el `Err` y,
    /// sobre todo, que el árbol de recuperación y el recibo de la transacción colisionada siguen
    /// **byte a byte** como estaban.
    #[test]
    fn una_reversion_no_pisa_el_material_de_una_transaccion_vigente() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = siembra_documentos(root, &["uno"]);

        aplica(&ws, "e28-h01-colision");
        let d = diff();
        let cs_id = ChangeSetId("e28-h01-colision".to_string());
        let observada = ws.workspace_revision().unwrap();
        ws.revert_transaction_con_recibo(
            "e28-h01-colision",
            "e28-h01-colision-revert",
            &observada,
            Some((&cs_id, &d)),
        )
        .expect("la primera reversión debe publicar");

        let recovery_antes = ficheros_bajo(&recovery_de(root, "e28-h01-colision-revert"));
        let recibo_antes = std::fs::read(
            root.join(".lodestar")
                .join("runtime")
                .join("receipts")
                .join("e28-h01-colision-revert.json"),
        )
        .expect("precondición: la primera reversión deja su recibo persistido");
        assert!(
            !recovery_antes.is_empty(),
            "precondición: y su árbol de recuperación, que guarda el estado REDO"
        );
        let canonico_antes = canonical_md(root);

        // Segunda reversión con el MISMO `new_txn_id` que la primera: la colisión que hoy destruye
        // el redo en silencio.
        let observada2 = ws.workspace_revision().unwrap();
        let resultado = ws.revert_transaction_con_recibo(
            "e28-h01-colision-revert",
            "e28-h01-colision-revert",
            &observada2,
            Some((&cs_id, &d)),
        );

        assert!(
            resultado.is_err(),
            "reutilizar el `txnId` de una transacción con material vigente tiene que fallar de \
             forma RUIDOSA: publicar bajo ese id sobrescribe su `recovery/` y su recibo, y el \
             estado que guardaban se pierde para siempre. Devolvió: {resultado:?}"
        );
        assert_eq!(
            ficheros_bajo(&recovery_de(root, "e28-h01-colision-revert")),
            recovery_antes,
            "y no puede haber tocado ni un byte del árbol de recuperación colisionado: ahí vive el \
             estado con el que se deshace el *undo*"
        );
        assert_eq!(
            std::fs::read(
                root.join(".lodestar")
                    .join("runtime")
                    .join("receipts")
                    .join("e28-h01-colision-revert.json")
            )
            .expect("el recibo colisionado debe seguir en disco"),
            recibo_antes,
            "ni reescrito su recibo como un registro degenerado"
        );
        assert_eq!(
            canonical_md(root),
            canonico_antes,
            "ni movido el canónico: el rechazo ocurre ANTES de la primera escritura"
        );
        assert!(
            !ws.recovery_pending(),
            "y no deja recuperación pendiente: nada llegó a prepararse"
        );
    }
}

// ===========================================================================
// E28-H03 — La identidad de transacción se resuelve LIBRE también en el `apply`
// (`requirements/epica-28-defectos-destructivos-testbench.md`, adenda correctiva). Fase ROJA.
//
// EL DEFECTO QUE H01 DEJÓ ABIERTO
//
// H01 protegió el camino del `revert` con `assert_txn_id_libre` (`recovery.rs:912`), pero
// `apply_transaction_con_recibo` (`transaction.rs:280`) sigue derivando su `txnId` con
// `transaction_id(&change_set.id)` y llamando a `backup_originals`/`create_journal`/
// `write_pending_receipt` **sin pasar por ningún guard**. Y el `changeSetId` es determinista
// (`blake3(baseRevision, normalizedOperations)`), así que replanificar el mismo cambio sobre la misma
// base produce el mismo id: el segundo apply sobrescribe `recovery/X/` y `receipts/X.json` de la
// primera transacción, que es la única copia con la que aquella se deshacía.
//
// Peor todavía en combinación: el `revert` posterior a ese re-apply deriva `X-revert`, que ya tiene
// recibo (el de la primera reversión), y el guard de H01 lo rechaza `WRITE_CONFLICT` **sin ningún id
// alternativo que probar**. El re-apply queda permanentemente no revertible.
//
// EL CONTRATO QUE FIJAN ESTOS TESTS
//
// 1. Publicar bajo un `txnId` ya tomado por una transacción con material vigente no pisa ese
//    material: la publicación ocurre bajo otra identidad (la primera variante libre, determinista).
// 2. Una reversión cuyo `txnId` derivado ya está tomado tampoco muere: encuentra la siguiente
//    variante libre de su familia y revierte de verdad, en vez de fallar sin salida.
// 3. Anti-vacuo: sin colisión, la derivación de ids sigue siendo EXACTAMENTE la de hoy — `X`,
//    `X-revert`, `X-revert-2` — y `revert_transaction_id` sigue la tabla de su rustdoc, incluidos
//    el borde de `u64::MAX` y los sufijos no canónicos.
// ===========================================================================

mod identidad_de_transaccion_libre {
    use super::*;
    use lodestar_workspace::{revert_transaction_id, transaction_id, WorkspaceError};

    /// Un `SemanticDiff` de préstamo para el recibo (esta capa no lo interpreta: lo copia).
    fn diff() -> SemanticDiff {
        SemanticDiff::default()
    }

    /// Ruta del recibo persistido de una transacción (`receipts/<txnId>.json`), exista o no.
    fn recibo_de(root: &Path, txn_id: &str) -> PathBuf {
        root.join(".lodestar")
            .join("runtime")
            .join("receipts")
            .join(format!("{txn_id}.json"))
    }

    /// Testigo de identidad de fichero de todo lo que cuelga de `ruta`.
    ///
    /// La comparación por bytes no basta para este defecto y hay que decirlo: cuando dos
    /// transacciones comparten `txnId`, lo que la segunda escribe encima de la primera puede ser
    /// **byte a byte idéntico** (mismo estado respaldado, mismas revisiones en el recibo), así que
    /// «intacto byte a byte» pasaría sin que nada esté intacto. La identidad sí distingue «no lo
    /// tocó» de «lo reescribió con lo mismo»: `io::write_atomic` publica por `temp+rename` y
    /// `backup_originals` empieza por `remove_dir_all`.
    ///
    /// Multiplataforma con garantías distintas por SO:
    /// - **Unix**: `(dev, ino)`. El inodo es estable frente a cualquier operación que no sea
    ///   crear/borrar el fichero, así que distingue con precisión «no lo tocó» de «lo reescribió».
    /// - **Windows**: no hay noción de inodo portable, así que se usa
    ///   `(creation_time, last_write_time, file_size)`. Un `rename` atómico crea un fichero nuevo
    ///   con `creation_time` distinto del original, que es justo el mecanismo que el motor usa para
    ///   publicar (`temp+rename`), así que la garantía observable —distinguir «intacto» de
    ///   «reescrito»— se conserva aunque el campo no sea el mismo concepto de bajo nivel.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct IdentidadFichero(u64, u64, u64);

    fn testigo(ruta: &Path) -> BTreeMap<String, IdentidadFichero> {
        #[cfg(unix)]
        fn identidad(m: &std::fs::Metadata) -> IdentidadFichero {
            use std::os::unix::fs::MetadataExt;
            IdentidadFichero(m.dev(), m.ino(), 0)
        }
        #[cfg(windows)]
        fn identidad(m: &std::fs::Metadata) -> IdentidadFichero {
            use std::os::windows::fs::MetadataExt;
            IdentidadFichero(m.creation_time(), m.last_write_time(), m.file_size())
        }
        fn walk(d: &Path, base: &Path, out: &mut BTreeMap<String, IdentidadFichero>) {
            let Ok(entries) = std::fs::read_dir(d) else {
                return;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    walk(&path, base, out);
                    continue;
                }
                let rel = path
                    .strip_prefix(base)
                    .unwrap_or(&path)
                    .to_string_lossy()
                    .replace('\\', "/");
                if let Ok(m) = std::fs::metadata(&path) {
                    out.insert(rel, identidad(&m));
                }
            }
        }
        let mut out = BTreeMap::new();
        if ruta.is_dir() {
            walk(ruta, ruta, &mut out);
        } else if let Ok(m) = std::fs::metadata(ruta) {
            out.insert(String::new(), identidad(&m));
        }
        assert!(
            !out.is_empty(),
            "precondición del testigo: «{}» tiene que existir para poder vigilarlo",
            ruta.display()
        );
        out
    }

    /// **Criterio «el apply nunca pisa»** — **Dado** un `txnId` ya tomado por una transacción con
    /// material vigente (recibo persistido), **Cuando** se publica un change set cuyo `changeSetId`
    /// deriva ese mismo `txnId`, **Entonces** el material previo no se toca: ni sus bytes ni la
    /// identidad de sus ficheros.
    ///
    /// Es la aserción a nivel de disco del defecto: hoy `backup_originals` hace `remove_dir_all` del
    /// árbol previo y `write_pending_receipt` reescribe su recibo, sin que nada lo frene, así que la
    /// primera transacción se queda sin las copias con las que se revierte y `apply_transaction`
    /// devuelve `Ok`.
    ///
    /// La secuencia es la del defecto real (`apply → revert → re-apply idéntico`): entre las dos
    /// publicaciones hay una reversión, que es lo que devuelve el canónico al estado A y hace que el
    /// re-apply tenga algo que publicar. Sin ella el segundo apply no afectaría a ninguna ruta y la
    /// destrucción tomaría otra forma (el árbol previo vaciado sin sustituto), que no es el escenario
    /// que la historia describe.
    #[test]
    fn un_apply_con_txn_id_colisionado_no_pisa_el_material_previo() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = siembra_documentos(root, &["uno"]);
        let a = std::fs::read_to_string(root.join("uno.md")).unwrap();

        // (1) Primera transacción bajo el id `e28-h03-colision`, con su recibo persistido: material
        //     VIGENTE según el criterio del GC del plano de control (`journal/ ∪ receipts/`).
        let cs1 = cs_modifica(&ws, "e28-h03-colision", &["uno.md"]);
        let d = diff();
        let cs_id = ChangeSetId("e28-h03-colision".to_string());
        ws.apply_transaction_con_recibo(&cs1, Some(&d))
            .expect("la primera transacción debe publicar");
        let b = std::fs::read_to_string(root.join("uno.md")).unwrap();
        assert_ne!(a, b, "precondición: el primer apply publica el estado B");

        let recovery1 = recovery_de(root, "e28-h03-colision");
        let recibo1 = recibo_de(root, "e28-h03-colision");
        assert_eq!(
            std::fs::read_to_string(recovery1.join("uno.md")).unwrap_or_default(),
            a,
            "precondición: sus copias de recuperación guardan el estado A, con el que se deshace"
        );
        assert!(
            recibo1.exists(),
            "precondición: y su recibo está persistido, así que el id está TOMADO"
        );
        let bytes_recovery_antes = ficheros_bajo(&recovery1);
        let bytes_recibo_antes = std::fs::read(&recibo1).unwrap();
        let testigo_recovery_antes = testigo(&recovery1);
        let testigo_recibo_antes = testigo(&recibo1);

        // (2) Reversión → el canónico vuelve a A, que es lo que da al re-apply algo que publicar.
        let observada = ws.workspace_revision().unwrap();
        ws.revert_transaction_con_recibo(
            "e28-h03-colision",
            &revert_transaction_id("e28-h03-colision"),
            &observada,
            Some((&cs_id, &d)),
        )
        .expect("la reversión intermedia debe publicar");
        assert_eq!(
            std::fs::read_to_string(root.join("uno.md")).unwrap(),
            a,
            "precondición: la reversión devuelve `uno.md` al estado A"
        );

        // (3) Re-apply con el MISMO `changeSetId` —lo que produce un re-plan idéntico por
        //     determinismo del planHash— y por tanto el mismo `txnId` «natural».
        let mut cs2 = cs_modifica(&ws, "e28-h03-colision", &["uno.md"]);
        cs2.base_revision = ws.workspace_revision().unwrap();
        let resultado = ws.apply_transaction_con_recibo(&cs2, Some(&d));
        assert!(
            resultado.is_ok(),
            "publicar de nuevo el mismo cambio es legítimo y no puede quedarse sin salida: {resultado:?}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("uno.md")).unwrap(),
            b,
            "control anti-vacuo: el re-apply publica de verdad el estado B"
        );

        assert_eq!(
            ficheros_bajo(&recovery1),
            bytes_recovery_antes,
            "el segundo apply no puede tocar ni un byte de `recovery/e28-h03-colision/`: ahí viven \
             las únicas copias con las que se deshace la primera transacción"
        );
        assert_eq!(
            std::fs::read(&recibo1).unwrap_or_default(),
            bytes_recibo_antes,
            "ni reescribir su recibo"
        );
        assert_eq!(
            testigo(&recovery1),
            testigo_recovery_antes,
            "y tienen que ser LOS MISMOS ficheros, no unos reescritos encima con el mismo \
             contenido: el re-apply respalda el mismo estado A, así que la sobrescritura es \
             invisible byte a byte y solo el inodo la delata"
        );
        assert_eq!(
            testigo(&recibo1),
            testigo_recibo_antes,
            "ídem para el recibo de la primera transacción"
        );
    }

    /// **Criterio «la reversión no se queda sin salida»** — **Dado** un `txnId` de reversión ya
    /// tomado por una transacción con material vigente, **Cuando** se revierte, **Entonces** la
    /// reversión se publica bajo la siguiente variante libre en vez de fallar `WRITE_CONFLICT`.
    ///
    /// Es la otra mitad del bloqueante: `assert_txn_id_libre` es un guard rechaza-o-nada, así que
    /// hoy la reversión muere aunque exista un id libre a un paso. El test lo ejerce por la vía en la
    /// que se manifiesta: un id derivado (`X-revert`) que ya tiene recibo.
    #[test]
    fn una_reversion_con_txn_id_tomado_publica_bajo_la_siguiente_variante_libre() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = siembra_documentos(root, &["uno"]);
        let a = std::fs::read_to_string(root.join("uno.md")).unwrap();

        // (1) apply → `uno.md` queda en B, con material vigente bajo `e28-h03-salida`.
        let cs = cs_modifica(&ws, "e28-h03-salida", &["uno.md"]);
        let d = diff();
        let cs_id = ChangeSetId("e28-h03-salida".to_string());
        ws.apply_transaction_con_recibo(&cs, Some(&d))
            .expect("el apply de partida debe publicar");
        let b = std::fs::read_to_string(root.join("uno.md")).unwrap();

        // (2) Primera reversión, bajo el id derivado `e28-h03-salida-revert` → deja `uno.md` en A y
        //     OCUPA ese id con un recibo persistido.
        let observada = ws.workspace_revision().unwrap();
        let revert_id = revert_transaction_id("e28-h03-salida");
        assert_eq!(
            revert_id, "e28-h03-salida-revert",
            "precondición: el primer escalón conserva la convención `<txnId>-revert`"
        );
        ws.revert_transaction_con_recibo(
            "e28-h03-salida",
            &revert_id,
            &observada,
            Some((&cs_id, &d)),
        )
        .expect("la primera reversión debe publicar");
        assert_eq!(
            std::fs::read_to_string(root.join("uno.md")).unwrap(),
            a,
            "precondición: la primera reversión devuelve `uno.md` al estado A"
        );
        assert!(
            recibo_de(root, &revert_id).exists(),
            "precondición: y deja su recibo, así que `{revert_id}` queda TOMADO"
        );
        let testigo_previo = testigo(&recovery_de(root, &revert_id));

        // (3) Se re-publica el mismo cambio bajo un id nuevo (lo que el arreglo del apply hará solo:
        //     aquí se fuerza a mano para aislar el camino de la reversión) y se revierte pidiendo el
        //     MISMO id derivado, que ya está tomado.
        let mut cs2 = cs_modifica(&ws, "e28-h03-salida-2", &["uno.md"]);
        cs2.base_revision = ws.workspace_revision().unwrap();
        ws.apply_transaction_con_recibo(&cs2, Some(&d))
            .expect("el re-apply bajo id propio debe publicar");
        assert_eq!(
            std::fs::read_to_string(root.join("uno.md")).unwrap(),
            b,
            "precondición: el re-apply vuelve a dejar `uno.md` en B"
        );

        let observada2 = ws.workspace_revision().unwrap();
        let resultado = ws.revert_transaction_con_recibo(
            "e28-h03-salida-2",
            &revert_id,
            &observada2,
            Some((&cs_id, &d)),
        );

        assert!(
            resultado.is_ok(),
            "una reversión cuyo `txnId` derivado ya está tomado no puede quedarse SIN SALIDA: hay \
             que resolver la identidad buscando la siguiente variante libre, no rechazar. Un `Err` \
             aquí deja la transacción permanentemente no revertible. Devolvió: {resultado:?}"
        );
        assert_eq!(
            std::fs::read_to_string(root.join("uno.md")).unwrap(),
            a,
            "y revierte de VERDAD: `uno.md` vuelve al estado A que el re-apply pisó"
        );
        assert_eq!(
            testigo(&recovery_de(root, &revert_id)),
            testigo_previo,
            "sin tocar el material de la reversión que ya ocupaba «{revert_id}»: publicó bajo otra \
             identidad"
        );
    }

    /// **Control anti-vacuo de la derivación sin colisión** — **Dado** el camino normal (cada
    /// `changeSetId` es nuevo), **Cuando** se derivan los ids, **Entonces** son EXACTAMENTE los de
    /// hoy: `X`, `X-revert`, `X-revert-2`.
    ///
    /// Sin esto, un arreglo que numerara siempre (`X-2` desde la primera publicación) pasaría los dos
    /// criterios de arriba rompiendo la convención que `crash_senal.rs` y `escritura.rs` fijan.
    #[test]
    fn sin_colision_los_ids_derivados_son_los_de_hoy() {
        let cs = ChangeSetId("changeset:abc123".to_string());
        assert_eq!(
            transaction_id(&cs),
            "abc123",
            "el `txnId` de una transacción es el hash DESNUDO de su `changeSetId`"
        );
        assert_eq!(
            revert_transaction_id("abc123"),
            "abc123-revert",
            "y el primer escalón de la cadena de reversiones conserva `<txnId>-revert`"
        );
        assert_eq!(
            revert_transaction_id("abc123-revert"),
            "abc123-revert-2",
            "y el segundo apila el contador, no repite el sufijo"
        );
    }

    /// **Criterio de la tabla del rustdoc** — **Dado** el catálogo de casos que documenta
    /// `revert_transaction_id` (`transaction.rs:91-95`), **Cuando** se ejercen uno a uno, **Entonces**
    /// cada fila de la tabla tiene su aserción.
    ///
    /// Una tabla en un rustdoc que nadie ejecuta es una promesa sin testigo: si el arreglo de
    /// identidad cambia la derivación, esto es lo que lo caza.
    #[test]
    fn revert_transaction_id_sigue_la_tabla_del_rustdoc() {
        let tabla = [
            // (`txn_id`, reversión) — las tres filas de la tabla, literales.
            ("abc123", "abc123-revert"),
            ("abc123-revert", "abc123-revert-2"),
            ("abc123-revert-2", "abc123-revert-3"),
        ];
        for (entrada, esperado) in tabla {
            assert_eq!(
                revert_transaction_id(entrada),
                esperado,
                "fila de la tabla del rustdoc: revertir «{entrada}» produce «{esperado}»"
            );
        }
        // Y la cadena compone: aplicar la derivación N veces recorre la tabla sin repetir un id.
        let mut id = "abc123".to_string();
        let mut vistos = std::collections::BTreeSet::new();
        vistos.insert(id.clone());
        for escalon in 1..=6 {
            id = revert_transaction_id(&id);
            assert!(
                vistos.insert(id.clone()),
                "el escalón {escalon} de la cadena repitió el id «{id}»: dos transacciones \
                 compartiendo `recovery/`, `journal/` y `receipts/`"
            );
        }
        assert_eq!(
            id, "abc123-revert-6",
            "seis escalones desde `abc123` llegan a `abc123-revert-6`: el primero es `-revert` (sin \
             número) y a partir de ahí el contador arranca en 2, así que el escalón N-ésimo lleva el \
             número N"
        );
    }

    /// **Criterio del borde `u64::MAX`** — **Dado** un `txn_id` en `-revert-{u64::MAX}`, **Cuando** se
    /// deriva su reversión, **Entonces** el comportamiento es un **punto fijo declarado**: devuelve el
    /// mismo id.
    ///
    /// Comportamiento que fija la fase roja, y por qué se elige el punto fijo frente a un fallo
    /// ruidoso: la derivación es una **función pura infalible** (`fn(&str) -> String`), y hacerla
    /// falible por un borde inalcanzable —hacen falta 2^64 reversiones encadenadas, cada una con su
    /// transacción publicada en disco— obligaría a propagar un `Result` por los dos caminos de
    /// publicación para un caso que nunca ocurre. Lo que sí deja de ser aceptable es que el punto
    /// fijo sea **silencioso**: con la resolución de identidad libre de esta historia, un id repetido
    /// ya no sobrescribe nada (la publicación busca la siguiente variante libre), y el rustdoc tiene
    /// que reconocer el borde en vez de prometer composición ilimitada sin matiz.
    #[test]
    fn revert_transaction_id_en_el_borde_u64_max() {
        let borde = format!("abc123-revert-{}", u64::MAX);
        assert_eq!(
            revert_transaction_id(&borde),
            borde,
            "en `u64::MAX` la derivación es un PUNTO FIJO declarado (`saturating_add`), no un \
             desbordamiento ni un pánico: el contador no puede crecer más y la función es infalible \
             por contrato. Lo que impide que eso destruya nada es la resolución de identidad libre \
             de E28-H03, no la derivación"
        );
        // Y el escalón inmediatamente anterior sí avanza: el punto fijo es EXACTAMENTE el borde, no
        // un colapso prematuro de la cadena.
        let previo = format!("abc123-revert-{}", u64::MAX - 1);
        assert_eq!(
            revert_transaction_id(&previo),
            borde,
            "el escalón anterior al borde sí avanza: el punto fijo empieza justo en `u64::MAX`"
        );
    }

    /// **Criterio de los sufijos no canónicos** — **Dado** un `txn_id` con un sufijo que esta función
    /// nunca produce (`-revert-+2`, `-revert-01`, `-revert--1`, `-revert-`), **Cuando** se deriva su
    /// reversión, **Entonces** el resultado es una **decisión explícita**, no un accidente de
    /// `parse::<u64>()`.
    ///
    /// La regla que fija la fase roja: **solo el formato canónico** —el que la propia función emite:
    /// `-revert-<n>` con `n` en decimal sin signo ni ceros a la izquierda— incrementa el contador.
    /// Cualquier otra cosa se trata como un id opaco y recibe el sufijo del primer escalón, que es lo
    /// único seguro: no se puede «continuar» una cadena cuyo formato no se emitió aquí, y adivinar el
    /// número produciría un id que podría colisionar con uno ya usado.
    ///
    /// Casos y por qué:
    /// - `-revert-1` **sí** es canónico (`parse::<u64>()` lo acepta y `1` es su forma mínima), aunque
    ///   la función nunca lo emita —arranca en `2`—: incrementa a `-revert-2`. Es el único de esta
    ///   familia que se acepta, y se declara aquí para que quede claro que es a propósito.
    /// - `-revert-+2` lleva signo: `+2` no es la forma canónica de `2`.
    /// - `-revert-01` lleva cero a la izquierda: `01` no es la forma canónica de `1`.
    /// - `-revert--1` y `-revert-` no son números en absoluto.
    #[test]
    fn revert_transaction_id_con_sufijos_no_canonicos() {
        assert_eq!(
            revert_transaction_id("abc123-revert-1"),
            "abc123-revert-2",
            "`-revert-1` SÍ es canónico (decimal, sin signo, sin ceros a la izquierda): incrementa"
        );
        for entrada in ["abc123-revert-+2", "abc123-revert-01"] {
            assert_eq!(
                revert_transaction_id(entrada),
                format!("{entrada}-revert"),
                "«{entrada}» no está en la forma canónica que esta función emite (`+`/ceros a la \
                 izquierda), así que se trata como un id OPACO y recibe el sufijo del primer \
                 escalón: continuar una cadena cuyo formato no se emitió aquí produciría un id que \
                 podría colisionar con uno ya usado"
            );
        }
        for entrada in ["abc123-revert--1", "abc123-revert-"] {
            assert_eq!(
                revert_transaction_id(entrada),
                format!("{entrada}-revert"),
                "«{entrada}» no lleva número alguno tras el sufijo: id opaco, primer escalón"
            );
        }
        // Y el resultado de cualquiera de ellos sigue siendo derivable sin colisionar consigo mismo:
        // la composición no se rompe por haber entrado con un id raro.
        let raro = revert_transaction_id("abc123-revert-01");
        assert_ne!(
            revert_transaction_id(&raro),
            raro,
            "y desde ahí la cadena vuelve a avanzar: la derivación nunca devuelve su propia entrada \
             salvo en el punto fijo de `u64::MAX`"
        );
    }

    // -----------------------------------------------------------------------
    // E28-H03 (cierre de reservas) — las dos salidas de `resolve_free_txn_id` que quedaron sin
    // testigo: la rama AGOTADA (y la calidad de su mensaje) y el rechazo duro `new == orig`.
    // -----------------------------------------------------------------------

    /// Un `ChangeReceipt` mínimo bajo `id`, para OCUPAR ese `txnId` con material vigente según el
    /// criterio del GC del plano de control (`journal/ ∪ receipts/`). El contenido no se interpreta:
    /// lo que decide es la existencia de `receipts/<id>.json`.
    fn ocupa_con_recibo(ws: &Workspace, id: &str) {
        ws.write_receipt(&ChangeReceipt {
            id: ReceiptId(id.to_string()),
            change_set_id: ChangeSetId(format!("changeset:{id}")),
            previous_revision: WorkspaceRevision("blake3:previa".to_string()),
            result_revision: WorkspaceRevision("blake3:resultante".to_string()),
            changed_paths: vec![RelPath::new("uno.md").unwrap()],
            semantic_diff: SemanticDiff::default(),
        })
        .expect("sembrar el recibo que ocupa el id");
    }

    /// **Criterio de la rama AGOTADA** — **Dado** un `txnId` candidato en el punto fijo
    /// `-revert-{u64::MAX}` **ya ocupado** por un recibo vigente, **Cuando** se intenta publicar una
    /// reversión bajo él, **Entonces** `resolve_free_txn_id` no puede avanzar (la variante siguiente
    /// es la misma) y devuelve `WriteConflict` **antes de escribir nada**, con un mensaje
    /// **accionable para un agente**: dice qué hacer y no filtra rutas internas del runtime.
    ///
    /// Es la única rama de la resolución de identidad de E28-H03 que ningún test ejercía. Y su
    /// mensaje no es un detalle cosmético: el delta de contrato de la historia exige que *«el mensaje
    /// de error de cualquier `WRITE_CONFLICT` que sobreviva debe ser accionable para un agente (qué
    /// hacer: replanificar, no rutas internas de `recovery/`/`receipts/` que un agente no puede
    /// interpretar ni actuar)»*. Hoy el motivo que compone `senal_de_txn_id_tomado` interpola
    /// `recibo.display()` —la ruta ABSOLUTA de `.lodestar/runtime/receipts/…`, con el directorio
    /// temporal de la máquina incluido—, así que el `WriteConflict` la arrastra entera.
    ///
    /// Cómo se alcanza el punto fijo sin 2^64 reversiones: se pide explícitamente ese `new_txn_id`
    /// (la API lo admite: es un **candidato**) y se siembra su recibo. `siguiente_variante_de_txn_id`
    /// satura en `u64::MAX`, así que la cadena no avanza y la rama se ejerce en un test determinista.
    #[test]
    fn una_reversion_sin_variante_libre_falla_con_mensaje_accionable_y_sin_rutas_internas() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = siembra_documentos(root, &["uno"]);
        let a = std::fs::read_to_string(root.join("uno.md")).unwrap();

        // (1) apply real: deja material revertible bajo `e28-h03-agotado`.
        let cs = cs_modifica(&ws, "e28-h03-agotado", &["uno.md"]);
        let d = diff();
        let cs_id = ChangeSetId("e28-h03-agotado".to_string());
        ws.apply_transaction_con_recibo(&cs, Some(&d))
            .expect("el apply de partida debe publicar");
        let b = std::fs::read_to_string(root.join("uno.md")).unwrap();
        assert_ne!(a, b, "precondición: el apply publica el estado B");

        // (2) El candidato de la reversión es el PUNTO FIJO, y está ocupado por un recibo vigente:
        //     la cadena de variantes no tiene adónde ir.
        let borde = format!("e28-h03-agotado-revert-{}", u64::MAX);
        assert_eq!(
            revert_transaction_id(&borde),
            borde,
            "precondición: el borde `u64::MAX` es el punto fijo declarado de la derivación"
        );
        ocupa_con_recibo(&ws, &borde);
        let recibo_del_borde = std::fs::read(recibo_de(root, &borde))
            .expect("precondición: el recibo sembrado ocupa el id");
        let canonico_antes = canonical_md(root);

        let observada = ws.workspace_revision().unwrap();
        let error = ws
            .revert_transaction_con_recibo(
                "e28-h03-agotado",
                &borde,
                &observada,
                Some((&cs_id, &d)),
            )
            .expect_err(
                "sin ninguna variante libre que probar, la reversión tiene que fallar RUIDOSAMENTE \
                 antes de escribir: la alternativa es pisar el material de la transacción que ocupa \
                 el id",
            );
        let mensaje = format!("{error}");

        // (a) Falla, y no ha tocado nada: ni el canónico ni el material del id ocupado.
        assert!(
            matches!(error, WorkspaceError::WriteConflict(_)),
            "el agotamiento de la cadena de variantes es un `WriteConflict` (wire `WRITE_CONFLICT`), \
             no un pánico ni un error de IO; fue {error:?}"
        );
        assert_eq!(
            canonical_md(root),
            canonico_antes,
            "y el rechazo es anterior a la primera escritura: el canónico no se mueve"
        );
        assert_eq!(
            std::fs::read(recibo_de(root, &borde)).unwrap_or_default(),
            recibo_del_borde,
            "ni se reescribe el recibo que ocupaba el id (que es justo lo que el guard existe para \
             proteger)"
        );
        assert!(
            !recovery_de(root, &borde).exists(),
            "ni se prepara un árbol de recuperación bajo el id ocupado"
        );
        assert!(
            !ws.recovery_pending(),
            "ni queda recuperación pendiente: nada llegó a prepararse"
        );

        // (b) El mensaje es para un AGENTE: acción concreta, cero rutas internas del runtime.
        assert!(
            mensaje.contains("replanifica"),
            "el mensaje debe decir QUÉ HACER (replanificar sobre el estado actual), que es lo único \
             que un agente puede accionar; fue {mensaje:?}"
        );
        for fuga in [".lodestar", "/runtime/", "receipts/", "recovery/"] {
            assert!(
                !mensaje.contains(fuga),
                "el mensaje NO puede filtrar rutas internas del plano de control («{fuga}»): un \
                 agente no las puede interpretar ni actuar, y el delta de contrato de E28-H03 lo \
                 exige explícitamente. Fue {mensaje:?}"
            );
        }
        assert!(
            !mensaje.contains(&root.to_string_lossy().to_string()),
            "y mucho menos la ruta ABSOLUTA del workspace en esta máquina; fue {mensaje:?}"
        );
    }

    /// **Control anti-regresión del rechazo duro** — **Dado** un `new_txn_id` **igual** al
    /// `orig_txn_id`, **Cuando** se pide la reversión, **Entonces** `WriteConflict` y no se escribe
    /// nada.
    ///
    /// Nace VERDE a propósito: el guard existe (`recovery.rs`, paso (2c)) y esta es la única
    /// igualdad que E28-H03 decidió **no** resolver moviendo el id — pedir que la inversa publique
    /// bajo la identidad de la transacción que deshace es una contradicción del llamante, no una
    /// colisión de nombres: restauraría desde el mismo árbol que estaría reescribiendo. Sin este
    /// testigo, un refactor que sustituyera el guard por la resolución de variante libre (que
    /// devolvería `X-2` tan campante) pasaría inadvertido y volvería a abrir el defecto M-01 por otra
    /// puerta.
    #[test]
    fn una_reversion_bajo_la_identidad_de_la_transaccion_que_deshace_se_rechaza() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let ws = siembra_documentos(root, &["uno"]);

        let cs = cs_modifica(&ws, "e28-h03-misma-identidad", &["uno.md"]);
        let d = diff();
        let cs_id = ChangeSetId("e28-h03-misma-identidad".to_string());
        ws.apply_transaction_con_recibo(&cs, Some(&d))
            .expect("el apply de partida debe publicar");

        let recovery_antes = ficheros_bajo(&recovery_de(root, "e28-h03-misma-identidad"));
        let recibo_antes = std::fs::read(recibo_de(root, "e28-h03-misma-identidad"))
            .expect("precondición: el apply deja su recibo persistido");
        let canonico_antes = canonical_md(root);

        let observada = ws.workspace_revision().unwrap();
        let error = ws
            .revert_transaction_con_recibo(
                "e28-h03-misma-identidad",
                "e28-h03-misma-identidad",
                &observada,
                Some((&cs_id, &d)),
            )
            .expect_err(
                "revertir bajo la MISMA identidad que se deshace tiene que rechazarse: restauraría \
                 desde el árbol de recuperación que estaría reescribiendo",
            );
        assert!(
            matches!(error, WorkspaceError::WriteConflict(_)),
            "el rechazo es `WriteConflict` (wire `WRITE_CONFLICT`); fue {error:?}"
        );
        let mensaje = format!("{error}");
        assert!(
            mensaje.contains("e28-h03-misma-identidad"),
            "y nombra la transacción implicada, para que el llamante sepa qué pidió mal; fue \
             {mensaje:?}"
        );

        assert_eq!(
            ficheros_bajo(&recovery_de(root, "e28-h03-misma-identidad")),
            recovery_antes,
            "nada se escribió: el árbol de recuperación de la transacción sigue igual"
        );
        assert_eq!(
            std::fs::read(recibo_de(root, "e28-h03-misma-identidad")).unwrap_or_default(),
            recibo_antes,
            "ni su recibo"
        );
        assert_eq!(canonical_md(root), canonico_antes, "ni el canónico");
        assert!(!ws.recovery_pending(), "ni queda recuperación pendiente");
    }
}

// ===========================================================================
// E30-H02 — Un lock cuyo CUERPO no llegó a escribirse es huérfano irreclamable
// (`requirements/epica-30-higiene-escoba.md`, H02). Fase ROJA.
//
// CAUSA RAÍZ DIAGNOSTICADA (reproducida, no supuesta)
//
// `acquire_lock` (`lock.rs:139-188`) publica el lock en DOS pasos que no son atómicos entre sí:
//
//   1. `OpenOptions::create_new(true).open(&path)` — el fichero de lock aparece en disco **vacío**.
//      Es aquí donde se gana la exclusión mutua (`O_CREAT | O_EXCL`).
//   2. `escribir_cuerpo(&mut file, &token)` — recién ahora el fichero declara `pid`, `host`,
//      `timestamp` y `token`.
//
// Entre (1) y (2) hay una ventana. Si el proceso muere ahí —`SIGKILL`, OOM killer, corte de luz—,
// queda en disco un fichero de lock **existente y vacío**. Y ese estado es TERMINAL para
// `reclamar_si_huerfano` (`lock.rs:237-294`):
//
//   - `read_to_string` devuelve `Ok("")` → NO entra por la rama «ilegible».
//   - `serde_json::from_str("")` falla → `unwrap_or(Value::Null)` → `meta` es `Null`.
//   - `meta.get("pid")` es `None`  → `vida = Vida::Desconocida` (no hay pid al que preguntar).
//   - `meta.get("timestamp")` es `None` → `edad = None` → `caducado = false`.
//   - `reclamable = match Desconocida => caducado` = **false**.
//
// Resultado: `Reclamo::Vivo("")` — y como el detalle es la cadena vacía, el mensaje que llega al
// wire es exactamente «el lock de publicación ya está tomado (…)» **sin sufijo de pid**, que es el
// síntoma que registraron los tres jueces. Lo grave no es el mensaje: es que **no hay salida**. El
// `LOCK_TTL` es la «red portable» para cuando no se puede afirmar nada, pero el TTL se computa
// sobre el `timestamp` del cuerpo… que es justo lo que no se escribió. Sin `timestamp` no hay edad,
// sin edad no hay caducidad, y el workspace queda **cerrado a la escritura para siempre**, hasta
// que un humano borre el fichero a mano. Es la misma clase de defecto que E23-H23 cerró para el
// lock con pid muerto, por la única vía que aquélla no cubrió: el lock que ni siquiera llegó a
// tener pid.
//
// EVIDENCIA DE REPRODUCCIÓN (E30-H02, fase roja)
//
//   - `crates/lodestar-mcp/tests/diagnostico_lock_e30h02.rs::sonda_e30h02_sigkill_apuntado_a_la_ventana_del_cuerpo`
//     apunta el `SIGKILL` a la ventana sondeando la EXISTENCIA del fichero de lock desde otro
//     proceso (mismo truco de disparo por estado durable que ya usa `crash_senal.rs`): **30/30**
//     muertes dejan el lock creado y vacío, y **30/30** veces el primer `change_apply` del proceso
//     siguiente responde `WRITE_CONFLICT`.
//   - `…::sonda_e30h02_lock_con_cuerpo_no_escrito` fabrica los tres cuerpos posibles de la ventana
//     (vacío, JSON truncado, JSON sin `pid`/`timestamp`) y los tres producen el mismo conflicto
//     terminal.
//   - `…::sonda_e30h02_reclamo_tras_sigkill_cosechado` (40 repeticiones, retrasos a ciegas) da
//     **0/40** fallos: con el cuerpo ya escrito la reclamación por pid muerto funciona. La ventana
//     del cuerpo es, por tanto, el discriminante — no la prueba de vida.
//
// POR QUÉ ESTO EXPLICA LA FLAKINESS DE `crash_por_senal_no_deja_parciales`
//
// Ese test escalona `SIGKILL` a 40/70/100/130/170 ms del inicio del `change_apply` y luego exige
// que el PRIMER `change_apply` del proceso siguiente funcione. En una máquina descargada el paso
// (1)→(2) tarda microsegundos y ningún retraso cae dentro (medido: sonda D, 44 muertes, cuerpo
// íntegro de 132 bytes siempre). Bajo `cargo test --workspace` la máquina está saturada y el
// proceso puede ser **desalojado por el scheduler justo entre las dos llamadas**, ensanchando la
// ventana de microsegundos a decenas de milisegundos — que es exactamente la escala de los retrasos
// del test. De ahí que falle ~50 % con la suite entera y nunca en aislamiento.
//
// EL CRITERIO QUE FIJA ESTE TEST
//
// Un lock cuyo cuerpo no se pudo interpretar —porque no llegó a escribirse— **no** puede tratarse
// como un dueño vivo indefinidamente. La forma del arreglo la elige el implementador (publicar el
// cuerpo de forma atómica para que la ventana no exista; o tratar el cuerpo no interpretable como
// reclamable con el mismo criterio conservador que el resto); lo que este test fija es el EFECTO
// observable: el workspace no queda cerrado a la escritura para siempre por un fichero vacío.
//
// Es unix-only por coherencia con el resto del módulo de lock: el escenario nace de un `SIGKILL`.
// ===========================================================================

#[cfg(unix)]
mod lock_con_cuerpo_no_escrito {
    use super::*;

    /// Planta en `.lodestar/runtime/lock.json` el `cuerpo` indicado, creando el directorio si falta.
    /// Es el estado que deja un proceso muerto entre `create_new` y `escribir_cuerpo`.
    fn plantar_cuerpo(ws: &Workspace, cuerpo: &str) {
        let path = ws.lock_path();
        let runtime = path
            .parent()
            .expect("el lock cuelga de `.lodestar/runtime/`");
        std::fs::create_dir_all(runtime).expect("crear `.lodestar/runtime/`");
        std::fs::write(&path, cuerpo).expect("plantar el fichero de lock del escenario");
    }

    /// **E30-H02** · Criterio — **Dado** un fichero de lock que existe pero cuyo cuerpo nunca se
    /// escribió (la ventana `[create_new, escribir_cuerpo)` de `acquire_lock`), **Cuando** un
    /// proceso posterior intenta adquirir el lock, **Entonces** lo consigue: un cuerpo que no
    /// declara ni `pid` ni `timestamp` no puede sostener un lock vivo para siempre.
    ///
    /// ROJO HOY: `reclamar_si_huerfano` resuelve `Vida::Desconocida` (sin pid) + `caducado = false`
    /// (sin timestamp) → `reclamable = false` → `Reclamo::Vivo("")`, y `acquire_lock` devuelve
    /// `WriteConflict`. El `LOCK_TTL` no rescata nada porque se computa sobre el `timestamp` que
    /// falta: el workspace queda cerrado a la escritura de forma **permanente**.
    #[test]
    fn un_lock_con_cuerpo_vacio_no_bloquea_para_siempre() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        plantar_cuerpo(&ws, "");

        let guard = ws.acquire_lock().unwrap_or_else(|e| {
            panic!(
                "un fichero de lock VACÍO es el rastro de un proceso que murió entre `create_new` y \
                 `escribir_cuerpo`: no declara pid ni timestamp, así que ni la prueba de vida ni el \
                 LOCK_TTL pueden liberarlo jamás y el workspace queda cerrado a la escritura para \
                 siempre. Tiene que poder reclamarse; fue: {e}"
            )
        });
        assert!(
            ws.lock_path().exists(),
            "y tras reclamarlo el lock es del nuevo dueño: su fichero existe mientras el guard viva"
        );
        drop(guard);
        assert!(
            !ws.lock_path().exists(),
            "y el guard lo libera al dropearse, como cualquier lock legítimamente adquirido"
        );
    }

    /// **E30-H02** · Misma ventana, cuerpo **a medio escribir**: el proceso alcanzó a emitir unos
    /// bytes del JSON antes de morir. `serde_json` falla igual que con el vacío, así que el estado
    /// resultante es el mismo huérfano irreclamable.
    #[test]
    fn un_lock_con_cuerpo_truncado_no_bloquea_para_siempre() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        plantar_cuerpo(&ws, "{\"owner\":\"dbar");

        let guard = ws.acquire_lock().unwrap_or_else(|e| {
            panic!(
                "un cuerpo de lock truncado a mitad de escritura tampoco declara pid ni timestamp: \
                 mismo huérfano irreclamable que el cuerpo vacío. Tiene que poder reclamarse; \
                 fue: {e}"
            )
        });
        drop(guard);
    }

    /// **E30-H02** · Control ANTI-VACUO — el arreglo no puede consistir en «reclamar cualquier lock
    /// que no entienda». Un lock cuyo cuerpo **sí** es interpretable y declara un dueño **vivo de
    /// esta máquina** (el propio proceso de test) sigue siendo intocable: es el invariante de
    /// escritor único que `reclamar_si_huerfano` garantiza hoy (`Vida::Viva` ⇒ el TTL no manda) y
    /// que este test impide relajar de rebote.
    #[test]
    fn un_lock_de_un_dueño_vivo_sigue_sin_reclamarse() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();

        // Cuerpo REAL (para que viajen todos los campos que escriba la implementación), con el pid
        // de ESTE proceso —indudablemente vivo— y una marca de tiempo rancia: ni siquiera el TTL
        // vencido puede reclamar el lock de alguien que está vivo (E25-H06).
        let guard = ws
            .acquire_lock()
            .expect("tomar el lock para leer su cuerpo");
        let raw = std::fs::read_to_string(ws.lock_path()).expect("leer el fichero de lock");
        drop(guard);
        let mut meta: serde_json::Value =
            serde_json::from_str(&raw).expect("el cuerpo del lock es JSON legible desde fuera");
        meta["pid"] = serde_json::json!(std::process::id());
        meta["timestamp"] = serde_json::json!(0);
        plantar_cuerpo(&ws, &format!("{meta}\n"));

        let error = ws.acquire_lock().err().unwrap_or_else(|| {
            panic!(
                "el lock de un dueño VIVO de esta máquina no se reclama nunca, ni con el TTL \
                 vencido (E25-H06): reclamarlo rompería el escritor único. Si este test se pone \
                 rojo, el arreglo de E30-H02 ha relajado el criterio de «vivo» en vez de cerrar la \
                 ventana del cuerpo no escrito"
            )
        });
        assert!(
            matches!(error, lodestar_workspace::WorkspaceError::WriteConflict(_)),
            "y se rechaza como conflicto de escritura (wire `WRITE_CONFLICT`); fue {error:?}"
        );
    }

    /// Antedata `mtime`/`atime` de `path` en `segundos`. Es la única forma de fabricar un temporal
    /// «rancio» sin esperar el [`LOCK_TTL`] real (15 minutos). `libc` ya es dependencia unix de este
    /// crate para la prueba de vida por pid, así que no entra código externo nuevo.
    fn envejecer(path: &std::path::Path, segundos: i64) {
        let ahora = i64::try_from(
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("reloj posterior a la época")
                .as_secs(),
        )
        .expect("epoch cabe en i64");
        let t = libc::timeval {
            tv_sec: ahora - segundos,
            tv_usec: 0,
        };
        let times = [t, t];
        let c = std::ffi::CString::new(path.to_str().expect("ruta UTF-8")).expect("ruta sin NUL");
        // SAFETY: `utimes` solo lee el puntero a la ruta (C-string válida y viva) y el array de dos
        // `timeval` que se le pasa; no toca memoria del llamante ni retiene los punteros.
        let rc = unsafe { libc::utimes(c.as_ptr(), times.as_ptr()) };
        assert_eq!(rc, 0, "antedatar {}", path.display());
    }

    /// **E30-H02 · remate de higiene** — **Dado** un `.lodestar/runtime/` sembrado de temporales de
    /// publicación huérfanos (`.lock.<token>.tmp`) más viejos que el TTL, **Cuando** se adquiere el
    /// lock, **Entonces** desaparecen — y **solo** ellos: ni el temporal reciente de un publicador
    /// concurrente ni los ficheros que no son temporales de lock se tocan.
    ///
    /// POR QUÉ: `publicar_lock` retira su temporal en todos sus caminos de retorno, pero un
    /// `SIGKILL` entre el `create_new` y ese borrado no ejecuta ninguno (medido: 34/40 muertes
    /// reales lo dejan). Ese temporal es inerte —fuera del índice, fuera de git, ajeno a la
    /// exclusión— pero **nadie más lo recoge**, así que crece una entrada por crash y nunca decrece.
    /// El barrido por antigüedad es la escoba; este test es su red.
    ///
    /// ANTI-VACUO doble: si el barrido pasara a borrar por patrón sin mirar la edad se llevaría el
    /// temporal reciente (el de un publicador concurrente en plena carrera); si borrara por edad sin
    /// mirar el patrón se llevaría los recibos y el journal, que viven en el mismo directorio.
    #[test]
    fn los_temporales_de_publicacion_rancios_se_barren_al_adquirir() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();
        let runtime = ws
            .lock_path()
            .parent()
            .expect("el lock cuelga de `.lodestar/runtime/`")
            .to_path_buf();
        std::fs::create_dir_all(&runtime).expect("crear `.lodestar/runtime/`");

        // Basura acumulada por crashes pasados: rancia de sobra (una hora > TTL de 15 min).
        for i in 0..20 {
            let p = runtime.join(format!(".lock.token{i}.tmp"));
            std::fs::write(&p, "{}").expect("plantar temporal huérfano");
            envejecer(&p, 3600);
        }
        // (a) Temporal RECIENTE: el de un publicador que podría estar en plena carrera. Intocable.
        let reciente = runtime.join(".lock.enCurso.tmp");
        std::fs::write(&reciente, "{}").expect("plantar temporal reciente");
        // (b) Fichero ajeno y viejo: el barrido va por patrón, no por edad a secas.
        let ajeno = runtime.join("receipt-viejo.json");
        std::fs::write(&ajeno, "{}").expect("plantar fichero ajeno");
        envejecer(&ajeno, 3600);

        let guard = ws.acquire_lock().expect("adquirir el lock");
        let restantes: Vec<String> = std::fs::read_dir(&runtime)
            .expect("listar `.lodestar/runtime/`")
            .flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned())
            .collect();
        drop(guard);

        let temporales: Vec<&String> = restantes
            .iter()
            .filter(|n| n.starts_with(".lock.") && n.ends_with(".tmp"))
            .collect();
        assert_eq!(
            temporales,
            vec![&".lock.enCurso.tmp".to_string()],
            "los 20 temporales rancios se barren y SOLO sobrevive el reciente (el de la propia \
             publicación ya se retiró por su camino normal); quedaron: {restantes:?}"
        );
        assert!(
            ajeno.exists(),
            "y el barrido no toca nada que no sea `.lock.*.tmp`: los recibos y el journal viven en \
             este mismo directorio"
        );
    }

    /// Cuerpo REAL del lock (el que escribe una adquisición de verdad, con todos los campos que la
    /// implementación ponga: `host`, `token`, …) con las claves de `quitar` **eliminadas**. Es la
    /// forma de fabricar los cuerpos PARCIALES de los dos tests de frontera de abajo sin tener que
    /// enumerar a mano lo que el cuerpo lleva.
    fn cuerpo_real_sin(ws: &Workspace, quitar: &[&str]) -> serde_json::Value {
        let guard = ws
            .acquire_lock()
            .expect("tomar el lock para leer el cuerpo que escribe la implementación");
        let raw = std::fs::read_to_string(ws.lock_path()).expect("leer el fichero de lock");
        drop(guard);
        let mut meta: serde_json::Value = serde_json::from_str(&raw).unwrap_or_else(|e| {
            panic!("el cuerpo del lock debe ser JSON legible: {e}; era {raw:?}")
        });
        let obj = meta
            .as_object_mut()
            .expect("el cuerpo del lock es un objeto JSON");
        for clave in quitar {
            obj.remove(*clave);
        }
        meta
    }

    /// **E30-H02 · frontera del criterio de cuerpo no interpretable** — **Dado** un lock cuyo cuerpo
    /// declara un `pid` **vivo de esta máquina** pero **no** declara `timestamp`, **Cuando** otro
    /// proceso intenta adquirirlo, **Entonces** falla con `WRITE_CONFLICT` y el lock sobrevive byte
    /// a byte.
    ///
    /// POR QUÉ EXISTE: el reclamo del cuerpo no interpretable se decide con `pid.is_none() &&
    /// ts.is_none()`. Ese `&&` es la frontera exacta entre «nadie escribió nada» (reclamable) y
    /// «hay un dueño declarado» (intocable), y sin este test la mutación a `||` **sobrevive a la
    /// suite entera**: con ella, un escritor VIVO que declarase pid sin timestamp perdería su lock
    /// y dos procesos publicarían a la vez — el invariante #5 roto en silencio. Los tests que ya
    /// existían no la muerden porque ninguno ejercita un cuerpo **parcial**: o declaran los dos
    /// campos (dueño vivo/muerto de E25-H06) o ninguno (cuerpo vacío/truncado de arriba).
    ///
    /// UNIX-ONLY, como el resto del módulo: la prueba de vida por pid solo existe en Unix.
    #[test]
    fn un_lock_con_pid_vivo_y_sin_timestamp_no_se_reclama() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();

        let mut meta = cuerpo_real_sin(&ws, &["timestamp"]);
        meta["pid"] = serde_json::json!(std::process::id());
        let cuerpo = format!("{meta}\n");
        plantar_cuerpo(&ws, &cuerpo);

        let error = ws.acquire_lock().err().unwrap_or_else(|| {
            panic!(
                "un cuerpo que declara `pid` NO es un cuerpo «no interpretable»: hay un dueño, y \
                 aquí está VIVO en esta máquina, así que el lock es intocable (E25-H06). Si esto \
                 se pone rojo, el criterio de E30-H02 se ha relajado de `pid.is_none() && \
                 ts.is_none()` a un `||` y un escritor vivo pierde su lock"
            )
        });
        assert!(
            matches!(error, lodestar_workspace::WorkspaceError::WriteConflict(_)),
            "y se rechaza como conflicto de escritura (wire `WRITE_CONFLICT`); fue {error:?}"
        );
        assert_eq!(
            std::fs::read_to_string(ws.lock_path()).expect("el lock vivo debe seguir en disco"),
            cuerpo,
            "y sobrevive byte a byte: no se reclama, no se reescribe, no se toca"
        );
    }

    /// **E30-H02 · frontera del criterio de cuerpo no interpretable** — **Dado** un lock cuyo cuerpo
    /// **no** declara `pid` pero sí un `timestamp` **reciente**, **Cuando** otro proceso intenta
    /// adquirirlo, **Entonces** falla con `WRITE_CONFLICT` y el lock sobrevive byte a byte.
    ///
    /// Es la otra mitad de la frontera del `&&`: sin pid no hay prueba de vida, pero el `timestamp`
    /// sí da edad, y con el TTL sin vencer el criterio portable de E25-H06 dice **no reclamar**. Un
    /// `||` en `pid.is_none() && ts.is_none()` lo reclamaría al instante, saltándose el TTL entero.
    #[test]
    fn un_lock_sin_pid_y_con_timestamp_reciente_no_se_reclama() {
        let dir = tempfile::tempdir().unwrap();
        let ws = Workspace::open(dir.path()).unwrap();

        let mut meta = cuerpo_real_sin(&ws, &["pid"]);
        let ahora = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("reloj posterior a la época")
            .as_secs();
        meta["timestamp"] = serde_json::json!(ahora);
        let cuerpo = format!("{meta}\n");
        plantar_cuerpo(&ws, &cuerpo);

        let error = ws.acquire_lock().err().unwrap_or_else(|| {
            panic!(
                "un cuerpo sin `pid` pero con `timestamp` reciente SÍ es interpretable: no hay \
                 prueba de vida, así que manda el LOCK_TTL (E25-H06) y con el TTL sin vencer el \
                 lock no se reclama. Reclamarlo aquí sería saltarse el TTL entero"
            )
        });
        assert!(
            matches!(error, lodestar_workspace::WorkspaceError::WriteConflict(_)),
            "y se rechaza como conflicto de escritura (wire `WRITE_CONFLICT`); fue {error:?}"
        );
        assert_eq!(
            std::fs::read_to_string(ws.lock_path()).expect("el lock reciente debe seguir en disco"),
            cuerpo,
            "y sobrevive byte a byte: no se reclama, no se reescribe, no se toca"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// E31-H02 — el lote VACÍO (§26)
// ---------------------------------------------------------------------------------------------

/// **E31-H02 (riesgo bloqueante)** · Una transacción cuyo resultado NO difiere del canónico
/// produce un lote afectado **vacío** (`affected_paths`, `transaction.rs:236`), y esa ruta no
/// tiene ningún guard: `assert_writable`, `backup_originals` y `create_journal` la recorren
/// sobre un slice de cero elementos.
///
/// Hoy ese camino es inalcanzable desde `replace_text` porque la reserialización del frontmatter
/// siempre cambia bytes (§26). En cuanto el splice preserve la cabecera **pasa a ser alcanzable**,
/// así que esta prueba fija que la mecánica lo tolera: publica sin error, no mueve la revisión y
/// deja el `.md` byte a byte.
///
/// **Dado** un workspace con un documento, **Cuando** se aplica una transacción cuyo resultado es
/// idéntico al canónico, **Entonces** no falla, `changed_paths` va vacío y la revisión no se mueve.
#[test]
fn transaccion_con_lote_vacio_no_degenera() {
    let dir = tempfile::tempdir().unwrap();
    let ws = Workspace::open(dir.path()).unwrap();

    // Frontmatter en estilo FLOW a propósito (§26): antes de E31-H02 este documento no podía
    // producir un lote vacío —la reserialización lo reescribía siempre—, y ese era justamente el
    // defecto. Con la cabecera preservada, un `ReplaceBody` con el cuerpo que ya tiene no cambia
    // ni un byte, así que el lote sale vacío y esta prueba ejerce el camino que interesa.
    let contenido = "---\ntype: Nota\ntitle: A\ntags: [a, b]\n---\n\n# A\n\ncuerpo\n";
    std::fs::write(dir.path().join("a.md"), contenido).unwrap();

    let rev_antes = ws.workspace_revision().unwrap();

    // Un `ReplaceBody` con EXACTAMENTE el cuerpo que el documento ya tiene: el resultado
    // hipotético coincide con el canónico, así que el lote afectado sale vacío. El cuerpo se toma
    // de `parse_file` en vez de escribirlo a mano porque `SplitFront::body` incluye el salto que
    // sigue al `---` de cierre: un literal «a ojo» no casa, y el test fallaría por su propio error
    // en vez de por el del motor.
    let cuerpo_actual = lodestar_core::model::parse_file("a.md", contenido).body;
    let cs = change_set(
        "changeset:lote-vacio",
        vec![NormalizedOperation::ReplaceBody {
            path: RelPath::new("a.md").unwrap(),
            body: cuerpo_actual,
        }],
    );
    let cs = ChangeSet {
        base_revision: rev_antes.clone(),
        ..cs
    };

    let publicada = ws
        .apply_transaction_con_recibo(&cs, None)
        .expect("una transacción con lote vacío debe publicar sin error");

    assert!(
        publicada.changed_paths.is_empty(),
        "un lote vacío no cambia ningún path: {:?}",
        publicada.changed_paths
    );
    assert_eq!(
        ws.workspace_revision().unwrap(),
        rev_antes,
        "si no se escribe nada, la revisión del workspace no puede moverse"
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.md")).unwrap(),
        contenido,
        "el documento debe quedar byte a byte como estaba"
    );

    // La otra mitad del riesgo: el recibo de una transacción vacía tiene que ser REVERSIBLE, no
    // un callejón sin salida. Deshacer «nada» debe ser un no-op limpio, no un `Err` de copias
    // ausentes: `backup_originals` corrió sobre un lote vacío, así que el árbol de recuperación
    // existe pero no contiene ni un fichero.
    let rev_tras_apply = ws.workspace_revision().unwrap();
    let revertida = ws
        .revert_transaction_con_recibo(
            &publicada.txn_id,
            &format!("{}-revert", publicada.txn_id),
            &rev_tras_apply,
            None,
        )
        .expect("revertir una transacción de lote vacío no puede fallar: no hay nada que deshacer");

    assert!(
        revertida.changed_paths.is_empty(),
        "deshacer un lote vacío tampoco cambia ningún path: {:?}",
        revertida.changed_paths
    );
    assert_eq!(
        std::fs::read_to_string(dir.path().join("a.md")).unwrap(),
        contenido,
        "y el documento sigue byte a byte como estaba, tras aplicar Y revertir"
    );
}
