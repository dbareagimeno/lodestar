//! `lodestar-app` — servicios de caso de uso finos sobre `lodestar-workspace`.
//!
//! Capa compartida por las dos fachadas de superficie (`lodestar-mcp`, `lodestar-cli`): compone
//! el `Envelope<T>` de protocolo (framing, no dominio — decisión **D3**, `docs/REFACTOR_DISENO_PROPUESTA.md`)
//! y la fachada `App`, que envuelve un [`lodestar_workspace::Workspace`] y expone los métodos de
//! caso de uso (`workspace_status` desde E10-H08; `knowledge_search`, … se irán poblando en
//! E10-H09+).
//!
//! Este crate depende solo de `lodestar-core` + `lodestar-workspace` + `serde`/`serde_json` — nunca
//! directamente de `rusqlite`/`git2`/`tokio` (invariante #2 de `CLAUDE.md`, verificado por
//! `cargo tree -p lodestar-app`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use lodestar_core::types::{
    workspace_revision, Check, ConceptRef, ErrorCode, RelPath, WorkspaceRevision,
};
use lodestar_core::CoreError;
use lodestar_workspace::{Workspace, WorkspaceConfig, WorkspaceError, WorkspaceSchema};

/// Envelope común de protocolo (`ARCHITECTURE.md §19.6`, `docs/REFACTOR.md §13`, decisión **D3**).
///
/// Todas las respuestas de las tools MCP y de los comandos de la CLI se enmarcan en esta forma:
/// un veredicto (`ok`), la revisión del workspace en el momento de la respuesta, un resumen
/// compacto pensado para el modelo (`summary`), el payload específico de la operación (`data`) y
/// tres colecciones auxiliares siempre presentes (`diagnostics`/`warnings`/`resource_links`), nunca
/// omitidas aunque estén vacías. Wire en camelCase: `workspaceRevision`/`resourceLinks`.
///
/// Framing de transporte, no dominio — por eso vive aquí y no en `lodestar_core::types` (los
/// campos `data`/`diagnostics` sí reusan tipos del core: `Check` y lo que cada servicio devuelva).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Envelope<T> {
    /// `true` si la operación tuvo éxito (con o sin advertencias); `false` si fue rechazada.
    pub ok: bool,
    /// Revisión determinista del workspace en el momento de responder (ver E10-H03).
    pub workspace_revision: WorkspaceRevision,
    /// Resumen compacto en lenguaje natural, pensado para que un agente lo lea sin parsear `data`.
    pub summary: String,
    /// Payload específico de la operación.
    pub data: T,
    /// Diagnósticos de conformidad relevantes para esta respuesta (puede estar vacío).
    pub diagnostics: Vec<Check>,
    /// Avisos no bloqueantes (puede estar vacío).
    pub warnings: Vec<String>,
    /// Enlaces a recursos adicionales que el agente puede querer inspeccionar (puede estar vacío).
    pub resource_links: Vec<ResourceLink>,
}

impl<T> Envelope<T> {
    /// Construye un envelope de éxito con las colecciones auxiliares vacías — el caso común para
    /// un servicio que no tiene diagnósticos/avisos/enlaces que añadir.
    pub fn ok(workspace_revision: WorkspaceRevision, summary: impl Into<String>, data: T) -> Self {
        Envelope {
            ok: true,
            workspace_revision,
            summary: summary.into(),
            data,
            diagnostics: Vec::new(),
            warnings: Vec::new(),
            resource_links: Vec::new(),
        }
    }
}

/// Enlace a un recurso adicional referenciado desde una respuesta (`resourceLinks` del envelope,
/// `docs/REFACTOR.md §13`), p. ej. un concepto relacionado que el agente puede pedir con
/// `knowledge_get` a continuación. Forma mínima: URI del recurso (dirección estable, no
/// necesariamente un `RelPath` — puede referirse a recursos fuera del bundle) y un título
/// legible opcional.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResourceLink {
    /// Dirección estable del recurso.
    pub uri: String,
    /// Título legible por humanos/agentes, si se conoce.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
}

