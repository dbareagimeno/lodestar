//! Handlers de las tools del MCP (`ARCHITECTURE.md §7.2`). Cada uno = shell sobre `Workspace`.
//!
//! Scope = **semántica, no CRUD**. El valor es lo que los ficheros crudos no dan barato:
//! backlinks resueltos, huérfanos, dangling, impacto, la puerta OKF, query y escrituras validadas.

use std::collections::BTreeMap;

use lodestar_app::{App, Profile};
use lodestar_core::types::{Direction, FrontmatterPatch, RelPath};
#[cfg(test)]
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

/// Lista las tools con descripción e `inputSchema` (obligatorio en el spec MCP: sin él,
/// los clientes conformes rechazan la tool o el modelo no sabe qué argumentos pasar).
pub fn list() -> Value {
    // Schema de un objeto sin parámetros.
    let empty = json!({ "type": "object", "properties": {}, "additionalProperties": false });
    json!([
        {"name": "find_backlinks", "description": "Quién enlaza a un concept (inbound + index + dangling).",
         "inputSchema": { "type": "object", "properties": {
             "concept": { "type": "string", "description": "Ruta relativa del concept (p. ej. «notas/alfa.md»)." }
         }, "required": ["concept"], "additionalProperties": false }},
        {"name": "find_orphans", "description": "Concepts sin enlaces entrantes.", "inputSchema": empty},
        {"name": "find_dangling", "description": "Enlaces que apuntan a páginas inexistentes.", "inputSchema": empty},
        {"name": "neighborhood", "description": "Subgrafo dirigido alrededor de un concept (depth, direction).",
         "inputSchema": { "type": "object", "properties": {
             "concept": { "type": "string", "description": "Ruta relativa del concept centro." },
             "depth": { "type": "integer", "minimum": 1, "default": 1 },
             "direction": { "type": "string", "enum": ["out", "in", "both"], "default": "out" }
         }, "required": ["concept"], "additionalProperties": false }},
        {"name": "conformance_check", "description": "Gate OKF del bundle (o de un path).",
         "inputSchema": { "type": "object", "properties": {
             "path": { "type": "string", "description": "Opcional: solo los checks de este fichero." }
         }, "additionalProperties": false }},
        {"name": "query", "description": "Query estructurada (DSL de subcadena).",
         "inputSchema": { "type": "object", "properties": {
             "dsl": { "type": "string", "description": "DSL del prototipo: «type:X tag:y is:orphan texto suelto…»." }
         }, "required": ["dsl"], "additionalProperties": false }},
        {"name": "create_concept", "description": "Crea un concept validado (rechaza no conforme).",
         "inputSchema": { "type": "object", "properties": {
             "path": { "type": "string" },
             "type": { "type": "string" },
             "title": { "type": "string" },
             "body": { "type": "string", "description": "Cuerpo Markdown. Si se omite, se genera «# {Tipo} - {Nombre}»." },
             "allow_nonconformant": { "type": "boolean", "default": false }
         }, "required": ["path", "type"], "additionalProperties": false }},
        {"name": "update_frontmatter", "description": "Patch de frontmatter (null borra).",
         "inputSchema": { "type": "object", "properties": {
             "path": { "type": "string" },
             "patch": { "type": "object", "description": "Merge-patch RFC 7386: valor escribe, null borra." }
         }, "required": ["path", "patch"], "additionalProperties": false }},
        {"name": "generate_index", "description": "Regenera el index de un directorio.",
         "inputSchema": { "type": "object", "properties": {
             "dir": { "type": "string", "description": "Directorio relativo («» = raíz).", "default": "" }
         }, "additionalProperties": false }},
        {"name": "generate_tag_indexes", "description": "Regenera los índices de tags.", "inputSchema": empty},
        {"name": "workspace_status", "description": "Config activa, capacidades del perfil, conformidad y recuento agregado del workspace (llámala primero en cada sesión).", "inputSchema": empty},
    ])
}

/// ¿Existe una tool con este nombre? Distingue «tool desconocida» (error de protocolo,
/// `-32602`) de un error de ejecución (que va como `isError` en el result).
pub fn exists(name: &str) -> bool {
    list()
        .as_array()
        .is_some_and(|ts| ts.iter().any(|t| t["name"] == name))
}

