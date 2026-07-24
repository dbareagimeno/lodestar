//! Implementación de los subcomandos. Shells finos sobre `lodestar-core`.

use std::path::Path;
use std::process::ExitCode;

use lodestar_core::types::{RelPath, Severity};

use crate::sarif;

/// `lodestar check`: la puerta de CI sobre el **working tree**. Juzga la conformidad **completa**
/// (documento + `SCHEMA-*` + `REL-*` + refs externas) con el **mismo motor** que
/// [`lodestar_app::App::knowledge_check`] scope `workspace` (invariante #3 — una sola verdad
/// computada): ambos comparten la fusión documento+schema-driven de
/// `App::schema_diagnostics_by_path`. La CLI es una fachada fina y **no** recompone la validación
/// schema-driven a mano.
///
/// La salida (`--json`/`--sarif`/humano) se renderiza desde un **único** `Analysis` completo
/// ([`lodestar_app::App::full_analysis`], un solo `analyze()`): así los diagnósticos `SCHEMA-*`/
/// `REL-*` que disparan el fallo se **surface an** en el wire, no solo el veredicto. El veredicto
/// `conformant` (== sin `Err` entre los documentos, misma semántica que `knowledge_check`) decide el
/// bloqueo; además, `.lodestar/config.yaml` puede endurecer la puerta bloqueando también avisos
/// (`gate.blockWarnings`). Exit codes congelados: `0` conforme · `1` bloqueado · `3` runtime/IO
/// (error del servicio o `.lodestar/config.yaml` inválido — lo detecta ya `App::open`).
pub fn check(root: &Path, json: bool, sarif_out: bool) -> anyhow::Result<ExitCode> {
    // Motor completo (documento + schema-driven) en `App`, computado UNA sola vez: `full_analysis` corre
    // `analyze()` y fusiona los `SCHEMA-*`/`REL-*` en `diagnostics` con la misma lógica que
    // `knowledge_check`. De ese `Analysis` sale tanto el veredicto como la salida.
    let app = lodestar_app::App::open(root).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let analysis = app
        .full_analysis()
        .map_err(|e| anyhow::anyhow!(e.as_str().to_string()))?;

    // Veredicto == sin `Err` entre los documentos, misma semántica que `knowledge_check` scope
    // `workspace` (que también itera `analysis.documents`): así la fachada no reimplementa el motor,
    // solo lee el mismo `Analysis`.
    let conformant = !analysis
        .documents
        .iter()
        .filter_map(|p| analysis.diagnostics.get(p))
        .flatten()
        .any(|c| c.level == Severity::Err);

    // Un `.lodestar/config.yaml` inválido es exit 3, NO defaults silenciosos: con
    // `blockWarnings: true` y un typo en el YAML, la puerta de CI se relajaría sin ningún aviso.
    // El error lo levanta ya `App::open` (la config se valida una vez, al abrir el workspace). El
    // motor cubre los `Err` (incluidos los schema-driven) vía `conformant`; la config solo puede
    // endurecer la puerta para que los avisos también bloqueen.
    let blocked = !conformant || app.workspace().config().gate_blocked(&analysis);
    render_analysis(&analysis, conformant, json, sarif_out, blocked)
}

/// Imprime un `Analysis` en el formato pedido y devuelve el exit code (0 conforme / 1 bloqueado).
/// `conformant` es el veredicto del motor completo (misma semántica que `knowledge_check`), que en
/// `--json` se emite de forma ADITIVA junto a los campos históricos del `Analysis`; los `SCHEMA-*`/
/// `REL-*` viajan dentro de `diagnostics` (y por tanto en `--sarif`/humano) al haberse fusionado en
/// [`lodestar_app::App::full_analysis`]. `blocked` decide el exit code: lo endurece la strictness de
/// `.lodestar/config.yaml` (`gate.blockWarnings`) sobre el veredicto del motor.
pub fn render_analysis(
    analysis: &lodestar_core::types::Analysis,
    conformant: bool,
    json: bool,
    sarif_out: bool,
    blocked: bool,
) -> anyhow::Result<ExitCode> {
    if json {
        // Aditivo: serializar el `Analysis` (conserva `documents`/`hardFail`/… en el wire) y añadir
        // el veredicto `conformant` del motor completo sin tocar el resto del objeto.
        let mut value = serde_json::to_value(analysis)?;
        if let serde_json::Value::Object(map) = &mut value {
            map.insert(
                "conformant".to_string(),
                serde_json::Value::Bool(conformant),
            );
        }
        println!("{}", serde_json::to_string_pretty(&value)?);
    } else if sarif_out {
        println!("{}", sarif::to_sarif(analysis)?);
    } else {
        print_human(analysis, blocked);
    }
    if blocked {
        Ok(ExitCode::from(1))
    } else {
        Ok(ExitCode::SUCCESS)
    }
}

