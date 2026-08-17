//! Handlers de las tools del MCP (`ARCHITECTURE.md §7.2`). Cada uno = shell sobre `Workspace`.
//!
//! Scope = **semántica, no CRUD**. El valor es lo que los ficheros crudos no dan barato:
//! backlinks resueltos, aislados, dangling, impacto, la puerta de validación, query y escrituras
//! validadas.

use lodestar_app::{schemas, App, CheckScope, Profile};
use lodestar_core::plan::PlanPolicy;
use lodestar_core::types::{
    ChangeSetId, DocumentRef, ErrorCode, InboundLinksPolicy, ReceiptId, Severity, WorkspaceRevision,
};
#[cfg(test)]
use lodestar_workspace::Workspace;
use serde_json::{json, Value};

/// Error de tool con un mensaje legible (la fachada lo envuelve en el error JSON-RPC).
pub type ToolResult = Result<Value, String>;

/// El primer `inboundLinksPolicy` de `raw_ops` que **no** esté en
/// [`InboundLinksPolicy::WIRE_VALUES`], o `None` si todos son válidos (E23-H05).
///
/// Recorre tanto el array `operations` como el `operation` de una selección masiva, que son las dos
/// formas de wire que acepta `change_plan`.
fn politica_de_borrado_invalida(raw_ops: &Value) -> Option<String> {
    fn de_op(op: &Value) -> Option<String> {
        // Forma suelta (`{"op":"delete", "inboundLinksPolicy":…}`) y forma de selección masiva
        // (`{"delete": {"inboundLinksPolicy":…}}`), que anida los params bajo el tipo de op.
        let directo = op.get("inboundLinksPolicy");
        let anidado = op.get("delete").and_then(|d| d.get("inboundLinksPolicy"));
        let valor = directo.or(anidado)?.as_str()?;
        (!InboundLinksPolicy::WIRE_VALUES.contains(&valor)).then(|| valor.to_string())
    }

    match raw_ops {
        Value::Array(ops) => ops.iter().find_map(de_op),
        // Selección masiva: `{selection, operation}`; la op viaja en `operation`.
        Value::Object(_) => raw_ops.get("operation").and_then(de_op),
        _ => None,
    }
}

