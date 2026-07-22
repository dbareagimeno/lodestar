//! E14-H02 — Convivencia con proyectos de software (config por proyecto + detección de escrituras
//! externas).
//!
//! Fase ROJA (TDD). Esta historia es de **composición**: la base de conocimiento vive DENTRO de un
//! repo de código sin que Lodestar toque el código ni git. Compone piezas ya construidas —
//! `writableRoots`/`referenceRoots`/`ignored` de la config por proyecto (E9-H05,
//! `crates/lodestar-workspace/src/config.rs`), el guard del único escritor `assert_writable`
//! (E11-H04, `src/external_refs.rs`) y la re-verificación optimista de la `WorkspaceRevision` base
//! `reverify_base_revision` (E13-H02, `src/lib.rs`) — a través del **orquestador transaccional
//! completo** `Workspace::apply_transaction` (E13-H08, `src/transaction.rs`).
//!
//! Estos tests ejercitan `apply_transaction` **end-to-end**, algo que ningún test de
//! `transactions.rs` hace hoy (esos ejercitan las primitivas por separado: `materialize_staging`,
//! `create_journal`, `backup_originals`, `publish`…). Verifican que, al COMPONERSE dentro del único
//! escritor:
//!   1. `apply_transaction` rechaza con `PERMISSION_DENIED` un cambio que escribiría FUERA de
//!      `writableRoots` (bajo un `referenceRoot` `src/`) **sin tocar disco**, y aplica un cambio bajo
//!      `writableRoots` (`knowledge/`) — el paso 5 (`assert_writable`) sobre CADA path afectado.
//!   2. `apply_transaction` detecta una escritura EXTERNA (un agente que editó un `.md` writable
//!      entre el plan y el apply) con `WRITE_CONFLICT` — el paso 7 (`reverify_base_revision`), que
//!      relee la revisión del disco: Lodestar **no asume acceso exclusivo**.
//!
//! Forma de config asumida (E9-H05, ya existente en `WorkspaceConfig`): YAML `.lodestar/config.yaml`
//! con `workspace.writableRoots` / `workspace.referenceRoots` como listas de paths relativos. Estos
//! campos YA EXISTEN en `WorkspaceConfig` (no hace falta que el implementador los añada); estos tests
//! fijan el comportamiento COMPUESTO end-to-end, no la forma del dato.

use lodestar_core::plan;
use lodestar_core::types::{
    ChangeSet, ChangeSetId, FrontmatterPatch, NormalizedOperation, PlanHash, RelPath,
    RiskAssessment, SemanticDiff, ValidationReport, WorkspaceRevision,
};
use lodestar_workspace::Workspace;
use std::collections::BTreeMap;
use std::path::Path;

/// Escribe `<root>/.lodestar/config.yaml` con `writableRoots`/`referenceRoots` dados (E9-H05).
/// Modela un repo de código adoptado: `knowledge/` escribible, `src/` visible pero inmutable.
fn escribe_config(root: &Path, writable: &str, reference: &str) {
    let dir = root.join(".lodestar");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.yaml"),
        format!("workspace:\n  writableRoots: [{writable}]\n  referenceRoots: [{reference}]\n"),
    )
    .unwrap();
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

/// Un `Create` conforme (con `type` y `title`, sin tags) que resuelve al `.md` `path`. Sin tags a
/// propósito: así la auto-regeneración de índices/tags de la transacción no añade paths afectados
/// fuera de `writableRoots` (mantiene el conjunto afectado = solo el `.md` del plan).
fn create_conforme(path: &str, title: &str) -> NormalizedOperation {
    NormalizedOperation::Create {
        path: RelPath::new(path).unwrap(),
        frontmatter: patch(&[("type", "Nota"), ("title", title)]),
        body: Some(format!("# {title}\n\ncuerpo\n")),
    }
}