fn print_human(a: &lodestar_core::types::Analysis, blocked: bool) {
    let mut errs = 0usize;
    let mut warns = 0usize;
    for (path, checks) in &a.diagnostics {
        for c in checks {
            match c.level {
                Severity::Err => {
                    errs += 1;
                    println!("  ✗ [{}] {}: {}", c.code.as_str(), path, c.msg);
                }
                Severity::Warn => {
                    warns += 1;
                    println!("  ! [{}] {}: {}", c.code.as_str(), path, c.msg);
                }
                _ => {}
            }
        }
    }
    println!(
        "\n{} documentos · {} con errores · {} avisos · {}",
        a.documents.len(),
        // `errs` cuenta TODOS los diagnósticos `Err` de `diagnostics` (documento + `SCHEMA-*`/`REL-*`
        // fusionados en `full_analysis`), no solo `a.hard_fail()`: así el resumen humano es
        // coherente con las líneas `✗` de arriba.
        errs,
        warns,
        if blocked {
            // Cubre también el gate estricto: no imprimir «CONFORME» con exit code 1.
            if errs == 0 {
                "NO CONFORME (avisos bloqueados por .lodestar/config.yaml)"
            } else {
                "NO CONFORME"
            }
        } else {
            "CONFORME"
        }
    );
}

/// `lodestar reindex`: reconstruye la cache `.lodestar/index.db` desde disco. La cache es una
/// vista derivada y desechable de los `.md` (invariante #1): reconstruirla nunca los toca.
pub fn reindex(root: &Path) -> anyhow::Result<ExitCode> {
    let mut ws =
        lodestar_workspace::Workspace::open(root).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    ws.enable_cache()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    println!(
        "cache reconstruida en {}",
        root.join(".lodestar/index.db").display()
    );
    Ok(ExitCode::SUCCESS)
}

