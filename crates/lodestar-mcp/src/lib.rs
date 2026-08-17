//! Servicio neutral MCP: semántica compartida por cualquier transporte.
//!
//! Este crate separa deliberadamente el servicio del bucle de stdio. El transporte sólo traduce
//! frames; el catálogo, la validación y el dispatcher viven aquí, junto al [`App`] que poseen.

pub mod protocol_policy;
mod tools;
mod validacion;

use std::{borrow::Cow, future::Future, sync::Arc};

use lodestar_app::{App, Profile};
use rmcp::{
    handler::server::ServerHandler,
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ClientRequest,
        CompleteRequestMethod, CompleteRequestParams, CompleteResult, DiscoverRequestMethod,
        DiscoverResult, ErrorData, Implementation, InitializeRequestParams, InitializeResult,
        InitializeResultMethod, ListPromptsRequestMethod, ListPromptsResult,
        ListResourceTemplatesRequestMethod, ListResourceTemplatesResult,
        ListResourcesRequestMethod, ListResourcesResult, ListToolsResult, PaginatedRequestParams,
        ProtocolVersion, ServerCapabilities, ServerInfo, Tool,
    },
    service::{MaybeSendFuture, NotificationContext, RequestContext, Service, ServiceRole},
    RoleServer,
};
use serde_json::{json, Value};
use tokio::sync::Mutex;

/// Servicio MCP independiente del transporte.
///
/// Posee una instancia de [`App`] y el perfil efectivo de la sesión. Todos los transportes deben
/// consultar este objeto para discovery, catálogo, llamadas y ping; no existe un registro paralelo
/// por transporte.
pub struct LodestarMcpService {
    app: App,
    profile: Profile,
}

/// Ejecutor único para el estado mutable de una sesión MCP.
///
/// El `Mutex` cubre toda la llamada al servicio, incluidos los efectos de una tool de cambio. De
/// este modo rmcp puede recibir trabajo concurrente sin crear un segundo escritor ni permitir que
/// dos operaciones observen y publiquen sobre revisiones distintas a la vez.
pub struct SerialExecutor<T> {
    state: Arc<Mutex<T>>,
}

impl<T> Clone for SerialExecutor<T> {
    fn clone(&self) -> Self {
        Self {
            state: Arc::clone(&self.state),
        }
    }
}

impl<T> SerialExecutor<T> {
    /// Crea un executor que serializa el valor compartido.
    pub fn new(value: T) -> Self {
        Self {
            state: Arc::new(Mutex::new(value)),
        }
    }

    /// Ejecuta una operación mientras posee el lock exclusivo del servicio.
    pub async fn run<R, F>(&self, operation: F) -> R
    where
        F: FnOnce(&mut T) -> R,
        R: Send,
        T: Send,
    {
        let mut state = self.state.lock().await;
        operation(&mut *state)
    }
}

impl SerialExecutor<LodestarMcpService> {
    /// Despacha una tool mediante el servicio neutral, conservando su envelope JSON.
    pub async fn call(&self, name: &str, args: Value) -> Result<Value, String> {
        let name = name.to_owned();
        self.run(move |service| service.call(&name, &args)).await
    }
}

impl LodestarMcpService {
    /// Crea un servicio para una sesión y un perfil concretos.
    pub fn new(app: App, profile: Profile) -> Self {
        Self { app, profile }
    }

    /// Devuelve el perfil configurado para la sesión.
    pub fn profile(&self) -> Profile {
        self.profile
    }

    /// Discovery estructurado del servicio neutral.
    pub fn discover(&self) -> Value {
        json!({
            "capabilities": { "tools": {} },
            "tools": self.list()["tools"],
            "instructions": self.instructions(),
        })
    }

    /// Catálogo estructurado filtrado por el perfil activo.
    pub fn list(&self) -> Value {
        json!({ "tools": tools::available_tools(self.profile) })
    }

    /// Ping neutral, sin tocar el workspace ni ejecutar una tool.
    pub fn ping(&self) -> Value {
        json!({})
    }

    /// Ejecuta una tool y conserva el envelope MCP de éxito o error de ejecución.
    ///
    /// Un nombre desconocido (o no disponible en `readonly`) es un error de protocolo y se
    /// devuelve como `Err`; los errores de validación/negocio de una tool permanecen en `result`
    /// con `isError`, igual que en la fachada stdio.
    pub fn call(&mut self, name: &str, params: &Value) -> Result<Value, String> {
        if !tools::available(self.profile, name) {
            return Err(format!("tool desconocida: {name}"));
        }
        match tools::call(&self.app, self.profile, name, params) {
            Ok(value) => Ok(json!({
            "content": [{ "type": "text", "text": value.to_string() }],
            "structuredContent": value,
            })),
            Err(error) => Ok(json!({
                "content": [{ "type": "text", "text": error }],
                "isError": true,
            })),
        }
    }

