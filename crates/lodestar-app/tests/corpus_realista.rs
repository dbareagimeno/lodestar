//! E33-H01 · BDD-2 — El perfil **realista** del generador de escala no es un corpus trivial
//! (`ARCHITECTURE.md §22.2`).
//!
//! El perfil `plano` de E14-H05 es homogéneo a propósito (comparabilidad con las cifras
//! históricas): 10k documentos sin enlaces entre sí y con el mismo frontmatter. Medir **solo**
//! sobre él mentiría, porque el coste real del motor está en resolver enlaces, computar backlinks y
//! consultar frontmatter heterogéneo. El perfil `realista` existe para eso, y este test fija su
//! contrato mínimo: abierto con la API real (`App::open` + `knowledge_check` + `graph_query`), el
//! corpus tiene **grafo** (documentos enlazados y huérfanos) y **fauna de diagnósticos** (al menos
//! un diagnóstico de enlace).
//!
//! Se juzga a través de los servicios de `App` —no leyendo los `.md` a mano— por dos razones: es la
//! misma superficie que medirá el banco (`§22.4`), y así el test caza también un corpus que
//! *parezca* rico en disco pero que el motor no vea (p. ej. enlaces que no resuelven a documentos).

use lodestar_app::{App, CheckScope};
use lodestar_core::types::{CheckCode, Severity};
use lodestar_fixtures::escala::{self, Perfil};

/// Escala mínima de `§22.2` (~100 documentos): la propiedad es cualitativa, no de volumen.
const TAMANO_MINIMO: usize = 100;

/// Semilla fija: el corpus que este test juzga es siempre el mismo (BDD-1 garantiza que lo sea).
const SEMILLA: u64 = 0xE33_0001;

// ===========================================================================
// E33-H01 · BDD-2 — `perfil_realista_produce_grafo_y_diagnosticos`.
//
// Dado el perfil realista a escala mínima, Cuando se abre con `App::open` y se corre
// `knowledge_check`, Entonces el corpus contiene documentos enlazados, huérfanos y al menos un
// diagnóstico de enlace — no es un corpus trivial.
// ===========================================================================
#[test]
fn perfil_realista_produce_grafo_y_diagnosticos() {
    let dir = tempfile::tempdir().unwrap();
    escala::genera(dir.path(), Perfil::Realista, TAMANO_MINIMO, SEMILLA)
        .expect("el generador de escala debe escribir el corpus realista");

    let app = App::open(dir.path()).expect("el corpus realista debe abrir");

    // --- Propiedad 1: hay ARISTAS reales — documentos enlazados entre sí que el motor resuelve. ---
    // `components` parte del grafo completo, así que sus `edges` son las aristas del workspace
    // (acotadas a la página). Con `limit` == tamaño no hay truncamiento.
    let grafo = app
        .graph_query(
            "components",
            None,
            None,
            None,
            None,
            Some(TAMANO_MINIMO),
            None,
        )
        .expect("graph_query components debe responder sobre el corpus realista");
    assert!(
        grafo.edges.len() >= TAMANO_MINIMO / 10,
        "el perfil realista debe tener un grafo denso de verdad (≥ {} aristas para {} documentos), \
         no {} — si no, medir sobre él no dice nada del coste de resolver enlaces",
        TAMANO_MINIMO / 10,
        TAMANO_MINIMO,
        grafo.edges.len(),
    );

    // --- Propiedad 2: hay HUÉRFANOS (documentos aislados: ni entrantes ni salientes). ---
    // Son el caso que distingue un grafo modelado de una cadena sintética donde todo está conectado.
    let aislados = app
        .graph_query(
            "isolated",
            None,
            None,
            None,
            None,
            Some(TAMANO_MINIMO),
            None,
        )
        .expect("graph_query isolated debe responder sobre el corpus realista");
    assert!(
        !aislados.nodes.is_empty(),
        "el perfil realista debe contener al menos un documento huérfano (aislado); \
         un corpus donde todo está enlazado no ejercita `isolated` ni el coste de detectarlo"
    );
    assert!(
        aislados.nodes.len() < TAMANO_MINIMO,
        "…pero NO todos los documentos pueden ser huérfanos ({} de {}): eso sería el corpus \
         trivial sin grafo que este criterio descarta",
        aislados.nodes.len(),
        TAMANO_MINIMO,
    );

    // --- Propiedad 3 (LO ESENCIAL): al menos un DIAGNÓSTICO DE ENLACE en `knowledge_check`. ---
    let reporte = app
        .knowledge_check(&CheckScope::Workspace, None, false, Some(1_000), None)
        .expect("knowledge_check debe responder sobre el corpus realista");

    let codigos_de_enlace = [
        CheckCode::LinkTargetMissing,
        CheckCode::LinkCaseMismatch,
        CheckCode::LinkEscapesWorkspace,
    ];
    let de_enlace: Vec<&lodestar_core::types::Check> = reporte
        .diagnostics
        .iter()
        .filter(|c| codigos_de_enlace.contains(&c.code))
        .collect();
    assert!(
        !de_enlace.is_empty(),
        "el perfil realista debe producir al menos un diagnóstico de enlace \
         (LINK-TARGET-MISSING / LINK-CASE-MISMATCH / LINK-ESCAPES-WORKSPACE); \
         diagnósticos observados = {:?}",
        reporte
            .diagnostics
            .iter()
            .map(|c| c.code.as_str())
            .collect::<Vec<_>>(),
    );

    // El summary es coherente con lo devuelto: si hay un dangling a documento (`Err`), el corpus no
    // es «válido», y eso es deliberado — el banco de conformidad necesita fauna, no un corpus limpio.
    assert!(
        reporte.summary.errors + reporte.summary.warnings > 0,
        "el corpus realista debe traer fauna de diagnósticos (errores o avisos), no silencio: {:?}",
        reporte.summary
    );

    // Cada diagnóstico de enlace apunta a un documento real del corpus (targets no vacío): un
    // diagnóstico sin ancla no serviría para escribir esperados en el banco (H02).
    for c in &de_enlace {
        assert!(
            !c.targets.is_empty(),
            "cada diagnóstico de enlace debe nombrar el documento que lo produce: {c:?}"
        );
        assert!(
            c.level >= Severity::Warn,
            "los diagnósticos de enlace del corpus deben ser Warn o Err (visibles por defecto): {c:?}"
        );
    }
}

