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

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use lodestar_core::eval::{evaluate, EvalDocument};
use lodestar_core::metadata;
use lodestar_core::model;
use lodestar_core::plan::{self, PlanPolicy};
use lodestar_core::text::loose_text_match;
use lodestar_core::types::{
    workspace_revision, Analysis, Backlinks, ChangeReceipt, ChangeSet, ChangeSetId, Check,
    Direction, DocumentRef, DocumentRevision, Edge, EditSectionMode, ErrorCode, Expression,
    FieldInspection, FieldPath, FrontmatterPatch, GraphNode, InboundLinksPolicy, MetadataCatalog,
    NormalizedOperation, PlanHash, ReceiptId, RelPath, ResolvedLink, RiskAssessment, SemanticDiff,
    Severity, ValidationReport, ValidationSummary, WorkspaceRevision,
};
use lodestar_core::{CoreError, DocumentSet};
use lodestar_workspace::{transaction_id, ExternalReference, Workspace, WorkspaceError};

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
/// `docs/REFACTOR.md §13`), p. ej. un documento relacionado que el agente puede pedir con
/// `knowledge_get` a continuación. Forma mínima: URI del recurso (dirección estable, no
/// necesariamente un `RelPath` — puede referirse a recursos fuera del workspace) y un título
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
/// `PermissionDenied`: un intento de escapar del workspace es semánticamente un permiso denegado, no
/// un error de esquema. El resto son mapeos razonables a falta de que E12/E13 los produzcan en
/// flujos reales (fuera de alcance de esta historia):
/// - `SizeGuardExceeded` → `ResultTooLarge` (guarda de tamaño excedida en una operación).
/// - `ReplaceTextMismatch` → `InvalidSchema` (precondición de `replace_text` incumplida, E12-H05).
/// - `NormalizeTargetNotFound` → `DocumentNotFound` (path/sección objetivo inexistente, E12-H05).
/// - `InboundLinksExist` → `InboundLinksExist` (borrar `reject` con entrantes, E12-H06).
/// - `RelationConstraintViolation` → `RelationConstraintViolation` (E12-H07; sin productor desde
///   E20-H03, ver [`CoreError`]).
/// - `InvalidStatusTransition` → `InvalidSchema` (E12-H07; sin productor desde E20-H03, ver
///   [`CoreError`]).
/// - `FixNotFound` → `DocumentNotFound` (`apply_fix` con un `fixId` inexistente/no aplicable, E12-H07).
/// - `InvalidFieldPath` → `InvalidSchema` (ruta a propiedad de frontmatter mal formada, E16-H01:
///   entrada del agente que no designa ningún campo).
/// - `UnreadableFrontmatter` → `InvalidSchema` (E16-H04: el bloque de frontmatter del documento
///   no se puede interpretar, así que no se puede parchear). Se descartan `DocumentNotFound` —el
///   documento **existe**, y decir lo contrario mandaría al agente a buscar una ruta correcta— e
///   `InternalIoError` —culparía al motor de un estado del fichero del usuario, cuando lo que hay
///   es una **precondición de la operación** incumplida por el dato de entrada, exactamente igual
///   que `ReplaceTextMismatch`/`InvalidStatusTransition`. `InvalidSchema` es además accionable: le
///   dice al agente que repare el documento (o lo escriba crudo) antes de tocar su metadata.
pub fn error_code(err: &CoreError) -> ErrorCode {
    match err {
        CoreError::InvalidRelPath(_) => ErrorCode::PermissionDenied,
        CoreError::InvalidFieldPath(_) => ErrorCode::InvalidSchema,
        CoreError::SizeGuardExceeded(_) => ErrorCode::ResultTooLarge,
        CoreError::ReplaceTextMismatch(_, _) => ErrorCode::InvalidSchema,
        CoreError::NormalizeTargetNotFound(_) => ErrorCode::DocumentNotFound,
        CoreError::InboundLinksExist(_) => ErrorCode::InboundLinksExist,
        CoreError::RelationConstraintViolation(_) => ErrorCode::RelationConstraintViolation,
        CoreError::InvalidStatusTransition(_) => ErrorCode::InvalidSchema,
        CoreError::FixNotFound(_) => ErrorCode::DocumentNotFound,
        // Invariante interno (E12-H08): el aplicador recibió una op sin normalizar a forma
        // terminal — fallo de infraestructura, no del agente.
        CoreError::OperationNotApplicable(_) => ErrorCode::InternalIoError,
        CoreError::UnreadableFrontmatter(_) => ErrorCode::InvalidSchema,
    }
}

/// Mapea un [`WorkspaceError`] a su [`ErrorCode`] estable de protocolo.
///
/// `WorkspaceError::Core` envuelve el `CoreError` original ya **serializado a `String`**
/// (`error.rs` de `lodestar-workspace`), así que aquí no se puede recuperar su variante original
/// para reusar [`error_code`] — se documenta como limitación conocida, a resolver si una historia
/// futura decide preservar la variante en vez de aplanarla a texto. Mapeos:
/// - `Core`/`Store`/`Io`/`NoCache` → `InternalIoError`: fallos de infraestructura/IO o
///   precondiciones internas sin un código más específico todavía en el catálogo de 16.
/// - `PermissionDenied` (E11-H04: escritura bajo un `referenceRoot`, o fuera de `writableRoots`) →
///   `ErrorCode::PermissionDenied`, mapeo directo por nombre (mismo caso que `error_code` con
///   `CoreError::InvalidRelPath`).
/// - `NonconformantResult` (E13-H01) / `WriteConflict` (E13-H02) / `WorkspaceRecoveryRequired`
///   (E13-H06) → sus códigos homónimos del catálogo, mapeo directo por nombre.
pub fn workspace_error_code(err: &WorkspaceError) -> ErrorCode {
    match err {
        WorkspaceError::Core(_) => ErrorCode::InternalIoError,
        WorkspaceError::Io(_) => ErrorCode::InternalIoError,
        WorkspaceError::NoCache => ErrorCode::InternalIoError,
        WorkspaceError::Store(_) => ErrorCode::InternalIoError,
        WorkspaceError::PermissionDenied(_) => ErrorCode::PermissionDenied,
        WorkspaceError::NonconformantResult(_) => ErrorCode::NonconformantResult,
        WorkspaceError::WriteConflict(_) => ErrorCode::WriteConflict,
        WorkspaceError::WorkspaceRecoveryRequired(_) => ErrorCode::WorkspaceRecoveryRequired,
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
    /// Solo las tools de lectura/verificación — sin las tres de cambio (`change_plan`/
    /// `change_apply`/`change_revert`), que además de ocultarse se **rechazan** si se invocan.
    Readonly,
    /// Añade las tools de cambio a las de lectura/verificación (perfil por defecto).
    Standard,
}

impl Profile {
    /// `true` si este perfil habilita las tools de cambio (`change_plan`/`change_apply`/
    /// `change_revert`). Fuente única del efecto del perfil: gobierna a la vez
    /// `capabilities.writes` de [`WorkspaceStatus`] y la disponibilidad de las tools de cambio en
    /// la superficie MCP (filtrado de `tools/list` y gating de invocación, E14-H03).
    pub fn writes_enabled(self) -> bool {
        matches!(self, Profile::Standard)
    }
}

/// Recuento agregado de documentos/enlaces/diagnósticos de un workspace (`counts` de
/// `WorkspaceStatus`, `docs/REFACTOR.md §9.1`).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusCounts {
    /// Nº de documentos (`Analysis::documents`).
    pub documents: usize,
    /// Nº total de enlaces salientes resueltos (suma de `Analysis::out` sobre todos los documentos).
    pub links: usize,
    /// Nº de documentos **aislados** —sin enlaces internos entrantes ni salientes—
    /// (`Analysis::isolated`). Antes `orphans`, con otra definición (E16-H02).
    pub isolated: usize,
    /// Nº de enlaces colgantes (`Analysis::dangling`).
    pub dangling: usize,
    /// Nº de ficheros con al menos un check `Err` (`Analysis::hard_fail`).
    pub errors: usize,
    /// Nº de checks `Warn` (`Analysis::warn_count`).
    pub warnings: usize,
}

/// Capacidades habilitadas por el perfil de arranque (`capabilities` de `WorkspaceStatus`,
/// `docs/REFACTOR.md §9.1`).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusCapabilities {
    /// `true` si el perfil admite tools de cambio (`change_plan`/`change_apply`/`change_revert`).
    pub writes: bool,
    /// `true` si el perfil admite transacciones (`change_apply`, E13). Hoy igual a `writes`: la
    /// mecánica transaccional real es de E13, pero el perfil que la habilitará es el mismo que
    /// habilita escrituras.
    pub transactions: bool,
    /// `true` si el perfil admite revertir la última transacción (`change_revert`, E13). Misma
    /// nota que `transactions`.
    pub revert: bool,
    /// Capacidad histórica de esquemas. **Desde E20-H03 el motor NO tiene esquemas** (`§20.10`,
    /// modelo universal): `core::schema`/`.lodestar/schema.yaml` se retiraron. El campo se conserva
    /// en el wire, fijo a `false`, para no romper a un cliente que lo lea.
    pub schemas: bool,
    /// `true` si el servidor entiende `referenceRoots` (siempre, desde E9-H05).
    pub external_references: bool,
}

/// Estado de una posible transacción interrumpida (`recovery` de `WorkspaceStatus`). E13 lo
/// puebla de verdad (staging/journal/crash-recovery); hasta entonces siempre `false` — no hay
/// mecánica transaccional que pueda dejar el workspace a medio escribir.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
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
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceStatus {
    /// Revisión determinista de las raíces escribibles (`WorkspaceRevision`, E10-H03).
    pub workspace_revision: WorkspaceRevision,
    /// Directorio raíz del workspace abierto.
    pub root: String,
    /// Raíces de escritura/lectura (`WorkspaceConfig::workspace.writable_roots`).
    pub knowledge_roots: Vec<RelPath>,
    /// Raíces visibles pero no escribibles (`WorkspaceConfig::workspace.reference_roots`).
    pub reference_roots: Vec<RelPath>,
    /// Versión del formato de documento que sirve el motor. **Constante** desde E16-H02: el motor
    /// ya no lee `okf_version` del `index.md` raíz — esa clave es metadata del usuario como
    /// cualquier otra (`§20.13`) y ningún nombre de fichero activa reglas especiales.
    pub format_version: String,
    /// Campo histórico de versión de esquema. Fijo a `"1"` desde E20-H03: el motor ya no tiene
    /// esquemas (`§20.10`); se conserva en el wire para no romper a un cliente que lo lea.
    pub schema_version: String,
    /// `true` si el workspace no tiene ningún check `Err` (`Analysis::hard_fail == 0`).
    pub conformant: bool,
    /// Recuento agregado de documentos/enlaces/diagnósticos.
    pub counts: StatusCounts,
    /// Capacidades habilitadas por el perfil de arranque.
    pub capabilities: StatusCapabilities,
    /// Estado de recuperación de transacciones (siempre `pendingTransaction: false` hasta E13).
    pub recovery: StatusRecovery,
}

/// Versión del formato de documento que reporta `workspace_status` (`ARCHITECTURE.md §19.6`).
/// Desde E16-H02 es un valor fijo: ya no se deriva de ningún documento del workspace.
const DEFAULT_FORMAT_VERSION: &str = "0.2";

/// Versión del formato de esquema que reporta `workspace_status.schemaVersion`. Desde E20-H03 es un
/// valor fijo: `core::schema` y `.lodestar/schema.yaml` se retiraron (modelo universal, `§20.10`),
/// así que ya no hay un esquema del que derivarla. Se conserva en el wire (`"1"`) para no romper a un
/// cliente que lea el campo.
const DEFAULT_SCHEMA_VERSION: &str = "1";

/// Fachada fina de servicios de caso de uso sobre un [`Workspace`] abierto.
///
/// `App` es lo que consumen `lodestar-mcp` y `lodestar-cli`: un punto de entrada único que
/// traduce peticiones de protocolo a operaciones del `Workspace` y envuelve las respuestas en
/// [`Envelope`]. Expone `workspace_status` (E10-H08), `knowledge_search` (E10-H09),
/// `knowledge_get` (E10-H10), `metadata_inspect` (E20-H03), `knowledge_check`, … .
pub struct App {
    workspace: Workspace,
}

impl App {
    /// Abre el workspace en `root` y construye la fachada de servicios. Delega en
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

    /// Resuelve un [`DocumentRef`] al `RelPath` del documento que referencia (E10-H04).
    ///
    /// v2 resuelve identidad **únicamente por `path`**: comprueba contra la lista autoritativa de
    /// documentos que computa el core (`Analysis::documents`, invariante #3 — "una sola verdad
    /// computada"), no contra la mera presencia de un fichero en el `FileMap` — así la resolución
    /// pasa por el mismo inventario que analiza el core, sin criterios paralelos. Si el `path` no
    /// está en esa lista, `Err(ErrorCode::DocumentNotFound)`.
    ///
    /// `ErrorCode::AmbiguousReference` queda RESERVADO para cuando exista resolución por `id`
    /// (`REFACTOR §6.1`) — no-goal de esta historia (IDs estables/federación). En v2 `DocumentRef.id`
    /// es siempre `None`, así que esta función nunca lo produce todavía.
    pub fn resolve_ref(&self, r: &DocumentRef) -> Result<RelPath, ErrorCode> {
        let analysis = self
            .workspace
            .analyze()
            .map_err(|e| workspace_error_code(&e))?;
        if analysis.documents.contains(&r.path) {
            Ok(r.path.clone())
        } else {
            Err(ErrorCode::DocumentNotFound)
        }
    }

    /// Proyección de estado del workspace (E10-H08): config activa, capacidades del perfil,
    /// conformidad y recuento agregado — la primera tool que debe llamar un agente en cada
    /// sesión (`docs/REFACTOR.md §7`).
    ///
    /// Compone `DocumentSet::analyze` (una sola verdad computada, invariante #3) +
    /// `core::types::workspace_revision` (E10-H03) + `WorkspaceConfig::load` (I/O de `workspace`,
    /// nunca del core) — sin lógica de dominio propia.
    pub fn workspace_status(&self, profile: Profile) -> Result<WorkspaceStatus, WorkspaceError> {
        let doc_set = self.workspace.document_set()?;
        let files = doc_set.files();
        let analysis = doc_set.analyze();
        let root = self.workspace.root();
        let cfg = self.workspace.config();

        let revision = workspace_revision(files, &cfg.workspace.writable_roots);
        // Aristas del grafo: los enlaces INTERNOS (documentos y fantasmas). Los externos, los
        // anchors propios y los que apuntan a ficheros del proyecto viajan en `Analysis::outgoing`
        // pero no conectan documentos, así que no cuentan como enlaces del workspace (`§20.7`).
        let links = analysis
            .outgoing
            .values()
            .flatten()
            .filter(|l| l.target.is_internal())
            .count();
        let writes = profile.writes_enabled();

        Ok(WorkspaceStatus {
            workspace_revision: revision,
            root: root.display().to_string(),
            knowledge_roots: cfg.workspace.writable_roots.clone(),
            reference_roots: cfg.workspace.reference_roots.clone(),
            format_version: DEFAULT_FORMAT_VERSION.to_string(),
            schema_version: DEFAULT_SCHEMA_VERSION.to_string(),
            conformant: analysis.hard_fail() == 0,
            counts: StatusCounts {
                documents: analysis.documents.len(),
                links,
                isolated: analysis.isolated.len(),
                dangling: analysis.dangling.len(),
                errors: analysis.hard_fail(),
                warnings: analysis.warn_count(),
            },
            capabilities: StatusCapabilities {
                writes,
                transactions: writes,
                revert: writes,
                schemas: false,
                external_references: true,
            },
            recovery: StatusRecovery {
                pending_transaction: false,
            },
        })
    }

