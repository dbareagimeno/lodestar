//! E14-H05 — Métricas de evaluación y presupuesto de escala (`ARCHITECTURE.md §19.8`, `§11`;
//! `REFACTOR §16 fase 6`, `§17`).
//!
//! Arnés de medición sobre fixtures **sintéticas grandes** generadas en runtime (nunca commiteadas):
//! los `.md` se escriben en un `tempdir` dentro del propio test. Mide latencia (registro, sin umbral
//! duro que haga el test frágil), tamaño de payload (proxy de tokens) y concurrencia (dos
//! publicadores → lock).
//!
//! Se miden los **servicios de `App` directamente** (no la superficie MCP por stdio) porque:
//!   - `bench_search_payload_acotado` asevera una propiedad de la **forma del payload**
//!     ([`SearchResult`] no tiene campo `body`), que se decide en `lodestar-app`; el MCP es un mero
//!     paso de serialización. Medir aquí es directo y no acopla la aserción al framing JSON-RPC.
//!   - `bench_concurrencia_segura` necesita **control determinista** del lock/revisión base entre dos
//!     `change_apply`; la API de `App` (`change_plan`/`change_apply` con [`ErrorCode`] tipado) da esa
//!     precisión, imposible de inspeccionar con la misma finura sobre el wire de error MCP.
//!
//! ## Estado rojo/verde (honesto, para el implementador)
//!   - `bench_search_payload_acotado`: **VERDE (composición/regresión)**. `knowledge_search` YA acota
//!     el payload desde E10-H09: [`SearchResult`] no expone `body`, solo `snippet` + metadatos. Este
//!     test **fija esa garantía como no-regresión de escala**: con ~10k documentos de cuerpo grande,
//!     ningún cuerpo completo viaja en la respuesta y el payload es estrictamente menor que la suma de
//!     los cuerpos que representa. NO es un rojo artificial: si una optimización futura reintrodujera
//!     el `body` en la respuesta, este test lo cazaría.
//!   - `bench_concurrencia_segura`: **VERDE (composición/regresión)**. El lock exclusivo de publicación
//!     (E13-H02, `acquire_lock` `O_CREAT|O_EXCL`) + el control optimista bajo lock
//!     (`reverify_base_revision`) + la verificación de `planHash` en `change_apply` YA garantizan que
//!     de dos aplicaciones concurrentes exactamente una gana. Este test fija esa propiedad de
//!     integridad bajo concurrencia real (dos hilos).
//!
//! ## Código de conflicto REAL (anotado — la spec dice `WRITE_CONFLICT`)
//! La spec (E14-H05) predice `WRITE_CONFLICT` para el perdedor. El motor emite, de forma
//! determinista según el punto en que el perdedor pierde la carrera, **uno de dos** códigos limpios
//! de la familia «conflicto» (ambos rechazan sin corromper, ANTES de publicar):
//!   - `WRITE_CONFLICT` (`WorkspaceError::WriteConflict`): si el perdedor llega a `apply_transaction`
//!     y o bien el lock ya está tomado (`acquire_lock`) o bien la base cambió bajo el lock
//!     (`reverify_base_revision`).
//!   - `PLAN_STALE`: si el perdedor recomputa el `planHash` (paso 3 de `change_apply`) DESPUÉS de que
//!     el ganador sellara — la base actual ya no casa el hash del plan, así que ni siquiera entra a la
//!     transacción.
//!
//! El test acepta la familia `{WRITE_CONFLICT, PLAN_STALE}` (robusto y no frágil) y **registra** cuál
//! ocurrió. La propiedad esencial —exactamente uno tiene éxito, el otro se rechaza limpiamente, el
//! workspace queda íntegro— es determinista con cualquier entrelazado (ver la prueba en el cuerpo del
//! test).

use std::path::Path;
use std::time::Instant;

use lodestar_app::{App, Profile};
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::ErrorCode;

/// Escribe un `.md` (creando los directorios intermedios) dentro del workspace temporal.
fn escribe(root: &Path, rel: &str, contenido: &str) {
    let ruta = root.join(rel);
    if let Some(dir) = ruta.parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(ruta, contenido).unwrap();
}

