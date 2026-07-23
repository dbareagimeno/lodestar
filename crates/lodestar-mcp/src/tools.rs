//! Handlers de las tools del MCP (`ARCHITECTURE.md §7.2`). Cada uno = shell sobre `Workspace`.
//!
//! Scope = **semántica, no CRUD**. El valor es lo que los ficheros crudos no dan barato:
//! backlinks resueltos, aislados, dangling, impacto, la puerta de validación, query y escrituras
//! validadas.

use lodestar_app::{schemas, App, CheckScope, Profile, SearchFilters};
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::{ChangeSetId, DocumentRef, ReceiptId, Severity, WorkspaceRevision};
#[cfg(test)]
use lodestar_workspace::Workspace;
use serde_json::{json, Value};

/// Error de tool con un mensaje legible (la fachada lo envuelve en el error JSON-RPC).
pub type ToolResult = Result<Value, String>;

/// Lista las tools con descripción e `inputSchema` (obligatorio en el spec MCP: sin él,
/// los clientes conformes rechazan la tool o el modelo no sabe qué argumentos pasar).
pub fn list() -> Value {
    // Schema de un objeto sin parámetros.
    let empty = json!({ "type": "object", "properties": {}, "additionalProperties": false });
    json!([
        {"name": "workspace_status", "description": "Config activa, capacidades del perfil, conformidad y recuento agregado del workspace (llámala primero en cada sesión).", "inputSchema": empty,
         "outputSchema": schemas::workspace_status_schema()},
        {"name": "knowledge_search", "description": "Localiza documentos por texto y filtros, con snippets y paginación por cursor (nunca devuelve cuerpos).",
         "inputSchema": { "type": "object", "properties": {
             "text": { "type": "string", "description": "Texto libre (subcadena, misma semántica que la DSL del prototipo). Vacío = todos los documentos." },
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
        {"name": "knowledge_get", "description": "Obtiene un documento concreto con `include` selectivo y selección de secciones por headingPath.",
         "inputSchema": { "type": "object", "properties": {
             "ref": { "type": "object", "description": "DocumentRef: identidad del documento a leer.", "properties": {
                 "path": { "type": "string", "description": "Ruta relativa del documento (p. ej. «notas/alfa.md»)." }
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
        {"name": "knowledge_check", "description": "Audita el conocimiento (diagnósticos del documento + esquema) con scopes y severidad mínima; diagnósticos con id estable y paginación por cursor.",
         "inputSchema": { "type": "object", "properties": {
             "scope": { "type": "object", "description": "Qué auditar. Discriminado por «kind».", "properties": {
                 "kind": { "type": "string", "enum": ["workspace", "document", "paths", "affected"] },
                 "ref": { "type": "object", "description": "DocumentRef (solo con kind «document»).", "properties": {
                     "path": { "type": "string" }
                 }, "required": ["path"] },
                 "paths": { "type": "array", "description": "Lista de paths (solo con kind «paths»).", "items": { "type": "string" } },
                 "refs": { "type": "array", "description": "DocumentRefs centro del vecindario (solo con kind «affected»).",
                     "items": { "type": "object", "properties": { "path": { "type": "string" } }, "required": ["path"] } },
                 "depth": { "type": "integer", "minimum": 1, "default": 1, "description": "Distancia máxima del vecindario (solo con kind «affected»)." }
             }, "required": ["kind"] },
             "minimumSeverity": { "type": "string", "enum": ["err", "warn", "info"], "description": "Umbral de severidad de los diagnósticos devueltos (por defecto «info»)." },
             "includeSuggestedFixes": { "type": "boolean", "default": false, "description": "Si false, los diagnósticos no llevan «fixes»." },
             "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100 },
             "cursor": { "type": "string", "description": "Cursor opaco de paginación devuelto en «nextCursor»." }
         }, "required": ["scope"], "additionalProperties": false },
         "outputSchema": schemas::knowledge_check_schema()},
        {"name": "graph_query", "description": "Consulta el grafo: backlinks/outgoing/neighborhood/isolated/dangling/path_between/cycles/components en una sola tool (consolida find_backlinks/find_orphans/find_dangling/neighborhood).",
         "inputSchema": { "type": "object", "properties": {
             "operation": { "type": "string", "enum": ["backlinks", "outgoing", "neighborhood", "isolated", "dangling", "path_between", "cycles", "components"], "description": "Qué subgrafo computar. «backlinks»/«outgoing»/«neighborhood» requieren «ref»; «path_between» requiere «ref» (origen) y «to» (destino); «isolated»/«dangling»/«cycles»/«components» no requieren refs. «isolated» = documentos sin enlaces internos entrantes NI salientes (antes «orphans»)." },
             "ref": { "type": "object", "description": "DocumentRef: el documento centro (requerido en backlinks/outgoing/neighborhood; origen en path_between).", "properties": {
                 "path": { "type": "string", "description": "Ruta relativa del documento (p. ej. «notas/alfa.md»)." }
             }, "required": ["path"], "additionalProperties": false },
             "to": { "type": "object", "description": "DocumentRef destino, solo «path_between» (extremo final del camino dirigido).", "properties": {
                 "path": { "type": "string", "description": "Ruta relativa del documento destino." }
             }, "required": ["path"], "additionalProperties": false },
             "depth": { "type": "integer", "minimum": 1, "default": 1, "description": "Solo «neighborhood»." },
             "direction": { "type": "string", "enum": ["out", "in", "both"], "default": "out", "description": "Solo «neighborhood»." },
             "limit": { "type": "integer", "minimum": 1, "description": "Trunca el nº de nodos devueltos (paginación por cursor)." },
             "cursor": { "type": "string", "description": "Cursor opaco de paginación devuelto en «nextCursor»." }
         }, "required": ["operation"], "additionalProperties": false },
         "outputSchema": schemas::graph_query_schema()},
        {"name": "impact_analyze", "description": "Analiza el impacto de un cambio hipotético sobre un documento (sin aplicarlo): afectados directos/transitivos, relaciones tipadas obligatorias que romperían (bloqueos) y nivel de riesgo. Reusa el blast-radius entrante y las relaciones del schema.",
         "inputSchema": { "type": "object", "properties": {
             "ref": { "type": "object", "description": "DocumentRef: el documento sobre el que se propone el cambio.", "properties": {
                 "path": { "type": "string", "description": "Ruta relativa del documento (p. ej. «notas/alfa.md»)." }
             }, "required": ["path"], "additionalProperties": false },
             "proposedOperation": { "type": "object", "description": "El cambio hipotético a evaluar.", "properties": {
                 "kind": { "type": "string", "enum": ["move", "delete", "deprecate", "transition_status", "change_relation", "replace_document"], "description": "Tipo de operación propuesta. Solo «delete» computa bloqueos estructurales en v1." }
             }, "required": ["kind"], "additionalProperties": false },
             "depth": { "type": "integer", "minimum": 1, "description": "Profundidad del blast-radius entrante; por defecto cubre todo el alcance transitivo." }
         }, "required": ["ref", "proposedOperation"], "additionalProperties": false },
         "outputSchema": schemas::impact_analyze_schema()},
        {"name": "change_plan", "description": "Planifica un cambio complejo SIN escribir: normaliza las operaciones propuestas, simula su aplicación en memoria y valida el resultado. Devuelve un único change set (normalizedOperations, semanticDiff, risk, impact, diagnosticsBefore/After) con un planHash determinista. No toca disco (aplicar es change_apply, E13).",
         "inputSchema": { "type": "object", "properties": {
             "expectedWorkspaceRevision": { "type": "string", "description": "Control optimista a nivel de workspace («blake3:…»). Si se omite, se toma la revisión actual; si no coincide → REVISION_CONFLICT." },
             "operations": { "type": "array", "description": "Operaciones propuestas, discriminadas por «op» (create/patch_frontmatter/replace_body/replace_text/edit_section/move/delete/add_relation/remove_relation/transition_status/apply_fix). Cada op puede llevar «expectedRevision» (DocumentRevision «blake3:…») para control optimista por documento.",
                 "items": { "type": "object", "properties": {
                     "op": { "type": "string", "enum": ["create", "patch_frontmatter", "replace_body", "replace_text", "edit_section", "move", "delete", "add_relation", "remove_relation", "transition_status", "apply_fix"] },
                     "path": { "type": "string" },
                     "ref": { "type": "object", "properties": { "path": { "type": "string" } } },
                     "expectedRevision": { "type": "string", "description": "DocumentRevision que el agente cree vigente («blake3:…»); si el documento cambió → REVISION_CONFLICT." }
                 }, "required": ["op"] } },
             "policy": { "type": "object", "description": "Política de aplicación del plan.", "properties": {
                 "requireConformantResult": { "type": "boolean", "description": "Si true, un resultado no conforme bloquea canApply." },
                 "allowWarnings": { "type": "boolean", "description": "Si false, cualquier warning bloquea canApply." }
             } }
         }, "required": ["operations"], "additionalProperties": false },
         "outputSchema": schemas::change_plan_schema()},
        {"name": "change_apply", "description": "Aplica un plan previamente calculado y vigente por el ÚNICO ESCRITOR, con todas las salvaguardas transaccionales (staging → lock → copias de recuperación → write-ahead journal → renames atómicos → receipt). Verifica caducidad (PLAN_EXPIRED) y planHash (PLAN_STALE si el workspace cambió bajo el plan) y rechaza escrituras fuera de writableRoots (PERMISSION_DENIED). Devuelve el recibo con las revisiones antes/después y el semanticDiff.",
         "inputSchema": { "type": "object", "properties": {
             "changeSetId": { "type": "string", "description": "El «changeset:<hash>» que devolvió change_plan (E12-H08); el plan se recupera de runtime por este id." },
             "expectedWorkspaceRevision": { "type": "string", "description": "Control optimista a nivel de workspace («blake3:…»). Si se omite, se adopta la revisión actual; si no coincide → REVISION_CONFLICT." }
         }, "required": ["changeSetId"], "additionalProperties": false },
         "outputSchema": schemas::change_apply_schema()},
        {"name": "change_revert", "description": "Revierte una transacción RECIENTE y no alterada por el ÚNICO ESCRITOR, devolviendo el conocimiento canónico al estado anterior al apply desde sus copias de recuperación (transacción inversa recuperable con journal propio). Requiere que el receipt siga disponible (PLAN_EXPIRED si caducó/purgado por retención), que el workspace no haya cambiado tras el apply (WRITE_CONFLICT si un fichero afectado se alteró) y —opcionalmente— control optimista de workspace (REVISION_CONFLICT). Devuelve el recibo de la reversión con las revisiones antes/después: el workspace vuelve a la previousRevision del apply.",
         "inputSchema": { "type": "object", "properties": {
             "receiptId": { "type": "string", "description": "El «receiptId» que devolvió change_apply (E13-H08); localiza el receipt persistido (E13-H07) y sus copias de recuperación." },
             "expectedWorkspaceRevision": { "type": "string", "description": "Control optimista a nivel de workspace («blake3:…»). Si se omite, se adopta la revisión actual; si no coincide → REVISION_CONFLICT." }
         }, "required": ["receiptId"], "additionalProperties": false },
         "outputSchema": schemas::change_revert_schema()},
    ])
}

/// Las 3 tools **de cambio** (perfil `standard` en `contracts/mcp.yml`): planifican, aplican o
/// revierten cambios sobre el conocimiento. `change_plan` cuenta como tool de cambio aunque no
/// escriba en disco (planifica un cambio). **Fuente única** del efecto del perfil sobre la
/// superficie: el perfil `readonly` las oculta de [`available_tools`] y hace que [`available`] rechace su
/// invocación (E14-H03, `ARCHITECTURE.md §19.6`).
pub const CHANGE_TOOLS: [&str; 3] = ["change_plan", "change_apply", "change_revert"];

/// ¿Es `name` una tool de cambio (requiere perfil `standard` para usarse)?
pub fn is_change_tool(name: &str) -> bool {
    CHANGE_TOOLS.contains(&name)
}

/// ¿Existe una tool con este nombre? Distingue «tool desconocida» (error de protocolo,
/// `-32602`) de un error de ejecución (que va como `isError` en el result).
pub fn exists(name: &str) -> bool {
    list()
        .as_array()
        .is_some_and(|ts| ts.iter().any(|t| t["name"] == name))
}

/// Catálogo de tools **visible bajo `profile`**: el perfil `readonly` oculta las tools de cambio
/// (`change_plan`/`change_apply`/`change_revert`); `standard` las incluye. Deriva del predicado
/// único [`is_change_tool`] + [`Profile::writes_enabled`] (E14-H03).
pub fn available_tools(profile: Profile) -> Value {
    if profile.writes_enabled() {
        return list();
    }
    let visibles: Vec<Value> = list()
        .as_array()
        .map(|ts| {
            ts.iter()
                .filter(|t| !is_change_tool(t["name"].as_str().unwrap_or("")))
                .cloned()
                .collect()
        })
        .unwrap_or_default();
    Value::Array(visibles)
}

/// ¿Está disponible la tool `name` bajo `profile`? Existe y, si es de cambio, el perfil habilita
/// escrituras. Un `false` sobre una tool de cambio en `readonly` produce el mismo `-32602` que una
/// tool desconocida: ocultarla de la lista no basta, un cliente que la llame igualmente NO debe
/// ejecutarla (E14-H03).
pub fn available(profile: Profile, name: &str) -> bool {
    exists(name) && (profile.writes_enabled() || !is_change_tool(name))
}

/// Despacha una tool por nombre sobre la superficie objetivo de 10 tools (E14-H06 retiró las 10
/// heredadas: un nombre heredado cae ahora en el brazo por defecto → tool desconocida). `profile`
/// solo lo consume `workspace_status` hoy (E10-H08); el resto de tools no dependen del perfil de
/// arranque (las de cambio se filtran antes, en [`available`]).
pub fn call(app: &App, profile: Profile, name: &str, params: &Value) -> ToolResult {
    match name {
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
            let r: DocumentRef = match params.get("ref") {
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
            // `ErrorCode::as_str()` (p. ej. «DOCUMENT_NOT_FOUND»), NUNCA el `Debug` de la variante
            // (`DocumentNotFound`) — el catálogo de 16 códigos es el contrato, no el nombre Rust.
            let document = app
                .knowledge_get(&r, &include, sections.as_deref())
                .map_err(|e| e.as_str().to_string())?;
            Ok(json!({ "document": to_json(&document)? }))
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
            let r: Option<DocumentRef> = match params.get("ref") {
                Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| e.to_string())?),
                None => None,
            };
            // Segundo extremo, solo para `path_between` (destino del camino dirigido).
            let to: Option<DocumentRef> = match params.get("to") {
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
                .graph_query(
                    operation,
                    r.as_ref(),
                    to.as_ref(),
                    depth,
                    direction,
                    limit,
                    cursor,
                )
                .map_err(|e| e.as_str().to_string())?;
            to_json(&result)
        }
        "impact_analyze" => {
            let r: DocumentRef = match params.get("ref") {
                Some(v) => serde_json::from_value(v.clone()).map_err(|e| e.to_string())?,
                None => return Err("falta el parámetro «ref»".to_string()),
            };
            let kind = params
                .get("proposedOperation")
                .and_then(|op| op.get("kind"))
                .and_then(Value::as_str)
                .ok_or("falta el parámetro «proposedOperation.kind»")?;
            let depth = params
                .get("depth")
                .and_then(Value::as_u64)
                .map(|n| n as u32);
            // Mismo mapeo de error a wire que las demás tools (E10-H02): el código estable
            // `ErrorCode::as_str()`, nunca el `Debug` de la variante.
            let report = app
                .impact_analyze(&r, kind, depth)
                .map_err(|e| e.as_str().to_string())?;
            to_json(&report)
        }
        "change_plan" => {
            let expected = params
                .get("expectedWorkspaceRevision")
                .and_then(Value::as_str)
                .map(|s| WorkspaceRevision(s.to_string()));
            let operations = params.get("operations").cloned().unwrap_or(Value::Null);
            let policy: PlanPolicy = match params.get("policy") {
                Some(v) => serde_json::from_value(v.clone()).map_err(|e| e.to_string())?,
                None => PlanPolicy::default(),
            };
            // Mismo mapeo de error a wire que las demás tools (E10-H02): el código estable
            // `ErrorCode::as_str()` (p. ej. «REVISION_CONFLICT»), nunca el `Debug` de la variante.
            let result = app
                .change_plan(expected, &operations, policy)
                .map_err(|e| e.as_str().to_string())?;
            to_json(&result)
        }
        "change_apply" => {
            let change_set_id = ChangeSetId(
                params
                    .get("changeSetId")
                    .and_then(Value::as_str)
                    .ok_or("falta el parámetro «changeSetId»")?
                    .to_string(),
            );
            let expected = params
                .get("expectedWorkspaceRevision")
                .and_then(Value::as_str)
                .map(|s| WorkspaceRevision(s.to_string()));
            // Mismo mapeo de error a wire que las demás tools (E10-H02): el código estable
            // `ErrorCode::as_str()` (p. ej. «PLAN_STALE»/«PERMISSION_DENIED»), nunca el `Debug`.
            let result = app
                .change_apply(&change_set_id, expected)
                .map_err(|e| e.as_str().to_string())?;
            to_json(&result)
        }
        "change_revert" => {
            let receipt_id = ReceiptId(
                params
                    .get("receiptId")
                    .and_then(Value::as_str)
                    .ok_or("falta el parámetro «receiptId»")?
                    .to_string(),
            );
            let expected = params
                .get("expectedWorkspaceRevision")
                .and_then(Value::as_str)
                .map(|s| WorkspaceRevision(s.to_string()));
            // Mismo mapeo de error a wire que las demás tools (E10-H02): el código estable
            // `ErrorCode::as_str()` (p. ej. «WRITE_CONFLICT»/«PLAN_EXPIRED»), nunca el `Debug`.
            let result = app
                .change_revert(&receipt_id, expected)
                .map_err(|e| e.as_str().to_string())?;
            to_json(&result)
        }
        other => Err(format!("tool desconocida: {other}")),
    }
}

fn to_json<T: serde::Serialize>(v: &T) -> ToolResult {
    serde_json::to_value(v).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    //! Golden cross-fachada (E7-H06): la salida de cada tool == la del `Workspace` directo.
    //! Verifica que la fachada MCP es un shell fino sin lógica de dominio propia (`§2`, `§7`).
    use super::*;

    /// Como antes (`Workspace` efímero sobre un fixture en disco), pero envuelto en `App` —
    /// `call()` despacha sobre `App` desde E10-H08 (necesita `App::workspace_status`). Las
    /// comparaciones «directas» del golden test siguen yendo contra el mismo `Workspace`, vía
    /// `App::workspace()`.
    fn app_with_fixture() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in [
            ("index.md", "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Alfa](alfa.md)\n"),
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

    // NOTA E14-H06: los golden `golden_backlinks_igual_workspace`,
    // `golden_orphans_y_dangling_igual_workspace` y `golden_query_igual_workspace` se RETIRARON al
    // retirar las tools heredadas `find_backlinks`/`find_orphans`/`find_dangling`/`query`. Su
    // cobertura vive hoy en la superficie objetivo (e2e en `tests/mcp.rs`): `find_backlinks` →
    // `graph_query(backlinks)` (`graph_backlinks`); `find_orphans` → `graph_query(isolated)`
    // (`graph_isolated`); `find_dangling` → `graph_query(dangling)` (`graph_dangling`); `query` →
    // `knowledge_search` (`search_sin_cuerpos`/`search_filtra_tipo`/`search_paginacion`). El golden
    // cross-fachada de que la tool == el `Workspace` directo lo sigue verificando
    // `golden_workspace_status_igual_app` para una tool objetivo.

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