    /// Localiza documentos por texto y por el **lenguaje de consulta tipado**, con snippets y
    /// paginación por cursor, **sin devolver cuerpos completos** (E19-H05, `ARCHITECTURE.md §20.10`).
    ///
    /// La **verdad** del casado la da el core (invariante #3). Hay dos criterios, que se combinan por
    /// **intersección** (un documento aparece si los pasa todos):
    /// - `text`: subcadena case-insensitive sobre basename + valores de frontmatter + cuerpo, con la
    ///   misma [`loose_text_match`] que usa la cache FTS. Un `text` vacío casa todos los documentos.
    /// - `where_expr`/`filter`: la consulta textual (`§20.8`) y el filtro JSON estructurado
    ///   (`§20.10`) se traducen al **mismo** [`Expression`] ([`lodestar_core::parse::parse`] /
    ///   [`lodestar_core::filter::from_json`]) y se evalúan por documento con
    ///   [`evaluate`] —el evaluador tipado que ve el frontmatter, el propio documento (`document.*`) y
    ///   el grafo (`graph.*`)—, de modo que `where` y `filter` equivalentes dan el mismo resultado. Si
    ///   llegan **ambos**, se combinan con `and` (intersección), coherente con cómo `text` ya se
    ///   intersecta; ningún filtro por sí solo abre la selección.
    ///
    /// El filtrado por metadata (antes los filtros OKF privilegiados `types`/`statuses`/`tags`/
    /// `pathPrefix`, retirados en E19-H05) pasa **enteramente** por el lenguaje: `status =
    /// "accepted"`, `type = "x"`, `tags contains "y"`, `document.path starts_with "docs/"`… — sin
    /// campos privilegiados.
    ///
    /// **Orden determinista**: `score` descendente y, a igualdad, `path` ascendente — total y estable
    /// (los paths son únicos), así la partición en páginas es reproducible entre procesos frescos.
    ///
    /// **Paginación por cursor autosuficiente**: el cursor es la codificación hexadecimal opaca de un
    /// **offset** dentro del orden determinista. Como el orden depende solo del contenido (no de
    /// ningún estado de sesión ni de la caché), un mismo cursor reanuda idénticamente en un servidor
    /// recién arrancado. `limit` por defecto 20, tope 100; `nextCursor` es `None` al agotar.
    ///
    /// `sort` queda reservado para una futura elección de criterio explícito; hoy el orden es siempre
    /// el determinista descrito arriba.
    ///
    /// Cada resultado lleva `revision` = [`DocumentRevision`] del contenido en disco (blake3, E10-H03)
    /// y un `snippet` compacto NO vacío; la estructura [`SearchResult`] **no tiene** campo `body`, así
    /// que es imposible filtrar el cuerpo completo por esta vía.
    ///
    /// Un `where_expr`/`filter` **malformado** (no parseable) se surface como un
    /// [`WorkspaceError::Core`] genérico —el mapeo fino a `INVALID_SCHEMA` es E20—; un
    /// [`lodestar_core::types::TypeError`] al **evaluar** una expresión bien formada contra un
    /// documento concreto (p. ej. `priority >= "high"` sobre un `priority` numérico) **excluye ese
    /// documento** del resultado, sin abortar la búsqueda: el corpus es heterogéneo y un tipo
    /// incompatible en un documento no debe tumbar la consulta sobre los demás.
    pub fn knowledge_search(
        &self,
        text: &str,
        where_expr: Option<&str>,
        filter: Option<&Value>,
        _sort: Option<&str>,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<SearchResults, WorkspaceError> {
        let doc_set = self.workspace.document_set()?;
        let analysis = doc_set.analyze();
        let files = doc_set.files();

        let text_trim = text.trim();
        let needle = text_trim.to_lowercase();
        // Compila `where`/`filter` al mismo AST (E19-H01…H04). Ambos → `and` (intersección).
        let expr = build_search_expression(where_expr, filter)?;

        let mut results: Vec<SearchResult> = Vec::new();
        for path in &analysis.documents {
            let Some(raw) = files.get(path) else { continue };
            let parsed = model::parse_file(path.as_str(), raw);
            let fm = parsed.frontmatter.clone().unwrap_or_default();

            // (1) Intersección con el FTS de `text` (subcadena, verdad del core).
            if !text_trim.is_empty() && !loose_text_match(path, &fm, &parsed.body, &needle) {
                continue;
            }

            // (2) Intersección con el lenguaje de consulta. Un `TypeError` sobre ESTE documento lo
            //     excluye (no casa), sin propagarse a la búsqueda entera.
            if let Some(expr) = &expr {
                let doc = EvalDocument {
                    path,
                    frontmatter: parsed.frontmatter.as_ref(),
                    body: &parsed.body,
                };
                if !matches!(evaluate(expr, &doc, analysis), Ok(true)) {
                    continue;
                }
            }

            let title = model::derived_title(Some(&fm), &parsed.body, path);
            let snippet = {
                let s = snippet_of(&parsed.body, &needle);
                if s.is_empty() {
                    // Garantía de snippet NO vacío: cae al título (o al path si no hay título).
                    if title.is_empty() {
                        path.as_str().to_string()
                    } else {
                        title.clone()
                    }
                } else {
                    s
                }
            };
            let revision = DocumentRevision::from_hash(*blake3::hash(raw.as_bytes()).as_bytes());

            results.push(SearchResult {
                path: path.clone(),
                id: None,
                title,
                snippet,
                score: score_of(raw, &needle),
                revision,
            });
        }

        // Orden total y estable: score desc, path asc.
        results.sort_by(|a, b| {
            b.score
                .partial_cmp(&a.score)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| a.path.cmp(&b.path))
        });

        let total = results.len();
        let limit = limit.unwrap_or(DEFAULT_SEARCH_LIMIT).min(MAX_SEARCH_LIMIT);
        let start = cursor.map(decode_cursor).unwrap_or(0).min(total);
        let end = start.saturating_add(limit).min(total);
        let page = results.get(start..end).unwrap_or(&[]).to_vec();
        // `nextCursor` solo si hubo progreso y quedan resultados (evita bucles con `limit == 0`).
        let next_cursor = (end > start && end < total).then(|| encode_cursor(end));

        Ok(SearchResults {
            results: page,
            next_cursor,
            total_approximate: total,
        })
    }

    /// Obtiene un documento concreto, con `include` selectivo y selección de secciones por
    /// `headingPath` (E10-H10, `ARCHITECTURE.md §19.6`, `REFACTOR §9.3`).
    ///
    /// Resuelve con [`App::resolve_ref`] (E10-H04) — `Err(ErrorCode::DocumentNotFound)` si el path
    /// no está en la lista autoritativa de documentos. `revision` (== [`DocumentRevision`], E10-H03)
    /// se calcula **siempre**, sin depender de `include`: es la identidad de contenido, no un
    /// campo opcional.
    ///
    /// `include` es la lista de campos wire pedidos (`"frontmatter"`, `"body"`, `"outgoingLinks"`,
    /// `"backlinks"`, `"diagnostics"`, `"externalReferences"`; `"revision"` es aceptado pero no-op,
    /// ya que ese campo siempre se puebla). Un campo **no** pedido queda en `None` en el
    /// [`DocumentView`] — nunca en su valor por defecto "vacío" disfrazado de "no pedido", para que
    /// el `include` selectivo sea significativo (criterio `get_incluye_revision`).
    ///
    /// `sections`, si está presente y no vacío, acota el `body` devuelto (solo aplica si `body` fue
    /// pedido en `include`): cada `headingPath` (p. ej. `["Security","Token rotation"]`) localiza
    /// esa subsección anidada del Markdown (vía `model::extract_sections`, en el core) y
    /// el resultado final es la concatenación de todos los `headingPath` pedidos. Sin `sections`,
    /// `body` es el cuerpo completo.
    ///
    /// `externalReferences` resuelve `implemented_by`/`verified_by` contra disco vía
    /// [`Workspace::external_refs`] (E11-H04) — `{path, exists}` por cada referencia declarada. Desde
    /// E20-H03 esa llamada NO produce diagnósticos (el `EXTREF-MISSING` se retiró con `core::schema`);
    /// el campo `diagnostics` de esta proyección viene solo de `Analysis::diagnostics` (invariante #3);
    /// un agente que quiera detectar una ref rota la deriva de `exists:false` en `externalReferences`.
    pub fn knowledge_get(
        &self,
        r: &DocumentRef,
        include: &[String],
        sections: Option<&[Vec<String>]>,
    ) -> Result<DocumentView, ErrorCode> {
        let path = self.resolve_ref(r)?;
        let doc_set = self
            .workspace
            .document_set()
            .map_err(|e| workspace_error_code(&e))?;
        let files = doc_set.files();
        // `resolve_ref` ya comprobó que `path` está en `Analysis::documents`, que se computa a
        // partir de este mismo `FileMap` (invariante #3) — así que el fichero existe.
        let raw = files
            .get(&path)
            .expect("resolve_ref garantiza presencia en el FileMap");
        let parsed = model::parse_file(path.as_str(), raw);
        let revision = DocumentRevision::from_hash(*blake3::hash(raw.as_bytes()).as_bytes());

        let wants = |field: &str| include.iter().any(|s| s == field);

        // El frontmatter que viaja es el YAML ARBITRARIO del documento (E16-H01): un objeto con
        // las claves del usuario, no una proyección de campos conocidos. Sin bloque → `{}`.
        let frontmatter =
            wants("frontmatter").then(|| parsed.frontmatter.clone().unwrap_or_default().value);
        let body = wants("body").then(|| match sections {
            Some(secs) if !secs.is_empty() => model::extract_sections(&parsed.body, secs),
            _ => parsed.body.clone(),
        });
        let outgoing_links = wants("outgoingLinks").then(|| {
            doc_set
                .analyze()
                .outgoing
                .get(&path)
                .cloned()
                .unwrap_or_default()
        });
        let backlinks = wants("backlinks").then(|| doc_set.backlinks(&path));
        let diagnostics = wants("diagnostics").then(|| {
            doc_set
                .analyze()
                .diagnostics
                .get(&path)
                .cloned()
                .unwrap_or_default()
        });
        let external_references = if wants("externalReferences") {
            let report = self
                .workspace
                .external_refs(&path)
                .map_err(|e| workspace_error_code(&e))?;
            Some(report.references)
        } else {
            None
        };

        Ok(DocumentView {
            path,
            revision,
            frontmatter,
            body,
            outgoing_links,
            backlinks,
            external_references,
            diagnostics,
        })
    }

    /// Inspección genérica de metadata (E20-H03, `ARCHITECTURE.md §20.10`, `REFACTOR_PHASE_2 §Fase
    /// 6`): lo que un agente consulta para **comprender las convenciones de una base desconocida sin
    /// necesitar un schema**. Sustituye a `schema_inspect` (retirado con `core::schema`).
    ///
    /// Dos modos, ambos servidos por las funciones **puras** del core (`core::metadata`, una sola
    /// verdad de qué es un campo y de qué tipo, invariante #3):
    /// - `"catalog"` → [`metadata::catalog`]: por cada `field_path` que aparece en algún documento,
    ///   en cuántos documentos aparece y qué tipos toma (`MetadataCatalog`).
    /// - `"field"` → [`metadata::inspect_field`]: para un `field` dado (dot-path, p. ej.
    ///   `"service.tier"`), presencia/ausencia, tipos y valores escalares frecuentes
    ///   (`FieldInspection`). Requiere el parámetro `field`; su ausencia o un dot-path inválido →
    ///   `Err(ErrorCode::InvalidSchema)`.
    ///
    /// Un `mode` sin reconocer → `Err(ErrorCode::InvalidSchema)` (nunca entra en pánico). Un
    /// workspace sin frontmatter en ningún documento NO es un error: el catálogo sale vacío.
    ///
    /// `Result<_, ErrorCode>` (no `WorkspaceError`) — mismo patrón que [`App::knowledge_get`]: es un
    /// servicio de cara a la fachada MCP/CLI, y el catálogo de códigos estables es lo que el llamante
    /// necesita para el wire de error.
    pub fn metadata_inspect(
        &self,
        mode: &str,
        field: Option<&str>,
    ) -> Result<MetadataInspection, ErrorCode> {
        let doc_set = self
            .workspace
            .document_set()
            .map_err(|e| workspace_error_code(&e))?;

        match mode {
            "catalog" => Ok(MetadataInspection::Catalog(metadata::catalog(&doc_set))),
            "field" => {
                let field = field.ok_or(ErrorCode::InvalidSchema)?;
                let field_path = FieldPath::parse(field).map_err(|_| ErrorCode::InvalidSchema)?;
                Ok(MetadataInspection::Field(metadata::inspect_field(
                    &doc_set,
                    &field_path,
                )))
            }
            _ => Err(ErrorCode::InvalidSchema),
        }
    }