    /// Instrucciones de sesión, generadas a partir del mismo predicado que `list`.
    pub fn instructions(&self) -> String {
        server_instructions(self.profile)
    }
}

fn server_info(service: &LodestarMcpService) -> ServerInfo {
    InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
        .with_server_info(Implementation::new(
            "lodestar-mcp",
            env!("CARGO_PKG_VERSION"),
        ))
        .with_instructions(service.instructions())
}

fn rmcp_tools(value: Value) -> Result<Vec<Tool>, ErrorData> {
    let tools = value
        .get("tools")
        .cloned()
        .ok_or_else(|| ErrorData::internal_error("tools/list no devolvió catálogo", None))?;
    serde_json::from_value(tools)
        .map_err(|error| ErrorData::internal_error(format!("schema MCP inválido: {error}"), None))
}

fn rmcp_call_result(value: Value) -> Result<CallToolResult, ErrorData> {
    serde_json::from_value(value)
        .map_err(|error| ErrorData::internal_error(format!("envelope MCP inválido: {error}"), None))
}

fn validate_modern_request_metadata(context: &RequestContext<RoleServer>) -> Result<(), ErrorData> {
    let Some(policy) = protocol_policy::policy_for_protocol(context.protocol_version().as_ref())
    else {
        return Ok(());
    };
    if !policy.requires_request_metadata() {
        return Ok(());
    }

    let Some(client_info) = context.meta.client_info() else {
        return Err(ErrorData::invalid_params(
            "request _meta requiere clientInfo con name y version no vacíos",
            None,
        ));
    };
    if client_info.name.trim().is_empty() || client_info.version.trim().is_empty() {
        return Err(ErrorData::invalid_params(
            "request _meta requiere clientInfo con name y version no vacíos",
            None,
        ));
    }
    Ok(())
}

impl ServerHandler for SerialExecutor<LodestarMcpService> {
    #[allow(clippy::manual_async_fn)]
    fn discover(
        &self,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<DiscoverResult, ErrorData>> + MaybeSendFuture + '_ {
        async move {
            let Some(policy) =
                protocol_policy::policy_for_protocol(_context.protocol_version().as_ref())
            else {
                return Err(ErrorData::method_not_found::<DiscoverRequestMethod>());
            };
            if !policy.is_stateless() {
                return Err(ErrorData::method_not_found::<DiscoverRequestMethod>());
            }
            validate_modern_request_metadata(&_context)?;
            let info = self.run(|service| server_info(service)).await;
            let mut result = DiscoverResult::from_server_info(
                vec![protocol_policy::modern_protocol_version()],
                info,
            );
            if let Some(result_type) = policy.result_type_wire() {
                result.result_type = result_type;
            }
            if let Some(ttl_ms) = policy.cache_ttl_ms {
                result = result.with_ttl_ms(ttl_ms);
            }
            if let Some(cache_scope) = policy.cache_scope_wire() {
                result = result.with_cache_scope(cache_scope);
            }
            Ok(result)
        }
    }

    fn complete(
        &self,
        _request: CompleteRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CompleteResult, ErrorData>> + MaybeSendFuture + '_ {
        std::future::ready(Err(ErrorData::method_not_found::<CompleteRequestMethod>()))
    }

    fn list_prompts(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListPromptsResult, ErrorData>> + MaybeSendFuture + '_ {
        std::future::ready(Err(
            ErrorData::method_not_found::<ListPromptsRequestMethod>(),
        ))
    }

    fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourcesResult, ErrorData>> + MaybeSendFuture + '_ {
        std::future::ready(Err(
            ErrorData::method_not_found::<ListResourcesRequestMethod>(),
        ))
    }

    fn list_resource_templates(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<ListResourceTemplatesResult, ErrorData>> + MaybeSendFuture + '_
    {
        std::future::ready(Err(ErrorData::method_not_found::<
            ListResourceTemplatesRequestMethod,
        >()))
    }

    fn initialize(
        &self,
        request: InitializeRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<InitializeResult, ErrorData>> + MaybeSendFuture + '_ {
        let metadata_version = context.meta.protocol_version();
        if protocol_policy::policy_for_protocol(metadata_version.as_ref())
            .is_some_and(|policy| !policy.initialize)
        {
            return std::future::ready(
                Err(ErrorData::method_not_found::<InitializeResultMethod>()),
            );
        }
        context.peer.set_peer_info(request);
        let mut info = self
            .state
            .try_lock()
            .map(|service| server_info(&service))
            .unwrap_or_else(|_| {
                InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
                    .with_server_info(Implementation::new(
                        "lodestar-mcp",
                        env!("CARGO_PKG_VERSION"),
                    ))
            });
        info.protocol_version = protocol_policy::legacy_protocol_version();
        std::future::ready(Ok(info))
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(vec![protocol_policy::modern_protocol_version()])
    }

    async fn list_tools(
        &self,
        _request: Option<PaginatedRequestParams>,
        context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, ErrorData> {
        validate_modern_request_metadata(&context)?;
        let catalog = self.run(|service| service.list()).await;
        let mut result = ListToolsResult::with_all_items(rmcp_tools(catalog)?);
        if let Some(policy) =
            protocol_policy::policy_for_protocol(context.protocol_version().as_ref())
        {
            if let Some(result_type) = policy.result_type_wire() {
                result.result_type = Some(result_type);
            }
            if let Some(ttl_ms) = policy.cache_ttl_ms {
                result = result.with_ttl_ms(ttl_ms);
            }
            if let Some(cache_scope) = policy.cache_scope_wire() {
                result = result.with_cache_scope(cache_scope);
            }
        }
        Ok(result)
    }

    fn call_tool(
        &self,
        request: CallToolRequestParams,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<CallToolResponse, ErrorData>> + MaybeSendFuture + '_ {
        let name = request.name.into_owned();
        let args = request.arguments.map(Value::Object).unwrap_or(Value::Null);
        async move {
            validate_modern_request_metadata(&context)?;
            let result = self
                .call(&name, args)
                .await
                .map_err(|error| ErrorData::invalid_params(error, None))?;
            let mut result = rmcp_call_result(result)?;
            if let Some(policy) =
                protocol_policy::policy_for_protocol(context.protocol_version().as_ref())
            {
                result.result_type = policy.result_type_wire();
            }
            Ok(CallToolResponse::Complete(result))
        }
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.state
            .try_lock()
            .ok()
            .and_then(|service| rmcp_tools(service.list()).ok())
            .and_then(|tools| tools.into_iter().find(|tool| tool.name == name))
    }

    fn get_info(&self) -> ServerInfo {
        self.state
            .try_lock()
            .map(|service| server_info(&service))
            .unwrap_or_else(|_| {
                InitializeResult::new(ServerCapabilities::builder().enable_tools().build())
                    .with_server_info(Implementation::new(
                        "lodestar-mcp",
                        env!("CARGO_PKG_VERSION"),
                    ))
            })
    }
}

/// Adaptador fino de lifecycle: el handler real conserva la semántica y anuncia Modern para las
/// requests stateless, mientras el transporte negocia `initialize` exclusivamente como Legacy.
#[derive(Clone)]
pub struct LodestarMcpServer {
    inner: SerialExecutor<LodestarMcpService>,
    /// rmcp spawns one task per inbound request. This shared gate serializes entry into the
    /// service, while JSON-RPC continues to correlate responses by id rather than wire order.
    /// Cancellation simply drops a waiter and does not strand later requests behind it.
    fifo: Arc<Mutex<()>>,
}

impl LodestarMcpServer {
    pub fn new(inner: SerialExecutor<LodestarMcpService>) -> Self {
        Self {
            inner,
            fifo: Arc::new(Mutex::new(())),
        }
    }
}

impl Service<RoleServer> for LodestarMcpServer {
    fn handle_request(
        &self,
        request: <RoleServer as ServiceRole>::PeerReq,
        context: RequestContext<RoleServer>,
    ) -> impl Future<Output = Result<<RoleServer as ServiceRole>::Resp, rmcp::ErrorData>>
           + MaybeSendFuture
           + '_ {
        let fifo = Arc::clone(&self.fifo);
        async move {
            // A cancelled request may still be queued behind another request. Observe its
            // request token while waiting for the serial turn, and check it once more after the
            // lock wins the race. Once admitted, do not select on cancellation: the App-owned
            // executor must finish a transaction indivisibly.
            let _turn = tokio::select! {
                turn = fifo.lock() => turn,
                _ = context.ct.cancelled() => {
                    return Err(rmcp::ErrorData::internal_error(
                        "request cancelled before serialized execution",
                        None,
                    ));
                }
            };
            if context.ct.is_cancelled() {
                return Err(rmcp::ErrorData::internal_error(
                    "request cancelled before serialized execution",
                    None,
                ));
            }
            // rmcp falls back to `CustomRequest` when a known method has an unsupported
            // parameter shape (for example MRTR's `inputResponses`). Keep that malformed
            // `tools/call` on its protocol error surface instead of exposing it as an unknown
            // method; the real dispatcher is never reached.
            if let ClientRequest::CustomRequest(custom) = &request {
                if custom.method == "tools/call" {
                    return Err(rmcp::ErrorData::invalid_params(
                        "tools/call contiene parámetros no soportados",
                        None,
                    ));
                }
            }
            // rmcp's generic handler treats every `server/discover` as an inline Modern
            // opener and validates its metadata before invoking `ServerHandler::discover`.
            // A session that already negotiated Legacy must instead expose the method as
            // unavailable, without that Modern validation or a DiscoverResult response.
            if matches!(&request, ClientRequest::DiscoverRequest(_))
                && context.peer.peer_info().is_some_and(|info| {
                    info.protocol_version == protocol_policy::legacy_protocol_version()
                })
            {
                return Err(rmcp::ErrorData::method_not_found::<DiscoverRequestMethod>());
            }
            <SerialExecutor<LodestarMcpService> as Service<RoleServer>>::handle_request(
                &self.inner,
                request,
                context,
            )
            .await
        }
    }

    fn handle_notification(
        &self,
        notification: <RoleServer as ServiceRole>::PeerNot,
        context: NotificationContext<RoleServer>,
    ) -> impl Future<Output = Result<(), rmcp::ErrorData>> + MaybeSendFuture + '_ {
        let fifo = Arc::clone(&self.fifo);
        async move {
            let _turn = fifo.lock().await;
            <SerialExecutor<LodestarMcpService> as Service<RoleServer>>::handle_notification(
                &self.inner,
                notification,
                context,
            )
            .await
        }
    }

    fn get_info(&self) -> <RoleServer as ServiceRole>::Info {
        <SerialExecutor<LodestarMcpService> as Service<RoleServer>>::get_info(&self.inner)
    }

    fn supported_protocol_versions(&self) -> Cow<'static, [ProtocolVersion]> {
        Cow::Owned(vec![protocol_policy::legacy_protocol_version()])
    }
}

