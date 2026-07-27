//! E11-H04 — Paths externos (`referenceRoots`): **write policy**.
//!
//! ## Qué queda aquí y por qué (E23-H12)
//!
//! `referenceRoots` tenía DOS responsabilidades. La primera —resolver contra disco los campos de
//! frontmatter `implemented_by`/`verified_by` y devolver `{path, exists}`— se **RETIRA en E23-H12
//! sin sustituto**: eran las últimas claves de frontmatter con semántica impuesta y no configurable
//! (`crates/lodestar-workspace/src/external_refs.rs`), lo que contradice el invariante 3 de
//! `ARCHITECTURE.md §20.2` (ningún nombre de campo activa reglas especiales). Con ella cae la opción
//! `include:["externalReferences"]` de `knowledge_get`, que se quedaría sin fuente. La segunda —la
//! **contención** de `assert_writable`: raíces visibles pero NUNCA escribibles— se conserva intacta,
//! y es lo único que este fichero cubre ya (`reference_roots_inmutable`).
//!
//! ## Dónde fue a parar la cobertura retirada
//!
//! - `ref_externa_rota` / `ref_externa_ok` (E11-H04): RETIRADOS con la capacidad. No hay sustituto
//!   porque no hay nada que sustituir: `implemented_by` pasa a ser metadata del usuario como
//!   `autor` o `tags`, y quien quiera verificar rutas de código lo hace fuera de Lodestar.
//! - `ref_externa_traversal` (regresión de SEGURIDAD hallada por un juez ciego: `external_refs` no
//!   podía volverse un **oráculo de existencia de ficheros arbitrarios del host** vía
//!   `implemented_by: [/etc/hosts]` / `verified_by: [../secreto.txt]`): el vector desaparece con la
//!   función —ya no queda ningún camino que resuelva contra disco una cadena cruda del
//!   frontmatter—, pero la propiedad NO se deja huérfana. Se MIGRA, endurecida, a
//!   `frontmatter_no_es_oraculo_de_ficheros_del_host` (`crates/lodestar-mcp/tests/mcp.rs`), que la
//!   asevera en la superficie donde importaba (`knowledge_get`) y con un contrato más fuerte: antes
//!   se prohibía `exists:true`, ahora se prohíbe **cualquier** resolución. El chokepoint que la
//!   sostiene sigue cubierto aparte: `RelPath::new` rechaza absolutas y `..`
//!   (`crates/lodestar-core/tests/core.rs::relpath_rechaza_absolutas_y_dotdot`), la
//!   deserialización de `DocumentRef` también (`core.rs::ref_rechaza_traversal`), el wire lo rechaza
//!   como error de ejecución (`mcp.rs::protocolo_errores_y_ping`, `rechaza_absoluta`,
//!   `rechaza_escape`) y la config rechaza raíces con traversal
//!   (`workspace.rs::roots_rechazan_traversal`).

use lodestar_core::types::RelPath;
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