/// `lodestar migrate-from-okf --dry-run`: diagnóstico de cortesía para repos OKF legados
/// (`REFACTOR_PHASE_2 §Fase 14`, `ARCHITECTURE.md §20.13`).
///
/// Recorre el workspace, LISTA las convenciones OKF que `§Fase 14` enumera —`index.md` raíz,
/// índices anidados, metadata `okf_version`, índices de tags generados— y declara explícitamente
/// que **no modificó ningún fichero**. Nunca es una puerta: mientras pueda leer el workspace sale
/// `0` (no exit `1` por «detectó OKF»); solo `3` si no puede leerlo. El informe va a **stdout**; los
/// errores, a stderr.
///
/// **Cero escrituras** (invariante de la historia): abre en modo **hermético** con
/// [`Workspace::open_ephemeral`](lodestar_workspace::Workspace::open_ephemeral) —que **no** toca
/// `.gitignore` ni crea el scaffold de `.lodestar/runtime/`, a diferencia de
/// [`App::open`](lodestar_app::App::open)— y solo lee el inventario descubierto. La detección reusa
/// el descubrimiento (E15) y el parseo de frontmatter (E16): no reimplementa ni lo uno ni lo otro.
///
/// Sin `--dry-run` es **error de uso** (exit `2`): en v0.3 solo existe la forma diagnóstica; exigir
/// el flag explícito deja la palabra libre para una futura forma «aplicadora» sin invocarla por
/// accidente. No es un alias del dry-run.
pub fn migrate_from_okf(root: &Path, dry_run: bool) -> anyhow::Result<ExitCode> {
    if !dry_run {
        eprintln!(
            "error: `migrate-from-okf` requiere `--dry-run`. En v0.3 solo existe la forma \
             diagnóstica (no modifica ficheros); no hay alias implícito."
        );
        return Ok(ExitCode::from(2));
    }

    // Modo HERMÉTICO: `open_ephemeral` no escribe nada en el workspace (ni `.gitignore` ni el
    // scaffold de runtime que `App::open`/`Workspace::open` crearían), así el diagnóstico no puede
    // dejar rastro en disco. Se envuelve en `App` como capa de servicios (invariante de fachada).
    let ws = lodestar_workspace::Workspace::open_ephemeral(root)
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let app = lodestar_app::App::from_workspace(ws);
    let doc_set = app
        .workspace()
        .document_set()
        .map_err(|e| anyhow::anyhow!(e.to_string()))?;

    // --- Detección de las convenciones OKF de `§Fase 14` -----------------------------------------
    // Los generadores se borraron en E15, así que no hay rastro del generador: se detecta por
    // convención heurística de cortesía (no tiene que ser perfecta).
    let mut root_index: Option<String> = None;
    let mut nested_indexes: Vec<String> = Vec::new();
    let mut okf_version_docs: Vec<String> = Vec::new();
    let mut tag_indexes: Vec<String> = Vec::new();

    for (path, content) in doc_set.files() {
        let rel = path.as_str();
        // `index.md`: raíz (sin directorio contenedor) vs anidado (`<dir>/index.md`).
        if path.basename() == "index.md" {
            if path.dir().is_empty() {
                root_index = Some(rel.to_string());
            } else {
                nested_indexes.push(rel.to_string());
            }
        }
        // Índice de tags generado: un `.md` bajo un directorio `tags/`.
        if es_indice_de_tags(path) {
            tag_indexes.push(rel.to_string());
        }
        // Metadata `okf_version`: frontmatter con esa clave. Reusa el parseo de E16
        // (`model::parse_file`) y el único accesor de metadata (`ParsedFrontmatter::contains_key`).
        if lodestar_core::model::parse_file(rel, content)
            .frontmatter
            .is_some_and(|fm| fm.contains_key("okf_version"))
        {
            okf_version_docs.push(rel.to_string());
        }
    }

    // --- Informe (stdout) ------------------------------------------------------------------------
    println!("Diagnóstico migrate-from-okf (--dry-run)\n");

    let algo_detectado = root_index.is_some()
        || !nested_indexes.is_empty()
        || !okf_version_docs.is_empty()
        || !tag_indexes.is_empty();

    if !algo_detectado {
        // Workspace sin convenciones OKF: nada que migrar. No se menciona `okf_version` (el informe
        // solo lo nombra cuando lo detecta de verdad — discriminante de `dry_run_workspace_limpio`).
        println!("No se detectaron convenciones OKF legadas: no hay nada que migrar.");
        println!("No se modificó ningún fichero.");
        return Ok(ExitCode::SUCCESS);
    }

    println!(
        "Se detectaron convenciones OKF legadas. Los documentos siguen siendo Markdown válido.\n"
    );
    println!("Detectado:");
    if let Some(idx) = &root_index {
        println!("- index.md raíz: {idx}");
    }
    if !nested_indexes.is_empty() {
        println!(
            "- índices anidados ({}): {}",
            nested_indexes.len(),
            nested_indexes.join(", ")
        );
    }
    if !okf_version_docs.is_empty() {
        println!(
            "- metadata okf_version ({}): {}",
            okf_version_docs.len(),
            okf_version_docs.join(", ")
        );
    }
    if !tag_indexes.is_empty() {
        println!(
            "- índices de tags generados ({}): {}",
            tag_indexes.len(),
            tag_indexes.join(", ")
        );
    }

    // Garantía dura de cero cambios (la verifica `dry_run_no_modifica` byte a byte); el ancla laxa
    // `modif` vive en esta frase.
    println!("\nNo se modificó ningún fichero.");

    // Recomendaciones de `§Fase 14` (cortesía), cada una condicionada a su convención detectada: en
    // un workspace sin `okf_version` el informe nunca sugiere «eliminar okf_version».
    println!("\nLimpieza recomendada:");
    if root_index.is_some() || !nested_indexes.is_empty() {
        println!("- Trata los índices como documentos de navegación opcionales.");
    }
    if !okf_version_docs.is_empty() {
        println!("- Elimina okf_version cuando convenga.");
    }
    if !tag_indexes.is_empty() {
        println!("- Revisa los índices de tags generados antes de borrarlos.");
    }

    Ok(ExitCode::SUCCESS)
}

/// ¿Es `path` un **índice de tags** generado? Heurística de cortesía de `§Fase 14`: un `.md` con un
/// segmento de directorio `tags` (lo que producía el retirado `gen_tag_indexes`). Un fichero en la
/// raíz —sin directorio contenedor— nunca lo es.
fn es_indice_de_tags(path: &RelPath) -> bool {
    match path.as_str().rsplit_once('/') {
        Some((dirs, _fichero)) => dirs.split('/').any(|seg| seg == "tags"),
        None => false,
    }
}