    /// Audita el conocimiento con scopes y severidad mínima (E10-H12, `ARCHITECTURE.md §19.6`,
    /// `REFACTOR §10/§17`), respondiendo la pregunta de `§20.9`: *"¿puede Lodestar interpretar y
    /// modificar este workspace de forma consistente y segura?"* (no *"¿cumple una especificación
    /// documental?"*).
    ///
    /// **Composición de diagnósticos** (invariante #3 — una sola verdad computada): por cada
    /// documento (`Analysis::documents`) se toman sus diagnósticos de documento
    /// (`Analysis::diagnostics`). Tras E20-H03 la validación schema-driven (`SCHEMA-*`/`REL-*`) se
    /// retiró con `core::schema`. **En scope workspace** se añaden además los **diagnósticos de
    /// descubrimiento** (`§20.9`, E20-H04): `DOC-NOT-UTF8`, `DOC-TOO-LARGE`, `SYMLINK-UNSUPPORTED`,
    /// `PATH-NOT-UTF8` y las colisiones de capitalización del inventario, que describen ficheros que
    /// Lodestar no pudo incorporar (su objetivo **no** es un documento, o no tienen objetivo) y por
    /// eso el recorrido por `Analysis::documents` no los vería. Los checks `Pass` se descartan.
    ///
    /// **Política de severidad** (`§20.9`, E20-H04): cada diagnóstico se reclasifica por
    /// [`lodestar_workspace::config::ValidationSection::effective_severity`] según la sección
    /// `validation` de la config — un override (p. ej. `caseMismatch: error`) reclasifica **cada**
    /// diagnóstico de esa familia, venga del documento o del descubrimiento; una familia a `ignore`
    /// lo **suprime**. Con la config por defecto (los defaults de `§20.9` coinciden con las
    /// severidades hardcodeadas) es la identidad.
    ///
    /// **Scopes** (`scope`): `workspace` = todos los documentos; `document{ref}` = solo ese documento
    /// (resuelto con [`App::resolve_ref`], `DOCUMENT_NOT_FOUND` si no existe); `paths{paths}` = esos
    /// paths; `affected{refs,depth}` = el vecindario a distancia ≤ `depth` de cada `ref`
    /// (`DocumentSet::neighborhood(_, depth, Direction::Both)`, unión de los nodos alcanzados más los
    /// propios refs) — los documentos desconectados quedan fuera.
    ///
    /// **IDs estables dentro de una revisión**: cada diagnóstico lleva
    /// `diag:blake3:<hex>` con `hex = blake3(path ‖ 0x00 ‖ code ‖ 0x00 ‖ range ‖ 0x00 ‖ msg)`.
    /// Como solo depende de los datos del diagnóstico (nunca de timestamps/orden/caché), la misma
    /// revisión produce los mismos `id` incluso entre procesos frescos (criterio `check_ids_estables`).
    ///
    /// `summary` (errors/warnings/info) y `conformant` (== `errors == 0`) se computan sobre **todo**
    /// el conjunto de diagnósticos del scope, antes de aplicar `minimum_severity` o la paginación —
    /// son un agregado del scope, no de la página devuelta. `minimum_severity` (por defecto `Info`,
    /// que ya excluye los `Pass`) eleva el umbral de lo que se **devuelve** en `diagnostics`.
    /// `include_suggested_fixes == false` vacía `fixes` (hoy siempre vacío: ningún check propone
    /// fixes tras el retiro de `REL-TARGET` en E20-H03). `limit`/`cursor` paginan de forma determinista sobre el
    /// orden total estable `(anchor, code, id)` —el `anchor` es el path del documento, o el primer
    /// `target` del diagnóstico de descubrimiento— (mismo patrón de cursor-offset opaco que
    /// `knowledge_search`); `limit` por defecto 100 (`REFACTOR §10`), `next_cursor` `None` al agotar.
    pub fn knowledge_check(
        &self,
        scope: &CheckScope,
        minimum_severity: Option<Severity>,
        include_suggested_fixes: bool,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<CheckReport, ErrorCode> {
        let (doc_set, discovery_diagnostics) = self
            .workspace
            .document_set_with_discovery()
            .map_err(|e| workspace_error_code(&e))?;
        let analysis = doc_set.analyze();
        let cfg = self.workspace.config();
        // Política de severidad por familia (`§20.9`, E20-H04): reclasifica o suprime cada
        // diagnóstico según `validation`. Con la config por defecto es la identidad.
        let validation = &cfg.validation;

        let revision = workspace_revision(doc_set.files(), &cfg.workspace.writable_roots);

        // Conjunto de paths del scope.
        let allowed = self.scope_paths(&doc_set, analysis, scope)?;

        // Compón (anchor, check) por cada documento del scope, con id estable. El `anchor` es el
        // path del documento (para el orden determinista); en los diagnósticos de descubrimiento sin
        // documento-objetivo es su primer `target` (o cadena vacía si no tiene ninguno).
        let mut items: Vec<(String, Check)> = Vec::new();
        for path in &analysis.documents {
            if !allowed.contains(path) {
                continue;
            }
            let checks: Vec<Check> = analysis.diagnostics.get(path).cloned().unwrap_or_default();
            for mut check in checks {
                // Los `Pass` no son diagnósticos: no computan en summary ni se devuelven.
                if check.level == Severity::Pass {
                    continue;
                }
                // Aplica la política de severidad; `None` (familia a `ignore`) suprime el diagnóstico.
                let Some(level) = validation.effective_severity(&check) else {
                    continue;
                };
                check.level = level;
                check.id = Some(diagnostic_id(path.as_str(), &check));
                if !include_suggested_fixes {
                    check.fixes.clear();
                }
                items.push((path.as_str().to_string(), check));
            }
        }

        // Diagnósticos de **descubrimiento** (`§20.9`, E20-H04): son de workspace, no de documento
        // (su objetivo no está en `analysis.documents` —un `.md` no-UTF8, un symlink— o no tiene
        // objetivo —`PATH-NOT-UTF8`—), así que el bucle de arriba, que itera `analysis.documents`,
        // nunca los vería. Se añaden **solo** en scope workspace: describen el inventario entero.
        if matches!(scope, CheckScope::Workspace) {
            for mut check in discovery_diagnostics {
                if check.level == Severity::Pass {
                    continue;
                }
                let Some(level) = validation.effective_severity(&check) else {
                    continue;
                };
                check.level = level;
                let anchor = check
                    .targets
                    .first()
                    .map(|t| t.as_str().to_string())
                    .unwrap_or_default();
                check.id = Some(diagnostic_id(&anchor, &check));
                if !include_suggested_fixes {
                    check.fixes.clear();
                }
                items.push((anchor, check));
            }
        }

        // Summary/conformant sobre TODO el scope (antes de minimum_severity y paginación).
        let errors = items
            .iter()
            .filter(|(_, c)| c.level == Severity::Err)
            .count();
        let warnings = items
            .iter()
            .filter(|(_, c)| c.level == Severity::Warn)
            .count();
        let info = items
            .iter()
            .filter(|(_, c)| c.level == Severity::Info)
            .count();
        let conformant = errors == 0;

        // Umbral de severidad para lo que se DEVUELVE (por defecto Info, que ya excluye Pass).
        let floor = minimum_severity.unwrap_or(Severity::Info);
        items.retain(|(_, c)| c.level >= floor);

        // Orden total estable para paginación determinista: (anchor, code, id).
        items.sort_by(|(pa, ca), (pb, cb)| {
            pa.cmp(pb)
                .then_with(|| ca.code.as_str().cmp(cb.code.as_str()))
                .then_with(|| ca.id.cmp(&cb.id))
        });

        let diagnostics_all: Vec<Check> = items.into_iter().map(|(_, c)| c).collect();
        let total = diagnostics_all.len();
        let limit = limit.unwrap_or(DEFAULT_CHECK_LIMIT).min(MAX_CHECK_LIMIT);
        let start = cursor.map(decode_cursor).unwrap_or(0).min(total);
        let end = start.saturating_add(limit).min(total);
        let page = diagnostics_all.get(start..end).unwrap_or(&[]).to_vec();
        let next_cursor = (end > start && end < total).then(|| encode_cursor(end));

        Ok(CheckReport {
            conformant,
            summary: CheckSummary {
                errors,
                warnings,
                info,
            },
            diagnostics: page,
            workspace_revision: revision,
            next_cursor,
        })
    }

    /// Resuelve el conjunto de paths que abarca un [`CheckScope`] (E10-H12). Ver la doc de
    /// [`App::knowledge_check`] para la semántica de cada variante.
    fn scope_paths(
        &self,
        doc_set: &DocumentSet,
        analysis: &Analysis,
        scope: &CheckScope,
    ) -> Result<BTreeSet<RelPath>, ErrorCode> {
        match scope {
            CheckScope::Workspace => Ok(analysis.documents.iter().cloned().collect()),
            CheckScope::Document { r#ref } => {
                let path = self.resolve_ref(r#ref)?;
                Ok(std::iter::once(path).collect())
            }
            CheckScope::Paths { paths } => Ok(paths.iter().cloned().collect()),
            CheckScope::Affected { refs, depth } => {
                let mut set: BTreeSet<RelPath> = BTreeSet::new();
                for r in refs {
                    let path = self.resolve_ref(r)?;
                    let nb = doc_set.neighborhood(&path, *depth, Direction::Both);
                    for node in &nb.nodes {
                        set.insert(node.id.clone());
                    }
                    set.insert(path);
                }
                Ok(set)
            }
        }
    }

    /// Computa el `Analysis` del working tree que alimenta la **salida** de `lodestar check`
    /// (`--json`/`--sarif`/humano). Tras E20-H03 es exactamente el `Analysis` de
    /// [`DocumentSet::analyze`] (`§20.9`): la validación schema-driven (`SCHEMA-*`/`REL-*`) se retiró
    /// con `core::schema`, así que ya no hay diagnósticos que fusionar. Se conserva el método (en vez
    /// de exponer `analyze()` directo) para no acoplar la CLI al `Workspace` y dejar un único punto
    /// por si futuras historias vuelven a componer más fuentes de diagnóstico.
    pub fn full_analysis(&self) -> Result<Analysis, ErrorCode> {
        let doc_set = self
            .workspace
            .document_set()
            .map_err(|e| workspace_error_code(&e))?;
        Ok(doc_set.analyze().clone())
    }

    /// Consulta el grafo, consolidando en una sola tool lo que hoy son 4 tools separadas
    /// (`find_backlinks`/`neighborhood`/`find_orphans`/`find_dangling`, E11-H01,
    /// `ARCHITECTURE.md §19.6`, `REFACTOR §9.5/§15`).
    ///
    /// `operation` ∈ `"backlinks"`/`"outgoing"`/`"neighborhood"`/`"isolated"`/`"dangling"`:
    /// - `backlinks`/`outgoing`/`neighborhood` requieren `r` (resuelto con [`App::resolve_ref`]);
    ///   su ausencia es `Err(ErrorCode::DocumentNotFound)` — no hay un código de "falta parámetro"
    ///   dedicado en el catálogo de 16 códigos estables, y es el mismo error que produciría un
    ///   `ref` que no resuelve, así que reusarlo aquí no inventa semántica nueva.
    /// - `backlinks` reusa [`DocumentSet::backlinks`] (invariante #3, "una sola verdad computada"):
    ///   `nodes` = el propio documento + sus fuentes entrantes (`inbound`); `edges` = fuente→ref.
    /// - `outgoing` reusa [`DocumentSet::neighborhood`] con `Direction::Out` a profundidad 1: mismo
    ///   tratamiento de dangling que `graph_model`/`neighborhood` (invariante #3), así que no
    ///   reimplementa ese criterio en esta capa.
    /// - `neighborhood` reexpone [`DocumentSet::neighborhood`]`(ref, depth, direction)` **tal cual**
    ///   (paridad exacta con el core — el criterio `graph_neighborhood_paridad` lo compara
    ///   directamente contra la salida del core). `depth` por defecto 1; `direction` por defecto
    ///   `"out"` (cualquier valor no reconocido cae también a `Out`, mismo criterio que la tool
    ///   heredada `neighborhood`).
    /// - `isolated`/`dangling` no requieren `r`: se computan de [`Analysis::isolated`]/
    ///   [`Analysis::dangling`] directamente. `isolated` (antes `orphans`, E16-H02: documentos sin
    ///   enlaces entrantes NI salientes) no tiene `edges` — por definición no hay ninguna que
    ///   mostrar; `dangling` empareja cada target colgante con las aristas `origen→target` que lo
    ///   referencian (recorriendo `Analysis::out`).
    ///
    /// **Operaciones estructurales (E11-H02)**, funciones puras del core reexpuestas en la misma
    /// forma `{nodes,edges}` (invariante #3):
    /// - `path_between` requiere `r` (origen) y `to` (destino); reusa [`DocumentSet::path_between`]
    ///   (camino más corto dirigido). `nodes` = los nodos del camino, `edges` = los enlaces
    ///   consecutivos `[a→..→b]`. Si algún ref no resuelve → `Err(ErrorCode::DocumentNotFound)`; si
    ///   no hay camino, `nodes`/`edges` vacíos (nunca error). **Nota**: la paginación genérica
    ///   ordena `nodes` por `id`, así que el orden del camino se recupera de `edges`, no de `nodes`.
    /// - `cycles` no requiere `r`: reusa [`DocumentSet::cycles`]. `nodes` = la unión de los nodos que
    ///   participan en algún ciclo (SCC no trivial); `edges` = los enlaces del grafo internos a ese
    ///   conjunto. La partición en ciclos concretos la da el core; aquí se sirve el subgrafo cíclico
    ///   agregado (coherente con la forma `{nodes,edges}` de esta tool).
    /// - `components` no requiere `r`: reusa [`DocumentSet::components`]. Como las componentes conexas
    ///   particionan **todo** el grafo, se sirve el grafo completo (`nodes`/`edges` de
    ///   [`DocumentSet::graph_model`]); el cliente reconstruye la partición con [`DocumentSet::components`] o
    ///   recorriendo las aristas.
    ///
    /// **Paginación**: orden total y estable de `nodes` por `id` (mismo criterio que
    /// `knowledge_search`/`knowledge_check`); `limit` trunca esa página con un cursor-offset opaco
    /// (mismo esquema hex, autosuficiente entre procesos). Sin `limit`, o con `limit` mayor o igual
    /// al total, no hay truncamiento y `nextCursor` es `None`. Las `edges` devueltas se acotan a
    /// los `nodes` que sobreviven a la página (origen y destino ambos presentes), así el subgrafo
    /// que se sirve es siempre coherente consigo mismo — nunca una arista "colgando" de un nodo que
    /// la paginación dejó fuera.
    // Dispatcher de wire: cada argumento mapea 1:1 a un campo del `inputSchema` de la tool MCP
    // `graph_query` (operation/ref/to/depth/direction/limit/cursor). Agruparlos en un struct sería
    // una capa de framing paralela sin valor; el listado plano es el contrato.
    #[allow(clippy::too_many_arguments)]
    pub fn graph_query(
        &self,
        operation: &str,
        r: Option<&DocumentRef>,
        to: Option<&DocumentRef>,
        depth: Option<u32>,
        direction: Option<&str>,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<GraphQueryResult, ErrorCode> {
        let doc_set = self
            .workspace
            .document_set()
            .map_err(|e| workspace_error_code(&e))?;

        let (mut nodes, mut edges): (Vec<GraphNode>, Vec<Edge>) = match operation {
            "backlinks" => {
                let path = self.resolve_ref(r.ok_or(ErrorCode::DocumentNotFound)?)?;
                let bl = doc_set.backlinks(&path);
                let mut ids: BTreeSet<RelPath> = BTreeSet::new();
                ids.insert(path.clone());
                for lr in &bl.inbound {
                    ids.insert(lr.from.clone());
                }
                let nodes = ids.iter().map(|id| doc_set.node(id)).collect();
                // Un origen que enlaza VARIAS veces produce varias referencias entrantes pero UNA
                // sola arista: el grafo es un conjunto de aristas (E17-H04).
                let mut vistas: BTreeSet<RelPath> = BTreeSet::new();
                let edges = bl
                    .inbound
                    .iter()
                    .filter(|lr| vistas.insert(lr.from.clone()))
                    .map(|lr| Edge {
                        source: lr.from.clone(),
                        target: path.clone(),
                        dangling: false,
                    })
                    .collect();
                (nodes, edges)
            }
            "outgoing" => {
                let path = self.resolve_ref(r.ok_or(ErrorCode::DocumentNotFound)?)?;
                let nb = doc_set.neighborhood(&path, 1, Direction::Out);
                (nb.nodes, nb.edges)
            }
            "neighborhood" => {
                let path = self.resolve_ref(r.ok_or(ErrorCode::DocumentNotFound)?)?;
                let dir = match direction {
                    Some("in") => Direction::In,
                    Some("both") => Direction::Both,
                    _ => Direction::Out,
                };
                let nb = doc_set.neighborhood(&path, depth.unwrap_or(1), dir);
                (nb.nodes, nb.edges)
            }
            "isolated" => {
                let a = doc_set.analyze();
                let nodes = a.isolated.iter().map(|id| doc_set.node(id)).collect();
                (nodes, Vec::new())
            }
            "dangling" => {
                let a = doc_set.analyze();
                // Cada colgante ya trae su origen y su destino (E17-H04); dos enlaces rotos del
                // mismo origen al mismo destino son una sola arista.
                let mut vistas: BTreeSet<(RelPath, RelPath)> = BTreeSet::new();
                let edges: Vec<Edge> = a
                    .dangling
                    .iter()
                    .filter(|d| vistas.insert((d.from.clone(), d.target.clone())))
                    .map(|d| Edge {
                        source: d.from.clone(),
                        target: d.target.clone(),
                        dangling: true,
                    })
                    .collect();
                let ids: BTreeSet<RelPath> = a.dangling.iter().map(|d| d.target.clone()).collect();
                let nodes = ids.iter().map(|id| doc_set.node(id)).collect();
                (nodes, edges)
            }
            "path_between" => {
                let from = self.resolve_ref(r.ok_or(ErrorCode::DocumentNotFound)?)?;
                let dest = self.resolve_ref(to.ok_or(ErrorCode::DocumentNotFound)?)?;
                let path = doc_set.path_between(&from, &dest);
                let nodes = path.iter().map(|id| doc_set.node(id)).collect();
                // Aristas consecutivas del camino; `dangling` si el destino no es un fichero real.
                let edges = path
                    .windows(2)
                    .map(|w| Edge {
                        source: w[0].clone(),
                        target: w[1].clone(),
                        dangling: !doc_set.files().contains_key(&w[1]),
                    })
                    .collect();
                (nodes, edges)
            }
            "cycles" => {
                // Unión de los nodos que participan en algún ciclo (SCC no trivial).
                let en_ciclo: BTreeSet<RelPath> = doc_set.cycles().into_iter().flatten().collect();
                let nodes = en_ciclo.iter().map(|id| doc_set.node(id)).collect();
                // Aristas del grafo internas al conjunto cíclico.
                let edges = doc_set
                    .graph_model()
                    .edges
                    .into_iter()
                    .filter(|e| en_ciclo.contains(&e.source) && en_ciclo.contains(&e.target))
                    .collect();
                (nodes, edges)
            }
            "components" => {
                // Las componentes particionan todo el grafo: se sirve el grafo completo y el
                // cliente reconstruye la partición (DocumentSet::components) si la necesita.
                let model = doc_set.graph_model();
                (model.nodes, model.edges)
            }
            // Ninguna historia ejerce todavía una `operation` fuera de las anteriores; mismo
            // criterio que `metadata_inspect` para un `mode` no reconocido — no hay un código de
            // "argumento inválido" dedicado en el catálogo de 16.
            _ => return Err(ErrorCode::InvalidSchema),
        };

        // Orden total y estable por `id` — paginación reproducible entre procesos frescos.
        nodes.sort_by(|a, b| a.id.cmp(&b.id));

        let total = nodes.len();
        let start = cursor.map(decode_cursor).unwrap_or(0).min(total);
        let end = match limit {
            Some(l) => start.saturating_add(l).min(total),
            None => total,
        };
        let truncated = end < total;
        let next_cursor = truncated.then(|| encode_cursor(end));
        let page_nodes: Vec<GraphNode> = nodes.get(start..end).unwrap_or(&[]).to_vec();
        let page_ids: BTreeSet<&RelPath> = page_nodes.iter().map(|n| &n.id).collect();
        edges.retain(|e| page_ids.contains(&e.source) && page_ids.contains(&e.target));

        Ok(GraphQueryResult {
            summary: GraphQuerySummary {
                node_count: page_nodes.len(),
                edge_count: edges.len(),
                truncated,
            },
            nodes: page_nodes,
            edges,
            next_cursor,
        })
    }

    /// Analiza el **impacto** de un cambio hipotético sobre un documento sin materializarlo
    /// (E11-H05, `ARCHITECTURE.md §19.6`/`§20.10`, `REFACTOR §9.6/§17`): cuántos documentos se
    /// verían afectados directa y transitivamente, y un nivel de riesgo derivado. No materializa
    /// ningún cambio (aplicar es E12/E13).
    ///
    /// **E17-H05**: el impacto se calcula **solo sobre el grafo de enlaces**. Los tipos y las
    /// relaciones tipadas del `schema.yaml` dejaron de mirarse (`§20.10`: una relación es un
    /// enlace Markdown y nada más), así que un workspace con `type:` y relaciones declaradas
    /// produce exactamente el mismo informe que uno sin nada de eso.
    ///
    /// - `directlyAffected` = nº de backlinks **directos** entrantes del `ref`
    ///   ([`DocumentSet::backlinks`]`.inbound`).
    /// - `transitivelyAffected` = tamaño del blast-radius entrante
    ///   ([`DocumentSet::neighborhood`]`(_, _, Direction::In)`, excluido el propio `ref`) — la **verdad
    ///   del core** (invariante #3); `Store::blast_radius` es la proyección SQL equivalente,
    ///   verificada idéntica por el test `impacto_paridad_core`.
    /// - `blockingReferences`: **siempre vacío** desde E17-H05. Los bloqueos derivados de
    ///   relaciones tipadas obligatorias desaparecieron con el modelo que los definía; el campo se
    ///   conserva en el wire (`contracts/mcp.yml`) aun tras el retiro de `core::schema` (E20-H03),
    ///   para no romper a un cliente que lo lea — su retirada es una historia propia.
    /// - `risk`: `"high"` si el nº de afectados directos es alto; `"medium"` para un impacto
    ///   moderado; `"low"` en caso contrario.
    ///
    /// `kind` está restringido a las operaciones que `§20.10` lista para impacto: `move` y `delete`.
    /// E21-H01 retiró los `kind` semánticos (`deprecate`/`transition_status`/`change_relation`/
    /// `replace_document`) del contrato — un `kind` fuera de `{move, delete}` es un esquema de entrada
    /// inválido → `Err(ErrorCode::InvalidSchema)`.
    ///
    /// `Err(ErrorCode::DocumentNotFound)` si el `ref` no resuelve a un documento
    /// ([`App::resolve_ref`]).
    pub fn impact_analyze(
        &self,
        r: &DocumentRef,
        kind: &str,
        depth: Option<u32>,
    ) -> Result<ImpactReport, ErrorCode> {
        // E21-H01: `kind` restringido a las operaciones de impacto del modelo universal (`§20.10`).
        // Los `kind` semánticos retirados caen aquí como esquema de entrada inválido.
        if kind != "move" && kind != "delete" {
            return Err(ErrorCode::InvalidSchema);
        }
        let path = self.resolve_ref(r)?;
        let doc_set = self
            .workspace
            .document_set()
            .map_err(|e| workspace_error_code(&e))?;

        // `directlyAffected`: backlinks DIRECTOS entrantes (verdad del core).
        let directly_affected = doc_set.backlinks(&path).inbound.len();

        // `transitivelyAffected`: blast-radius entrante (`neighborhood(In)`), excluido el propio
        // `ref`. Profundidad grande por defecto para cubrir todo el alcance transitivo, no solo el
        // vecindario inmediato (paridad con `Store::blast_radius`, invariante #3).
        let nb = doc_set.neighborhood(&path, depth.unwrap_or(u32::MAX), Direction::In);
        let mut affected_documents: Vec<RelPath> = nb
            .nodes
            .into_iter()
            .map(|n| n.id)
            .filter(|id| id != &path)
            .collect();
        affected_documents.sort();
        let transitively_affected = affected_documents.len();

        // `blockingReferences`: vacío por construcción (E17-H05). Se conserva en el wire —y con él
        // su contador— aun tras el retiro de `core::schema` (E20-H03); ya no hay nada que lo alimente,
        // porque una relación es un enlace Markdown y un enlace roto no bloquea, se reporta.
        let blocking_references: Vec<BlockingReference> = Vec::new();

        // Riesgo derivado del GRAFO (conjunto cerrado {"low","medium","high"}, wire en inglés).
        let risk = if directly_affected >= HIGH_IMPACT_BACKLINKS {
            "high"
        } else if directly_affected >= MEDIUM_IMPACT_BACKLINKS
            || transitively_affected >= MEDIUM_IMPACT_BACKLINKS
        {
            "medium"
        } else {
            "low"
        };

        // Recomendaciones accionables (texto español); vacías para un cambio de bajo riesgo. Solo
        // hablan de ENLACES: el vocabulario de tipos y relaciones dejó de existir (`§20.3`).
        let mut recommendations = Vec::new();
        if directly_affected > 0 {
            recommendations.push(format!(
                "Revisa los {directly_affected} enlaces entrantes que apuntan a este documento tras aplicar «{kind}»."
            ));
        }

        Ok(ImpactReport {
            summary: ImpactSummary {
                directly_affected,
                transitively_affected,
                blocking_references: blocking_references.len(),
                risk: risk.to_string(),
            },
            affected_documents,
            blocking_references,
            recommendations,
        })
    }

    /// Orquesta un plan de cambios (`change_plan`, E12-H08, `ARCHITECTURE.md §19.5/§19.6`): normaliza
    /// las operaciones propuestas, simula su aplicación sobre un `DocumentSet` **en memoria** y valida el
    /// resultado — **sin tocar disco** (invariante #1 de `CLAUDE.md`; la escritura real es E13).
    ///
    /// Pasos:
    /// 1. Toma el workspace actual (`Workspace::document_set`, en memoria) y calcula
    ///    `baseWorkspaceRevision` = [`workspace_revision`] sobre las raíces escribibles. Si
    ///    `expected_workspace_revision` viene y **no** coincide → [`ErrorCode::RevisionConflict`]
    ///    (control optimista a nivel de workspace); si viene `None`, se adopta la revisión actual.
    /// 2. **Control optimista por operación**: cada op cruda con `expectedRevision` se compara con la
    ///    [`DocumentRevision`] actual del documento objetivo (`blake3` del `.md` en disco/memoria); si
    ///    difiere (o el documento ya no existe) → [`ErrorCode::RevisionConflict`].
    /// 3. Despacha cada op cruda a su normalizador del core (E12-H05/H06/H07 y los de contenido
    ///    `patch_frontmatter`/`replace_body`), acumulando TODAS las [`NormalizedOperation`] en un
    ///    **único** `ChangeSet` (una op de estructura puede producir varias).
    /// 4. Construye el workspace hipotético con [`plan::apply_normalized_ops`] y deriva
    ///    [`plan::semantic_diff`], [`plan::assess_risk`] y [`plan::validate_result`] (antes y
    ///    después); `canApply` = [`plan::can_apply`] bajo `policy`.
    ///    - **Guard de descubrimiento** (E15-H09, `REFACTOR_PHASE_2 §Principio 8`): cada path que
    ///      el plan crearía o modificaría pasa por [`Workspace::assert_discoverable`]; si el
    ///      descubrimiento lo deja fuera del inventario (`.lodestar/**`, un `.gitignore`/
    ///      `.lodestarignore` del árbol, `discovery.exclude` o el filtro `discovery.include`) →
    ///      [`ErrorCode::PermissionDenied`] y **no se persiste plan alguno**. Se rechaza aquí —y
    ///      no solo en el apply— porque un plan aceptado que revienta al aplicarse le devuelve al
    ///      agente un `semanticDiff.created` con el path colado y le hace descubrir el fallo
    ///      tarde. **Solo** se consulta el descubrimiento: `writableRoots`/`referenceRoots`
    ///      siguen comprobándose exclusivamente en el apply (E11-H04), donde vive el único
    ///      escritor.
    /// 5. **`planHash` DETERMINISTA**: `blake3(baseWorkspaceRevision ‖ 0x00 ‖ serialización JSON
    ///    canónica de las normalizedOperations)` — mismo input + misma base ⇒ mismo hash; input
    ///    distinto ⇒ hash distinto. **No** depende del reloj (`expiresAt` sí es wall-clock, pero
    ///    queda FUERA del hash). `changeSetId` se deriva del `planHash`.
    ///
    /// Devuelve un [`PlanResult`] (proyección de servicio) con el plan completo. `Err(ErrorCode)`
    /// para el wire de error (mismo patrón que el resto de servicios de `App`).
    pub fn change_plan(
        &self,
        expected_workspace_revision: Option<WorkspaceRevision>,
        raw_ops: &Value,
        policy: PlanPolicy,
    ) -> Result<PlanResult, ErrorCode> {
        let doc_set = self
            .workspace
            .document_set()
            .map_err(|e| workspace_error_code(&e))?;
        let root = self.workspace.root();
        let cfg = self.workspace.config();
        let files = doc_set.files();
        let writable = &cfg.workspace.writable_roots;

        // (1) Revisión base del workspace + control optimista a nivel de workspace.
        let base_revision = workspace_revision(files, writable);
        if let Some(expected) = &expected_workspace_revision {
            if expected != &base_revision {
                return Err(ErrorCode::RevisionConflict);
            }
        }

        let ops_arr = raw_ops.as_array().ok_or(ErrorCode::InvalidSchema)?;

        // (2)+(3) Control optimista por op y normalización, acumulando en un único change set.
        let mut normalized: Vec<NormalizedOperation> = Vec::new();
        for raw in ops_arr {
            if let Some(expected) = raw.get("expectedRevision").and_then(Value::as_str) {
                let target = op_target_path(raw)?;
                let actual = files.get(&target).map(|raw_md| {
                    DocumentRevision::from_hash(*blake3::hash(raw_md.as_bytes()).as_bytes())
                });
                if actual.as_ref().map(|r| r.0.as_str()) != Some(expected) {
                    return Err(ErrorCode::RevisionConflict);
                }
            }
            normalized.extend(normalize_raw_op(&doc_set, raw)?);
        }

        // (4) DocumentSet hipotético + análisis del plan (todo en memoria, sin escribir).
        let after_files =
            plan::apply_normalized_ops(files, &normalized).map_err(|e| error_code(&e))?;
        let after = DocumentSet::from_files(after_files);

        // (4-bis) Guard de descubrimiento (E15-H09): ningún path que el plan escribiría puede
        //     quedar fuera del inventario. Se comprueban los creados/modificados —los borrados
        //     estaban en el inventario por construcción— y se hace ANTES de persistir el plan, de
        //     modo que un plan rechazado no queda aplicable después. Nótese que aquí NO se llama a
        //     `assert_writable`: las raíces de la config se siguen juzgando en el apply.
        for (path, contenido) in after.files() {
            if files.get(path) != Some(contenido) {
                self.workspace
                    .assert_discoverable(path)
                    .map_err(|e| workspace_error_code(&e))?;
            }
        }

        let risk = plan::assess_risk(&normalized, &doc_set, &after);
        let semantic_diff = plan::semantic_diff(&doc_set, &after);
        let before_report = plan::validate_result(&doc_set);
        let after_report = plan::validate_result(&after);
        let can_apply = plan::can_apply(&after_report, &policy);
        let impact = PlanImpact::from_diff(&semantic_diff);

        // (5) planHash determinista (independiente del reloj) + id derivado.
        let plan_hash = compute_plan_hash(&base_revision, &normalized);
        let change_set_id = ChangeSetId(format!(
            "changeset:{}",
            plan_hash.0.strip_prefix("blake3:").unwrap_or(&plan_hash.0)
        ));

        let result = PlanResult {
            change_set_id,
            base_workspace_revision: base_revision,
            plan_hash,
            can_apply,
            expires_at: expires_at_string(),
            normalized_operations: normalized,
            risk,
            semantic_diff,
            impact,
            diagnostics_before: before_report.summary,
            diagnostics_after: after_report.summary,
        };

        // (6) Persistencia en runtime (E12-H09, `ARCHITECTURE.md §19.4/§19.5`): un plan exitoso se
        // guarda entero en `.lodestar/runtime/plans/` para que `load_plan` (y, más adelante,
        // `change_apply`, E13) lo recupere por `changeSetId`. Es runtime — gitignored, fuera de
        // `WorkspaceRevision` (E9-H06/E10-H03) — así que NO usa el único-escritor atómico de
        // `lodestar_workspace::io` (ese protocolo protege el conocimiento canónico, no el scratch).
        persist_plan(root, &result)?;

        Ok(result)
    }

    /// Carga el plan persistido `id` desde `.lodestar/runtime/plans/` (E12-H09,
    /// `ARCHITECTURE.md §19.4/§19.5`).
    ///
    /// `Err(ErrorCode::PlanStale)` si el fichero no existe, no se puede leer o no deserializa a un
    /// `PlanResult` válido — el wire no distingue "changeSetId desconocido" de "runtime purgado" y
    /// `PLAN_STALE` es el código ya reservado para "este plan ya no es utilizable" (E12-H08 lo deja
    /// declarado y sin emisor; aquí gana su primer uso real).
    ///
    /// `Err(ErrorCode::PlanExpired)` si `expiresAt` (segundos epoch, wall-clock) ya quedó en el
    /// pasado respecto de `SystemTime::now()`. El reloj de pared vive aquí, en la fachada de `app`
    /// — **nunca** en `lodestar-core`, que es puro y no puede depender del reloj del sistema
    /// (invariante #2 de `CLAUDE.md`).
    ///
    /// Si el plan existe y está vigente, `Ok(PlanResult)` con el contenido persistido tal cual
    /// (mismo `planHash` que devolvió `change_plan`).
    pub fn load_plan(&self, id: &ChangeSetId) -> Result<PlanResult, ErrorCode> {
        let path = plan_file_path(self.workspace.root(), id);
        let raw = std::fs::read(&path).map_err(|_| ErrorCode::PlanStale)?;
        let plan: PlanResult = serde_json::from_slice(&raw).map_err(|_| ErrorCode::PlanStale)?;

        let expires_at: u64 = plan.expires_at.parse().map_err(|_| ErrorCode::PlanStale)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if expires_at < now {
            return Err(ErrorCode::PlanExpired);
        }

        Ok(plan)
    }

    /// Aplica un plan previamente calculado y vigente por el ÚNICO ESCRITOR, con todas las
    /// salvaguardas de la publicación recuperable (`change_apply`, E13-H08,
    /// `ARCHITECTURE.md §19.5/§19.6`, `REFACTOR §11.2`). Es la orquestación de servicio que rodea a
    /// la mecánica transaccional de [`Workspace::apply_transaction`] con los pasos de **plan**:
    ///
    /// 1. **Cargar el plan** persistido por `change_set_id` ([`App::load_plan`], E12-H09) →
    ///    `Err(PlanExpired)` si caducó, `Err(PlanStale)` si no existe/es ilegible.
    /// 2. **Control optimista de workspace**: si viene `expected_workspace_revision` y no coincide
    ///    con la revisión actual → `Err(RevisionConflict)`.
    /// 3. **Verificar `planHash`**: recomputa el hash determinista sobre la base ACTUAL del workspace
    ///    (`compute_plan_hash(revisión_actual, plan.normalizedOperations)`, la misma función que
    ///    `change_plan`) y lo compara con el `planHash` persistido; si difiere, el workspace cambió bajo
    ///    el plan → `Err(PlanStale)` y **no escribe**. (El `planHash` mezcla la base y las
    ///    operaciones, así que un cambio del canónico bajo el plan lo invalida.)
    /// 4. **Transacción**: [`Workspace::apply_transaction`] publica por el único escritor (staging →
    ///    lock → backup → journal → renames atómicos), devolviendo `(previous, result, changedPaths)`.
    ///    Su `assert_writable` rechaza cualquier path fuera de `writableRoots` → `PERMISSION_DENIED`
    ///    ANTES de tocar el canónico.
    /// 5. **Receipt + GC**: persiste el [`ChangeReceipt`] de la aplicación completada (E13-H07) y
    ///    ejecuta la retención (`gc_receipts`).
    /// 6. Devuelve un [`ApplyResult`] (proyección de servicio) con `applied:true`, las revisiones
    ///    antes/después, los paths cambiados, el `semanticDiff` del plan y la conformidad post-apply.
    ///
    /// # Mapeo de error y la reserva `WorkspaceError::Core` (E10-H02)
    /// Los errores de la transacción se mapean con [`workspace_error_code`]. El rechazo por permisos
    /// llega como [`WorkspaceError::PermissionDenied`] **directo** (lo emite `assert_writable` ANTES
    /// de cualquier operación que aplane un `CoreError` a texto), así que **preserva** su código wire
    /// `PERMISSION_DENIED` — la reserva de E10-H02 (un `WorkspaceError::Core` que degradaría un
    /// permiso denegado a `INTERNAL_IO_ERROR` al aplanar el `CoreError`) no se materializa aquí
    /// gracias al **orden** de la transacción (guard de escritura antes de publicar), no a un cambio
    /// del aplanamiento. `change_apply` no introduce ningún camino donde un permiso denegado pase por
    /// `WorkspaceError::Core`.
    ///
    /// # Auditoría (E13-H10, `ARCHITECTURE.md §19.7`)
    /// Cada intento (éxito **o** fallo, incluidos los rechazos de los pasos 1-4 que abortan ANTES de
    /// publicar) anexa una línea a `.lodestar/runtime/audit.jsonl` — ver `App::audit`. Es
    /// diagnóstico local, best-effort: nunca tumba el apply ni enmascara su error original. Delegado
    /// en `App::change_apply_uncounted`, que conserva la lógica de publicación intacta; este método
    /// público es solo el wrapper que garantiza que **ningún** `Err` se devuelve sin auditar primero.
    pub fn change_apply(
        &self,
        change_set_id: &ChangeSetId,
        expected_workspace_revision: Option<WorkspaceRevision>,
    ) -> Result<ApplyResult, ErrorCode> {
        let outcome = self.change_apply_uncounted(change_set_id, expected_workspace_revision);
        self.audit(audit_entry_for_apply(change_set_id, &outcome));
        outcome
    }

    /// Lógica real de `change_apply` (E13-H08) — ver el rustdoc de [`App::change_apply`], que la
    /// envuelve con la auditoría de E13-H10 sin alterar su comportamiento de éxito/error.
    fn change_apply_uncounted(
        &self,
        change_set_id: &ChangeSetId,
        expected_workspace_revision: Option<WorkspaceRevision>,
    ) -> Result<ApplyResult, ErrorCode> {
        // (1) Cargar el plan persistido (caducidad → PLAN_EXPIRED; ausente/ilegible → PLAN_STALE).
        let plan = self.load_plan(change_set_id)?;

        let cfg = self.workspace.config();
        let doc_set = self
            .workspace
            .document_set()
            .map_err(|e| workspace_error_code(&e))?;
        let current_base = workspace_revision(doc_set.files(), &cfg.workspace.writable_roots);

        // (2) Control optimista a nivel de workspace (si el llamante fijó una expectativa).
        if let Some(expected) = &expected_workspace_revision {
            if expected != &current_base {
                return Err(ErrorCode::RevisionConflict);
            }
        }

        // (3) Verificar `planHash` sobre la base ACTUAL: si el workspace cambió bajo el plan, el hash
        //     recomputado difiere del persistido → PLAN_STALE (no se escribe).
        let recomputed = compute_plan_hash(&current_base, &plan.normalized_operations);
        if recomputed != plan.plan_hash {
            return Err(ErrorCode::PlanStale);
        }

        // (4) Publicar por el único escritor (staging → lock → backup → journal → renames). El guard
        //     `assert_writable` de la transacción rechaza fuera de `writableRoots` → PERMISSION_DENIED
        //     antes de tocar el canónico.
        let change_set = plan_to_change_set(&plan);
        let (previous, result, changed_paths) = self
            .workspace
            .apply_transaction(&change_set)
            .map_err(|e| workspace_error_code(&e))?;

        // (5) Receipt de la aplicación completada + retención (E13-H07). El `receiptId` es el mismo
        //     id de transacción derivado del `changeSetId`, así el receipt localiza sus copias de
        //     recuperación por convención de nombre.
        let receipt_id = ReceiptId(transaction_id(&plan.change_set_id));
        let receipt = ChangeReceipt {
            id: receipt_id.clone(),
            change_set_id: plan.change_set_id.clone(),
            previous_revision: previous.clone(),
            result_revision: result.clone(),
            changed_paths: changed_paths.clone(),
            semantic_diff: plan.semantic_diff.clone(),
        };
        self.workspace
            .write_receipt(&receipt)
            .map_err(|e| workspace_error_code(&e))?;
        self.workspace
            .gc_receipts()
            .map_err(|e| workspace_error_code(&e))?;

        // (6) Conformidad del workspace ya publicado (una sola verdad computada, invariante #3).
        let analysis = self
            .workspace
            .analyze()
            .map_err(|e| workspace_error_code(&e))?;

        Ok(ApplyResult {
            receipt_id,
            applied: true,
            previous_workspace_revision: previous,
            workspace_revision: result,
            changed_paths,
            semantic_diff: plan.semantic_diff,
            validation: ApplyValidation {
                conformant: analysis.hard_fail() == 0,
                errors: analysis.hard_fail(),
                warnings: analysis.warn_count(),
            },
        })
    }

    /// Revierte una transacción **reciente y no alterada** desde sus copias de recuperación
    /// (E13-H09, `ARCHITECTURE.md §19.5/§19.6`, `REFACTOR §11.3`). Es la operación inversa de
    /// [`App::change_apply`]: devuelve el conocimiento canónico al estado ANTERIOR al apply
    /// identificado por `receipt_id`, por el **único escritor** (invariante #5), como una nueva
    /// transacción inversa recuperable (su propio journal y copias de recuperación).
    ///
    /// Condiciones (E13-H09), en orden:
    /// 1. **Receipt disponible**: carga el [`ChangeReceipt`] persistido (E13-H07). Si no existe
    ///    (purgado por retención / GC), la transacción ya no es reversible → [`ErrorCode::PlanExpired`].
    ///    Se **reusa** `PLAN_EXPIRED` —el catálogo de `ErrorCode` (invariante #4) está congelado y no
    ///    tiene una variante «receipt no encontrado»— por ser el match semántico más cercano a «la
    ///    transacción registrada ya no está disponible por retención», igual que `change_apply` reusa
    ///    `PLAN_EXPIRED` para el plan persistido ausente/vencido.
    /// 2. **Control optimista de workspace** (opcional): si `expected_workspace_revision` viene y no
    ///    coincide con la revisión actual → [`ErrorCode::RevisionConflict`].
    /// 3. **Ficheros afectados no alterados**: la revisión actual del workspace debe seguir siendo la
    ///    `resultRevision` que dejó el apply; si difiere, algún fichero afectado (o cualquier otro)
    ///    cambió tras el apply → [`ErrorCode::WriteConflict`] y **no** revierte (comprobación
    ///    conservadora y suficiente para el criterio: un cambio en el conocimiento escribible mueve la
    ///    `WorkspaceRevision`).
    /// 4. **Restauración recuperable**: delega en [`Workspace::revert_transaction`], que verifica que
    ///    las copias de recuperación (E13-H04) existen y restaura por el único escritor bajo su propio
    ///    journal; luego persiste el [`ChangeReceipt`] de la reversión (transacción inversa) y ejecuta
    ///    la retención (`gc_receipts`).
    ///
    /// Devuelve un [`RevertResult`] con `reverted:true`, las revisiones antes/después de la
    /// transacción INVERSA (`previousWorkspaceRevision` == `resultRevision` del apply;
    /// `workspaceRevision` == `previousRevision` del apply, el estado restaurado) y los paths
    /// restaurados.
    ///
    /// # Auditoría (E13-H10, `ARCHITECTURE.md §19.7`)
    /// Mismo wrapper que [`App::change_apply`]: audita éxito y fallo (incluidos los rechazos de los
    /// pasos 1-3) antes de devolver, sin alterar la semántica. El `changeSetId` auditado es el del
    /// receipt cuando se logra cargar; si el receipt ya no existe (el propio motivo del fallo
    /// `PLAN_EXPIRED`), se audita con el `receiptId` como mejor identificador disponible — ver
    /// `App::revert_change_set_hint`.
    pub fn change_revert(
        &self,
        receipt_id: &ReceiptId,
        expected_workspace_revision: Option<WorkspaceRevision>,
    ) -> Result<RevertResult, ErrorCode> {
        let outcome = self.change_revert_uncounted(receipt_id, expected_workspace_revision);
        let change_set_id_hint = self.revert_change_set_hint(receipt_id);
        self.audit(audit_entry_for_revert(&change_set_id_hint, &outcome));
        outcome
    }

    /// Lógica real de `change_revert` (E13-H09) — ver el rustdoc de [`App::change_revert`], que la
    /// envuelve con la auditoría de E13-H10 sin alterar su comportamiento de éxito/error.
    fn change_revert_uncounted(
        &self,
        receipt_id: &ReceiptId,
        expected_workspace_revision: Option<WorkspaceRevision>,
    ) -> Result<RevertResult, ErrorCode> {
        // (1) Cargar el receipt persistido. Ausente/purgado ⇒ transacción no disponible → PLAN_EXPIRED.
        let receipt = self
            .workspace
            .load_receipt(receipt_id)
            .map_err(|_| ErrorCode::PlanExpired)?;

        // (2) Revisión actual del conocimiento escribible.
        let cfg = self.workspace.config();
        let doc_set = self
            .workspace
            .document_set()
            .map_err(|e| workspace_error_code(&e))?;
        let current = workspace_revision(doc_set.files(), &cfg.workspace.writable_roots);

        // (3) Control optimista a nivel de workspace (si el llamante fijó una expectativa).
        if let Some(expected) = &expected_workspace_revision {
            if expected != &current {
                return Err(ErrorCode::RevisionConflict);
            }
        }

        // (4) Ficheros afectados no alterados: el workspace sigue en la `resultRevision` del apply.
        //     Si difiere, algo cambió tras el apply → WRITE_CONFLICT y NO se revierte.
        if current != receipt.result_revision {
            return Err(ErrorCode::WriteConflict);
        }

        // (5) Restaurar por el único escritor (transacción inversa recuperable con journal propio).
        let orig_txn_id = transaction_id(&receipt.change_set_id);
        let revert_txn_id = format!("{orig_txn_id}-revert");
        let (previous, result, changed_paths) = self
            .workspace
            .revert_transaction(&orig_txn_id, &revert_txn_id)
            .map_err(|e| workspace_error_code(&e))?;

        // (6) Receipt de la reversión (inversa: previous/result intercambiados respecto al apply) +
        //     retención. Su id nombra por convención las copias de recuperación de la inversa
        //     (`recovery/<revert_txn_id>/`), que el GC purgará junto al recibo.
        let revert_receipt_id = ReceiptId(revert_txn_id);
        let revert_receipt = ChangeReceipt {
            id: revert_receipt_id.clone(),
            change_set_id: receipt.change_set_id.clone(),
            previous_revision: previous.clone(),
            result_revision: result.clone(),
            changed_paths: changed_paths.clone(),
            semantic_diff: receipt.semantic_diff.clone(),
        };
        self.workspace
            .write_receipt(&revert_receipt)
            .map_err(|e| workspace_error_code(&e))?;
        self.workspace
            .gc_receipts()
            .map_err(|e| workspace_error_code(&e))?;

        Ok(RevertResult {
            reverted: true,
            receipt_id: revert_receipt_id,
            previous_workspace_revision: previous,
            workspace_revision: result,
            changed_paths,
            semantic_diff: receipt.semantic_diff,
        })
    }

    /// Mejor identificador de `changeSetId` disponible para auditar un `change_revert` (E13-H10),
    /// sin alterar el comportamiento de [`App::change_revert_uncounted`]: intenta cargar el receipt
    /// (misma llamada que hace el paso 1 de la reversión, idempotente y de solo lectura) y devuelve
    /// su `changeSetId`; si el receipt ya no existe — precisamente el motivo típico de un fallo
    /// `PLAN_EXPIRED` — cae al `receiptId` recibido como mejor identificador disponible (se sabe
    /// siempre, independientemente de dónde falle la reversión).
    fn revert_change_set_hint(&self, receipt_id: &ReceiptId) -> String {
        self.workspace
            .load_receipt(receipt_id)
            .map(|r| r.change_set_id.0)
            .unwrap_or_else(|_| receipt_id.0.clone())
    }

    /// Anexa `entry` como una línea JSON a `.lodestar/runtime/audit.jsonl` (E13-H10,
    /// `ARCHITECTURE.md §19.7`, `REFACTOR §14`): crea `.lodestar/runtime/` si falta y abre el
    /// fichero en modo `append` (nunca reescribe líneas previas — JSONL que solo crece).
    ///
    /// **Best-effort y silencioso para el llamante**: la auditoría es diagnóstico local, NUNCA debe
    /// tumbar una operación de escritura ni enmascarar su error original (regla de la historia). Un
    /// fallo al escribir (permisos, disco lleno, …) se reporta por stderr y se descarta — mismo
    /// criterio que `gitignore::ensure_gitignore`/`runtime::ensure_runtime_scaffold` en
    /// `lodestar-workspace`. Es runtime puro: gitignored, fuera de `WorkspaceRevision`
    /// (E9-H06/E10-H03), no indexado y no expuesto por ninguna tool MCP (solo diagnóstico local).
    fn audit(&self, entry: AuditEntry) {
        if let Err(e) = try_append_audit(self.workspace.root(), &entry) {
            eprintln!(
                "lodestar: aviso: no se pudo anexar la auditoría de `{}`: {e}",
                entry.tool
            );
        }
    }
}

/// Resultado de `change_revert` (E13-H09): el recibo de una transacción **revertida** por el único
/// escritor (transacción inversa). Proyección de servicio (framing, no dominio); wire en camelCase —
/// `reverted`, `receiptId`, `previousWorkspaceRevision`, `workspaceRevision`, `changedPaths`,
/// `semanticDiff`. La reversión es INVERSA al apply: `previousWorkspaceRevision` es la revisión de la
/// que parte la reversión (la `resultRevision` que dejó el apply) y `workspaceRevision` es la
/// resultante (la `previousRevision` original del apply, el estado restaurado). Sin `Eq` (transitivo
/// desde [`SemanticDiff`]).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct RevertResult {
    /// `true` cuando la reversión se publicó (siempre `true` en un `Ok`; los rechazos son `Err`).
    pub reverted: bool,
    /// Id del recibo persistido de esta reversión (la transacción inversa).
    pub receipt_id: ReceiptId,
    /// [`WorkspaceRevision`] ANTES de la reversión (== `resultRevision` del apply revertido).
    pub previous_workspace_revision: WorkspaceRevision,
    /// [`WorkspaceRevision`] resultante: el workspace vuelve a la `previousRevision` del apply.
    pub workspace_revision: WorkspaceRevision,
    /// Paths del canónico que la reversión restauró/borró, en orden determinista.
    pub changed_paths: Vec<RelPath>,
    /// Diff semántico de la transacción revertida (una sola verdad de diff, invariante #3).
    pub semantic_diff: SemanticDiff,
}

/// Construye el [`ChangeSet`] de dominio que consume [`Workspace::apply_transaction`] a partir del
/// [`PlanResult`] persistido. La transacción solo lee `id` y `operations` (para staging/publicación)
/// y `base_revision` (control optimista); `validation` se rellena a `Default` porque el `PlanResult`
/// no la almacena (guarda `diagnostics_before`/`diagnostics_after` en su lugar) y la transacción no
/// la consume.
fn plan_to_change_set(plan: &PlanResult) -> ChangeSet {
    ChangeSet {
        id: plan.change_set_id.clone(),
        base_revision: plan.base_workspace_revision.clone(),
        operations: plan.normalized_operations.clone(),
        plan_hash: plan.plan_hash.clone(),
        risk: plan.risk.clone(),
        semantic_diff: plan.semantic_diff.clone(),
        validation: ValidationReport::default(),
        expires_at: plan.expires_at.clone(),
    }
}

/// Resultado de `change_apply` (E13-H08): el recibo de una transacción **aplicada** por el único
/// escritor. Proyección de servicio (framing, no dominio); wire en camelCase — `receiptId`,
/// `applied`, `previousWorkspaceRevision`, `workspaceRevision`, `changedPaths`, `semanticDiff`,
/// `validation`. `workspaceRevision` es la revisión resultante: tras un apply OK el workspace
/// «queda en» ella. Sin `Eq` (transitivo desde [`SemanticDiff`]).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplyResult {
    /// Id del recibo persistido de esta aplicación (E13-H07); permite revertir (E13-H09).
    pub receipt_id: ReceiptId,
    /// `true` cuando la transacción se publicó (siempre `true` en un `Ok`; los rechazos son `Err`).
    pub applied: bool,
    /// [`WorkspaceRevision`] del workspace ANTES de la transacción (la base sobre la que se publicó).
    pub previous_workspace_revision: WorkspaceRevision,
    /// [`WorkspaceRevision`] resultante: el workspace queda en ella tras el apply.
    pub workspace_revision: WorkspaceRevision,
    /// Paths del canónico que la transacción creó/modificó/borró, en orden determinista.
    pub changed_paths: Vec<RelPath>,
    /// Diff semántico del plan aplicado (una sola verdad de diff, invariante #3).
    pub semantic_diff: SemanticDiff,
    /// Conformidad del workspace ya publicado.
    pub validation: ApplyValidation,
}

/// Veredicto de conformidad del workspace tras aplicar la transacción (`validation` de
/// [`ApplyResult`]). Mismo desglose que `hardFail`/`warnCount` de [`Analysis`]. Wire en camelCase.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplyValidation {
    /// `true` si el workspace publicado no tiene ningún check `Err` (`hardFail == 0`).
    pub conformant: bool,
    /// Nº de ficheros con al menos un check `Err`.
    pub errors: usize,
    /// Nº de checks `Warn`.
    pub warnings: usize,
}

