//! Salida SARIF 2.1.0 para integraciones de CI (`ARCHITECTURE.md §7.3`).

use lodestar_app::ANCHOR_WORKSPACE;
use lodestar_core::types::{Analysis, Severity};
use serde_json::{json, Value};

/// Serializa el `Analysis` como un documento SARIF 2.1.0.
///
/// Cada diagnóstico es un `result` con su `ruleId` (el código), su `level` y la `artifactLocation`
/// del documento al que pertenece — **salvo** los diagnósticos anclados a
/// [`ANCHOR_WORKSPACE`], que salen **sin `locations`** (E30-H03 seguimiento 8, `decisiones §23`).
///
/// Ese ancla (`.lodestar`) es una etiqueta SINTÉTICA con la que `App::full_analysis` indexa los
/// diagnósticos de descubrimiento que no tienen `targets` —`WORKSPACE-EMPTY`, `PATH-NOT-UTF8`—
/// porque el mapa de `Analysis::diagnostics` necesita una clave y la raíz no es un `RelPath`
/// (E29-H06, invariante #6). Emitirla como `artifactLocation` la convertía en una afirmación
/// FALSA sobre el disco: `.lodestar` no es un documento del workspace y, en el caso más común
/// —un workspace vacío, sin `.lodestar/` creado—, **ni siquiera existe**, así que un
/// `upload-sarif` colgaba la alerta de un fichero fantasma. SARIF 2.1.0 admite un `result` sin
/// `locations` justo para el hallazgo que no pertenece a ningún artefacto: es la representación
/// correcta de «esto es del workspace, no de un fichero tuyo». El hallazgo sigue emitiéndose con
/// su código, su nivel y su mensaje, y sigue contando para el exit code.
pub fn to_sarif(a: &Analysis) -> anyhow::Result<String> {
    let mut results: Vec<Value> = Vec::new();
    for (path, checks) in &a.diagnostics {
        let sintetico = path.as_str() == ANCHOR_WORKSPACE;
        for c in checks {
            // Solo se reportan err/warn/info; los `pass` no son hallazgos.
            let level = match c.level {
                Severity::Err => "error",
                Severity::Warn => "warning",
                Severity::Info => "note",
                Severity::Pass => continue,
            };
            let mut result = json!({
                "ruleId": c.code.as_str(),
                "level": level,
                "message": { "text": c.msg },
            });
            if !sintetico {
                result["locations"] = json!([{
                    "physicalLocation": {
                        "artifactLocation": { "uri": path.as_str() }
                    }
                }]);
            }
            results.push(result);
        }
    }
    let doc = json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": { "driver": {
                "name": "lodestar",
                "informationUri": "https://github.com/dbareagimeno/lodestar",
                "rules": []
            }},
            "results": results
        }]
    });
    Ok(serde_json::to_string_pretty(&doc)?)
}
