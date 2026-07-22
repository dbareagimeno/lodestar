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