/// Envuelve operaciones en un `ChangeSet` con la `base_revision` dada (la que el plan capturó) — es
/// la revisión que `apply_transaction` re-verifica en el paso 7. Los campos de análisis van a
/// `Default` (la mecánica transaccional no los consume).
fn change_set(
    id: &str,
    base: &WorkspaceRevision,
    operations: Vec<NormalizedOperation>,
) -> ChangeSet {
    ChangeSet {
        id: ChangeSetId(id.to_string()),
        base_revision: base.clone(),
        operations,
        plan_hash: PlanHash("blake3:test".to_string()),
        risk: RiskAssessment::default(),
        semantic_diff: SemanticDiff::default(),
        validation: ValidationReport::default(),
        expires_at: "0".to_string(),
    }
}

/// **E14-H02** · Criterio `solo_escribe_writable`: dado un repo con `knowledge/` (writable) y `src/`
/// (reference), Lodestar SOLO escribe bajo `knowledge/`. Verifica AMBOS lados a través del
/// orquestador transaccional completo:
///   (A) un `Create` que apunta a `src/` (bajo `referenceRoot`, fuera de `writableRoots`) se rechaza
///       con `PERMISSION_DENIED` **sin tocar disco** (el guard del paso 5 corre ANTES del staging y
///       de cualquier rename del canónico);
///   (B) un `Create` bajo `knowledge/` (writable) SÍ se aplica y aparece en disco.
#[test]
fn solo_escribe_writable() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    escribe_config(root, "knowledge", "src");

    let ws = Workspace::open(root).unwrap();

    // Revisión base del conocimiento escribible (vacío bajo `knowledge/` al abrir): lo que un plan
    // capturaría. Se reutiliza en (A) y (B) porque (A) no debe tocar disco (revisión invariante).
    let base = ws
        .workspace_revision()
        .expect("computar la revisión base del workspace");

    // ---- (A) Un cambio que escribiría FUERA de `writableRoots` (bajo `src/`) se rechaza. ----
    let cs_fuera = change_set(
        "changeset:escribe-fuera",
        &base,
        vec![create_conforme("src/prohibido.md", "Prohibido")],
    );

    let err = ws.apply_transaction(&cs_fuera).expect_err(
        "aplicar un cambio bajo un referenceRoot/fuera de writableRoots debe rechazarse",
    );
    assert_eq!(
        err.code(),
        "PERMISSION_DENIED",
        "escribir fuera de `writableRoots` (bajo `src/`) debe mapear al código estable \
         PERMISSION_DENIED; era: {err:?}"
    );

    // ...y NO tocó disco: ni el `.md` prohibido bajo `src/`, ni se creó el directorio `src/`.
    assert!(
        !root.join("src/prohibido.md").exists(),
        "un cambio rechazado por PERMISSION_DENIED no debe materializar el `.md` fuera de writable"
    );

    // La revisión del conocimiento escribible no cambió (el rechazo no publicó nada).
    let tras_rechazo = ws
        .workspace_revision()
        .expect("recomputar la revisión tras el rechazo");
    assert_eq!(
        tras_rechazo, base,
        "un apply rechazado por PERMISSION_DENIED no debe alterar la revisión del workspace"
    );

    // ---- (B) Un cambio bajo `knowledge/` (writable) SÍ se aplica. ----
    let cs_dentro = change_set(
        "changeset:escribe-dentro",
        &base,
        vec![create_conforme("knowledge/permitido.md", "Permitido")],
    );

    ws.apply_transaction(&cs_dentro)
        .expect("aplicar un cambio bajo `writableRoots` (`knowledge/`) debe publicarse");

    // El `.md` writable quedó en disco (el canónico es la única fuente de verdad, invariante #1).
    assert!(
        root.join("knowledge/permitido.md").is_file(),
        "un cambio bajo `writableRoots` (`knowledge/`) debe materializar el `.md` en disco"
    );
}