/// Schema de UNA operación de `change_plan.operations[]` — E23-H10.
///
/// Vive en su propia función por dos razones. La primera es mecánica: metido en el `json!` de
/// [`list`] hacía saltar el límite de recursión del macro. La segunda importa más: es el documento
/// que un cliente MCP lee para saber CÓMO escribir, así que merece estar donde se pueda revisar de
/// un vistazo contra `normalize_raw_op` (`lodestar-app`), que es quien de verdad lee estos campos.
///
/// E23-H11 retiró del enum la octava operación, `apply_fix` (y con ella su parámetro `fixId`): sin
/// productor de `Fix` desde E20-H03, siempre fallaba — ver `docs/history/PROPUESTA_FIXES.md`.
///
/// Hasta E23-H10 aquí solo se declaraban `op`/`path`/`ref`/`expectedRevision`: **ni uno** de los
/// parámetros reales de 7 de las 8 operaciones de entonces. Para un producto cuyo público objetivo
/// son agentes, era el mayor agujero de usabilidad de la superficie — el schema decía qué operaciones existen
/// pero no cómo invocarlas, así que había que adivinar.
///
/// Se declaran **planas** (todas las propiedades juntas, cada una diciendo a qué op pertenece) en
/// vez de con un `oneOf` por op: `path`/`ref` son intercambiables salvo en `create` y `body`
/// pertenece a **dos** ops (`create` y `replace_body`), así que un `oneOf` por operación rechazaría
/// entradas válidas —un lote en el que el agente reutiliza la misma plantilla de objeto— sin que
/// ningún test lo notara.
///
/// **E29-H08**: desde v0.6.0 estas propiedades **sí se ejecutan**, pero por **UNIÓN**: son la fuente
/// única de la que `validacion::union_de_campos_de_operacion` deriva los campos legales de una
/// operación. Un campo que no esté en la unión de los 17 se rechaza con `INVALID_SCHEMA`
/// nombrándolo; un campo legal de **otra** op se sigue ignorando (`decisiones §15`). Cerrar la
/// partición por op —el `oneOf` que sigue sin existir— es decisión **posterior**, declarada, no un
/// olvido.
fn operacion_item_schema() -> Value {
    json!({ "type": "object", "properties": {
                     "op": { "type": "string", "enum": ["create", "patch_frontmatter", "replace_body", "replace_text", "edit_section", "move", "delete"],
                             "description": "Qué operación es. Determina qué otros campos se leen; los que no pertenecen a esta op se IGNORAN." },
                     "path": { "type": "string", "description": "Ruta relativa del documento. Obligatoria en «create»; en «patch_frontmatter»/«replace_body»/«replace_text»/«edit_section»/«delete» es la alternativa corta a «ref.path» (se acepta cualquiera de las dos)." },
                     "ref": { "type": "object", "description": "DocumentRef, alternativa a «path» en las ops que operan sobre un documento existente.",
                              "properties": { "path": { "type": "string" } } },
                     "expectedRevision": { "type": "string", "description": "DocumentRevision que el agente cree vigente («blake3:…»); si el documento cambió → REVISION_CONFLICT." },

                     "frontmatter": { "type": "object", "description": "[create] Frontmatter YAML ARBITRARIO del documento nuevo. Opcional: sin él, el documento se crea SIN bloque de frontmatter (no uno vacío). Ninguna clave es obligatoria ni tiene semántica impuesta (§20.2 invariante 3); el título se DERIVA (frontmatter.title → primer H1 → nombre del fichero), no hace falta materializarlo." },
                     "body": { "type": "string", "description": "[create, replace_body] Cuerpo Markdown (sin el bloque de frontmatter). En «create» es opcional: sin él se genera un heading con el título derivado. En «replace_body» sustituye el cuerpo entero conservando el frontmatter existente —incluida su AUSENCIA: un documento sin bloque no gana uno." },
                     "patch": { "type": "object", "description": "[patch_frontmatter] Merge-patch RFC 7386 sobre el frontmatter: una clave con valor la fija, una clave con «null» la BORRA, y lo que no se menciona sobrevive byte a byte (patch quirúrgico, E16-H04). Sobre un frontmatter ilegible la operación falla en vez de reescribirlo encima." },
                     "find": { "type": "string", "description": "[replace_text] Texto literal a buscar (no es una regex). Solo se busca en el CUERPO, nunca en el frontmatter." },
                     "replace": { "type": "string", "description": "[replace_text] Texto literal de sustitución. Se sustituyen TODAS las ocurrencias." },
                     "expectedOccurrences": { "type": "integer", "minimum": 0, "description": "[replace_text] Si se indica y el nº real de ocurrencias no coincide, la operación falla en vez de aplicar un cambio distinto del que el agente creía." },
                     "headingPath": { "type": "array", "items": { "type": "string" }, "description": "[edit_section] Ruta de headings hasta la sección, p. ej. [\"Seguridad\",\"Rotación de tokens\"]. Si no existe → DOCUMENT_NOT_FOUND." },
                     "mode": { "type": "string", "enum": ["replace", "append", "prepend"], "default": "replace", "description": "[edit_section] Qué hacer con «content» respecto al contenido actual de la sección." },
                     "content": { "type": "string", "description": "[edit_section] Contenido Markdown de la sección." },
                     "from": { "type": "string", "description": "[move] Ruta actual del documento." },
                     "to": { "type": "string", "description": "[move] Ruta destino. Los enlaces relativos SALIENTES del propio documento se recalculan solos desde la ubicación nueva (E23-H03)." },
                     "rewriteInboundLinks": { "type": "boolean", "default": false, "description": "[move] Si true, reescribe además los enlaces ENTRANTES de todos los documentos que apuntan al movido (incluidas las definiciones de referencia), en la misma transacción. Con false los backlinks quedan apuntando a la ruta vieja y se rompen: actívalo salvo que sepas lo que haces." },
                     "inboundLinksPolicy": { "type": "string", "enum": ["reject", "remove_links"], "description": "[delete] Qué hacer con los enlaces entrantes. OBLIGATORIO cuando el documento tiene backlinks (§20.11 prohíbe elegir en silencio); sin backlinks no hay nada que decidir y puede omitirse. «reject» = fallar con INBOUND_LINKS_EXIST; «remove_links» = desenlazar en los emisores dejando su texto plano. («retarget» y «create_stub» se RETIRARON en E23-H05: se aceptaban sin ejecutarse.)" }
                 }, "required": ["op"] })
}