/// Despacha una tool por nombre. `profile` solo lo consume `workspace_status` hoy (E10-H08); el
/// resto de tools no dependen del perfil de arranque todavía (E12 lo cambiará: `create_concept`/
/// `update_frontmatter` quedarán fuera del perfil `readonly`).
pub fn call(app: &App, profile: Profile, name: &str, params: &Value) -> ToolResult {
    let ws = app.workspace();
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
                    // Objeto (no array): `structuredContent` del spec MCP exige objeto.
                    Ok(json!({ "checks": checks }))
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
            let paths = ws.query(dsl).map_err(|e| e.to_string())?;
            Ok(json!({ "paths": paths }))
        }
        "create_concept" => {
            let p = rel(params, "path")?;
            let ty = params.get("type").and_then(Value::as_str).unwrap_or("");
            let title = params.get("title").and_then(Value::as_str);
            let body = params.get("body").and_then(Value::as_str).unwrap_or("");
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
        "workspace_status" => {
            let status = app.workspace_status(profile).map_err(|e| e.to_string())?;
            to_json(&status)
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

    /// Como antes (`Workspace` efímero sobre un fixture en disco), pero envuelto en `App` —
    /// `call()` despacha sobre `App` desde E10-H08 (necesita `App::workspace_status`). Las
    /// comparaciones «directas» del golden test siguen yendo contra el mismo `Workspace`, vía
    /// `App::workspace()`.
    fn app_with_fixture() -> (tempfile::TempDir, App) {
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
        (dir, App::from_workspace(ws))
    }

    #[test]
    fn golden_backlinks_igual_workspace() {
        let (_d, app) = app_with_fixture();
        let p = RelPath::new("alfa.md").unwrap();
        let via_tool = call(
            &app,
            Profile::Standard,
            "find_backlinks",
            &json!({"concept": "alfa.md"}),
        )
        .unwrap();
        let direct = serde_json::to_value(app.workspace().backlinks(&p).unwrap()).unwrap();
        assert_eq!(via_tool, direct);
    }

    #[test]
    fn golden_orphans_y_dangling_igual_workspace() {
        let (_d, app) = app_with_fixture();
        let a = app.workspace().analyze().unwrap();
        let orphans = call(&app, Profile::Standard, "find_orphans", &json!({})).unwrap();
        assert_eq!(orphans, json!({ "orphans": a.orphans }));
        let dangling = call(&app, Profile::Standard, "find_dangling", &json!({})).unwrap();
        assert_eq!(dangling, json!({ "dangling": a.dangling }));
    }

    #[test]
    fn golden_query_igual_workspace() {
        let (_d, app) = app_with_fixture();
        let via_tool = call(
            &app,
            Profile::Standard,
            "query",
            &json!({"dsl": "is:orphan"}),
        )
        .unwrap();
        let direct = serde_json::to_value(app.workspace().query("is:orphan").unwrap()).unwrap();
        // Envuelto en objeto: `structuredContent` del spec MCP exige objeto, no array.
        assert_eq!(via_tool, json!({ "paths": direct }));
    }

    #[test]
    fn tools_list_lleva_input_schema() {
        // El spec MCP exige `inputSchema` en cada tool; sin él los clientes conformes las rechazan.
        let tools = list();
        for t in tools.as_array().unwrap() {
            assert!(
                t["inputSchema"]["type"] == "object",
                "tool sin inputSchema: {}",
                t["name"]
            );
        }
    }

    #[test]
    fn tool_desconocida_es_error() {
        let (_d, app) = app_with_fixture();
        assert!(call(&app, Profile::Standard, "no_existe", &json!({})).is_err());
    }

    #[test]
    fn golden_workspace_status_igual_app() {
        let (_d, app) = app_with_fixture();
        let via_tool = call(&app, Profile::Readonly, "workspace_status", &json!({})).unwrap();
        let direct =
            serde_json::to_value(app.workspace_status(Profile::Readonly).unwrap()).unwrap();
        assert_eq!(via_tool, direct);
        assert_eq!(via_tool["capabilities"]["writes"], false);
    }
}