// ===========================================================================
// E33-H01 · BDD-2 (contraste) — el perfil PLANO sigue siendo el corpus homogéneo de E14-H05.
//
// Sin este contraste, «realista» podría implementarse como un alias de «plano» y BDD-2 pasaría a
// medias. El plano es, por definición, el que NO tiene grafo: todos sus documentos son aislados.
// ===========================================================================
#[test]
fn perfil_plano_sigue_siendo_homogeneo_y_sin_grafo() {
    let dir = tempfile::tempdir().unwrap();
    escala::genera(dir.path(), Perfil::Plano, TAMANO_MINIMO, SEMILLA)
        .expect("el generador de escala debe escribir el corpus plano");

    let app = App::open(dir.path()).expect("el corpus plano debe abrir");

    let grafo = app
        .graph_query(
            "components",
            None,
            None,
            None,
            None,
            Some(TAMANO_MINIMO),
            None,
        )
        .expect("graph_query components debe responder sobre el corpus plano");
    assert!(
        grafo.edges.is_empty(),
        "el perfil plano de E14-H05 es homogéneo y SIN enlaces entre documentos (comparabilidad \
         con sus cifras históricas); se observaron {} aristas",
        grafo.edges.len()
    );

    // Y no produce fauna de enlaces: es el corpus limpio contra el que se miden latencias.
    let reporte = app
        .knowledge_check(&CheckScope::Workspace, None, false, Some(1_000), None)
        .expect("knowledge_check debe responder sobre el corpus plano");
    assert!(
        reporte.diagnostics.iter().all(|c| !matches!(
            c.code,
            CheckCode::LinkTargetMissing
                | CheckCode::LinkCaseMismatch
                | CheckCode::LinkEscapesWorkspace
        )),
        "el perfil plano no debe traer diagnósticos de enlace: {:?}",
        reporte
            .diagnostics
            .iter()
            .map(|c| c.code.as_str())
            .collect::<Vec<_>>()
    );
}
