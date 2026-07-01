//! Handlers de las tools del MCP (`ARCHITECTURE.md §7.2`). Cada uno = shell sobre `Workspace`.
//!
//! Scope = **semántica, no CRUD**. El valor es lo que los ficheros crudos no dan barato:
//! backlinks resueltos, huérfanos, dangling, impacto, la puerta OKF, query y escrituras validadas.

use std::collections::BTreeMap;

use lodestar_core::types::{Direction, FrontmatterPatch, RelPath};
use lodestar_workspace::Workspace;
use serde_json::{json, Value};

/// Error de tool con un mensaje legible (la fachada lo envuelve en el error JSON-RPC).
pub type ToolResult = Result<Value, String>;

fn rel(params: &Value, key: &str) -> Result<RelPath, String> {
    let s = params
        .get(key)
        .and_then(Value::as_str)
        .ok_or(format!("falta el parámetro «{key}»"))?;
    RelPath::new(s).map_err(|e| e.to_string())
}

/// Lista las tools con su descripción (para `tools/list`).
pub fn list() -> Value {
    json!([
        {"name": "find_backlinks", "description": "Quién enlaza a un concept (inbound + index + dangling)."},
        {"name": "find_orphans", "description": "Concepts sin enlaces entrantes."},
        {"name": "find_dangling", "description": "Enlaces que apuntan a páginas inexistentes."},
        {"name": "neighborhood", "description": "Subgrafo dirigido alrededor de un concept (depth, direction)."},
        {"name": "conformance_check", "description": "Gate OKF del bundle (o de un path)."},
        {"name": "query", "description": "Query estructurada (DSL de subcadena)."},
        {"name": "create_concept", "description": "Crea un concept validado (rechaza no conforme)."},
        {"name": "update_frontmatter", "description": "Patch de frontmatter (null borra)."},
        {"name": "generate_index", "description": "Regenera el index de un directorio."},
        {"name": "generate_tag_indexes", "description": "Regenera los índices de tags."},
        {"name": "history", "description": "Historial de commits."},
        {"name": "last_conforming_commit", "description": "Último commit conforme."},
        {"name": "commit", "description": "Commit del agente (checkpoint + conformidad post-commit)."},
    ])
}

/// Despacha una tool por nombre.
pub fn call(ws: &Workspace, name: &str, params: &Value) -> ToolResult {
    match name {
        "find_backlinks" => {
            let p = rel(params, "concept")?;
            to_json(&ws.backlinks(&p).map_err(|e| e.to_string())?)
        }
        "find_orphans" => {
            let a = ws.analyze().map_err(|e| e.to_string())?;
            Ok(json!({ "orphans": a.orphans }))
        }
        "find_dangling" => {
            let a = ws.analyze().map_err(|e| e.to_string())?;
            Ok(json!({ "dangling": a.dangling }))
        }
        "neighborhood" => {
            let p = rel(params, "concept")?;
            let depth = params.get("depth").and_then(Value::as_u64).unwrap_or(1) as u32;
            let dir = match params.get("direction").and_then(Value::as_str) {
                Some("in") => Direction::In,
                Some("both") => Direction::Both,
                _ => Direction::Out,
            };
            to_json(&ws.neighborhood(&p, depth, dir).map_err(|e| e.to_string())?)
        }
        "conformance_check" => {
            let a = ws.analyze().map_err(|e| e.to_string())?;
            match params.get("path").and_then(Value::as_str) {
                Some(path) => {
                    let p = RelPath::new(path).map_err(|e| e.to_string())?;
                    let checks = a.per_file.get(&p).cloned().unwrap_or_default();
                    to_json(&checks)
                }
                None => Ok(json!({
                    "hardFail": a.hard_fail,
                    "warnCount": a.warn_count,
                    "conform": a.hard_fail == 0,
                    "okfVersion": a.okf_version,
                })),
            }
        }
        "query" => {
            let dsl = params.get("dsl").and_then(Value::as_str).unwrap_or("");
            to_json(&ws.query(dsl).map_err(|e| e.to_string())?)
        }
        "create_concept" => {
            let p = rel(params, "path")?;
            let ty = params.get("type").and_then(Value::as_str).unwrap_or("");
            let title = params.get("title").and_then(Value::as_str);
            let body = params
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or("# Resumen\n");
            let allow = params
                .get("allow_nonconformant")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let outcome = ws
                .create_concept(&p, ty, title, body, allow)
                .map_err(|e| e.to_string())?;
            Ok(write_outcome_json(&outcome))
        }
        "update_frontmatter" => {
            let p = rel(params, "path")?;
            let patch = parse_patch(params.get("patch"))?;
            let outcome = ws.merge_frontmatter(&p, patch).map_err(|e| e.to_string())?;
            Ok(write_outcome_json(&outcome))
        }
        "generate_index" => {
            let dir = params.get("dir").and_then(Value::as_str).unwrap_or("");
            let r = ws.generate_index(dir).map_err(|e| e.to_string())?;
            Ok(json!({ "written": r.written, "removed": r.removed, "unchanged": r.unchanged }))
        }
        "generate_tag_indexes" => {
            let r = ws.generate_tags().map_err(|e| e.to_string())?;
            Ok(json!({ "written": r.written, "removed": r.removed, "unchanged": r.unchanged }))
        }
        "history" => {
            let limit = params.get("limit").and_then(Value::as_u64).unwrap_or(20) as usize;
            to_json(&ws.vcs_log(limit).map_err(|e| e.to_string())?)
        }
        "last_conforming_commit" => {
            let sha = ws.last_conforming().map_err(|e| e.to_string())?;
            Ok(json!({ "sha": sha.map(|s| s.as_str().to_string()) }))
        }
        "commit" => {
            let msg = params
                .get("message")
                .and_then(Value::as_str)
                .unwrap_or("Commit del agente");
            // El trailer Co-Authored-By distingue los commits del agente (§12); lo añade la fachada.
            let full = format!("{msg}\n\nCo-Authored-By: lodestar-agent <agent@lodestar>");
            let outcome = ws.commit(&full).map_err(|e| e.to_string())?;
            Ok(json!({
                "sha": outcome.sha.as_str(),
                "conformance": {
                    "hardFail": outcome.conformance.hard_fail,
                    "warnCount": outcome.conformance.warn_count,
                    "conform": outcome.conformance.conform,
                }
            }))
        }
        other => Err(format!("tool desconocida: {other}")),
    }
}

