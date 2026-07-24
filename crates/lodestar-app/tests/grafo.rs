//! `impact_analyze` sobre el grafo universal — **E17-H05** (`ARCHITECTURE.md §20.7`, `§20.10`).
//!
//! Fase ROJA del cuarto criterio de E17-H05. Los otros tres viven en
//! `crates/lodestar-mcp/tests/grafo.rs` (superficie de wire); este vive aquí porque
//! `App::impact_analyze` **es** su superficie: lo que la historia retira —el cálculo de impacto a
//! partir de tipos y relaciones tipadas del `schema.yaml`— es lógica de esta capa, no del wire.
//!
//! Fichero propio (y no una ampliación de `plan.rs`/`escritura.rs`) por la razón de siempre: cada
//! fichero de `tests/` es un binario independiente.
//!
//! ---
//!
//! ## Lo que fija este test
//!
//! `impact_analyze` deja de depender de tipos OKF y de relaciones tipadas (`§20.10`). Su impacto se
//! calcula **solo sobre el grafo de enlaces**: backlinks directos, blast-radius entrante y
//! documentos afectados. Los `blockingReferences` derivados de relaciones obligatorias del
//! `.lodestar/schema.yaml` **desaparecen**; el campo se conserva en el wire (siempre vacío) hasta
//! que E20 retire `core::schema` entero, para no romper `contracts/mcp.yml` en esta historia.
//!
//! Consecuencia buscada: un workspace **con** `schema.yaml`, **con** `type:` en el frontmatter y
//! **con** relaciones tipadas apuntando al documento produce exactamente el mismo informe que uno
//! sin nada de eso. Lo único que cuenta son los enlaces Markdown.

use std::collections::BTreeSet;
use std::path::Path;

use lodestar_app::App;
use lodestar_core::types::{DocumentRef, RelPath};

/// Escribe un fichero dentro del workspace temporal, creando los directorios intermedios.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// `RelPath` para rutas obviamente válidas (invariante #6: nunca un string crudo).
fn rp(p: &str) -> RelPath {
    RelPath::new(p).unwrap_or_else(|e| panic!("`{p}` debe ser un RelPath válido: {e:?}"))
}

/// `DocumentRef` por path (identidad v2).
fn dref(p: &str) -> DocumentRef {
    DocumentRef {
        path: rp(p),
        id: None,
    }
}

/// Workspace con `docs/target.md` enlazado desde **5 documentos a 4 profundidades distintas**, uno
/// de ellos con un enlace **de referencia**.
///
/// Y —a propósito— con todo el aparato OKF encima: un `.lodestar/schema.yaml` que declara la
/// relación tipada `depends_on`, y los 5 documentos declarándola hacia el target en su frontmatter.
/// Ese aparato es lo que hoy produce 5 `blockingReferences`, y lo que E17-H05 tiene que dejar de
/// mirar.
fn workspace_impacto() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();

    escribe(
        dir.path(),
        ".lodestar/schema.yaml",
        "\
version: \"1\"
types:
  component:
    name: component
    description: Un componente del sistema
  note:
    name: note
    description: Una nota libre
  task:
    name: task
    description: Una tarea que depende de un componente
    relations:
      depends_on:
        targetTypes: [component]
        cardinality: many
",
    );

    escribe(
        dir.path(),
        "docs/target.md",
        "---\ntype: component\ntitle: Objetivo\n---\n\n# Objetivo\n\nMe enlazan desde todas partes.\n",
    );

    // Los 5 afectados: mismo destino, distinta profundidad y distinta forma de href.
    let afectados: [(&str, &str); 5] = [
        ("README.md", "Ver [el objetivo](docs/target.md).\n"),
        ("docs/uno.md", "Ver [el objetivo](target.md).\n"),
        ("docs/sub/dos.md", "Ver [el objetivo](../target.md).\n"),
        (
            "packages/api/docs/tres.md",
            "Ver [el objetivo](../../../docs/target.md).\n",
        ),
        // Enlace DE REFERENCIA: la regex heredada no lo ve, así que hoy este documento no cuenta
        // como afectado. Es lo que hace que el recuento no sea vacuo.
        (
            "docs/cuatro.md",
            "Ver [el objetivo][t].\n\n[t]: target.md\n",
        ),
    ];
    for (ruta, cuerpo) in afectados {
        escribe(
            dir.path(),
            ruta,
            &format!(
                "---\ntype: task\ntitle: {ruta}\ndepends_on:\n  - docs/target.md\n---\n\n# Tarea\n\n{cuerpo}"
            ),
        );
    }

    // Decoy: enlaza a un documento APARTE (no al target ni a ninguno de sus afectados, para
    // quedarse también fuera del blast-radius transitivo) y NO declara la relación tipada. Sin él,
    // un cálculo que devolviera «todos los documentos» pasaría el criterio.
    escribe(
        dir.path(),
        "docs/decoy.md",
        "---\ntype: note\ntitle: Decoy\n---\n\n# Decoy\n\nVer [aparte](aparte.md).\n",
    );
    escribe(
        dir.path(),
        "docs/aparte.md",
        "---\ntype: note\ntitle: Aparte\n---\n\n# Aparte\n\nNo tengo nada que ver con el objetivo.\n",
    );

    dir
}