/// Lista las tools con descripción e `inputSchema` (obligatorio en el spec MCP: sin él,
/// los clientes conformes rechazan la tool o el modelo no sabe qué argumentos pasar).
pub fn list() -> Value {
    // Schema de un objeto sin parámetros.
    let empty = json!({ "type": "object", "properties": {}, "additionalProperties": false });
    json!([
        {"name": "workspace_status", "description": "Config activa, capacidades del perfil, validez y recuento agregado del workspace, recuperación pendiente y los recibos disponibles para revertir (llámala primero en cada sesión).", "inputSchema": empty,
         "outputSchema": schemas::workspace_status_schema()},
        {"name": "knowledge_search", "description": "Localiza documentos por texto libre y por el lenguaje de consulta tipado (where/filter), con snippets y paginación por cursor (nunca devuelve cuerpos).",
         "inputSchema": { "type": "object", "properties": {
             "text": { "type": "string", "description": "Texto libre (subcadena sobre basename + valores de frontmatter + cuerpo). Vacío = todos los documentos." },
             "where": { "type": "string", "description": "Consulta textual del lenguaje tipado (§20.8), p. ej. «status = \"accepted\" and graph.backlinks = 0». Se intersecta con «text» y con «filter»." },
             "filter": { "type": "object", "description": "Filtro JSON estructurado (§20.10) equivalente a «where»: {field, operator, value} o envolturas and/or/not/has/missing. Si llegan «where» y «filter», se combinan con AND." },
             "include": { "type": "array", "description": "Campos de FRONTMATTER a proyectar en cada resultado, como «frontmatter.<fieldPath>» (p. ej. «frontmatter.status» o el anidado «frontmatter.owner.name»). Ahorra un knowledge_get por documento solo para leer un campo. Los valores viajan con su tipo YAML real y un campo que un documento no tiene NO aparece en su mapa (nunca null). Una entrada que no empiece por «frontmatter.», o cuyo sufijo no sea un field path válido, se RECHAZA con INVALID_SCHEMA.",
                 // El «pattern» declara el prefijo obligatorio y un sufijo NO vacío, sin cerrar los
                 // valores: el field path es abierto por naturaleza (§20.2), así que solo se puede
                 // acotar la forma. Es deliberadamente un SUPERCONJUNTO de lo que acepta el
                 // despachador —«frontmatter.a..b» casa el patrón y aun así falla en
                 // `FieldPath::parse`—, que es la dirección segura: un cliente que valide contra el
                 // schema caza el error más común antes de la llamada, y ningún `include` válido se
                 // rechaza de más. La validación de ejecución (E23-H11) se queda como está: es la
                 // que da el mensaje bueno y la única que puede ser exacta.
                 "items": { "type": "string", "pattern": "^frontmatter\\..+" } },
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
                 "items": { "type": "string", "enum": ["frontmatter", "body", "revision", "outgoingLinks", "backlinks", "diagnostics"] } },
             "sections": { "type": "array", "description": "Acota «body» a estas subsecciones (solo si «body» está en include). Cada elemento es un headingPath, p. ej. [\"Security\",\"Token rotation\"].",
                 "items": { "type": "array", "items": { "type": "string" } } }
         }, "required": ["ref"], "additionalProperties": false },
         "outputSchema": schemas::knowledge_get_schema()},
        {"name": "metadata_inspect", "description": "Descubre las convenciones de metadata de una base desconocida SIN necesitar un schema: el catálogo de propiedades (qué campos existen, en cuántos documentos y de qué tipos) o la inspección de una propiedad (presencia/ausencia, tipos y valores frecuentes).",
         "inputSchema": { "type": "object", "properties": {
             "mode": { "type": "string", "description": "«catalog» (todos los campos con presencia y tipos) o «field» (inspección de un campo concreto, requiere «field»).", "enum": ["catalog", "field"] },
             "field": { "type": "string", "description": "Dot-path del campo a inspeccionar (p. ej. «status» o «service.tier»); solo con mode «field». Mismo dialecto que «where»/«filter»: «frontmatter.status» ≡ «status», y una clave de TU frontmatter que colisione con un namespace reservado se alcanza anclada («frontmatter.graph.backlinks»). Un namespace reservado a secas («graph.backlinks», «document.path») NO es metadata: es INVALID_SCHEMA (para el grafo, graph_query)." },
             "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100, "description": "Trunca la lista de la página: «fields» con mode «catalog», «values» con mode «field». Los agregados (presentIn/missingIn/inferredTypes) siguen describiendo TODO el workspace: se pagina la lista, no la estadística." },
             "cursor": { "type": "string", "description": "Cursor opaco de paginación devuelto en «nextCursor»." }
         }, "required": ["mode"], "additionalProperties": false },
         "outputSchema": schemas::metadata_inspect_schema()},
        {"name": "knowledge_check", "description": "Audita el conocimiento (diagnósticos de interpretabilidad y enlaces del documento) con scopes y severidad mínima; diagnósticos con id estable y paginación por cursor.",
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
             "limit": { "type": "integer", "minimum": 1, "maximum": 1000, "default": 100, "description": "Trunca el nº de nodos devueltos (paginación por cursor). Sin él se sirven 100 nodos: hasta v0.4.0 no había default ni máximo, así que «components» sobre una base grande devolvía el grafo COMPLETO en una respuesta (E26-H10)." },
             "cursor": { "type": "string", "description": "Cursor opaco de paginación devuelto en «nextCursor»." }
         }, "required": ["operation"], "additionalProperties": false },
         "outputSchema": schemas::graph_query_schema()},
        {"name": "impact_analyze", "description": "Analiza el impacto de un cambio hipotético sobre un documento (sin aplicarlo): afectados directos/transitivos y nivel de riesgo, sobre el grafo de enlaces. Reusa el blast-radius entrante.",
         "inputSchema": { "type": "object", "properties": {
             "ref": { "type": "object", "description": "DocumentRef: el documento sobre el que se propone el cambio.", "properties": {
                 "path": { "type": "string", "description": "Ruta relativa del documento (p. ej. «notas/alfa.md»)." }
             }, "required": ["path"], "additionalProperties": false },
             "proposedOperation": { "type": "object", "description": "El cambio hipotético a evaluar.", "properties": {
                 "kind": { "type": "string", "enum": ["move", "delete"], "description": "Tipo de operación propuesta (modelo universal, §20.10). Solo «delete» computa bloqueos estructurales en v1." }
             }, "required": ["kind"], "additionalProperties": false },
             "depth": { "type": "integer", "minimum": 1, "description": "Profundidad del blast-radius entrante; por defecto cubre todo el alcance transitivo." }
         }, "required": ["ref", "proposedOperation"], "additionalProperties": false },
         "outputSchema": schemas::impact_analyze_schema()},
        {"name": "change_plan", "description": "Planifica un cambio complejo SIN escribir: normaliza las operaciones propuestas, simula su aplicación en memoria y valida el resultado. Devuelve un único change set (normalizedOperations, noOpOperations, semanticDiff, risk, impact, diagnosticsBefore/After) con un planHash determinista. «noOpOperations» lista las operaciones que se materializaron pero NO cambian nada (p. ej. un replace_text cuyo «find» no aparece): revísalo para distinguir «se ejecutó y no hizo nada» de «no se pidió». No toca disco (aplicar es change_apply, E13).",
         "inputSchema": { "type": "object", "properties": {
             "expectedWorkspaceRevision": { "type": "string", "description": "Control optimista a nivel de workspace («blake3:…»). Si se omite, se toma la revisión actual; si no coincide → REVISION_CONFLICT." },
             "operations": { "type": "array", "description": "Operaciones propuestas, discriminadas por «op»; las 7 universales (§20.11). Cada entrada lleva «op» más los parámetros de ESA op (declarados abajo, cada uno indica a cuál pertenece), y opcionalmente «expectedRevision» para control optimista por documento.",
                 "items": operacion_item_schema() },
             "selection": { "type": "object", "description": "Selección MASIVA por consulta (§20.11, alternativa a «operations»): «where» (lenguaje textual) o «filter» (JSON), como en knowledge_search. Requiere «operation».", "properties": {
                 "where": { "type": "string" },
                 "filter": { "type": "object" }
             } },
             "operation": { "type": "object", "description": "La operación a expandir sobre cada documento que casa la «selection», con el tipo como CLAVE y sus parámetros como valor (p. ej. {\"patch_frontmatter\": {\"status\": \"review\"}}). Los parámetros son los mismos que en «operations», sin repetir «op» ni el path (lo pone la selección). Solo las que tienen sentido en masa: patch_frontmatter/replace_text/delete.",
                 "properties": {
                     "patch_frontmatter": { "type": "object", "description": "El merge-patch a aplicar a cada documento seleccionado (mismo formato que «patch»)." },
                     "replace_text": { "type": "object", "description": "{find, replace, expectedOccurrences?} como en la op suelta." },
                     "delete": { "type": "object", "description": "{inboundLinksPolicy} como en la op suelta." }
                 } },
             "policy": { "type": "object", "description": "Política de aplicación del plan.", "properties": {
                 "requireValidResult": { "type": "boolean", "default": true, "description": "Si true (por defecto), un resultado NO VÁLIDO bloquea canApply." },
                 "allowWarnings": { "type": "boolean", "default": true, "description": "Si false, cualquier warning bloquea canApply." }
             } }
         }, "additionalProperties": false },
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
    // E29-H08: antes de leer nada, se RECHAZA lo que el `inputSchema` de la tool no declara. Hasta
    // v0.5.0 el schema decía `additionalProperties: false` y el despachador leía campo a campo sin
    // mirar las claves sobrantes, así que un `sort` retirado o un typo como `wheres` se descartaban
    // en silencio y el agente recibía la respuesta por defecto (`decisiones §15`).
    crate::validacion::valida_argumentos(name, params)?;
    match name {
        "workspace_status" => {
            let status = app.workspace_status(profile).map_err(|e| e.to_string())?;
            to_json(&status)
        }
        "knowledge_search" => {
            let text = str_validado(params, "text")?.unwrap_or("");
            // `where`/`filter` (E19-H05): la consulta textual y el filtro JSON estructurado, ambos
            // hacia el mismo `Expression` en la `App`. `where` es palabra reservada en Rust, así que
            // la clave del wire se lee por string, no por campo.
            let where_expr = str_validado(params, "where")?;
            let filter = params.get("filter");
            // Proyección de frontmatter (E23-H11). Se parsea AQUÍ, en el despachador, porque los
            // valores son abiertos (`frontmatter.<lo que sea>`) y no caben en un `enum` del schema
            // —a diferencia del `include` cerrado de `knowledge_get`—: este es el único sitio donde
            // la superficie puede ser honesta. Una entrada mal formada se RECHAZA con
            // `INVALID_SCHEMA`; aceptarla y descartarla sería reintroducir por detrás el defecto
            // que esta misma historia saca por delante con `sort`.
            let include: Vec<String> = match params.get("include") {
                Some(v) => serde_json::from_value(v.clone()).map_err(|e| {
                    format!(
                        "{}: «include» debe ser un array de cadenas «frontmatter.<fieldPath>»: {e}",
                        ErrorCode::InvalidSchema.as_str()
                    )
                })?,
                None => Vec::new(),
            };
            let proyecciones =
                lodestar_app::FrontmatterProjection::parse_all(&include).map_err(|code| {
                    format!(
                        "{}: cada entrada de «include» debe ser «frontmatter.<fieldPath>» (p. ej. \
                         «frontmatter.status» o «frontmatter.owner.name»); recibido {include:?}",
                        code.as_str()
                    )
                })?;
            // `inputSchema` declara `minimum: 1, maximum: 100` (E24-H09).
            let limit = limit_validado(params, 1, 100)?;
            let cursor = str_validado(params, "cursor")?;
            let results = app
                .knowledge_search(text, where_expr, filter, &proyecciones, limit, cursor)
                // E24-H10: el mensaje ABRE con el código estable. `Display` a secas dejaba
                // «entrada inválida: …», legible pero sin nada por lo que un agente pueda ramificar.
                // E26-H07: ese emparejado lo hace ahora `AppError` —en `lodestar-app`, para las diez
                // tools— y su `Display` YA es «CÓDIGO: mensaje», así que aquí no se antepone nada.
                .map_err(|e| e.to_string())?;
            to_json(&results)
        }
        "knowledge_get" => {
            let r: DocumentRef = match params.get("ref") {
                Some(v) => {
                    serde_json::from_value(v.clone()).map_err(|e| forma_invalida("ref", e))?
                }
                None => return Err(falta("ref")),
            };
            // E26-H07: también estos errores de FORMA llevan código; el `Display` de serde a secas
            // dejaba un texto sin nada por lo que ramificar (mismo criterio que `forma_invalida`).
            let include: Vec<String> = match params.get("include") {
                Some(v) => {
                    serde_json::from_value(v.clone()).map_err(|e| forma_invalida("include", e))?
                }
                None => Vec::new(),
            };
            let sections: Option<Vec<Vec<String>>> = match params.get("sections") {
                Some(v) => Some(
                    serde_json::from_value(v.clone()).map_err(|e| forma_invalida("sections", e))?,
                ),
                None => None,
            };
            // Mapeo de error a wire (E10-H02 + E26-H07): el `Display` de `AppError` compone
            // «CÓDIGO: mensaje» con el código estable `ErrorCode::as_str()` (p. ej.
            // «DOCUMENT_NOT_FOUND»), NUNCA el `Debug` de la variante (`DocumentNotFound`) — el
            // catálogo de códigos es el contrato, no el nombre Rust. Hasta v0.4.0 esta línea era
            // `e.as_str().to_string()`: el código PELADO, sin una palabra sobre qué corregir.
            let document = app
                .knowledge_get(&r, &include, sections.as_deref())
                .map_err(|e| e.to_string())?;
            Ok(json!({ "document": to_json(&document)? }))
        }
        "metadata_inspect" => {
            let mode = params
                .get("mode")
                .and_then(Value::as_str)
                .ok_or_else(|| falta("mode"))?;
            let field = params.get("field").and_then(Value::as_str);
            // `inputSchema` declara `minimum: 1, maximum: 1000, default: 100` (E26-H10): esta era la
            // única de las 10 tools sin cota, y sus dos modos devuelven una lista de tamaño
            // proporcional al workspace.
            let limit = limit_validado(params, 1, 1000)?;
            let cursor = str_validado(params, "cursor")?;
            // Mismo mapeo de error a wire que `knowledge_get` (E10-H02, E26-H07): «CÓDIGO:
            // mensaje» con el código estable `ErrorCode::as_str()`, nunca el `Debug` de la variante.
            let inspection = app
                .metadata_inspect(mode, field, limit, cursor)
                .map_err(|e| e.to_string())?;
            to_json(&inspection)
        }
        "knowledge_check" => {
            let scope: CheckScope = match params.get("scope") {
                Some(v) => {
                    serde_json::from_value(v.clone()).map_err(|e| forma_invalida("scope", e))?
                }
                None => return Err(falta("scope")),
            };
            // Wire de severidad mínima → `Severity` (err|warn|info); ausente = sin umbral extra.
            let min_severity = match str_validado(params, "minimumSeverity")? {
                Some("err") => Some(Severity::Err),
                Some("warn") => Some(Severity::Warn),
                Some("info") => Some(Severity::Info),
                Some(other) => {
                    return Err(format!(
                        "{}: «minimumSeverity» debe ser «err», «warn» o «info»; recibido «{other}»",
                        ErrorCode::InvalidSchema.as_str()
                    ));
                }
                None => None,
            };
            let include_fixes = bool_validado(params, "includeSuggestedFixes", false)?;
            // `inputSchema` declara `minimum: 1, maximum: 1000` (E24-H09).
            let limit = limit_validado(params, 1, 1000)?;
            let cursor = str_validado(params, "cursor")?;
            // Mismo mapeo de error a wire que `knowledge_get`/`metadata_inspect` (E10-H02,
            // E26-H07): «CÓDIGO: mensaje», nunca el `Debug` de la variante.
            let report = app
                .knowledge_check(&scope, min_severity, include_fixes, limit, cursor)
                .map_err(|e| e.to_string())?;
            to_json(&report)
        }
        "graph_query" => {
            let operation = params
                .get("operation")
                .and_then(Value::as_str)
                .ok_or_else(|| falta("operation"))?;
            // Presente pero mal formado se juzga AQUÍ (forma); AUSENTE lo juzga `App::graph_query`,
            // que es quien sabe qué operación exige qué extremo (E26-H07).
            let r: Option<DocumentRef> = match params.get("ref") {
                Some(v) => {
                    Some(serde_json::from_value(v.clone()).map_err(|e| forma_invalida("ref", e))?)
                }
                None => None,
            };
            // Segundo extremo, solo para `path_between` (destino del camino dirigido).
            let to: Option<DocumentRef> = match params.get("to") {
                Some(v) => {
                    Some(serde_json::from_value(v.clone()).map_err(|e| forma_invalida("to", e))?)
                }
                None => None,
            };
            // `inputSchema` declara `minimum: 1` (E24-H09).
            let depth = entero_validado(params, "depth", 1)?.map(|n| n as u32);
            let direction = str_validado(params, "direction")?;
            // `inputSchema` declara `minimum: 1, maximum: 1000, default: 100` (E24-H09 + E26-H10:
            // hasta v0.4.0 esta llamada era `limit_validado(params, 1, u64::MAX)` sobre un schema sin
            // máximo, así que un `limit` arbitrario se aceptaba y `limit` ausente servía el grafo
            // entero).
            let limit = limit_validado(params, 1, 1000)?;
            let cursor = str_validado(params, "cursor")?;
            // Mismo mapeo de error a wire que `knowledge_get`/`metadata_inspect`/`knowledge_check`
            // (E10-H02, E26-H07): «CÓDIGO: mensaje», nunca el `Debug` de la variante. Que FALTE
            // «ref»/«to» lo juzga `App::graph_query` como INVALID_SCHEMA (E26-H07): aquí no se
            // pre-valida, para que el mensaje pueda nombrar la operación que lo exige.
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
                .map_err(|e| e.to_string())?;
            to_json(&result)
        }
        "impact_analyze" => {
            let r: DocumentRef = match params.get("ref") {
                Some(v) => {
                    serde_json::from_value(v.clone()).map_err(|e| forma_invalida("ref", e))?
                }
                None => return Err(falta("ref")),
            };
            let kind = params
                .get("proposedOperation")
                .and_then(|op| op.get("kind"))
                .and_then(Value::as_str)
                .ok_or_else(|| falta("proposedOperation.kind"))?;
            // `inputSchema` declara `minimum: 1` (E24-H09).
            let depth = entero_validado(params, "depth", 1)?.map(|n| n as u32);
            // Mismo mapeo de error a wire que las demás tools (E10-H02, E26-H07): «CÓDIGO:
            // mensaje», nunca el `Debug` de la variante.
            let report = app
                .impact_analyze(&r, kind, depth)
                .map_err(|e| e.to_string())?;
            to_json(&report)
        }
        "change_plan" => {
            let expected = params
                .get("expectedWorkspaceRevision")
                .and_then(Value::as_str)
                .map(|s| WorkspaceRevision(s.to_string()));
            // `App::change_plan` acepta dos formas de wire: el array `operations` (ops sueltas) o el
            // objeto `{selection, operation}` de la selección MASIVA por consulta (§20.11, E21-H02).
            // El dispatch pasa la que venga: si hay `selection`, el objeto entero de params (que ya
            // lleva `selection` + `operation`); si no, el array `operations`. (Sin esto, la selección
            // masiva no llegaría a la superficie MCP aunque `App` la sepa interpretar.)
            let raw_ops = if params.get("selection").is_some() {
                params.clone()
            } else {
                params.get("operations").cloned().unwrap_or(Value::Null)
            };
            let policy: PlanPolicy = match params.get("policy") {
                Some(v) => {
                    serde_json::from_value(v.clone()).map_err(|e| forma_invalida("policy", e))?
                }
                None => PlanPolicy::default(),
            };
            // E23-H05: validación de FORMA del enum, igual que las comprobaciones de «falta el
            // parámetro «ref»» de las otras tools. `App` ya rechaza el valor con `INVALID_SCHEMA`,
            // pero ese código pelado no le dice al agente cuáles son las válidas — y `retarget`/
            // `create_stub` estuvieron aceptándose sin ejecutarse desde E12-H06, así que un cliente
            // que las use viene de una versión donde «funcionaban». La lista sale de
            // `InboundLinksPolicy::WIRE_VALUES`, la misma que declara el `inputSchema`.
            if let Some(mala) = politica_de_borrado_invalida(&raw_ops) {
                return Err(format!(
                    "INVALID_SCHEMA: «{mala}» no es una política válida ante enlaces entrantes; \
                     usa una de {:?}. «retarget» y «create_stub» se retiraron en E23-H05: se \
                     aceptaban sin ejecutarse, dejando enlaces rotos.",
                    InboundLinksPolicy::WIRE_VALUES
                ));
            }
            // Mismo mapeo de error a wire que las demás tools (E10-H02, E26-H07): «CÓDIGO:
            // mensaje» (p. ej. «REVISION_CONFLICT: …»), nunca el `Debug` de la variante. El
            // diagnóstico del parser de un `selection.where` malformado viaja ENTERO en ese mensaje.
            let result = app
                .change_plan(expected, &raw_ops, policy)
                .map_err(|e| e.to_string())?;
            to_json(&result)
        }
        "change_apply" => {
            let change_set_id = ChangeSetId(
                params
                    .get("changeSetId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| falta("changeSetId"))?
                    .to_string(),
            );
            let expected = params
                .get("expectedWorkspaceRevision")
                .and_then(Value::as_str)
                .map(|s| WorkspaceRevision(s.to_string()));
            // Mismo mapeo de error a wire que las demás tools (E10-H02, E26-H07): «CÓDIGO:
            // mensaje» (p. ej. «PLAN_STALE: …»/«PERMISSION_DENIED: …»), nunca el `Debug`.
            let result = app
                .change_apply(&change_set_id, expected)
                .map_err(|e| e.to_string())?;
            to_json(&result)
        }
        "change_revert" => {
            let receipt_id = ReceiptId(
                params
                    .get("receiptId")
                    .and_then(Value::as_str)
                    .ok_or_else(|| falta("receiptId"))?
                    .to_string(),
            );
            let expected = params
                .get("expectedWorkspaceRevision")
                .and_then(Value::as_str)
                .map(|s| WorkspaceRevision(s.to_string()));
            // Mismo mapeo de error a wire que las demás tools (E10-H02, E26-H07): «CÓDIGO:
            // mensaje» (p. ej. «WRITE_CONFLICT: …»/«PLAN_EXPIRED: …»), nunca el `Debug`.
            let result = app
                .change_revert(&receipt_id, expected)
                .map_err(|e| e.to_string())?;
            to_json(&result)
        }
        other => Err(format!("tool desconocida: {other}")),
    }
}

fn to_json<T: serde::Serialize>(v: &T) -> ToolResult {
    // E26-H07: hasta el fallo improbable (una respuesta del motor que no serializa) abre con un
    // código del catálogo, para que «CÓDIGO: mensaje» sea invariante de TODA salida de error.
    serde_json::to_value(v).map_err(|e| {
        format!(
            "{}: la respuesta no se pudo serializar a JSON: {e}",
            ErrorCode::InternalIoError.as_str()
        )
    })
}

// ---------------------------------------------------------------------------
// Extracción VALIDANTE de parámetros (E24-H09)
//
// `contracts/mcp.yml` («regla de la casa») dice que el servidor **valida los VALORES de los
// parámetros que declara**. No lo hacía: cada brazo leía con
// `params.get("x").and_then(Value::as_u64).unwrap_or(default)`, así que un `limit: "10"` (tipo
// equivocado), un `limit: 0` o un `limit: 9999` (fuera del `minimum`/`maximum` declarados) caían al
// default **en silencio**. El peor caso es `limit: 0`: devolvía 0 resultados, indistinguible de
// «no hay nada».
//
// E24-H09 no cambió la política sobre parámetros **no declarados** (entonces se seguían ignorando).
// **E29-H08 sí**: `decisiones §15` decidió ejecutar lo que el schema declara, así que hoy una clave
// no declarada se RECHAZA en `validacion::valida_argumentos`, antes de que ningún brazo lea nada.
// Estos helpers siguen ocupándose solo de los VALORES de las claves que sí existen.
// ---------------------------------------------------------------------------

/// `limit` validado contra el rango que declara el `inputSchema` de la tool.
///
/// `None` si el parámetro no viene (el llamante aplica su default). `Err(INVALID_SCHEMA)` si viene
/// con un tipo que no es entero o con un valor fuera de `[min, max]`.
fn limit_validado(params: &Value, min: u64, max: u64) -> Result<Option<usize>, String> {
    let Some(v) = params.get("limit") else {
        return Ok(None);
    };
    let n = v
        .as_u64()
        .filter(|n| (min..=max).contains(n))
        .ok_or_else(|| {
            format!(
                "{}: «limit» debe ser un entero entre {min} y {max}; recibido {v}",
                ErrorCode::InvalidSchema.as_str()
            )
        })?;
    Ok(Some(n as usize))
}

/// Entero positivo validado (`depth`), con mínimo declarado y sin máximo.
fn entero_validado(params: &Value, nombre: &str, min: u64) -> Result<Option<u64>, String> {
    let Some(v) = params.get(nombre) else {
        return Ok(None);
    };
    let n = v.as_u64().filter(|n| *n >= min).ok_or_else(|| {
        format!(
            "{}: «{nombre}» debe ser un entero mayor o igual que {min}; recibido {v}",
            ErrorCode::InvalidSchema.as_str()
        )
    })?;
    Ok(Some(n))
}

/// Booleano validado: un `"true"` (string) deja de tratarse como `false`.
fn bool_validado(params: &Value, nombre: &str, default: bool) -> Result<bool, String> {
    match params.get(nombre) {
        None => Ok(default),
        Some(v) => v.as_bool().ok_or_else(|| {
            format!(
                "{}: «{nombre}» debe ser un booleano; recibido {v}",
                ErrorCode::InvalidSchema.as_str()
            )
        }),
    }
}

/// Error de un parámetro cuya FORMA no encaja, con código del catálogo (E24-H10).
///
/// Los `serde_json::from_value` de los brazos producen mensajes internos
/// (`"unknown variant `nope`, expected one of …"`): útiles, pero sin código estable por el que un
/// agente pueda ramificar. Se conserva el texto de serde —dice qué valores se esperaban— y se le
/// antepone el código.
fn forma_invalida(nombre: &str, e: impl std::fmt::Display) -> String {
    format!(
        "{}: «{nombre}» no tiene la forma esperada: {e}",
        ErrorCode::InvalidSchema.as_str()
    )
}

/// Error de parámetro obligatorio ausente, **con código del catálogo** (E24-H10).
///
/// Hasta v0.3.0 estos errores viajaban como texto suelto (`"falta el parámetro «ref»"`): 10 de los
/// 21 errores de superficie no llevaban ningún código, así que un agente no podía ramificar por
/// ellos sin parsear prosa.
fn falta(nombre: &str) -> String {
    format!(
        "{}: falta el parámetro obligatorio «{nombre}»",
        ErrorCode::InvalidSchema.as_str()
    )
}

/// Cadena validada: un `text: 42` deja de tratarse como cadena vacía (o sea, «todos los
/// documentos»), que es una respuesta silenciosamente equivocada.
fn str_validado<'a>(params: &'a Value, nombre: &str) -> Result<Option<&'a str>, String> {
    match params.get(nombre) {
        None | Some(Value::Null) => Ok(None),
        Some(v) => v.as_str().map(Some).ok_or_else(|| {
            format!(
                "{}: «{nombre}» debe ser una cadena; recibido {v}",
                ErrorCode::InvalidSchema.as_str()
            )
        }),
    }
}