// ---------------------------------------------------------------------------
// Auditoría local `.lodestar/runtime/audit.jsonl` (E13-H10, `ARCHITECTURE.md §19.7`,
// `REFACTOR §14`). Registra localmente cada operación de escritura (`change_apply`/
// `change_revert`) — éxito Y fallo, incluidos los intentos rechazados antes de publicar. Runtime,
// NO conocimiento canónico: gitignored, fuera de `WorkspaceRevision` (E9-H06/E10-H03), nunca
// indexado y no expuesto por ninguna tool MCP (solo diagnóstico local).
// ---------------------------------------------------------------------------

/// Cliente por defecto de las entradas de auditoría. El protocolo (MCP/CLI) no identifica hoy un
/// cliente concreto — no hay mecanismo de identidad de cliente todavía (E13-H10 solo pide un valor
/// «razonable», no resolver identidad; ver el rustdoc de la historia). Placeholder documentado,
/// no una decisión de producto.
const AUDIT_CLIENT_DEFAULT: &str = "mcp";

/// Una línea del registro de auditoría local `.lodestar/runtime/audit.jsonl` (E13-H10). Proyección
/// de servicio, wire en camelCase — `changeSetId`/`baseRevision`/`resultRevision`. `timestamp` es
/// `SystemTime::now()` en segundos epoch, tomado aquí (fachada de superficie) — `lodestar-core`
/// sigue sin tocar tiempo de pared (pureza, invariante #2).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuditEntry {
    /// Instante de la operación, en segundos epoch (wall-clock).
    pub timestamp: String,
    /// Cliente que originó la operación (hoy siempre `AUDIT_CLIENT_DEFAULT`).
    pub client: String,
    /// Nombre de la operación de escritura auditada (misma etiqueta que la tool MCP:
    /// `"change_apply"`/`"change_revert"`).
    pub tool: String,
    /// El `ChangeSetId` de la operación auditada, tal cual se intentó. Para `change_revert` que
    /// falla antes de poder resolver el receipt, es el `receiptId` (ver
    /// `App::revert_change_set_hint`).
    pub change_set_id: String,
    /// [`WorkspaceRevision`] base ANTES de la operación, solo en éxito (en fallo no se conoce de
    /// forma fiable sin duplicar los pasos de la operación, y el criterio de la historia solo fija
    /// las revisiones para el camino de éxito).
    pub base_revision: Option<String>,
    /// [`WorkspaceRevision`] resultante DESPUÉS de la operación, solo en éxito.
    pub result_revision: Option<String>,
    /// Paths del canónico afectados, solo en éxito (vacío en fallo: nada se publicó).
    pub paths: Vec<String>,
    /// `"success"` en éxito; en fallo, el código wire del [`ErrorCode`] rechazado (p. ej.
    /// `"REVISION_CONFLICT"`), vía [`ErrorCode::as_str`] — un audit trail cubre también los
    /// intentos rechazados, y el código wire es más útil como diagnóstico que un literal genérico.
    pub result: String,
}