/// Política permisiva: no exige resultado conforme, así el plan/apply no se rechaza por
/// conformidad y el test aísla la concurrencia.
fn policy_permisiva() -> PlanPolicy {
    PlanPolicy {
        require_conformant_result: false,
        allow_warnings: true,
    }
}

// ===========================================================================
// E14-H05 · Criterio 1 — `bench_search_payload_acotado`.
//
// Dado una fixture de ~10k documentos (cuerpos grandes), Cuando se mide `knowledge_search`, Entonces
// la latencia se registra Y el payload queda acotado (resúmenes/snippets, NO el cuerpo entero).
// ===========================================================================

/// Nº de documentos sintéticos: respeta el «~10k» de la spec (search es O(n), representativo de escala).
const N_CONCEPTOS: usize = 10_000;

/// Marca única enterrada al FINAL del cuerpo de cada documento, lejos de cualquier término buscado y
/// más allá de la ventana del snippet (160 chars): si aparece en la respuesta, un cuerpo completo
/// viajó. Sirve de centinela robusto de «payload NO incluye el body».
const CENTINELA: &str = "CENTINELA-CUERPO-QUE-NO-DEBE-VIAJAR";

/// Cuerpo grande (~2 KB) por documento: un arranque con el término buscable, mucho relleno, y el
/// centinela al final (bien pasado el snippet window de 160 chars).
fn cuerpo_grande(i: usize) -> String {
    let relleno = "Contenido de relleno sintetico para dar cuerpo al documento. ".repeat(40);
    format!(
        "# Documento {i}\n\nEste documento sintetico numero {i} describe un tema de prueba.\n\n{relleno}\n\n{CENTINELA}-{i}\n"
    )
}

/// Construye en `root` un workspace con `N_CONCEPTOS` documentos de cuerpo grande. El `index.md` es
/// mínimo (no lista los 10k: la conformidad no importa para search, y listar 10k enlaces solo
/// ralentizaría sin cambiar el conjunto que casa). Cada documento casa el término «documento» por su
/// título/descripción/cuerpo.
fn genera_workspace_grande(root: &Path) {
    escribe(
        root,
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle grande\n",
    );
    for i in 0..N_CONCEPTOS {
        escribe(
            root,
            &format!("c/documento-{i:05}.md"),
            &format!(
                "---\ntype: Concept\ntitle: Documento {i}\ndescription: documento sintetico numero {i}\n---\n\n{}",
                cuerpo_grande(i)
            ),
        );
    }
}

