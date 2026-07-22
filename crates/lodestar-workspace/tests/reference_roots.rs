//! E11-H04 — Validación de paths externos (`referenceRoots`).
//!
//! Fase ROJA (TDD). Estos tests fijan el contrato de la historia ANTES de que exista la
//! implementación. La validación de paths de código (`implemented_by`/`verified_by`) es **I/O**
//! (comprobar la existencia de un fichero en disco bajo un `referenceRoot`), así que vive en
//! `lodestar-workspace` — el core es puro y no abre ficheros (invariante #2). El core recibe el
//! resultado (existe/no) y emite el diagnóstico; la workspace resuelve la existencia.
//!
//! Se aíslan en su PROPIO fichero (y no en `workspace.rs`) a propósito: la tarea prohíbe stubs en
//! producción, así que los símbolos objetivo no existen todavía y el fichero **no compilará** hasta
//! que se implemente la historia. Manteniéndolos aquí, `workspace.rs` (y el resto de tests del
//! crate) siguen compilando y en verde. Este es el "rojo" esperado: fallo de compilación por
//! símbolo inexistente, no un assert vacuo.
//!
//! -------------------------------------------------------------------------------------------------
//! API OBJETIVO ASUMIDA (documentada para el implementador — los tests son el contrato de "hecho"):
//!
//!   // Referencia externa de un concepto (`implemented_by`/`verified_by`) resuelta contra disco.
//!   pub struct ExternalReference {
//!       pub path: String,   // el path crudo del frontmatter, p. ej. "src/x.rs"
//!       pub exists: bool,   // si el fichero existe en disco bajo un `referenceRoot`
//!   }
//!
//!   // Informe de validación de las referencias externas de UN concepto contra `referenceRoots`.
//!   pub struct ExternalRefsReport {
//!       pub references: Vec<ExternalReference>,   // {path, exists} — alimenta knowledge_get (E10-H10)
//!       pub diagnostics: Vec<lodestar_core::types::Check>, // referencia externa rota (nuevo código
//!                                                          // o `LINK-REL` reusado: decisión abierta,
//!                                                          // el test NO fija el CheckCode)
//!   }
//!
//!   impl Workspace {
//!       // Resuelve las referencias externas del concepto contra `referenceRoots` del
//!       // `.lodestar/config.yaml` y produce los diagnósticos de referencia rota.
//!       pub fn external_refs(&self, concept: &RelPath)
//!           -> Result<ExternalRefsReport, WorkspaceError>;
//!
//!       // Guard del único escritor: `Err` si `path` cae bajo un `referenceRoot` (inmutable),
//!       // `Ok(())` si es escribible. El `WorkspaceError` resultante debe tener
//!       // `code() == "PERMISSION_DENIED"` (mapea a `ErrorCode::PermissionDenied` en la fachada).
//!       pub fn assert_writable(&self, path: &RelPath) -> Result<(), WorkspaceError>;
//!   }
//!
//! Se eligió una API DIRECTA de `lodestar-workspace` (no `App::knowledge_get`) porque estos tests
//! viven en el crate `workspace`, que NO depende de `lodestar-app`; además la validación de I/O es
//! competencia de la workspace. La forma `{path, exists}` casa con el `externalReferences:[{path,
//! exists}]` que E11-H04 debe exponer en `knowledge_get` (hoy `Vec` vacío, E10-H10).
//! -------------------------------------------------------------------------------------------------

use lodestar_core::types::{Check, RelPath};
use lodestar_workspace::Workspace;

/// Escribe `<root>/.lodestar/config.yaml` con `writableRoots`/`referenceRoots` dados.
fn escribe_config(root: &std::path::Path, writable: &str, reference: &str) {
    let dir = root.join(".lodestar");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("config.yaml"),
        format!("workspace:\n  writableRoots: [{writable}]\n  referenceRoots: [{reference}]\n"),
    )
    .unwrap();
}

/// Escribe un concepto conforme bajo `knowledge/` con un `implemented_by` dado (lista de un path).
fn escribe_concepto_con_implemented_by(root: &std::path::Path, rel: &str, code_path: &str) {
    let target = root.join(rel);
    std::fs::create_dir_all(target.parent().unwrap()).unwrap();
    let raw = format!(
        "---\ntype: Nota\ntitle: C\ndescription: d\nimplemented_by:\n  - {code_path}\n---\n\n# C\n\ncuerpo\n"
    );
    std::fs::write(&target, raw).unwrap();
}