// ---------------------------------------------------------------------------
// Códigos de error estables (E10-H02, `ARCHITECTURE.md §19.3`, `REFACTOR.md §13`).
//
// `ErrorCode` se define UNA sola vez en `core::types` (invariante #4) — aquí solo vive el MAPEO
// desde los errores reales del núcleo/workspace. Por el orphan rule no podemos escribir
// `impl From<&CoreError> for ErrorCode` en este crate (ni `CoreError` ni `ErrorCode` son locales),
// así que el mapeo es una función libre — el patrón natural para una traducción N:1 que además
// necesita ver el error completo (no solo su variante) para casos futuros con contexto adicional.
// ---------------------------------------------------------------------------

/// Mapea un [`CoreError`] a su [`ErrorCode`] estable de protocolo.
///
/// `InvalidRelPath` (el único chokepoint de path-traversal, invariante #6 de `CLAUDE.md`) mapea a
/// `PermissionDenied`: un intento de escapar del bundle es semánticamente un permiso denegado, no
/// un error de esquema. El resto son mapeos razonables a falta de que E12/E13 los produzcan en
/// flujos reales (fuera de alcance de esta historia):
/// - `InvalidSha` → `InvalidSchema` (formato de entrada inválido).
/// - `SizeGuardExceeded` → `ResultTooLarge` (guarda de tamaño excedida en una operación).
/// - `Export` → `InternalIoError` (fallo de IO/serialización al exportar).
pub fn error_code(err: &CoreError) -> ErrorCode {
    match err {
        CoreError::InvalidRelPath(_) => ErrorCode::PermissionDenied,
        CoreError::InvalidSha(_) => ErrorCode::InvalidSchema,
        CoreError::SizeGuardExceeded(_) => ErrorCode::ResultTooLarge,
        CoreError::Export(_) => ErrorCode::InternalIoError,
    }
}

/// Mapea un [`WorkspaceError`] a su [`ErrorCode`] estable de protocolo.
///
/// `WorkspaceError::Core` envuelve el `CoreError` original ya **serializado a `String`**
/// (`error.rs` de `lodestar-workspace`), así que aquí no se puede recuperar su variante original
/// para reusar [`error_code`] — se documenta como limitación conocida, a resolver si una historia
/// futura decide preservar la variante en vez de aplanarla a texto. Mapeos:
/// - `Core`/`Store`/`Io`/`NoVcs`/`NoCache` → `InternalIoError`: fallos de infraestructura/IO o
///   precondiciones internas sin un código más específico todavía en el catálogo de 16.
/// - `Vcs` → `WriteConflict`: el caso más común de un fallo de git durante una operación de la
///   fachada es un estado de escritura en conflicto (aproximación documentada; git puede fallar
///   por otras razones, p. ej. red, pero el catálogo actual no distingue más).
/// - `RepoBusy` (merge/rebase en curso) → `WriteConflict`: literalmente un conflicto de escritura.
pub fn workspace_error_code(err: &WorkspaceError) -> ErrorCode {
    match err {
        WorkspaceError::Core(_) => ErrorCode::InternalIoError,
        WorkspaceError::Vcs(_) => ErrorCode::WriteConflict,
        WorkspaceError::Io(_) => ErrorCode::InternalIoError,
        WorkspaceError::NoVcs => ErrorCode::InternalIoError,
        WorkspaceError::NoCache => ErrorCode::InternalIoError,
        WorkspaceError::Store(_) => ErrorCode::InternalIoError,
        WorkspaceError::RepoBusy => ErrorCode::WriteConflict,
    }
}