/// Instante actual en segundos epoch, como string (E13-H10). Wall-clock, en la fachada de
/// superficie — mismo patrón que `expires_at_string` para `change_plan`; `lodestar-core` sigue sin
/// tocar `SystemTime` (pureza, invariante #2).
fn audit_timestamp_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        .to_string()
}

/// Construye la entrada de auditoría de un `change_apply` (E13-H10) a partir de su resultado: en
/// éxito, las revisiones/paths del [`ApplyResult`]; en fallo, solo lo que se conoce siempre —el
/// `changeSetId` de entrada (el parámetro, el mismo en cualquier paso donde falle) y el código wire
/// del [`ErrorCode`] rechazado.
fn audit_entry_for_apply(
    change_set_id: &ChangeSetId,
    outcome: &Result<ApplyResult, ErrorCode>,
) -> AuditEntry {
    let (base_revision, result_revision, paths, result) = match outcome {
        Ok(apply) => (
            Some(apply.previous_workspace_revision.0.clone()),
            Some(apply.workspace_revision.0.clone()),
            apply
                .changed_paths
                .iter()
                .map(|p| p.as_str().to_string())
                .collect(),
            "success".to_string(),
        ),
        Err(err) => (None, None, Vec::new(), err.as_str().to_string()),
    };
    AuditEntry {
        timestamp: audit_timestamp_now(),
        client: AUDIT_CLIENT_DEFAULT.to_string(),
        tool: "change_apply".to_string(),
        change_set_id: change_set_id.0.clone(),
        base_revision,
        result_revision,
        paths,
        result,
    }
}