#[cfg(test)]
mod tests {
    //! Golden cross-fachada (E7-H06): la salida de cada tool == la del `Workspace` directo.
    //! Verifica que la fachada MCP es un shell fino sin lógica de dominio propia (`§2`, `§7`).
    use super::*;

    /// Como antes (`Workspace` sobre un fixture en disco), pero envuelto en `App` — `call()`
    /// despacha sobre `App` desde E10-H08 (necesita `App::workspace_status`). Las comparaciones
    /// «directas» del golden test siguen yendo contra el mismo `Workspace`, vía `App::workspace()`.
    ///
    /// Se abre con `Workspace::open`: E23-H12 retiró `open_ephemeral` porque, con los efectos
    /// secundarios fuera de la apertura, `open` **ya es** hermético (no toca el `.gitignore` del
    /// fixture ni le monta scaffold de runtime).
    fn app_with_fixture() -> (tempfile::TempDir, App) {
        let dir = tempfile::tempdir().unwrap();
        for (p, c) in [
            (
                "index.md",
                "---\ntype: Index\ntitle: Bundle\ndescription: Índice del bundle\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n* [Alfa](alfa.md)\n",
            ),
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
        let ws = Workspace::open(dir.path()).unwrap();
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
    fn tools_list_lleva_output_schema_de_tipo_object() {
        // El spec MCP exige que todo `outputSchema` sea un JSON Schema de tipo `object`. Una sola
        // tool inválida hace que un cliente estricto (Claude Code) rechace la lista ENTERA: no
        // degrada esa tool, tumba las diez. `metadata_inspect` lo incumplía por ser el único tipo
        // de salida de la superficie que es un `enum` `untagged` — schemars deriva `anyOf` en la
        // raíz y NO infiere `type`, así que hay que fijarlo (`schemas::metadata_inspect_schema`).
        let tools = list();
        for t in tools.as_array().unwrap() {
            assert_eq!(
                t["outputSchema"]["type"], "object",
                "el `outputSchema` de «{}» debe declarar `type: \"object\"` en la raíz",
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