/// `true` si algún diagnóstico menciona `needle` (en su mensaje, `targets` o `related`), para atar
/// el diagnóstico a la referencia rota concreta sin fijar el `CheckCode` (decisión abierta).
fn diag_menciona(diagnostics: &[Check], needle: &str) -> bool {
    diagnostics.iter().any(|d| {
        d.msg.contains(needle)
            || d.targets.iter().any(|p| p.as_str().contains(needle))
            || d.related.iter().any(|p| p.as_str().contains(needle))
    })
}

/// Criterio `ref_externa_rota`: un concepto con `implemented_by: [src/no_existe.rs]` inexistente
/// → un diagnóstico de referencia externa rota para ese path (benchmark §17: "Referenciar un
/// archivo de código inexistente → diagnóstico").
#[test]
fn ref_externa_rota() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    escribe_config(root, "knowledge", "src");
    // El concepto declara un path bajo el `referenceRoot` `src` que NO existe en disco.
    escribe_concepto_con_implemented_by(root, "knowledge/concepto.md", "src/no_existe.rs");
    // Deliberadamente NO creamos `src/no_existe.rs`.

    let ws = Workspace::open(root).unwrap();
    let concepto = RelPath::new("knowledge/concepto.md").unwrap();

    let report = ws
        .external_refs(&concepto)
        .expect("la validación de referencias externas no debe fallar por I/O aquí");

    // La referencia se resuelve como inexistente...
    assert!(
        report
            .references
            .iter()
            .any(|r| r.path == "src/no_existe.rs" && !r.exists),
        "la referencia a `src/no_existe.rs` debe resolverse con exists:false; eran: {:?}",
        report
            .references
            .iter()
            .map(|r| (r.path.clone(), r.exists))
            .collect::<Vec<_>>()
    );
    // ...y produce un diagnóstico de referencia externa rota que la menciona.
    assert!(
        !report.diagnostics.is_empty(),
        "una referencia externa rota debe producir al menos un diagnóstico"
    );
    assert!(
        diag_menciona(&report.diagnostics, "src/no_existe.rs"),
        "el diagnóstico debe referirse al path roto `src/no_existe.rs`; eran: {:?}",
        report
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.msg.clone()))
            .collect::<Vec<_>>()
    );
}

/// Regresión de SEGURIDAD (juez ciego): `external_refs` NO puede convertirse en un oráculo de
/// existencia de ficheros arbitrarios del host. Un `implemented_by`/`verified_by` con un path
/// ABSOLUTO (`/etc/hosts`) o con TRAVERSAL (`../secreto.txt`) NO debe resolverse por un `join`
/// crudo — debe pasar por `RelPath::new` y confinarse a `referenceRoots` ANTES de tocar disco
/// (invariante #6: `RelPath` es el único chokepoint de path-traversal). El contrato: un path
/// externo inválido/fuera de `referenceRoots` NUNCA se marca `exists:true`.
///
/// El vector de `..` es DETERMINISTA con independencia del entorno: montamos el bundle en un
/// subdirectorio y colocamos `secreto.txt` en su PADRE (fuera del bundle). Con el `root.join`
/// crudo actual, `../secreto.txt` escapa a ese fichero real → `exists:true` (fallo). La
/// implementación correcta lo rechaza en `RelPath::new` → `exists:false`. El vector absoluto
/// (`/etc/hosts`) refuerza el caso (en Unix, `join` de una ruta absoluta reemplaza la base).
#[test]
fn ref_externa_traversal() {
    let dir = tempfile::tempdir().unwrap();
    // El bundle vive en un SUBdirectorio; el "secreto" está fuera de él, en el padre.
    let root = dir.path().join("bundle");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(dir.path().join("secreto.txt"), "datos sensibles\n").unwrap();

    escribe_config(&root, "knowledge", "src");
    // Concepto con DOS vectores de ataque: absoluto (`implemented_by`) y traversal (`verified_by`).
    std::fs::create_dir_all(root.join("knowledge")).unwrap();
    std::fs::write(
        root.join("knowledge/concepto.md"),
        "---\ntype: Nota\ntitle: C\ndescription: d\n\
         implemented_by:\n  - /etc/hosts\n\
         verified_by:\n  - ../secreto.txt\n---\n\n# C\n\ncuerpo\n",
    )
    .unwrap();

    let ws = Workspace::open(&root).unwrap();
    let concepto = RelPath::new("knowledge/concepto.md").unwrap();

    let report = ws.external_refs(&concepto).unwrap();

    // Ningún path absoluto o con `..` puede resolverse como existente (sería un oráculo del host).
    // (Un implementador correcto puede INCLUIRLOS con exists:false o DESCARTARLOS; ambas cumplen
    // este contrato — lo prohibido es exists:true.)
    for r in &report.references {
        let sospechoso = r.path.starts_with('/') || r.path.contains("..");
        assert!(
            !(sospechoso && r.exists),
            "un path externo absoluto/con `..` se resolvió como existente (oráculo de ficheros \
             del host, viola invariante #6): {:?}",
            (r.path.clone(), r.exists)
        );
    }
    // Refuerzo explícito del vector determinista: `../secreto.txt` (fichero real fuera del bundle)
    // NUNCA debe verse como existente.
    assert!(
        !report
            .references
            .iter()
            .any(|r| r.path == "../secreto.txt" && r.exists),
        "el traversal `../secreto.txt` escapó del bundle y se resolvió como existente; refs: {:?}",
        report
            .references
            .iter()
            .map(|r| (r.path.clone(), r.exists))
            .collect::<Vec<_>>()
    );
}