fn to_json<T: serde::Serialize>(v: &T) -> ToolResult {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

fn write_outcome_json(o: &lodestar_core::types::WriteOutcome) -> Value {
    json!({
        "path": o.path.as_str(),
        "written": o.written,
        "rejected": o.rejected,
        "checks": o.checks,
        "bundleHardFail": o.bundle_hard_fail,
    })
}

fn parse_patch(v: Option<&Value>) -> Result<FrontmatterPatch, String> {
    let obj = v
        .and_then(Value::as_object)
        .ok_or("falta el parámetro «patch» (objeto)")?;
    let mut map: BTreeMap<String, Option<serde_yaml::Value>> = BTreeMap::new();
    for (k, val) in obj {
        let yaml = if val.is_null() {
            None
        } else {
            Some(json_to_yaml(val))
        };
        map.insert(k.clone(), yaml);
    }
    Ok(FrontmatterPatch(map))
}

/// Convierte un `serde_json::Value` a `serde_yaml::Value` (para los patches del agente).
fn json_to_yaml(v: &Value) -> serde_yaml::Value {
    match v {
        Value::Null => serde_yaml::Value::Null,
        Value::Bool(b) => serde_yaml::Value::Bool(*b),
        Value::Number(n) => serde_yaml::from_str(&n.to_string()).unwrap_or(serde_yaml::Value::Null),
        Value::String(s) => serde_yaml::Value::String(s.clone()),
        Value::Array(a) => serde_yaml::Value::Sequence(a.iter().map(json_to_yaml).collect()),
        Value::Object(o) => {
            let mut m = serde_yaml::Mapping::new();
            for (k, val) in o {
                m.insert(serde_yaml::Value::String(k.clone()), json_to_yaml(val));
            }
            serde_yaml::Value::Mapping(m)
        }
    }
}

#[cfg(test)]
mod tests {
    //! Golden cross-fachada (E7-H06): la salida de cada tool == la del `Workspace` directo.
    //! Verifica que la fachada MCP es un shell fino sin lógica OKF propia (`§2`, `§7`).
    use super::*;

    fn ws_with_fixture() -> (tempfile::TempDir, Workspace) {
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in [
            ("index.md", "---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Alfa](alfa.md)\n"),
            (
                "alfa.md",
                "---\ntype: Concept\ntitle: Alfa\ndescription: d\n---\n\n# H\n\n[huerfano falta](/no-existe.md)\n",
            ),
            (
                "beta.md",
                "---\ntype: Concept\ntitle: Beta\ndescription: d\n---\n\n# H\n\ncuerpo\n",
            ),
        ] {
            std::fs::write(dir.path().join(p), c).unwrap();
        }
        let ws = Workspace::open_ephemeral(dir.path()).unwrap();
        (dir, ws)
    }

    #[test]
    fn golden_backlinks_igual_workspace() {
        let (_d, ws) = ws_with_fixture();
        let p = RelPath::new("alfa.md").unwrap();
        let via_tool = call(&ws, "find_backlinks", &json!({"concept": "alfa.md"})).unwrap();
        let direct = serde_json::to_value(ws.backlinks(&p).unwrap()).unwrap();
        assert_eq!(via_tool, direct);
    }

    #[test]
    fn golden_orphans_y_dangling_igual_workspace() {
        let (_d, ws) = ws_with_fixture();
        let a = ws.analyze().unwrap();
        let orphans = call(&ws, "find_orphans", &json!({})).unwrap();
        assert_eq!(orphans, json!({ "orphans": a.orphans }));
        let dangling = call(&ws, "find_dangling", &json!({})).unwrap();
        assert_eq!(dangling, json!({ "dangling": a.dangling }));
    }

    #[test]
    fn golden_query_igual_workspace() {
        let (_d, ws) = ws_with_fixture();
        let via_tool = call(&ws, "query", &json!({"dsl": "is:orphan"})).unwrap();
        let direct = serde_json::to_value(ws.query("is:orphan").unwrap()).unwrap();
        assert_eq!(via_tool, direct);
    }

    #[test]
    fn tool_desconocida_es_error() {
        let (_d, ws) = ws_with_fixture();
        assert!(call(&ws, "no_existe", &json!({})).is_err());
    }
}
