//! Política única de protocolo MCP de la fachada.
//!
//! Las fechas son datos de wire y viven únicamente aquí. El resto de la fachada resuelve una
//! versión mediante [`resolve`] y consume la era resultante; no compara fechas ni mantiene listas
//! de fallback propias.

/// Era MCP soportada por la fachada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Era {
    Modern,
    Legacy,
}

/// Error explícito para cualquier fecha que no sea una de las dos eras congeladas.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnsupportedVersion;

/// Forma de las requests y respuestas de una era MCP.
///
/// Este tipo mantiene juntas las decisiones que cambian entre eras para que los transportes y
/// handlers posteriores no tengan que volver a interpretar fechas ni mantener tablas paralelas.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Policy {
    pub lifecycle: &'static str,
    pub request_metadata: &'static str,
    pub result_type: Option<String>,
    pub cache_ttl_ms: Option<u64>,
    pub cache_scope: Option<&'static str>,
    pub initialize: bool,
    pub ping: bool,
}

impl Policy {
    /// Indica si la era exige el lifecycle stateless de Modern.
    #[allow(dead_code)]
    pub fn is_stateless(&self) -> bool {
        self.lifecycle == "stateless"
    }

    /// Indica si cada request de la era debe llevar metadata MCP válida.
    #[allow(dead_code)]
    pub fn requires_request_metadata(&self) -> bool {
        self.request_metadata == "required"
    }

    /// Convierte el discriminador de resultado de la política al tipo oficial de rmcp.
    #[allow(dead_code)]
    pub fn result_type_wire(&self) -> Option<rmcp::model::ResultType> {
        match self.result_type.as_deref() {
            Some("complete") => Some(rmcp::model::ResultType::COMPLETE),
            None => None,
            Some(other) => panic!("resultType de política no soportado: {other}"),
        }
    }

    /// Convierte el scope de cache de la política al tipo oficial de rmcp.
    #[allow(dead_code)]
    pub fn cache_scope_wire(&self) -> Option<rmcp::model::CacheScope> {
        match self.cache_scope {
            Some("private") => Some(rmcp::model::CacheScope::Private),
            None => None,
            Some(other) => panic!("cacheScope de política no soportado: {other}"),
        }
    }
}

/// La versión más reciente es una selección explícita, no un fallback implícito.
pub const LATEST: &str = MODERN;

const MODERN: &str = "2026-07-28";
const LEGACY: &str = "2025-11-25";
const SUPPORTED: [&str; 2] = [MODERN, LEGACY];

/// Resuelve exclusivamente las dos versiones ratificadas; fechas futuras o intermedias fallan.
pub fn resolve(version: &str) -> Result<Era, UnsupportedVersion> {
    match version {
        LATEST => Ok(Era::Modern),
        LEGACY => Ok(Era::Legacy),
        _ => Err(UnsupportedVersion),
    }
}

/// Devuelve la política completa de una era, sin fallback entre eras.
pub fn policy_for(era: Era) -> Policy {
    match era {
        Era::Modern => Policy {
            lifecycle: "stateless",
            request_metadata: "required",
            result_type: Some(String::from("complete")),
            cache_ttl_ms: Some(0),
            cache_scope: Some("private"),
            initialize: false,
            ping: false,
        },
        Era::Legacy => Policy {
            lifecycle: "initialize",
            request_metadata: "absent",
            result_type: None,
            cache_ttl_ms: None,
            cache_scope: None,
            initialize: true,
            ping: true,
        },
    }
}

/// Devuelve la política de la versión tipada de una request, si pertenece a una era ratificada.
pub fn policy_for_protocol(version: Option<&rmcp::model::ProtocolVersion>) -> Option<Policy> {
    version.and_then(|version| era_for_protocol(version).map(policy_for))
}

/// Negocia el lifecycle `initialize`, que siempre pertenece a la era Legacy.
///
/// El texto ofrecido por un cliente se clasifica con el resolvedor cerrado para mantener una
/// única política de eras, pero su resultado no cambia esta negociación: Legacy acepta cualquier
/// oferta textual y responde siempre su baseline. La resolución moderna sigue rechazando fechas
/// ajenas cuando la use el transporte stateless en las historias posteriores.
pub fn initialize_version(offered: Option<&str>) -> &'static str {
    match offered.and_then(|version| resolve(version).ok()) {
        Some(Era::Modern) | Some(Era::Legacy) | None => legacy_version(),
    }
}

/// Devuelve la lista cerrada para mensajes de error y discovery, sin fallback adicional.
pub const fn supported_versions() -> &'static [&'static str; 2] {
    &SUPPORTED
}

/// Baseline única que selecciona `initialize`, con independencia de la revisión solicitada.
pub const fn legacy_version() -> &'static str {
    LEGACY
}

/// Versión Modern en la representación tipada que consume rmcp.
#[allow(dead_code)]
pub fn modern_protocol_version() -> rmcp::model::ProtocolVersion {
    rmcp::model::ProtocolVersion::V_2026_07_28
}

/// Clasifica una versión tipada sin repetir fechas fuera de esta política.
#[allow(dead_code)]
pub fn era_for_protocol(version: &rmcp::model::ProtocolVersion) -> Option<Era> {
    if version == &modern_protocol_version() {
        Some(Era::Modern)
    } else if version == &legacy_protocol_version() {
        Some(Era::Legacy)
    } else {
        None
    }
}

/// Indica si una request pertenece a la era Modern.
#[allow(dead_code)]
pub fn is_modern(version: Option<&rmcp::model::ProtocolVersion>) -> bool {
    policy_for_protocol(version).is_some_and(|policy| policy.is_stateless())
}

/// Versión Legacy en la representación tipada que consume rmcp.
///
/// La fachada no toma la lista de versiones que conoce el SDK: el handler solo anuncia esta
/// versión, y la negociación de `initialize` siempre vuelve a ella.
#[allow(dead_code)]
pub fn legacy_protocol_version() -> rmcp::model::ProtocolVersion {
    rmcp::model::ProtocolVersion::V_2025_11_25
}