/// Criterio `ref_externa_ok`: un `implemented_by` a un fichero real bajo `referenceRoots` →
/// `exists:true` y sin diagnóstico.
#[test]
fn ref_externa_ok() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    escribe_config(root, "knowledge", "src");
    escribe_concepto_con_implemented_by(root, "knowledge/concepto.md", "src/existe.rs");
    // El fichero de código referenciado SÍ existe bajo el `referenceRoot` `src`.
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(root.join("src/existe.rs"), "// real\n").unwrap();

    let ws = Workspace::open(root).unwrap();
    let concepto = RelPath::new("knowledge/concepto.md").unwrap();

    let report = ws.external_refs(&concepto).unwrap();

    assert!(
        report
            .references
            .iter()
            .any(|r| r.path == "src/existe.rs" && r.exists),
        "la referencia a `src/existe.rs` (fichero real) debe resolverse con exists:true; eran: {:?}",
        report
            .references
            .iter()
            .map(|r| (r.path.clone(), r.exists))
            .collect::<Vec<_>>()
    );
    assert!(
        report.diagnostics.is_empty(),
        "una referencia externa que existe NO debe producir diagnóstico; eran: {:?}",
        report
            .diagnostics
            .iter()
            .map(|d| (d.code.as_str(), d.msg.clone()))
            .collect::<Vec<_>>()
    );
}

/// Criterio `reference_roots_inmutable`: un intento de ESCRITURA sobre `referenceRoots` →
/// `PERMISSION_DENIED`. Los `referenceRoots` son visibles pero NUNCA escribibles por Lodestar.
#[test]
fn reference_roots_inmutable() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    escribe_config(root, "knowledge", "src");

    let ws = Workspace::open(root).unwrap();

    // Escribir bajo el `referenceRoot` `src` debe rechazarse con el código estable PERMISSION_DENIED.
    let bajo_reference = RelPath::new("src/nuevo.rs").unwrap();
    let err = ws
        .assert_writable(&bajo_reference)
        .expect_err("escribir bajo un referenceRoot debe rechazarse");
    assert_eq!(
        err.code(),
        "PERMISSION_DENIED",
        "el rechazo de escritura sobre `referenceRoots` debe llevar el código estable \
         PERMISSION_DENIED (mapea a ErrorCode::PermissionDenied en la fachada); era: {err:?}"
    );

    // Control (evita vacuidad): un path bajo un `writableRoot` SÍ es escribible.
    let bajo_writable = RelPath::new("knowledge/ok.md").unwrap();
    assert!(
        ws.assert_writable(&bajo_writable).is_ok(),
        "un path bajo `writableRoots` (`knowledge`) debe ser escribible"
    );
}