/// **E14-H02** · Criterio `detecta_escritura_externa`: dado que un agente externo editó un `.md`
/// writable ENTRE el plan y el apply, al aplicar el conflicto se detecta (`REVISION_CONFLICT`/
/// `WRITE_CONFLICT`) sin corromper ni publicar. Modela «no asumir acceso exclusivo»: la edición
/// externa cambia la revisión del conocimiento escribible que el plan capturó, y
/// `apply_transaction` la relee del disco al re-verificar (paso 7).
#[test]
fn detecta_escritura_externa() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    escribe_config(root, "knowledge", "src");

    // Un concepto writable canónico previo, conforme, sobre el que se "planifica".
    std::fs::create_dir_all(root.join("knowledge")).unwrap();
    let nota_disk = root.join("knowledge/nota.md");
    std::fs::write(
        &nota_disk,
        "---\ntype: Nota\ntitle: Nota\n---\n\n# Nota\n\ncuerpo original del plan\n",
    )
    .unwrap();

    let ws = Workspace::open(root).unwrap();
    let nota = RelPath::new("knowledge/nota.md").unwrap();

    // El plan captura la revisión base del conocimiento escribible ANTES de la edición externa.
    let base = ws
        .workspace_revision()
        .expect("el plan captura la baseWorkspaceRevision");

    // Change set que modifica el cuerpo de `knowledge/nota.md` (writable) partiendo de `base`.
    let cs = change_set(
        "changeset:modifica-nota",
        &base,
        vec![NormalizedOperation::ReplaceBody {
            path: nota.clone(),
            body: "# Nota\n\ncuerpo nuevo del change set\n".to_string(),
        }],
    );

    // El plan es válido contra el canónico capturado (precondición no vacua): produce un resultado
    // distinto del canónico, luego hay algo que publicar si nadie interfiere.
    let canonico = filemap_canonico(root);
    let previsto = plan::apply_normalized_ops(&canonico, &cs.operations)
        .expect("el plan aplica sobre el canónico capturado");
    assert_ne!(
        canonico, previsto,
        "precondición: el change set debe cambiar el canónico (test no vacuo)"
    );

    // --- Un AGENTE EXTERNO edita el mismo `.md` writable ENTRE el plan y el apply. ---
    // (Escritura directa a disco, como la haría otro proceso: Lodestar no tiene acceso exclusivo.)
    let contenido_externo =
        "---\ntype: Nota\ntitle: Nota\n---\n\n# Nota\n\ncuerpo REESCRITO por un agente externo\n";
    std::fs::write(&nota_disk, contenido_externo).unwrap();

    // La revisión del conocimiento escribible cambió respecto de la que el plan capturó.
    let tras_externa = ws
        .workspace_revision()
        .expect("recomputar la revisión tras la edición externa");
    assert_ne!(
        tras_externa, base,
        "precondición: la edición externa debe cambiar la revisión del workspace escribible"
    );

    // Al aplicar, el control optimista detecta que la base ya no es la actual → conflicto de
    // escritura. Sin corromper ni publicar el resultado del plan.
    let err = ws
        .apply_transaction(&cs)
        .expect_err("una escritura externa entre plan y apply debe detectarse, no pisarse");
    assert!(
        matches!(err.code(), "WRITE_CONFLICT" | "REVISION_CONFLICT"),
        "una escritura externa entre plan y apply debe mapear a WRITE_CONFLICT/REVISION_CONFLICT; \
         era: {err:?}"
    );

    // El canónico writable conserva la EDICIÓN EXTERNA íntegra (no se pisó con el plan ni se corrompió).
    let en_disco =
        std::fs::read_to_string(&nota_disk).expect("`knowledge/nota.md` debe seguir legible");
    assert_eq!(
        en_disco, contenido_externo,
        "el apply abortado no debe pisar ni corromper la edición externa del `.md` writable"
    );
}

/// `FileMap` del conocimiento `.md` canónico (claves relativas POSIX), excluyendo `.lodestar/` y
/// `.git/`. Es el `files` con el que el core prevé el resultado del plan.
fn filemap_canonico(root: &Path) -> lodestar_core::types::FileMap {
    let mut out = lodestar_core::types::FileMap::new();
    fn walk(dir: &Path, root: &Path, out: &mut lodestar_core::types::FileMap) {
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
                    .replace('\\', "/");
                let content = std::fs::read_to_string(&path).unwrap();
                out.insert(RelPath::new(&rel).unwrap(), content);
            }
        }
    }
    walk(root, root, &mut out);
    out
}