/// Construye la entrada de auditoría de un `change_revert` (E13-H10) a partir de su resultado —
/// mismo criterio que [`audit_entry_for_apply`]. El `changeSetId` ya viene resuelto por el llamante
/// (ver `App::revert_change_set_hint`), porque `change_revert` solo recibe un `receiptId`, no un
/// `ChangeSetId` directo.
fn audit_entry_for_revert(
    change_set_id_hint: &str,
    outcome: &Result<RevertResult, ErrorCode>,
) -> AuditEntry {
    let (base_revision, result_revision, paths, result) = match outcome {
        Ok(revert) => (
            Some(revert.previous_workspace_revision.0.clone()),
            Some(revert.workspace_revision.0.clone()),
            revert
                .changed_paths
                .iter()
                .map(|p| p.as_str().to_string())
                .collect(),
            "success".to_string(),
        ),
        Err(err) => (None, None, Vec::new(), err.as_str().to_string()),
    };
    AuditEntry {
        timestamp: audit_timestamp_now(),
        client: AUDIT_CLIENT_DEFAULT.to_string(),
        tool: "change_revert".to_string(),
        change_set_id: change_set_id_hint.to_string(),
        base_revision,
        result_revision,
        paths,
        result,
    }
}

/// Ruta completa de `.lodestar/runtime/audit.jsonl` bajo `root` (E13-H10). El directorio `runtime`
/// ya lo garantiza `ensure_runtime_scaffold` al abrir el workspace (E9-H06); `try_append_audit` lo
/// reafirma con `create_dir_all` por robustez (mismo patrón que `persist_plan`).
fn audit_file_path(root: &Path) -> PathBuf {
    root.join(".lodestar").join("runtime").join("audit.jsonl")
}

/// Anexa `entry` como una línea JSON (+ `\n`) a `.lodestar/runtime/audit.jsonl` bajo `root`
/// (E13-H10): crea `.lodestar/runtime/` si falta y abre en modo `append` — JSONL que solo crece,
/// nunca reescribe líneas previas. Devuelve el error de I/O sin envolver; `App::audit` es quien
/// decide que un fallo aquí es best-effort (no debe tumbar la operación auditada).
fn try_append_audit(root: &Path, entry: &AuditEntry) -> std::io::Result<()> {
    use std::io::Write;
    let path = audit_file_path(root);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut line = serde_json::to_string(entry).expect("AuditEntry siempre serializa a JSON");
    line.push('\n');
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)?;
    file.write_all(line.as_bytes())
}

// ---------------------------------------------------------------------------
// `change_plan` — dispatch de operaciones crudas y tipos de proyección (E12-H08,
// `ARCHITECTURE.md §19.5/§19.6`, `REFACTOR §11.1/§17`).
//
// Proyección de servicio (framing), NO dominio: `PlanResult`/`PlanImpact` viven en `lodestar-app`.
// Las `NormalizedOperation`/`RiskAssessment`/`SemanticDiff`/`ValidationSummary` que portan SÍ son
// dominio puro del core (`core::types`), reexpuestas tal cual. Wire en camelCase.
// ---------------------------------------------------------------------------

/// Vida útil (segundos) que se concede a un plan recién generado antes de `expiresAt`. La
/// caducidad real (rechazar planes vencidos → `PLAN_EXPIRED`) es E12-H09; aquí solo se estampa un
/// instante futuro. `expiresAt` es wall-clock y **no** entra en el `planHash`.
const PLAN_TTL_SECS: u64 = 3600;

/// El documento cuya [`DocumentRevision`] guarda el control optimista de una op cruda: `ref.path`,
/// `path`, `from` (move) o `source` (relaciones), en ese orden. `Err(InvalidSchema)` si la op trae
/// `expectedRevision` pero no un objetivo identificable.
fn op_target_path(op: &Value) -> Result<RelPath, ErrorCode> {
    let candidate = op
        .get("ref")
        .and_then(|r| r.get("path"))
        .and_then(Value::as_str)
        .or_else(|| op.get("path").and_then(Value::as_str))
        .or_else(|| op.get("from").and_then(Value::as_str))
        .or_else(|| op.get("source").and_then(Value::as_str))
        .ok_or(ErrorCode::InvalidSchema)?;
    RelPath::new(candidate).map_err(|e| error_code(&e))
}

/// `ref.path` o `path` de una op cruda como [`RelPath`]. `Err(InvalidSchema)` si falta, o el error
/// mapeado de [`RelPath::new`] (path-traversal → `PermissionDenied`) si es inválido.
fn op_ref_path(op: &Value) -> Result<RelPath, ErrorCode> {
    let s = op
        .get("ref")
        .and_then(|r| r.get("path"))
        .and_then(Value::as_str)
        .or_else(|| op.get("path").and_then(Value::as_str))
        .ok_or(ErrorCode::InvalidSchema)?;
    RelPath::new(s).map_err(|e| error_code(&e))
}

/// Un campo string obligatorio de una op cruda como [`RelPath`].
fn op_rel_field(op: &Value, key: &str) -> Result<RelPath, ErrorCode> {
    let s = op
        .get(key)
        .and_then(Value::as_str)
        .ok_or(ErrorCode::InvalidSchema)?;
    RelPath::new(s).map_err(|e| error_code(&e))
}

/// Despacha UNA op cruda (`{op, …}`) a su normalizador del core, devolviendo las
/// [`NormalizedOperation`] resultantes (una op de estructura puede producir varias, E12-H06).
/// El discriminador `op` usa el mismo vocabulario snake_case que [`NormalizedOperation`]. Un `op`
/// desconocido o un parámetro inválido → `Err(ErrorCode::InvalidSchema)`; los errores del core se
/// mapean con [`error_code`].
fn normalize_raw_op(
    doc_set: &DocumentSet,
    op: &Value,
) -> Result<Vec<NormalizedOperation>, ErrorCode> {
    let kind = op
        .get("op")
        .and_then(Value::as_str)
        .ok_or(ErrorCode::InvalidSchema)?;
    let one = |n: NormalizedOperation| vec![n];
    match kind {
        "create" => {
            let path = op_rel_field(op, "path")?;
            let ty = op.get("type").and_then(Value::as_str).unwrap_or("");
            let title = op.get("title").and_then(Value::as_str);
            let body = op.get("body").and_then(Value::as_str).map(str::to_string);
            plan::normalize_create(doc_set, &path, ty, title, body)
                .map(one)
                .map_err(|e| error_code(&e))
        }
        "patch_frontmatter" => {
            let path = op_ref_path(op)?;
            let patch = op_patch(op)?;
            plan::normalize_patch_frontmatter(doc_set, &path, patch)
                .map(one)
                .map_err(|e| error_code(&e))
        }
        "replace_body" => {
            let path = op_ref_path(op)?;
            let body = op
                .get("body")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            plan::normalize_replace_body(doc_set, &path, body)
                .map(one)
                .map_err(|e| error_code(&e))
        }
        "replace_text" => {
            let path = op_ref_path(op)?;
            let find = op.get("find").and_then(Value::as_str).unwrap_or("");
            let replace = op.get("replace").and_then(Value::as_str).unwrap_or("");
            let expected = op
                .get("expectedOccurrences")
                .and_then(Value::as_u64)
                .map(|n| n as usize);
            plan::normalize_replace_text(doc_set, &path, find, replace, expected)
                .map(one)
                .map_err(|e| error_code(&e))
        }
        "edit_section" => {
            let path = op_ref_path(op)?;
            let heading_path: Vec<String> = op
                .get("headingPath")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(Value::as_str)
                        .map(str::to_string)
                        .collect()
                })
                .unwrap_or_default();
            let mode = match op.get("mode").and_then(Value::as_str) {
                Some("append") => EditSectionMode::Append,
                Some("prepend") => EditSectionMode::Prepend,
                _ => EditSectionMode::Replace,
            };
            let content = op.get("content").and_then(Value::as_str).unwrap_or("");
            plan::normalize_edit_section(doc_set, &path, &heading_path, mode, content)
                .map(one)
                .map_err(|e| error_code(&e))
        }
        "move" => {
            let from = op_rel_field(op, "from")?;
            let to = op_rel_field(op, "to")?;
            let rewrite = op
                .get("rewriteInboundLinks")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            plan::normalize_move(doc_set, &from, &to, rewrite).map_err(|e| error_code(&e))
        }
        "delete" => {
            let path = op_ref_path(op)?;
            let policy = match op.get("inboundLinksPolicy").and_then(Value::as_str) {
                Some("retarget") => InboundLinksPolicy::Retarget,
                Some("remove_links") => InboundLinksPolicy::RemoveLinks,
                Some("create_stub") => InboundLinksPolicy::CreateStub,
                _ => InboundLinksPolicy::Reject,
            };
            plan::normalize_delete(doc_set, &path, policy).map_err(|e| error_code(&e))
        }
        "apply_fix" => {
            let fix_id = op
                .get("fixId")
                .and_then(Value::as_str)
                .ok_or(ErrorCode::InvalidSchema)?;
            plan::normalize_apply_fix(doc_set, fix_id)
                .map(one)
                .map_err(|e| error_code(&e))
        }
        _ => Err(ErrorCode::InvalidSchema),
    }
}

