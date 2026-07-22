//! Handlers de las tools del MCP (`ARCHITECTURE.md §7.2`). Cada uno = shell sobre `Workspace`.
//!
//! Scope = **semántica, no CRUD**. El valor es lo que los ficheros crudos no dan barato:
//! backlinks resueltos, huérfanos, dangling, impacto, la puerta OKF, query y escrituras validadas.

use std::collections::BTreeMap;

use lodestar_app::{schemas, App, CheckScope, Profile, SearchFilters};
use lodestar_core::types::{ConceptRef, Direction, FrontmatterPatch, RelPath, Severity};
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
        {"name": "workspace_status", "description": "Config activa, capacidades del perfil, conformidad y recuento agregado del workspace (llámala primero en cada sesión).", "inputSchema": empty,
         "outputSchema": schemas::workspace_status_schema()},
        {"name": "knowledge_search", "description": "Localiza conceptos por texto y filtros, con snippets y paginación por cursor (nunca devuelve cuerpos).",
         "inputSchema": { "type": "object", "properties": {
             "text": { "type": "string", "description": "Texto libre (subcadena, misma semántica que la DSL del prototipo). Vacío = todos los conceptos." },
             "filters": { "type": "object", "description": "Filtros: types/statuses/tags (listas) y pathPrefix (string).", "properties": {
                 "types": { "type": "array", "items": { "type": "string" } },
                 "statuses": { "type": "array", "items": { "type": "string" } },
                 "tags": { "type": "array", "items": { "type": "string" } },
                 "pathPrefix": { "type": "string" }
             } },
             "sort": { "type": "string", "description": "Reservado: hoy el orden es siempre determinista (score desc, path asc)." },
             "limit": { "type": "integer", "minimum": 1, "maximum": 100, "default": 20 },
             "cursor": { "type": "string", "description": "Cursor opaco de paginación devuelto en «nextCursor»." }
         }, "additionalProperties": false },
         "outputSchema": schemas::knowledge_search_schema()},
        {"name": "knowledge_get", "description": "Obtiene un concepto concreto con `include` selectivo y selección de secciones por headingPath.",
         "inputSchema": { "type": "object", "properties": {
             "ref": { "type": "object", "description": "ConceptRef: identidad del concepto a leer.", "properties": {
                 "path": { "type": "string", "description": "Ruta relativa del concepto (p. ej. «notas/alfa.md»)." }
             }, "required": ["path"], "additionalProperties": false },
             "include": { "type": "array", "description": "Campos a poblar; un campo no pedido queda sin poblar.",
                 "items": { "type": "string", "enum": ["frontmatter", "body", "revision", "outgoingLinks", "backlinks", "diagnostics", "externalReferences"] } },
             "sections": { "type": "array", "description": "Acota «body» a estas subsecciones (solo si «body» está en include). Cada elemento es un headingPath, p. ej. [\"Security\",\"Token rotation\"].",
                 "items": { "type": "array", "items": { "type": "string" } } }
         }, "required": ["ref"], "additionalProperties": false },
         "outputSchema": schemas::knowledge_get_schema()},
        {"name": "schema_inspect", "description": "Descubre el catálogo de tipos (`.lodestar/schema.yaml`): un DocType concreto o el catálogo completo.",
         "inputSchema": { "type": "object", "properties": {
             "mode": { "type": "string", "description": "«catalog» (todos los DocType) o «type» (uno concreto, requiere «type»).", "enum": ["catalog", "type"] },
             "type": { "type": "string", "description": "Nombre del DocType a inspeccionar (solo con mode «type»)." }
         }, "required": ["mode"], "additionalProperties": false },
         "outputSchema": schemas::schema_inspect_schema()},
        {"name": "knowledge_check", "description": "Audita el conocimiento (checks OKF + esquema) con scopes y severidad mínima; diagnósticos con id estable y paginación por cursor.",
         "inputSchema": { "type": "object", "properties": {
             "scope": { "type": "object", "description": "Qué auditar. Discriminado por «kind».", "properties": {
                 "kind": { "type": "string", "enum": ["workspace", "concept", "paths", "affected"] },
                 "ref": { "type": "object", "description": "ConceptRef (solo con kind «concept»).", "properties": {
                     "path": { "type": "string" }
                 }, "required": ["path"] },
                 "paths": { "type": "array", "description": "Lista de paths (solo con kind «paths»).", "items": { "type": "string" } },
                 "refs": { "type": "array", "description": "ConceptRefs centro del vecindario (solo con kind «affected»).",
                     "items": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] } },
                 "depth": { "type": "integer", "minimum": 1, "default": 1, "description": "Distancia máxima del vecindario (solo con kind «affected»)." }
             }, "required": ["kind"] },
             "minimumSeverity": { "type": "string", "enum": ["err", "warn", "info"], "description": "Umbral de severidad de los diagnósticos devueltos (por defecto «info»)." },
             "includeSuggestedFixes": { "type": "boolean", "default": false, "description": "Si false, los diagnósticos no llevan «fixes»." },
             "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 },
             "cursor": { "type": "string", "description": "Cursor opaco de paginación devuelto en «nextCursor»." }
         }, "required": ["scope"], "additionalProperties": false },
         "outputSchema": schemas::knowledge_check_schema()},
        {"name": "graph_query", "description": "Consulta el grafo: backlinks/outgoing/neighborhood/orphans/dangling en una sola tool (consolida find_backlinks/find_orphans/find_dangling/neighborhood).",
         "inputSchema": { "type": "object", "properties": {
             "operation": { "type": "string", "enum": ["backlinks", "outgoing", "neighborhood", "orphans", "dangling"], "description": "Qué subgrafo computar. «backlinks»/«outgoing»/«neighborhood» requieren «ref»; «orphans»/«dangling» no." },
             "ref": { "type": "object", "description": "ConceptRef: el concepto centro (requerido en backlinks/outgoing/neighborhood).", "properties": {
                 "path": { "type": "string", "description": "Ruta relativa del concepto (p. ej. «notas/alfa.md»)." }
             }, "required": ["path"], "additionalProperties": false },
             "depth": { "type": "integer", "minimum": 1, "default": 1, "description": "Solo «neighborhood»." },
             "direction": { "type": "string", "enum": ["out", "in", "both"], "default": "out", "description": "Solo «neighborhood»." },
             "limit": { "type": "integer", "minimum": 1, "description": "Trunca el nº de nodos devueltos (paginación por cursor)." },
             "cursor": { "type": "string", "description": "Cursor opaco de paginación devuelto en «nextCursor»." }
         }, "required": ["operation"], "additionalProperties": false },
         "outputSchema": schemas::graph_query_schema()},
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
        "knowledge_search" => {
            let text = params.get("text").and_then(Value::as_str).unwrap_or("");
            let filters: SearchFilters = match params.get("filters") {
                Some(v) => serde_json::from_value(v.clone()).map_err(|e| e.to_string())?,
                None => SearchFilters::default(),
            };
            let sort = params.get("sort").and_then(Value::as_str);
            let limit = params
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            let cursor = params.get("cursor").and_then(Value::as_str);
            let results = app
                .knowledge_search(text, &filters, sort, limit, cursor)
                .map_err(|e| e.to_string())?;
            to_json(&results)
        }
        "knowledge_get" => {
            let r: ConceptRef = match params.get("ref") {
                Some(v) => serde_json::from_value(v.clone()).map_err(|e| e.to_string())?,
                None => return Err("falta el parámetro «ref»".to_string()),
            };
            let include: Vec<String> = match params.get("include") {
                Some(v) => serde_json::from_value(v.clone()).map_err(|e| e.to_string())?,
                None => Vec::new(),
            };
            let sections: Option<Vec<Vec<String>>> = match params.get("sections") {
                Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| e.to_string())?),
                None => None,
            };
            // Mapeo de error a wire (E10-H02): el texto que ve el agente lleva el código estable
            // `ErrorCode::as_str()` (p. ej. «CONCEPT_NOT_FOUND»), NUNCA el `Debug` de la variante
            // (`ConceptNotFound`) — el catálogo de 16 códigos es el contrato, no el nombre Rust.
            let concept = app
                .knowledge_get(&r, &include, sections.as_deref())
                .map_err(|e| e.as_str().to_string())?;
            Ok(json!({ "concept": to_json(&concept)? }))
        }
        "schema_inspect" => {
            let mode = params
                .get("mode")
                .and_then(Value::as_str)
                .ok_or("falta el parámetro «mode»")?;
            let type_name = params.get("type").and_then(Value::as_str);
            // Mismo mapeo de error a wire que `knowledge_get` (E10-H02): el código estable
            // `ErrorCode::as_str()`, nunca el `Debug` de la variante.
            let inspection = app
                .schema_inspect(mode, type_name)
                .map_err(|e| e.as_str().to_string())?;
            to_json(&inspection)
        }
        "knowledge_check" => {
            let scope: CheckScope = match params.get("scope") {
                Some(v) => serde_json::from_value(v.clone()).map_err(|e| e.to_string())?,
                None => return Err("falta el parámetro «scope»".to_string()),
            };
            // Wire de severidad mínima → `Severity` (err|warn|info); ausente = sin umbral extra.
            let min_severity = match params.get("minimumSeverity").and_then(Value::as_str) {
                Some("err") => Some(Severity::Err),
                Some("warn") => Some(Severity::Warn),
                Some("info") => Some(Severity::Info),
                Some(other) => return Err(format!("minimumSeverity inválido: «{other}»")),
                None => None,
            };
            let include_fixes = params
                .get("includeSuggestedFixes")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            let limit = params
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            let cursor = params.get("cursor").and_then(Value::as_str);
            // Mismo mapeo de error a wire que `knowledge_get`/`schema_inspect` (E10-H02): el código
            // estable `ErrorCode::as_str()`, nunca el `Debug` de la variante.
            let report = app
                .knowledge_check(&scope, min_severity, include_fixes, limit, cursor)
                .map_err(|e| e.as_str().to_string())?;
            to_json(&report)
        }
        "graph_query" => {
            let operation = params
                .get("operation")
                .and_then(Value::as_str)
                .ok_or("falta el parámetro «operation»")?;
            let r: Option<ConceptRef> = match params.get("ref") {
                Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| e.to_string())?),
                None => None,
            };
            let depth = params
                .get("depth")
                .and_then(Value::as_u64)
                .map(|n| n as u32);
            let direction = params.get("direction").and_then(Value::as_str);
            let limit = params
                .get("limit")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            let cursor = params.get("cursor").and_then(Value::as_str);
            // Mismo mapeo de error a wire que `knowledge_get`/`schema_inspect`/`knowledge_check`
            // (E10-H02): el código estable `ErrorCode::as_str()`, nunca el `Debug` de la variante.
            let result = app
                .graph_query(operation, r.as_ref(), depth, direction, limit, cursor)
                .map_err(|e| e.as_str().to_string())?;
            to_json(&result)
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