/// Forma de error de protocolo (`REFACTOR.md §13`): lo que se sirve en vez de un [`Envelope`]
/// cuando una operación se rechaza. Wire en camelCase; `expected_revision`/`actual_revision`
/// solo se rellenan para `REVISION_CONFLICT` (control optimista, E12) y `recovery` es un mensaje
/// legible con el siguiente paso sugerido (p. ej. "vuelve a leer y reintenta").
///
/// Esta historia (E10-H02) solo fija la forma — nadie la construye todavía en un flujo real
/// (eso llega con las tools de E10-H08+ y la planificación de E12/E13).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ErrorEnvelope {
    /// Código estable del catálogo de 16 (`core::types::ErrorCode`).
    pub code: ErrorCode,
    /// Mensaje legible, en español, para un humano o un agente.
    pub message: String,
    /// Revisión que el llamante esperaba (solo relevante para `REVISION_CONFLICT`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_revision: Option<WorkspaceRevision>,
    /// Revisión real encontrada (solo relevante para `REVISION_CONFLICT`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actual_revision: Option<WorkspaceRevision>,
    /// Sugerencia legible del siguiente paso para recuperarse del error.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recovery: Option<String>,
}

impl ErrorEnvelope {
    /// Construye un `ErrorEnvelope` mínimo (sin campos de recuperación).
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        ErrorEnvelope {
            code,
            message: message.into(),
            expected_revision: None,
            actual_revision: None,
            recovery: None,
        }
    }
}

// ---------------------------------------------------------------------------
// `workspace_status` (E10-H08, `ARCHITECTURE.md §19.6`, `docs/REFACTOR.md §9.1/§7`).
// ---------------------------------------------------------------------------

/// Perfil con el que arranca el servidor (`lodestar-mcp --profile readonly|standard`,
/// `ARCHITECTURE.md §19.6`). Config de **runtime del proceso**, no contrato de wire — el cliente
/// nunca envía ni recibe un `Profile` serializado; solo ve su efecto en `capabilities.writes` (y,
/// en su día, `transactions`/`revert`) del `WorkspaceStatus`. Por eso vive en `lodestar-app` y no
/// en `core::types` (invariante #4: ese módulo es para el contrato de wire, no para flags de
/// arranque del proceso).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Profile {
    /// Solo las tools de lectura/verificación — sin `create_concept`/`update_frontmatter` ni,
    /// más adelante, `change_plan`/`change_apply`/`change_revert`.
    Readonly,
    /// Añade las tools de cambio a las de lectura/verificación (perfil por defecto).
    Standard,
}

impl Profile {
    /// `true` si este perfil habilita las tools de escritura (`capabilities.writes`).
    fn writes_enabled(self) -> bool {
        matches!(self, Profile::Standard)
    }
}

/// Recuento agregado de conceptos/enlaces/diagnósticos de un workspace (`counts` de
/// `WorkspaceStatus`, `docs/REFACTOR.md §9.1`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusCounts {
    /// Nº de conceptos (`Analysis::concepts`).
    pub concepts: usize,
    /// Nº total de enlaces salientes resueltos (suma de `Analysis::out` sobre todos los conceptos).
    pub links: usize,
    /// Nº de conceptos huérfanos (`Analysis::orphans`).
    pub orphans: usize,
    /// Nº de enlaces colgantes (`Analysis::dangling`).
    pub dangling: usize,
    /// Nº de ficheros con al menos un check `Err` (`Analysis::hard_fail`).
    pub errors: usize,
    /// Nº de checks `Warn` (`Analysis::warn_count`).
    pub warnings: usize,
}

/// Capacidades habilitadas por el perfil de arranque (`capabilities` de `WorkspaceStatus`,
/// `docs/REFACTOR.md §9.1`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusCapabilities {
    /// `true` si el perfil admite tools de cambio (`create_concept`/`update_frontmatter` hoy;
    /// `change_plan`/`change_apply` en E12).
    pub writes: bool,
    /// `true` si el perfil admite transacciones (`change_apply`, E13). Hoy igual a `writes`: la
    /// mecánica transaccional real es de E13, pero el perfil que la habilitará es el mismo que
    /// habilita escrituras.
    pub transactions: bool,
    /// `true` si el perfil admite revertir la última transacción (`change_revert`, E13). Misma
    /// nota que `transactions`.
    pub revert: bool,
    /// `true` si el servidor entiende `.lodestar/schema.yaml` (siempre, desde E10-H05).
    pub schemas: bool,
    /// `true` si el servidor entiende `referenceRoots` (siempre, desde E9-H05).
    pub external_references: bool,
}