/// Convierte el campo `patch` de una op cruda en un [`FrontmatterPatch`] (merge-patch RFC 7386:
/// `null` borra la clave, cualquier otro valor la escribe). `Err(InvalidSchema)` si `patch` falta o
/// no es un objeto.
fn op_patch(op: &Value) -> Result<FrontmatterPatch, ErrorCode> {
    let patch = op.get("patch").ok_or(ErrorCode::InvalidSchema)?;
    if !patch.is_object() {
        return Err(ErrorCode::InvalidSchema);
    }
    serde_json::from_value(patch.clone()).map_err(|_| ErrorCode::InvalidSchema)
}

/// `planHash` determinista: `blake3(baseWorkspaceRevision ‖ 0x00 ‖ serialización JSON de las
/// normalizedOperations)`. La serialización de `serde_json` es estable (orden de campos por
/// declaración; `FrontmatterPatch` es un `BTreeMap` ordenado), así que el mismo plan sobre la misma
/// base produce el mismo hash entre procesos frescos, y un plan distinto uno distinto. **No**
/// depende del reloj.
fn compute_plan_hash(base: &WorkspaceRevision, ops: &[NormalizedOperation]) -> PlanHash {
    let mut hasher = blake3::Hasher::new();
    hasher.update(base.0.as_bytes());
    hasher.update(b"\0");
    let serialized = serde_json::to_vec(ops).expect("NormalizedOperation siempre serializa a JSON");
    hasher.update(&serialized);
    PlanHash(format!("blake3:{}", hasher.finalize().to_hex()))
}

/// Nombre de fichero saneado para persistir un plan bajo `.lodestar/runtime/plans/` (E12-H09): el
/// hash hexadecimal DESNUDO del `changeSetId` (sin el prefijo `changeset:`) más `.json`. El
/// `changeSetId` completo lleva `:`, hostil a nombres de fichero en Windows — el hash desnudo basta
/// para la trazabilidad (el criterio de aceptación exige que el nombre CONTENGA el hash, no que
/// preserve el `changeSetId` literal) y es determinista/derivable en ambas direcciones (persistir y
/// cargar usan esta misma función).
fn plan_file_name(id: &ChangeSetId) -> String {
    let hex = id.0.strip_prefix("changeset:").unwrap_or(&id.0);
    format!("{hex}.json")
}

/// Ruta completa del fichero de plan persistido para `id`, bajo `.lodestar/runtime/plans/` del
/// `root` del workspace. El directorio ya lo garantiza `ensure_runtime_scaffold` al abrir el
/// workspace (E9-H06); [`persist_plan`] lo reafirma con `create_dir_all` por robustez.
fn plan_file_path(root: &Path, id: &ChangeSetId) -> PathBuf {
    root.join(".lodestar")
        .join("runtime")
        .join("plans")
        .join(plan_file_name(id))
}

/// Persiste el `PlanResult` completo (operaciones normalizadas, revisión base, hash, caducidad,
/// diff, impacto, validación) en `.lodestar/runtime/plans/<hash>.json` (E12-H09,
/// `ARCHITECTURE.md §19.4/§19.5`).
///
/// Runtime, no canónico: gitignored y excluido de `WorkspaceRevision` (E9-H06/E10-H03), por lo que
/// se escribe con `std::fs::write` normal — el protocolo temp+rename del único-escritor
/// (`lodestar_workspace::io::write_atomic`) protege el conocimiento `.md` canónico, no el scratch
/// de runtime, que ni el watcher ni el walker observan.
fn persist_plan(root: &Path, plan: &PlanResult) -> Result<(), ErrorCode> {
    let dir = root.join(".lodestar").join("runtime").join("plans");
    std::fs::create_dir_all(&dir).map_err(|_| ErrorCode::InternalIoError)?;
    let path = dir.join(plan_file_name(&plan.change_set_id));
    let json = serde_json::to_vec_pretty(plan).map_err(|_| ErrorCode::InternalIoError)?;
    std::fs::write(&path, json).map_err(|_| ErrorCode::InternalIoError)
}

/// Instante de caducidad del plan (`expiresAt`): ahora + [`PLAN_TTL_SECS`], en segundos epoch como
/// string. Wall-clock, FUERA del `planHash` (E12-H08). La semántica de caducidad real es E12-H09.
fn expires_at_string() -> String {
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    let at = SystemTime::now() + Duration::from_secs(PLAN_TTL_SECS);
    let secs = at
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    secs.to_string()
}

/// Resultado de `change_plan` (E12-H08): un plan de cambios completo, simulado en memoria y **sin
/// escribir** (invariante #1). Proyección de servicio; wire en camelCase — `changeSetId`,
/// `baseWorkspaceRevision`, `planHash`, `canApply`, `expiresAt`, `normalizedOperations`,
/// `semanticDiff`, `diagnosticsBefore`/`diagnosticsAfter`.
///
/// Sin `Eq` (transitivo desde `NormalizedOperation`/`FrontmatterPatch`).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanResult {
    /// Identificador del change set (derivado del `planHash`, determinista).
    pub change_set_id: ChangeSetId,
    /// Revisión del workspace sobre la que se computó el plan ([`workspace_revision`]).
    pub base_workspace_revision: WorkspaceRevision,
    /// Hash determinista del plan (mismo input + misma base ⇒ mismo hash).
    pub plan_hash: PlanHash,
    /// `true` si el plan es aplicable bajo la `policy` dada ([`plan::can_apply`]).
    pub can_apply: bool,
    /// Instante de caducidad (segundos epoch, wall-clock; fuera del `planHash`).
    pub expires_at: String,
    /// Todas las operaciones normalizadas del plan, en un único change set.
    pub normalized_operations: Vec<NormalizedOperation>,
    /// Evaluación de riesgo del plan (E12-H02).
    pub risk: RiskAssessment,
    /// Diff semántico entre el workspace actual y el hipotético (E12-H03).
    pub semantic_diff: SemanticDiff,
    /// Resumen de impacto (documentos afectados).
    pub impact: PlanImpact,
    /// Conteo de diagnósticos del workspace ANTES del plan.
    pub diagnostics_before: ValidationSummary,
    /// Conteo de diagnósticos del workspace hipotético DESPUÉS del plan.
    pub diagnostics_after: ValidationSummary,
}

/// Resumen de impacto de un plan (E12-H08): los documentos que el plan crea/modifica/borra/mueve, y
/// su recuento. Derivado del [`SemanticDiff`] (una sola verdad de diff, invariante #3). Wire en
/// camelCase.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct PlanImpact {
    /// Documentos afectados por el plan (unión de creados/modificados/borrados/movidos), orden estable.
    pub affected_documents: Vec<RelPath>,
    /// Número de documentos afectados (`affected_documents.len()`).
    pub affected_count: usize,
}

impl PlanImpact {
    /// Deriva el impacto de un [`SemanticDiff`]: unión (sin duplicados, orden estable) de los paths
    /// creados, modificados, borrados y de los extremos de cada movimiento.
    fn from_diff(diff: &SemanticDiff) -> Self {
        let mut set: BTreeSet<RelPath> = BTreeSet::new();
        set.extend(diff.created.iter().cloned());
        set.extend(diff.modified.iter().cloned());
        set.extend(diff.deleted.iter().cloned());
        for m in &diff.moved {
            set.insert(m.from.clone());
            set.insert(m.to.clone());
        }
        let affected_documents: Vec<RelPath> = set.into_iter().collect();
        let affected_count = affected_documents.len();
        PlanImpact {
            affected_documents,
            affected_count,
        }
    }
}

// ---------------------------------------------------------------------------
// `knowledge_check` — scope, informe y id estable de diagnóstico (E10-H12).
//
// Proyección de servicio (framing), NO dominio: viven en `lodestar-app`, no en `core::types`. Los
// diagnósticos que porta (`Check`) sí son dominio puro del core (`Analysis::diagnostics`, `§20.9`;
// tras E20-H03 ya no se fusionan `SCHEMA-*`/`REL-*`). Wire en camelCase.
// ---------------------------------------------------------------------------

/// Límite por defecto de diagnósticos por página de `knowledge_check` (`REFACTOR §10`).
const DEFAULT_CHECK_LIMIT: usize = 100;
/// Tope duro de diagnósticos por página (evita respuestas gigantes).
const MAX_CHECK_LIMIT: usize = 1000;

/// Scope de auditoría de [`App::knowledge_check`] (`ARCHITECTURE.md §19.6`, `REFACTOR §10`). El
/// discriminante de wire es `kind` (camelCase): `workspace` (todos los documentos), `document` (uno,
/// por `ref`), `paths` (una lista explícita) y `affected` (el vecindario/blast-radius de unos
/// `refs` a distancia ≤ `depth`).
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum CheckScope {
    /// Todos los documentos del workspace.
    Workspace,
    /// Un único documento, identificado por `ref` (`DocumentRef`).
    Document {
        /// El documento a auditar.
        r#ref: DocumentRef,
    },
    /// Una lista explícita de paths.
    Paths {
        /// Los paths a auditar.
        paths: Vec<RelPath>,
    },
    /// El vecindario (blast-radius) de unos `refs` a distancia ≤ `depth`.
    Affected {
        /// Los documentos centro del vecindario.
        refs: Vec<DocumentRef>,
        /// Distancia máxima de exploración (por defecto 1 si el cliente la omite).
        #[serde(default = "default_affected_depth")]
        depth: u32,
    },
}

/// Profundidad por defecto del scope `affected` cuando el cliente omite `depth`.
fn default_affected_depth() -> u32 {
    1
}

/// Recuento de diagnósticos por severidad de un informe de `knowledge_check`. Wire en camelCase.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CheckSummary {
    /// Nº de diagnósticos de severidad `Err` en el scope.
    pub errors: usize,
    /// Nº de diagnósticos de severidad `Warn` en el scope.
    pub warnings: usize,
    /// Nº de diagnósticos de severidad `Info` en el scope.
    pub info: usize,
}

/// Informe de `knowledge_check` (`ARCHITECTURE.md §19.6`, `REFACTOR §10`). Wire en camelCase
/// (`workspaceRevision`, `nextCursor`).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct CheckReport {
    /// `true` si el scope no tiene ningún diagnóstico de severidad `Err`.
    pub conformant: bool,
    /// Recuento por severidad sobre TODO el scope (independiente de `minimumSeverity`/paginación).
    pub summary: CheckSummary,
    /// La página de diagnósticos (cada uno con su `id` estable), tras filtrar por severidad y paginar.
    pub diagnostics: Vec<Check>,
    /// Revisión determinista del workspace en el momento de la auditoría (`WorkspaceRevision`).
    pub workspace_revision: WorkspaceRevision,
    /// Cursor opaco a la siguiente página, o `None` si no quedan más diagnósticos.
    pub next_cursor: Option<String>,
}

/// Id estable de un diagnóstico dentro de una revisión (E10-H12): `diag:blake3:<hex>` donde
/// `hex = blake3(path ‖ 0x00 ‖ code ‖ 0x00 ‖ range ‖ 0x00 ‖ msg)`. Determinista y derivado **solo**
/// de los datos del diagnóstico (nunca de timestamps, orden ni caché), así que la misma revisión
/// produce los mismos `id` incluso entre procesos frescos.
fn diagnostic_id(path: &str, check: &Check) -> String {
    let range_repr = match &check.range {
        Some(r) => format!("{}:{}", r.start_line, r.end_line),
        None => String::new(),
    };
    let mut hasher = blake3::Hasher::new();
    hasher.update(path.as_bytes());
    hasher.update(b"\0");
    hasher.update(check.code.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(range_repr.as_bytes());
    hasher.update(b"\0");
    hasher.update(check.msg.as_bytes());
    format!("diag:blake3:{}", hasher.finalize().to_hex())
}

// ---------------------------------------------------------------------------
// `graph_query` — tipos de proyección de servicio (E11-H01, `ARCHITECTURE.md §19.6`,
// `REFACTOR §9.5`).
//
// Proyección de servicio (framing), NO dominio: vive en `lodestar-app`, no en `core::types`. Los
// `nodes`/`edges` que porta SÍ son dominio puro (`GraphNode`/`Edge` de `core::types`), reexpuestos
// tal cual — esta capa nunca redefine su forma. Wire en camelCase.
// ---------------------------------------------------------------------------

/// Respuesta de `graph_query` (`ARCHITECTURE.md §19.6`, `REFACTOR §9.5`). Wire en camelCase
/// (`nextCursor`).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphQueryResult {
    /// Los nodos de la página actual (orden total y estable por `id`).
    pub nodes: Vec<GraphNode>,
    /// Las aristas cuyos dos extremos están en `nodes` (nunca "cuelgan" de un nodo paginado fuera).
    pub edges: Vec<Edge>,
    /// Recuento y estado de truncamiento de la página devuelta (no del total del grafo).
    pub summary: GraphQuerySummary,
    /// Cursor opaco a la siguiente página, o `None` si no quedan más nodos.
    pub next_cursor: Option<String>,
}

/// Recuento agregado de un `graph_query`, sobre la página efectivamente devuelta (`nodes`/`edges`
/// tras paginar). Wire en camelCase.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct GraphQuerySummary {
    /// `nodes.len()` de la página devuelta.
    pub node_count: usize,
    /// `edges.len()` de la página devuelta.
    pub edge_count: usize,
    /// `true` si `limit` recortó el total de nodos (hay más páginas vía `nextCursor`).
    pub truncated: bool,
}

// ---------------------------------------------------------------------------
// `impact_analyze` — tipos de proyección de servicio (E11-H05, `ARCHITECTURE.md §19.6`,
// `REFACTOR §9.6/§17`).
//
// Proyección de servicio (framing), NO dominio: vive en `lodestar-app`, no en `core::types`. Los
// recuentos los computa `App::impact_analyze` componiendo el core (`DocumentSet::backlinks`/
// `neighborhood`); `blockingReferences` va siempre vacío desde E17-H05. Wire en camelCase.
// ---------------------------------------------------------------------------

/// Umbral de backlinks directos a partir del cual el impacto de un cambio se considera **alto**
/// (E11-H05): mover/borrar un documento con muchos enlaces entrantes es intrínsecamente arriesgado.
const HIGH_IMPACT_BACKLINKS: usize = 20;
/// Umbral de afectados (directos o transitivos) a partir del cual el impacto se considera **medio**.
const MEDIUM_IMPACT_BACKLINKS: usize = 5;

/// Respuesta de `impact_analyze` (`ARCHITECTURE.md §19.6`, `REFACTOR §9.6`). Wire en camelCase.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImpactReport {
    /// Recuentos agregados y nivel de riesgo del cambio propuesto.
    pub summary: ImpactSummary,
    /// Documentos alcanzados por el blast-radius entrante (excluido el propio `ref`), orden estable.
    pub affected_documents: Vec<RelPath>,
    /// Relaciones tipadas obligatorias entrantes que quedarían rotas (solo para `kind:"delete"`).
    pub blocking_references: Vec<BlockingReference>,
    /// Acciones sugeridas antes de aplicar el cambio (texto en español); vacío si el riesgo es bajo.
    pub recommendations: Vec<String>,
}

/// Recuentos agregados de un `impact_analyze` y su nivel de riesgo. Wire en camelCase.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ImpactSummary {
    /// Nº de backlinks **directos** entrantes del `ref` (`DocumentSet::backlinks.inbound`).
    pub directly_affected: usize,
    /// Tamaño del blast-radius entrante (`neighborhood(In)`, excluido el propio `ref`).
    pub transitively_affected: usize,
    /// `blockingReferences.len()` — nº de relaciones obligatorias entrantes que romperían.
    pub blocking_references: usize,
    /// Nivel de riesgo derivado, del conjunto cerrado `{"low","medium","high"}` (wire en inglés).
    pub risk: String,
}