#[test]
fn bench_search_payload_acotado() {
    let dir = tempfile::tempdir().unwrap();

    let t_gen = Instant::now();
    genera_workspace_grande(dir.path());
    let gen_ms = t_gen.elapsed().as_millis();

    let t_open = Instant::now();
    let app = App::open(dir.path()).expect("el workspace grande debe abrir");
    let open_ms = t_open.elapsed().as_millis();

    // Página completa (tope 100) para tener suficientes resultados sobre los que medir la cota.
    let t_search = Instant::now();
    let res = app
        .knowledge_search("documento", None, None, None, Some(100), None)
        .expect("knowledge_search debe responder sobre el workspace grande");
    let search_ms = t_search.elapsed().as_millis();

    // (Registro de latencia — SIN umbral duro; solo se mide/imprime, para no hacer el test frágil.)
    let payload = serde_json::to_string(&res).expect("SearchResults debe serializar");
    eprintln!(
        "[bench_search] documentos={N_CONCEPTOS} gen={gen_ms}ms open={open_ms}ms search={search_ms}ms \
         resultados_pagina={} total_aprox={} payload_bytes={}",
        res.results.len(),
        res.total_approximate,
        payload.len(),
    );

    // --- Propiedad funcional 1: la búsqueda SÍ casó (no vacua) y devolvió una página no trivial. ---
    assert_eq!(
        res.total_approximate, N_CONCEPTOS,
        "todos los documentos casan «documento»; total_approximate debe ser {N_CONCEPTOS}"
    );
    assert_eq!(
        res.results.len(),
        100,
        "la página (limit 100) debe venir llena con {N_CONCEPTOS} coincidencias"
    );

    // --- Propiedad funcional 2 (LO ESENCIAL): NINGÚN cuerpo completo viaja en el payload. ---
    // El centinela vive al final de cada cuerpo, más allá del snippet window: si apareciera en la
    // respuesta serializada, un cuerpo entero se habría filtrado.
    assert!(
        !payload.contains(CENTINELA),
        "payload acotado: la respuesta de knowledge_search NO debe contener el centinela del cuerpo \
         (ningún body completo debe viajar): un fragmento del payload = {}",
        &payload[..payload.len().min(400)]
    );

    // --- Propiedad funcional 3: cada resultado trae un snippet NO vacío y compacto (no el body). ---
    let mut suma_cuerpos = 0usize;
    for r in &res.results {
        assert!(
            !r.snippet.is_empty(),
            "cada resultado debe traer un snippet no vacío: {:?}",
            r.path
        );
        assert!(
            !r.snippet.contains(CENTINELA),
            "el snippet no debe alcanzar el centinela del final del cuerpo: {:?}",
            r.path
        );
        // Reconstruye el tamaño del cuerpo COMPLETO que representa este resultado (para la cota).
        let i: usize = r
            .path
            .as_str()
            .trim_start_matches("c/documento-")
            .trim_end_matches(".md")
            .parse()
            .expect("path de documento sintético");
        suma_cuerpos += cuerpo_grande(i).len();
    }

    // --- Cota de payload (proxy de tokens): la respuesta es ESTRICTAMENTE menor que la suma de los
    // cuerpos completos que representa. Demuestra «resúmenes/snippets, no el body entero». ---
    assert!(
        payload.len() < suma_cuerpos,
        "el payload ({} bytes) debe ser mucho menor que la suma de los cuerpos completos que \
         representa ({} bytes): la búsqueda devuelve resúmenes, no cuerpos",
        payload.len(),
        suma_cuerpos,
    );
}

