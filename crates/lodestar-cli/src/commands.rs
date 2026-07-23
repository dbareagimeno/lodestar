//! Implementación de los subcomandos. Shells finos sobre `lodestar-core`.

use std::path::Path;
use std::process::ExitCode;

use lodestar_core::types::Severity;

use crate::sarif;

/// `lodestar check`: la puerta de CI sobre el **working tree**. Juzga la conformidad **completa**
/// (OKF + `SCHEMA-*` + `REL-*` + refs externas) con el **mismo motor** que
/// [`lodestar_app::App::knowledge_check`] scope `workspace` (invariante #3 — una sola verdad
/// computada): ambos comparten la fusión OKF+schema-driven de
/// `App::schema_diagnostics_by_path`. La CLI es una fachada fina y **no** recompone la validación
/// schema-driven a mano.
///
/// La salida (`--json`/`--sarif`/humano) se renderiza desde un **único** `Analysis` completo
/// ([`lodestar_app::App::full_analysis`], un solo `analyze()`): así los diagnósticos `SCHEMA-*`/
/// `REL-*` que disparan el fallo se **surface an** en el wire, no solo el veredicto. El veredicto
/// `conformant` (== sin `Err` entre los conceptos, misma semántica que `knowledge_check`) decide el
/// bloqueo; además, `lodestar.toml` puede endurecer la puerta bloqueando también avisos
/// (`gate.block_warnings`). Exit codes congelados: `0` conforme · `1` bloqueado · `3` runtime/IO
/// (error del servicio o `lodestar.toml` inválido).
pub fn check(root: &Path, json: bool, sarif_out: bool) -> anyhow::Result<ExitCode> {
    // Motor completo (OKF + schema-driven) en `App`, computado UNA sola vez: `full_analysis` corre
    // `analyze()` y fusiona los `SCHEMA-*`/`REL-*` en `per_file` con la misma lógica que
    // `knowledge_check`. De ese `Analysis` sale tanto el veredicto como la salida.
    let app = lodestar_app::App::open(root).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let analysis = app
        .full_analysis()
        .map_err(|e| anyhow::anyhow!(e.as_str().to_string()))?;

    // Veredicto == sin `Err` entre los conceptos, misma semántica que `knowledge_check` scope
    // `workspace` (que también itera `analysis.concepts`): así la fachada no reimplementa el motor,
    // solo lee el mismo `Analysis`.
    let conformant = !analysis
        .concepts
        .iter()
        .filter_map(|p| analysis.per_file.get(p))
        .flatten()
        .any(|c| c.level == Severity::Err);

    // Un `lodestar.toml` inválido es exit 3, NO defaults silenciosos: con `block_warnings=true`
    // y un typo TOML, la puerta de CI se relajaría sin ningún aviso. El motor ya cubre los `Err`
    // (incluidos los schema-driven) vía `conformant`; `lodestar.toml` solo puede endurecer la
    // puerta para que los avisos OKF también bloqueen.
    let blocked = !conformant
        || lodestar_workspace::Config::load(root)
            .map_err(|e| anyhow::anyhow!(e))?
            .gate_blocked(&analysis);
    render_analysis(&analysis, conformant, json, sarif_out, blocked)
}

/// Imprime un `Analysis` en el formato pedido y devuelve el exit code (0 conforme / 1 bloqueado).
/// `conformant` es el veredicto del motor completo (misma semántica que `knowledge_check`), que en
/// `--json` se emite de forma ADITIVA junto a los campos históricos del `Analysis`; los `SCHEMA-*`/
/// `REL-*` viajan dentro de `per_file` (y por tanto en `--sarif`/humano) al haberse fusionado en
/// [`lodestar_app::App::full_analysis`]. `blocked` decide el exit code: lo endurece la strictness de
/// `lodestar.toml` (avisos OKF) sobre el veredicto del motor.
pub fn render_analysis(
    analysis: &lodestar_core::types::Analysis,
    conformant: bool,
    json: bool,
    sarif_out: bool,
    blocked: bool,
) -> anyhow::Result<ExitCode> {
    if json {
        // Aditivo: serializar el `Analysis` (conserva `concepts`/`hardFail`/… en el wire) y añadir
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
    for (path, checks) in &a.per_file {
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
        "\n{} concepts · {} con errores · {} avisos · {}",
        a.concepts.len(),
        // `errs` cuenta TODOS los diagnósticos `Err` de `per_file` (OKF + `SCHEMA-*`/`REL-*`
        // fusionados en `full_analysis`), no solo `a.hard_fail` (OKF): así el resumen humano es
        // coherente con las líneas `✗` de arriba.
        errs,
        warns,
        if blocked {
            // Cubre también el gate estricto: no imprimir «CONFORME» con exit code 1.
            if errs == 0 {
                "NO CONFORME (avisos bloqueados por lodestar.toml)"
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