/// Estado de una posible transacción interrumpida (`recovery` de `WorkspaceStatus`). E13 lo
/// puebla de verdad (staging/journal/crash-recovery); hasta entonces siempre `false` — no hay
/// mecánica transaccional que pueda dejar el workspace a medio escribir.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusRecovery {
    /// `true` si hay una transacción sin terminar pendiente de recuperar. Fijo a `false` hasta
    /// E13-H06.
    pub pending_transaction: bool,
}

/// Proyección de estado del workspace — la primera tool que se espera que llame un agente en
/// cada sesión (`docs/REFACTOR.md §7`, §9.1). Compone `core::types::workspace_revision` +
/// `Analysis` + `WorkspaceConfig` + `Schema`, sin lógica de dominio nueva propia: es un servicio
/// que reusa lo que el core y la workspace ya calculan.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatus {
    /// Revisión determinista de las raíces escribibles (`WorkspaceRevision`, E10-H03).
    pub workspace_revision: WorkspaceRevision,
    /// Directorio raíz del bundle abierto.
    pub root: String,
    /// Raíces de escritura/lectura (`WorkspaceConfig::workspace.writable_roots`).
    pub knowledge_roots: Vec<RelPath>,
    /// Raíces visibles pero no escribibles (`WorkspaceConfig::workspace.reference_roots`).
    pub reference_roots: Vec<RelPath>,
    /// Versión del formato OKF del `index.md` raíz (`Analysis::okf_version`), o `"0.2"` si no
    /// está declarada.
    pub format_version: String,
    /// Versión del formato de `.lodestar/schema.yaml` (`Schema::version`; `"1"` si no hay schema).
    pub schema_version: String,
    /// `true` si el bundle no tiene ningún check `Err` (`Analysis::hard_fail == 0`).
    pub conformant: bool,
    /// Recuento agregado de conceptos/enlaces/diagnósticos.
    pub counts: StatusCounts,
    /// Capacidades habilitadas por el perfil de arranque.
    pub capabilities: StatusCapabilities,
    /// Estado de recuperación de transacciones (siempre `pendingTransaction: false` hasta E13).
    pub recovery: StatusRecovery,
}

/// Versión del formato OKF asumida cuando el `index.md` raíz no declara `okf_version`
/// (`ARCHITECTURE.md §19.6`).
const DEFAULT_FORMAT_VERSION: &str = "0.2";

/// Fachada fina de servicios de caso de uso sobre un [`Workspace`] abierto.
///
/// `App` es lo que consumen `lodestar-mcp` y `lodestar-cli`: un punto de entrada único que
/// traduce peticiones de protocolo a operaciones del `Workspace` y envuelve las respuestas en
/// [`Envelope`]. Expone `workspace_status` (E10-H08); `knowledge_search`, `knowledge_get`,
/// `schema_inspect`, `knowledge_check`, … se irán añadiendo en E10-H09 y siguientes.
pub struct App {
    workspace: Workspace,
}

impl App {
    /// Abre el bundle en `root` y construye la fachada de servicios. Delega en
    /// [`Workspace::open`] — mismas garantías (descubrimiento de git best-effort, identidad desde
    /// `lodestar.toml`, cache incremental **no** activada).
    pub fn open(root: &Path) -> Result<Self, WorkspaceError> {
        let workspace = Workspace::open(root)?;
        Ok(App { workspace })
    }

