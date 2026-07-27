//! Salida SARIF 2.1.0 para integraciones de CI (`ARCHITECTURE.md §7.3`).

use lodestar_core::types::{Analysis, Severity};
use serde_json::{json, Value};

/// Serializa el `Analysis` como un documento SARIF 2.1.0.
pub fn to_sarif(a: &Analysis) -> anyhow::Result<String> {
    let mut results: Vec<Value> = Vec::new();
    for (path, checks) in &a.diagnostics {
        for c in checks {
            // Solo se reportan err/warn/info; los `pass` no son hallazgos.
            let level = match c.level {
                Severity::Err => "error",
                Severity::Warn => "warning",
                Severity::Info => "note",
                Severity::Pass => continue,
            };
            results.push(json!({
                "ruleId": c.code.as_str(),
                "level": level,
                "message": { "text": c.msg },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": path.as_str() }
                    }
                }]
            }));
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