const SERVER_INSTRUCTIONS_PREAMBULO: &str = "\
Motor headless de integridad semántica para agentes. Opera sobre la red de documentos Markdown de \
un proyecto cualquiera: no exige estructura previa, ningún nombre de fichero activa reglas \
especiales, el frontmatter es YAML arbitrario tuyo y todas las rutas son relativas a la raíz.";

struct Paso {
    tool: &'static str,
    texto: &'static str,
}

const PASOS: [Paso; 10] = [
    Paso {
        tool: "workspace_status",
        texto: "oriéntate primero — config activa, capacidades del perfil, validez y recuento agregado del workspace, recuperación pendiente y los recibos disponibles para revertir.",
    },
    Paso {
        tool: "knowledge_search",
        texto: "localiza documentos por texto libre y por consulta tipada (`where`/`filter`); con `include: [\"frontmatter.<campo>\"]` proyectas metadata de cada resultado sin pedir el documento entero. Devuelve snippets y revisión, nunca cuerpos completos.",
    },
    Paso {
        tool: "knowledge_get",
        texto: "lee un documento concreto con `include` selectivo y secciones acotadas por `headingPath`.",
    },
    Paso {
        tool: "metadata_inspect",
        texto: "descubre las convenciones de metadata de la base (qué campos existen, de qué tipos y qué valores toman) sin necesitar un schema, antes de proponer cambios.",
    },
    Paso {
        tool: "graph_query",
        texto: "consulta el grafo de enlaces — operaciones `backlinks`, `outgoing`, `neighborhood`, `isolated`, `dangling`, `path_between`, `cycles`, `components`.",
    },
    Paso {
        tool: "impact_analyze",
        texto: "evalúa el impacto de un cambio hipotético (afectados directos y transitivos, riesgo) antes de proponerlo.",
    },
    Paso {
        tool: "change_plan",
        texto: "planifica el cambio SIN escribir — normaliza las operaciones, simula en memoria y valida el resultado; devuelve un change set con su hash determinista.",
    },
    Paso {
        tool: "change_apply",
        texto: "aplica el plan calculado con todas las salvaguardas transaccionales; devuelve el recibo.",
    },
    Paso {
        tool: "knowledge_check",
        texto: "audita el conocimiento tras aplicar para confirmar que sigue siendo interpretable y que sus enlaces siguen resolviendo.",
    },
    Paso {
        tool: "change_revert",
        texto: "si algo salió mal, revierte al estado anterior la transacción del `receiptId` que te dio `change_apply` (o el que listó `workspace_status`).",
    },
];

fn server_instructions(profile: Profile) -> String {
    let mut out = String::from(SERVER_INSTRUCTIONS_PREAMBULO);
    out.push_str("\n\nFlujo recomendado en esta sesión, en orden:\n\n");
    let mut n = 0usize;
    for paso in &PASOS {
        if !tools::available(profile, paso.tool) {
            continue;
        }
        n += 1;
        out.push_str(&format!("{n}. `{}`: {}\n", paso.tool, paso.texto));
    }
    out.push('\n');
    out.push_str(if profile.writes_enabled() {
        "Perfil `standard` (por defecto): el flujo completo."
    } else {
        "Perfil `readonly`: solo los pasos de lectura y verificación (las tools de cambio no están disponibles)."
    });
    out
}