/// Una relación tipada entrante que quedaría rota si se aplicara el cambio (E11-H05). `path` es el
/// documento que depende del `ref`; `reason` explica el bloqueo (nombre de la relación rota). Wire
/// en camelCase.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct BlockingReference {
    /// El documento origen que declara la relación tipada hacia el `ref`.
    pub path: RelPath,
    /// Texto (no vacío) que explica por qué bloquea (la relación tipada que se rompería).
    pub reason: String,
}

// E17-H05 retiró `blocking_relations`/`relation_field_targets`: los bloqueos estructurales salían
// de las relaciones tipadas del `schema.yaml`, vocabulario que el modelo universal ya no tiene
// (`§20.10`). `BlockingReference` sobrevive solo como forma del wire, siempre vacía, aun tras el
// retiro de `core::schema` (E20-H03); su retirada del wire es una historia propia.

// ---------------------------------------------------------------------------
// `knowledge_get` — tipos de proyección de servicio y extracción de secciones (E10-H10).
//
// Proyección de servicio (framing), NO dominio: vive en `lodestar-app`, no en `core::types`. No
// hay función equivalente en `prototype/index.html` (la selección por `headingPath` es superficie
// nueva de esta épica, no un port) — implementación propia. Wire en camelCase.
// ---------------------------------------------------------------------------

/// Proyección de un documento para `knowledge_get`. `path`/`revision` siempre presentes; el resto
/// es `None` cuando no se pidió en `include` (selectividad significativa, no vacua).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct DocumentView {
    /// Ruta relativa del documento (su identidad en v2).
    pub path: RelPath,
    /// Identidad de contenido (`blake3:…`, == [`DocumentRevision`] de E10-H03). Siempre presente.
    pub revision: DocumentRevision,
    /// Frontmatter del documento —metadata **arbitraria** del usuario, siempre un objeto YAML—,
    /// si se pidió `"frontmatter"` en `include` (`ARCHITECTURE.md §20.4`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[schemars(with = "Option<serde_json::Map<String, serde_json::Value>>")]
    pub frontmatter: Option<serde_yaml::Value>,
    /// Cuerpo Markdown (completo o acotado por `sections`), si se pidió `"body"` en `include`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Enlaces salientes ya **resueltos y clasificados** (`Analysis::outgoing`), si se pidió
    /// `"outgoingLinks"`. Desde E17-H05 no es una lista de paths: cada entrada lleva el href
    /// crudo, el texto, el `span` de bytes del destino, la forma sintáctica y el `LinkTarget`
    /// (`§20.6`) — es lo que un agente necesita para reescribir un destino sin volver a parsear
    /// el Markdown.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outgoing_links: Option<Vec<ResolvedLink>>,
    /// Vecindad de enlaces entrantes (`DocumentSet::backlinks`), si se pidió `"backlinks"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backlinks: Option<Backlinks>,
    /// Referencias externas (`implemented_by`/`verified_by`, E11-H04) resueltas contra
    /// `referenceRoots`, si se pidió `"externalReferences"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_references: Option<Vec<ExternalReference>>,
    /// Checks de conformidad del documento (`Analysis::diagnostics`), si se pidió `"diagnostics"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<Check>>,
}

// ---------------------------------------------------------------------------
// `knowledge_search` — tipos de proyección de servicio (E10-H09).
//
// Son proyección de servicio (framing), NO dominio: viven en `lodestar-app`, no en `core::types`.
// El casado, la revisión y el snippet reusan lógica pura del core. Wire en camelCase.
// ---------------------------------------------------------------------------

/// Límite por defecto de resultados por página de `knowledge_search`.
const DEFAULT_SEARCH_LIMIT: usize = 20;
/// Tope duro de resultados por página (evita respuestas gigantes).
const MAX_SEARCH_LIMIT: usize = 100;

/// Un resultado de `knowledge_search` — proyección **genérica** de un documento para localizarlo,
/// **nunca su cuerpo completo** (invariante de la historia). Wire en camelCase.
///
/// Desde E19-H05 no lleva campos privilegiados de OKF (`type`/`status`/`description`/`tags`): el
/// filtrado por metadata pasa por el lenguaje de consulta (`where`/`filter`), así que esos valores
/// dejan de ser campos de wire aunque sigan en el frontmatter del documento (recuperables por
/// `knowledge_get`). Conserva solo la identidad y lo derivado: `path`, `title`, `snippet`, `score`,
/// `revision` (y `id`, no-goal en v2 → siempre ausente).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// Ruta relativa del documento (su identidad en v2, E10-H04).
    pub path: RelPath,
    /// Id estable del documento, cuando exista (no-goal en v2 → siempre ausente).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// Título resuelto (`title` del frontmatter o derivado del path).
    pub title: String,
    /// Extracto compacto NO vacío alrededor del match (o del inicio del cuerpo). **No** es el cuerpo.
    pub snippet: String,
    /// Puntuación de relevancia (mayor = más relevante). Base simple por frecuencia del texto.
    pub score: f64,
    /// Revisión de contenido del documento (`blake3:…`, == [`DocumentRevision`] de E10-H03).
    pub revision: DocumentRevision,
}

/// Respuesta de `knowledge_search`: la página de resultados, el cursor a la siguiente página (o
/// `None` al agotar) y el total aproximado de coincidencias. Wire en camelCase (`nextCursor`,
/// `totalApproximate`).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    /// La página actual de resultados (nunca contiene cuerpos).
    pub results: Vec<SearchResult>,
    /// Cursor opaco a la siguiente página, o `None` si no quedan más resultados.
    pub next_cursor: Option<String>,
    /// Número total de documentos que casan (todas las páginas juntas).
    pub total_approximate: usize,
}

/// Compila el `where` textual y/o el `filter` JSON de `knowledge_search` a un único [`Expression`]
/// (E19-H01…H04), el que luego evalúa [`evaluate`] por documento.
///
/// - Ninguno → `Ok(None)` (no hay filtro de lenguaje; solo actúa el `text`).
/// - Solo uno → su AST ([`lodestar_core::parse::parse`] para el textual,
///   [`lodestar_core::filter::from_json`] para el JSON).
/// - **Ambos** → se combinan con `and` (intersección), coherente con cómo `text` ya se intersecta;
///   ningún test lo fija, pero es la elección menos sorprendente (un filtro extra solo puede
///   restringir, nunca abrir la selección).
///
/// Un `where`/`filter` **malformado** se surface como [`WorkspaceError::Core`] genérico: el mapeo
/// fino a `INVALID_SCHEMA` (con `location`/`suggestion`) es E20 y queda fuera de esta historia; aquí
/// basta con no tragarse el error ni entrar en pánico.
fn build_search_expression(
    where_expr: Option<&str>,
    filter: Option<&Value>,
) -> Result<Option<Expression>, WorkspaceError> {
    // Un `where` en blanco (solo espacios) se trata como ausente: no es una consulta malformada.
    let del_where = match where_expr.map(str::trim).filter(|s| !s.is_empty()) {
        Some(w) => Some(
            lodestar_core::parse::parse(w)
                .map_err(|e| WorkspaceError::Core(format!("«where» inválido: {}", e.message)))?,
        ),
        None => None,
    };
    let del_filter = match filter {
        Some(f) => Some(
            lodestar_core::filter::from_json(f)
                .map_err(|e| WorkspaceError::Core(format!("«filter» inválido: {}", e.message)))?,
        ),
        None => None,
    };
    Ok(match (del_where, del_filter) {
        (None, None) => None,
        (Some(e), None) | (None, Some(e)) => Some(e),
        (Some(w), Some(f)) => Some(Expression::And(vec![w, f])),
    })
}

/// Puntuación simple: nº de apariciones del texto (minúsculas) en el contenido crudo; `1.0` para un
/// texto vacío (todos los documentos empatan y el orden lo decide el `path`).
fn score_of(raw: &str, needle_lower: &str) -> f64 {
    if needle_lower.is_empty() {
        return 1.0;
    }
    let count = raw.to_lowercase().matches(needle_lower).count();
    if count == 0 {
        1.0
    } else {
        count as f64
    }
}

/// Genera un snippet compacto: una ventana de caracteres alrededor de la primera aparición del
/// `needle` (o del inicio del cuerpo si el texto está vacío o no aparece). Opera sobre `char`s
/// (nunca sobre bytes) para no romper en fronteras UTF-8, y colapsa los espacios en blanco. Devuelve
/// cadena vacía solo si el cuerpo colapsado está vacío (el llamante garantiza el no-vacío).
fn snippet_of(body: &str, needle_lower: &str) -> String {
    const WINDOW: usize = 160;
    const LEAD: usize = 30;
    let collapsed: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if collapsed.is_empty() {
        return String::new();
    }
    let chars: Vec<char> = collapsed.chars().collect();
    let match_idx = if needle_lower.is_empty() {
        None
    } else {
        let low: Vec<char> = chars
            .iter()
            .map(|c| c.to_lowercase().next().unwrap_or(*c))
            .collect();
        let needle: Vec<char> = needle_lower.chars().collect();
        find_subseq(&low, &needle)
    };
    let start = match_idx.map(|m| m.saturating_sub(LEAD)).unwrap_or(0);
    let end = (start + WINDOW).min(chars.len());
    let mut out = String::new();
    if start > 0 {
        out.push('…');
    }
    out.extend(&chars[start..end]);
    if end < chars.len() {
        out.push('…');
    }
    out
}

/// Índice del primer subslice contiguo de `hay` que iguala a `needle` (`None` si no aparece o
/// `needle` está vacío).
fn find_subseq(hay: &[char], needle: &[char]) -> Option<usize> {
    if needle.is_empty() || needle.len() > hay.len() {
        return None;
    }
    (0..=hay.len() - needle.len()).find(|&i| hay[i..i + needle.len()] == *needle)
}

/// Codifica un offset de paginación como cursor opaco (hexadecimal). Autosuficiente: como el orden de
/// resultados es determinista y solo depende del contenido, un offset reanuda idénticamente en
/// cualquier servidor fresco.
fn encode_cursor(offset: usize) -> String {
    format!("{offset:x}")
}

/// Decodifica un cursor a su offset. Un cursor malformado se interpreta como el inicio (offset 0).
fn decode_cursor(cursor: &str) -> usize {
    usize::from_str_radix(cursor, 16).unwrap_or(0)
}

// ---------------------------------------------------------------------------
// `metadata_inspect` — envoltorio de respuesta de la tool (E20-H03, sustituye a `schema_inspect`).
//
// Es solo el discriminador de modo: un enum `untagged` que envuelve los tipos de wire del CORE
// (`MetadataCatalog`/`FieldInspection`, `core::types`, con su serde ya fijado en E20-H03). No es una
// capa DTO paralela (invariante #4): el contrato de tipos vive en `core::types`; esto es framing de
// tool (qué proyección devuelve cada `mode`), igual que `KnowledgeGetResponse`.
// ---------------------------------------------------------------------------

/// Respuesta de la tool `metadata_inspect` (`ARCHITECTURE.md §20.10`, `REFACTOR_PHASE_2 §Fase 6`).
///
/// `untagged`: serializa como el valor interno directo, así que `Catalog` da `{ "fields": [ … ] }`
/// (la forma de [`MetadataCatalog`]) y `Field` da `{ "field": …, "presentIn": …, … }` (la de
/// [`FieldInspection`]) — sin envoltorio ni discriminador extra. Solo `Serialize` (+ `JsonSchema`
/// para el `outputSchema`): la tool PRODUCE esta respuesta, nunca la consume del wire.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum MetadataInspection {
    /// Modo `"catalog"`: el catálogo de propiedades del workspace.
    Catalog(MetadataCatalog),
    /// Modo `"field"`: la inspección de una propiedad concreta.
    Field(FieldInspection),
}

// ---------------------------------------------------------------------------
// `outputSchema` (E10-H13, `ARCHITECTURE.md §19.6`, decisión **D6b**, `docs/REFACTOR.md §13`).
//
// La tool MCP `knowledge_get` no sirve `DocumentView` a secas: la envuelve en `{ "document": … }`
// (`lodestar-mcp/src/tools.rs`, caso `"knowledge_get"`). El `outputSchema` declarado en
// `tools/list` debe describir la forma de wire REAL, así que aquí vive un wrapper mínimo — solo
// para derivar su `JsonSchema`, nunca construido por ningún servicio (`App::knowledge_get` sigue
// devolviendo `DocumentView`; el envoltorio lo aplica la fachada MCP).
// ---------------------------------------------------------------------------

/// Forma de wire de la respuesta de la tool `knowledge_get` (envoltorio de un único campo
/// `document`) — usado solo para derivar su `outputSchema`, ver nota de módulo arriba.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeGetResponse {
    /// El documento pedido.
    pub document: DocumentView,
}

/// Los `outputSchema` (JSON Schema, vía `schemars`) de las tools de lectura/verificación
/// (`workspace_status`/`knowledge_search`/`knowledge_get`/`metadata_inspect`/`knowledge_check`, …,
/// decisión **D6b**). `lodestar-mcp::tools::list` llama a estos helpers para poblar la clave
/// `outputSchema` de cada tool — así el schema se deriva del tipo Rust real que sirve cada
/// servicio (nunca se escribe a mano, no puede divergir silenciosamente del wire).
pub mod schemas {
    use serde_json::Value;

    use super::{
        ApplyResult, CheckReport, GraphQueryResult, ImpactReport, KnowledgeGetResponse,
        MetadataInspection, PlanResult, RevertResult, SearchResults, WorkspaceStatus,
    };

    /// Deriva el JSON Schema de `T` y lo serializa a `serde_json::Value`. `schemars::schema_for!`
    /// siempre produce una estructura serializable (nunca falla en la práctica) — el `expect`
    /// documenta esa garantía en vez de propagar un `Result` que ningún llamante puede fallar
    /// realmente.
    fn schema_of<T: schemars::JsonSchema>() -> Value {
        serde_json::to_value(schemars::schema_for!(T))
            .expect("un `RootSchema` de schemars siempre serializa a JSON")
    }

    /// `outputSchema` de `workspace_status` (== [`WorkspaceStatus`]).
    pub fn workspace_status_schema() -> Value {
        schema_of::<WorkspaceStatus>()
    }

    /// `outputSchema` de `knowledge_search` (== [`SearchResults`]).
    pub fn knowledge_search_schema() -> Value {
        schema_of::<SearchResults>()
    }

    /// `outputSchema` de `knowledge_get` (== [`KnowledgeGetResponse`], el envoltorio `{ document }`
    /// que sirve de verdad la tool — no [`super::DocumentView`] a secas).
    pub fn knowledge_get_schema() -> Value {
        schema_of::<KnowledgeGetResponse>()
    }

    /// `outputSchema` de `metadata_inspect` (== [`MetadataInspection`]).
    pub fn metadata_inspect_schema() -> Value {
        schema_of::<MetadataInspection>()
    }

    /// `outputSchema` de `knowledge_check` (== [`CheckReport`]).
    pub fn knowledge_check_schema() -> Value {
        schema_of::<CheckReport>()
    }

    /// `outputSchema` de `graph_query` (== [`GraphQueryResult`]).
    pub fn graph_query_schema() -> Value {
        schema_of::<GraphQueryResult>()
    }

    /// `outputSchema` de `impact_analyze` (== [`ImpactReport`]).
    pub fn impact_analyze_schema() -> Value {
        schema_of::<ImpactReport>()
    }

    /// `outputSchema` de `change_plan` (== [`PlanResult`]).
    pub fn change_plan_schema() -> Value {
        schema_of::<PlanResult>()
    }

    /// `outputSchema` de `change_apply` (== [`ApplyResult`]).
    pub fn change_apply_schema() -> Value {
        schema_of::<ApplyResult>()
    }

    /// `outputSchema` de `change_revert` (== [`RevertResult`]).
    pub fn change_revert_schema() -> Value {
        schema_of::<RevertResult>()
    }
}