    /// Envuelve un [`Workspace`] ya abierto (p. ej. [`Workspace::open_ephemeral`] en tests, o un
    /// caller que ya gestiona su propio ciclo de vida del workspace).
    pub fn from_workspace(workspace: Workspace) -> Self {
        App { workspace }
    }

    /// El `Workspace` subyacente, para los servicios que se implementen sobre `App`.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    /// Resuelve un [`ConceptRef`] al `RelPath` del concepto que referencia (E10-H04).
    ///
    /// v2 resuelve identidad **únicamente por `path`**: comprueba contra la lista autoritativa de
    /// concepts que computa el core (`Analysis::concepts`, invariante #3 — "una sola verdad
    /// computada"), no contra la mera presencia de un fichero en el `FileMap` (así un `.md`
    /// reservado como `index.md`/`log.md`, que el core no cuenta como concept, tampoco resuelve
    /// aquí). Si el `path` no está en esa lista, `Err(ErrorCode::ConceptNotFound)`.
    ///
    /// `ErrorCode::AmbiguousReference` queda RESERVADO para cuando exista resolución por `id`
    /// (`REFACTOR §6.1`) — no-goal de esta historia (IDs estables/federación). En v2 `ConceptRef.id`
    /// es siempre `None`, así que esta función nunca lo produce todavía.
    pub fn resolve_ref(&self, r: &ConceptRef) -> Result<RelPath, ErrorCode> {
        let analysis = self
            .workspace
            .analyze()
            .map_err(|e| workspace_error_code(&e))?;
        if analysis.concepts.contains(&r.path) {
            Ok(r.path.clone())
        } else {
            Err(ErrorCode::ConceptNotFound)
        }
    }

    /// Proyección de estado del workspace (E10-H08): config activa, capacidades del perfil,
    /// conformidad y recuento agregado — la primera tool que debe llamar un agente en cada
    /// sesión (`docs/REFACTOR.md §7`).
    ///
    /// Compone `Bundle::analyze` (una sola verdad computada, invariante #3) +
    /// `core::types::workspace_revision` (E10-H03) + `WorkspaceConfig::load`/`WorkspaceSchema::load`
    /// (I/O de `workspace`, nunca del core) — sin lógica de dominio propia.
    pub fn workspace_status(&self, profile: Profile) -> Result<WorkspaceStatus, WorkspaceError> {
        let bundle = self.workspace.bundle()?;
        let files = bundle.files();
        let analysis = bundle.analyze();
        let root = self.workspace.root();
        let cfg = WorkspaceConfig::load(root).map_err(WorkspaceError::Io)?;
        let schema = WorkspaceSchema::load(root).map_err(WorkspaceError::Io)?;

        let revision = workspace_revision(files, &cfg.workspace.writable_roots);
        let links = analysis.out.values().map(Vec::len).sum();
        let writes = profile.writes_enabled();

        Ok(WorkspaceStatus {
            workspace_revision: revision,
            root: root.display().to_string(),
            knowledge_roots: cfg.workspace.writable_roots.clone(),
            reference_roots: cfg.workspace.reference_roots.clone(),
            format_version: analysis
                .okf_version
                .clone()
                .unwrap_or_else(|| DEFAULT_FORMAT_VERSION.to_string()),
            schema_version: schema.version.clone(),
            conformant: analysis.hard_fail == 0,
            counts: StatusCounts {
                concepts: analysis.concepts.len(),
                links,
                orphans: analysis.orphans.len(),
                dangling: analysis.dangling.len(),
                errors: analysis.hard_fail,
                warnings: analysis.warn_count,
            },
            capabilities: StatusCapabilities {
                writes,
                transactions: writes,
                revert: writes,
                schemas: true,
                external_references: true,
            },
            recovery: StatusRecovery {
                pending_transaction: false,
            },
        })
    }

    // Los métodos de caso de uso (`knowledge_search`, `knowledge_get`, `schema_inspect`,
    // `knowledge_check`, …) llegan en E10-H09+.
}