/// **Dado** un `impact_analyze` sobre un documento con 5 backlinks, **Cuando** se calcula,
/// **Entonces** reporta los 5 afectados sin mencionar tipos ni relaciones.
#[test]
fn impacto_sin_tipos_okf() {
    let dir = workspace_impacto();
    let app = App::open(dir.path()).expect("el workspace temporal debe abrir");

    // `kind: "delete"` a propósito: es la ÚNICA operación que hoy dispara el cálculo de
    // `blockingReferences` a partir de las relaciones tipadas del schema.
    let informe = app
        .impact_analyze(&dref("docs/target.md"), "delete", None)
        .expect("el documento existe");

    // (1) Los 5 afectados directos, a cualquier profundidad y con cualquier forma de enlace.
    assert_eq!(
        informe.summary.directly_affected, 5,
        "el impacto directo es el nº de backlinks del GRAFO UNIVERSAL: 4 enlaces inline a 4 \
         profundidades + 1 de referencia (invisible para la regex heredada). Informe: {informe:?}"
    );
    let afectados: BTreeSet<String> = informe
        .affected_documents
        .iter()
        .map(|p| p.as_str().to_string())
        .collect();
    assert_eq!(
        afectados,
        BTreeSet::from([
            "README.md".to_string(),
            "docs/uno.md".to_string(),
            "docs/sub/dos.md".to_string(),
            "packages/api/docs/tres.md".to_string(),
            "docs/cuatro.md".to_string(),
        ]),
        "`affectedDocuments` son los 5 documentos que enlazan al target: {informe:?}"
    );
    for fuera in ["docs/decoy.md", "docs/aparte.md"] {
        assert!(
            !afectados.contains(fuera),
            "«{fuera}» no depende del target ni directa ni transitivamente: {informe:?}"
        );
    }
    assert_eq!(
        informe.summary.transitively_affected,
        informe.affected_documents.len(),
        "el recuento transitivo va con la lista que lo acompaña: {informe:?}"
    );

    // (2) Sin tipos ni relaciones: aunque el workspace traiga `schema.yaml`, `type:` en el
    //     frontmatter y `depends_on` apuntando al target, NO hay bloqueos estructurales.
    assert!(
        informe.blocking_references.is_empty(),
        "los `blockingReferences` derivados de relaciones tipadas obligatorias desaparecen \
         (`§20.10`): una relación es un enlace Markdown y nada más. Hoy este mismo workspace \
         produce 5: {:?}",
        informe.blocking_references
    );
    assert_eq!(
        informe.summary.blocking_references, 0,
        "…y el contador va con la lista: {informe:?}"
    );

    // (3) El riesgo sale SOLO del grafo. Hoy es «high» porque hay bloqueos; sin ellos, lo decide el
    //     número de afectados (5 sobre un umbral alto de 20).
    assert_ne!(
        informe.summary.risk, "high",
        "sin bloqueos estructurales, 5 backlinks no son un riesgo alto: {informe:?}"
    );
    assert!(
        ["low", "medium", "high"].contains(&informe.summary.risk.as_str()),
        "el nivel de riesgo sigue siendo del conjunto cerrado {{low,medium,high}}: {informe:?}"
    );

    // (4) Y el texto que lee el agente tampoco habla de tipos ni de relaciones tipadas: ese
    //     vocabulario dejó de existir en el modelo (`§20.3`).
    for r in &informe.recommendations {
        let bajo = r.to_lowercase();
        for palabra in ["relaci", "tipada", "obligatoria"] {
            assert!(
                !bajo.contains(palabra),
                "una recomendación de impacto no puede hablar de «{palabra}»: «{r}»"
            );
        }
    }
    assert!(
        informe
            .recommendations
            .iter()
            .any(|r| r.to_lowercase().contains("enlace")),
        "…pero sí de los enlaces entrantes, que es lo que hay que revisar: {:?}",
        informe.recommendations
    );

    // (5) Un documento sin backlinks no arrastra impacto: el informe describe el grafo, no el
    //     catálogo de tipos.
    let sin_impacto = app
        .impact_analyze(&dref("docs/decoy.md"), "delete", None)
        .expect("el decoy existe");
    assert_eq!(
        sin_impacto.summary.directly_affected, 0,
        "a nadie le importa el decoy: {sin_impacto:?}"
    );
    assert!(
        sin_impacto.blocking_references.is_empty(),
        "…y tampoco tiene bloqueos: {sin_impacto:?}"
    );
}