// ===========================================================================
// E14-H05 · Criterio 2 — `bench_concurrencia_segura`.
//
// Dado dos `change_apply` concurrentes, Cuando se ejecutan, Entonces uno gana el lock y el otro se
// rechaza limpiamente (conflicto), sin corrupción.
//
// Determinismo de la propiedad esencial (por qué NO es flaky): ambos planes se calculan sobre la
// MISMA revisión base r0 (planificar no escribe el canónico). En cualquier entrelazado de los dos
// hilos, el que adquiere el lock PRIMERO no puede fallar (bajo el lock la base sigue siendo r0, que
// casa su plan) → tiene éxito; el otro, o bien encuentra el lock tomado (`acquire_lock` →
// WRITE_CONFLICT), o bien pasa el lock tras el sellado del ganador y la base ya cambió
// (`reverify_base_revision` → WRITE_CONFLICT), o bien recomputa el `planHash` tras el sellado y no
// casa (→ PLAN_STALE). Nunca ambos publican (el lock serializa apply_transaction y el segundo falla
// reverify ANTES de publicar). Por tanto: exactamente uno gana, siempre.
// ===========================================================================
#[test]
fn bench_concurrencia_segura() {
    let dir = tempfile::tempdir().unwrap();
    escribe(
        dir.path(),
        "index.md",
        "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Alfa](alfa.md)\n",
    );
    escribe(
        dir.path(),
        "alfa.md",
        "---\ntype: Concept\ntitle: Alfa\ndescription: documento base\n---\n\n# Resumen\n\ncuerpo\n",
    );

    // Planifica AMBAS operaciones sobre la misma base r0 (planificar no toca el canónico). Cada
    // create escribe un fichero DISTINTO, así el ganador es identificable por su `.md`.
    let app = App::open(dir.path()).expect("el workspace debe abrir");
    let plan_a = app
        .change_plan(
            None,
            &serde_json::json!([
                { "op": "create", "path": "alfa2.md", "type": "Concept", "title": "Alfa2",
                  "body": "# Alfa2\n\ncuerpo del publicador A\n" }
            ]),
            policy_permisiva(),
        )
        .expect("el plan A debe producirse");
    let plan_b = app
        .change_plan(
            None,
            &serde_json::json!([
                { "op": "create", "path": "beta.md", "type": "Concept", "title": "Beta",
                  "body": "# Beta\n\ncuerpo del publicador B\n" }
            ]),
            policy_permisiva(),
        )
        .expect("el plan B debe producirse");
    drop(app);

    let id_a = plan_a.change_set_id.clone();
    let id_b = plan_b.change_set_id.clone();
    let raiz = dir.path().to_path_buf();

    // Dos publicadores concurrentes: cada hilo abre su propio `App` sobre el MISMO workspace root y
    // aplica su plan. Comparten el lock de fichero del sistema (E13-H02).
    let raiz_a = raiz.clone();
    let h_a = std::thread::spawn(move || {
        let app = App::open(&raiz_a).expect("A abre el workspace");
        app.change_apply(&id_a, None)
    });
    let raiz_b = raiz.clone();
    let h_b = std::thread::spawn(move || {
        let app = App::open(&raiz_b).expect("B abre el workspace");
        app.change_apply(&id_b, None)
    });

    let res_a = h_a.join().expect("el hilo A no debe entrar en pánico");
    let res_b = h_b.join().expect("el hilo B no debe entrar en pánico");

    // --- Propiedad 1: EXACTAMENTE uno tiene éxito. ---
    let exitos = res_a.is_ok() as u8 + res_b.is_ok() as u8;
    assert_eq!(
        exitos, 1,
        "exactamente un publicador concurrente debe ganar; A={res_a:?} B={res_b:?}"
    );

    // --- Propiedad 2: el perdedor se rechaza LIMPIAMENTE con un código de la familia «conflicto». ---
    let perdedor: &ErrorCode = match (&res_a, &res_b) {
        (Ok(_), Err(e)) | (Err(e), Ok(_)) => e,
        _ => unreachable!("la propiedad 1 garantiza un único perdedor"),
    };
    eprintln!("[bench_concurrencia] código del perdedor = {perdedor:?}");
    assert!(
        matches!(perdedor, ErrorCode::WriteConflict | ErrorCode::PlanStale),
        "el perdedor debe rechazarse con WRITE_CONFLICT o PLAN_STALE (familia conflicto), no {perdedor:?}"
    );

    // --- Propiedad 3: el workspace queda ÍNTEGRO (sin corrupción, revisión coherente). ---
    // Exactamente el fichero del ganador existe; el del perdedor NO.
    let existe_a = raiz.join("alfa2.md").is_file();
    let existe_b = raiz.join("beta.md").is_file();
    assert_ne!(
        existe_a, existe_b,
        "exactamente uno de los dos `.md` debe existir (el del ganador): alfa2={existe_a} beta={existe_b}"
    );
    assert_eq!(
        res_a.is_ok(),
        existe_a,
        "el `.md` presente debe ser el del publicador que reportó éxito (A)"
    );

    // El workspace reabierto es conforme (sin hard-fails / estado parcial) y su revisión coincide con
    // la que reportó el ganador (durabilidad/coherencia, sin parciales).
    let app = App::open(&raiz).expect("el workspace debe reabrir tras la concurrencia");
    let status = app
        .workspace_status(Profile::Standard)
        .expect("workspace_status tras la concurrencia");
    assert_eq!(
        status.counts.errors, 0,
        "el workspace tras la concurrencia debe quedar sin hard-fails (íntegro): {:?}",
        status.counts
    );
    let rev_ganador = match (&res_a, &res_b) {
        (Ok(r), _) => r.workspace_revision.clone(),
        (_, Ok(r)) => r.workspace_revision.clone(),
        _ => unreachable!(),
    };
    assert_eq!(
        status.workspace_revision, rev_ganador,
        "la revisión del workspace reabierto debe ser la que reportó el ganador (durable/coherente)"
    );
}
