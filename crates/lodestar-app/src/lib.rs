//! `lodestar-app` — servicios de caso de uso finos sobre `lodestar-workspace`.
//!
//! Capa compartida por las dos fachadas de superficie (`lodestar-mcp`, `lodestar-cli`): la
//! fachada `App`, que envuelve un [`lodestar_workspace::Workspace`] y expone los métodos de caso
//! de uso (`workspace_status`, `knowledge_search`, `knowledge_get`, `metadata_inspect`,
//! `knowledge_check`, `change_plan`, `change_apply`, `change_revert`, …).
//!
//! Este crate depende solo de `lodestar-core` + `lodestar-workspace` + `serde`/`serde_json` — nunca
//! directamente de `rusqlite`/`git2`/`tokio` (invariante #2 de `CLAUDE.md`, verificado por
//! `cargo tree -p lodestar-app`).
//!
//! **Nota histórica (E29-H11, `decisiones §16(b)`)**: hasta esta historia el crate declaraba
//! además un `Envelope<T>`/`ErrorEnvelope` de protocolo (framing, no dominio — decisión **D3**,
//! `docs/history/REFACTOR_DISENO_PROPUESTA.md`, construido en E10-H01/H02). Se retiró por no tener
//! consumidor: el wire real de las tools MCP es `structuredContent` + texto con el código
//! (`ARCHITECTURE.md §20.10`) y la CLI responde con exit codes — ninguna de las dos fachadas
//! construyó jamás un envelope. `contracts/mcp.yml` ya documenta esa ausencia («json directo, sin
//! envelope») en cada tool.

use std::collections::{BTreeMap, BTreeSet};
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
    Severity, TypeError, ValidationReport, ValidationSummary, WorkspaceRevision,
    FRONTMATTER_ANCHOR,
};
use lodestar_core::{CoreError, DocumentSet};
use lodestar_workspace::{revert_transaction_id, Workspace, WorkspaceError};

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
/// - `DocumentAlreadyExists` → `DocumentAlreadyExists` (E28-H02: el destino de un `create`, o el
///   `to` de un `move`, ya está ocupado). Es el simétrico del anterior y por eso tiene código
///   propio: reusar `DocumentNotFound` mandaría al agente a buscar un documento que **sí** existe.
/// - `InboundLinksExist` → `InboundLinksExist` (borrar `reject` con entrantes, E12-H06).
/// - `RelationConstraintViolation` → `RelationConstraintViolation` (E12-H07; sin productor desde
///   E20-H03, ver [`CoreError`]).
/// - `InvalidStatusTransition` → `InvalidSchema` (E12-H07; sin productor desde E20-H03, ver
///   [`CoreError`]).
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
        CoreError::DocumentAlreadyExists(_) => ErrorCode::DocumentAlreadyExists,
        CoreError::InboundLinksExist(_) => ErrorCode::InboundLinksExist,
        CoreError::RelationConstraintViolation(_) => ErrorCode::RelationConstraintViolation,
        CoreError::InvalidStatusTransition(_) => ErrorCode::InvalidSchema,
        // NOTA E23-H11: el mapeo `FixNotFound → DocumentNotFound` desapareció con la variante y con
        // la operación `apply_fix` que la producía. Era además un código mentiroso: el documento
        // existía, y `DOCUMENT_NOT_FOUND` mandaba al agente a buscar el problema donde no estaba.
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
///   precondiciones internas sin un código más específico todavía en el catálogo de 17 (E28-H02).
/// - `PermissionDenied` (E11-H04: escritura bajo un `referenceRoot`, o fuera de `writableRoots`) →
///   `ErrorCode::PermissionDenied`, mapeo directo por nombre (mismo caso que `error_code` con
///   `CoreError::InvalidRelPath`).
/// - `InvalidResult` (E13-H01) / `WriteConflict` (E13-H02) / `WorkspaceRecoveryRequired`
///   (E13-H06) / `RecoveryFailed` (E25-H02: una transacción interrumpida cuyas copias no verifican;
///   su material queda en `.lodestar/runtime/journal/quarantine/<txnId>/`) → sus códigos homónimos
///   del catálogo, mapeo directo por nombre.
pub fn workspace_error_code(err: &WorkspaceError) -> ErrorCode {
    match err {
        WorkspaceError::Core(_) => ErrorCode::InternalIoError,
        WorkspaceError::Io(_) => ErrorCode::InternalIoError,
        WorkspaceError::NoCache => ErrorCode::InternalIoError,
        WorkspaceError::Store(_) => ErrorCode::InternalIoError,
        WorkspaceError::PermissionDenied(_) => ErrorCode::PermissionDenied,
        WorkspaceError::InvalidResult(_) => ErrorCode::InvalidResult,
        WorkspaceError::WriteConflict(_) => ErrorCode::WriteConflict,
        WorkspaceError::WorkspaceRecoveryRequired(_) => ErrorCode::WorkspaceRecoveryRequired,
        WorkspaceError::RecoveryFailed(_) => ErrorCode::RecoveryFailed,
        WorkspaceError::InvalidSchema(_) => ErrorCode::InvalidSchema,
    }
}

/// El error que devuelven los servicios de [`App`]: el [`ErrorCode`] estable del catálogo
/// **emparejado con un mensaje accionable** (E26-H07).
///
/// # Por qué existe
/// Hasta v0.4.0 los servicios devolvían `Result<_, ErrorCode>` — el código **pelado**, sin un sitio
/// donde poner el mensaje—, así que ocho de las diez tools MCP le entregaban al agente literalmente
/// `INVALID_SCHEMA`, sin una palabra sobre qué parámetro, qué valor o qué se esperaba. Un agente
/// puede **ramificar** por el código, pero necesita el mensaje para **corregir**. `knowledge_search`
/// ya lo hacía desde E24-H10 usando `WorkspaceError::InvalidSchema` + [`workspace_error_code`];
/// este tipo generaliza ese patrón a **todos** los servicios, en un solo sitio.
///
/// # Lo que NO es
/// **No** es una jerarquía paralela de códigos (invariante #4 de `CLAUDE.md`): el catálogo tiene
/// 17 filas (E28-H02) y vive **solo** en [`lodestar_core::types::ErrorCode`]. `AppError` es un
/// envoltorio de fachada —un `ErrorCode` del core + un `String`—, no un catálogo nuevo, y su
/// `Display` compone el texto de wire `«CÓDIGO: mensaje»` con `ErrorCode::as_str()`, nunca con un
/// literal propio.
///
/// Los mensajes van en **español** (regla de idioma del repo), salvo los identificadores congelados:
/// nombres de código, de tool, de parámetro y de operación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppError {
    /// Código estable del catálogo de 17 (`core::types::ErrorCode`, E28-H02) — por él ramifica el
    /// agente.
    pub code: ErrorCode,
    /// Mensaje accionable en español: qué parámetro, qué valor y qué se esperaba.
    pub message: String,
}

impl AppError {
    /// Empareja un código del catálogo con su mensaje.
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        AppError {
            code,
            message: message.into(),
        }
    }

    /// Atajo para el código más frecuente en la frontera: la **entrada** del agente no es
    /// interpretable ([`ErrorCode::InvalidSchema`]).
    pub fn invalid_schema(message: impl Into<String>) -> Self {
        AppError::new(ErrorCode::InvalidSchema, message)
    }
}

impl std::fmt::Display for AppError {
    /// El texto de wire de las fachadas: `«CÓDIGO: mensaje»`, con el código estable de
    /// [`ErrorCode::as_str`] (nunca el `Debug` de la variante Rust).
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {}", self.code.as_str(), self.message)
    }
}

impl std::error::Error for AppError {}

impl From<WorkspaceError> for AppError {
    /// Código por [`workspace_error_code`] y mensaje por el `Display` del error real: la misma
    /// pareja que `knowledge_search` componía a mano en la fachada MCP desde E24-H10.
    fn from(err: WorkspaceError) -> Self {
        AppError::new(workspace_error_code(&err), err.to_string())
    }
}

impl From<&CoreError> for AppError {
    /// Código por [`error_code`] y mensaje por el `Display` del [`CoreError`] — el diagnóstico del
    /// core viaja **entero** hasta la superficie (invariante #3: una sola verdad computada).
    fn from(err: &CoreError) -> Self {
        AppError::new(error_code(err), err.to_string())
    }
}

// ---------------------------------------------------------------------------
// `workspace_status` (E10-H08, `ARCHITECTURE.md §19.6`, `docs/history/REFACTOR.md §9.1/§7`).
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
/// `WorkspaceStatus`, `docs/history/REFACTOR.md §9.1`).
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
/// `docs/history/REFACTOR.md §9.1`).
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

/// Estado de una posible transacción interrumpida (`recovery` de `WorkspaceStatus`), computado
/// desde los write-ahead journals no sellados que haya en `.lodestar/runtime/` (E23-H04).
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct StatusRecovery {
    /// `true` si hay una transacción sin terminar pendiente de recuperar
    /// ([`Workspace::recovery_pending`]).
    ///
    /// Fue un `false` **literal** desde E10-H08 hasta E23-H04, pese a que la mecánica de detección
    /// existía y funcionaba desde E13-H06: un hueco de cableado que hacía que, tras un crash, la
    /// primera tool que llama un agente en cada sesión le dijera que todo estaba bien. El agente
    /// planificaba con normalidad y solo descubría el problema cuando `change_apply` reventaba con
    /// `WORKSPACE_RECOVERY_REQUIRED`.
    pub pending_transaction: bool,
}

/// Proyección de estado del workspace — la primera tool que se espera que llame un agente en
/// cada sesión (`docs/history/REFACTOR.md §7`, §9.1). Compone `core::types::workspace_revision` +
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
    pub valid: bool,
    /// Recuento agregado de documentos/enlaces/diagnósticos.
    pub counts: StatusCounts,
    /// Capacidades habilitadas por el perfil de arranque.
    pub capabilities: StatusCapabilities,
    /// Estado de recuperación de transacciones. **Real desde E23-H04** (`Workspace::recovery_pending`
    /// sobre los write-ahead journals no sellados); antes era un `false` literal — ver
    /// [`StatusRecovery::pending_transaction`].
    pub recovery: StatusRecovery,
    /// Los recibos de transacción persistidos, **del más reciente al más antiguo** (E23-H11).
    /// Vacío si no hay ninguno — nunca ausente.
    pub receipts: Vec<ReceiptSummary>,
}

/// Entrada del listado de recibos de [`WorkspaceStatus::receipts`] (E23-H11): **lo justo para
/// elegir cuál revertir**, no el recibo entero.
///
/// El recibo completo (con `previousRevision`, `changedPaths` y el `semanticDiff`) se sigue leyendo
/// por `change_revert`; `workspace_status` se llama en CADA sesión y su payload no puede crecer con
/// hasta 20 recibos completos. `changedPathCount` es el número de rutas afectadas —eco directo de
/// `ChangeReceipt::changed_paths`, que es de donde sale—, suficiente para reconocer la transacción
/// sin arrastrar la lista.
#[derive(Debug, Clone, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ReceiptSummary {
    /// El id con el que `change_revert` localiza el recibo y sus copias de recuperación.
    pub receipt_id: ReceiptId,
    /// El change set que originó la transacción.
    pub change_set_id: ChangeSetId,
    /// La revisión de workspace que dejó el apply: «el estado al que se volvería» al revertir.
    pub result_revision: WorkspaceRevision,
    /// Cuántas rutas tocó la transacción.
    pub changed_path_count: usize,
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
/// traduce peticiones de protocolo a operaciones del `Workspace` y devuelve sus tipos de
/// servicio directamente (`structuredContent` en el MCP, sin envelope — E29-H11). Expone
/// `workspace_status` (E10-H08), `knowledge_search` (E10-H09), `knowledge_get` (E10-H10),
/// `metadata_inspect` (E20-H03), `knowledge_check`, … .
pub struct App {
    workspace: Workspace,
}

impl App {
    /// Abre el workspace en `root` y construye la fachada de servicios. Delega en
    /// [`Workspace::open`] — mismas garantías: cache incremental **no** activada y, desde E23-H12,
    /// apertura **hermética** (abrir no escribe nada en el proyecto del usuario).
    pub fn open(root: &Path) -> Result<Self, WorkspaceError> {
        let workspace = Workspace::open(root)?;
        Ok(App { workspace })
    }

    /// Envuelve un [`Workspace`] ya abierto (p. ej. en tests, o un caller que ya gestiona su propio
    /// ciclo de vida del workspace).
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
    /// está en esa lista, `Err` con [`ErrorCode::DocumentNotFound`] **y el path que no resolvió**
    /// (E26-H07): es la diferencia entre «te equivocaste de ruta» y «olvidaste el parámetro», que
    /// desde esa historia son dos errores distintos.
    ///
    /// `ErrorCode::AmbiguousReference` queda RESERVADO para cuando exista resolución por `id`
    /// (`REFACTOR §6.1`) — no-goal de esta historia (IDs estables/federación). En v2 `DocumentRef.id`
    /// es siempre `None`, así que esta función nunca lo produce todavía.
    pub fn resolve_ref(&self, r: &DocumentRef) -> Result<RelPath, AppError> {
        let analysis = self.workspace.analyze()?;
        if analysis.documents.contains(&r.path) {
            Ok(r.path.clone())
        } else {
            Err(AppError::new(
                ErrorCode::DocumentNotFound,
                format!(
                    "«{}» no es un documento del workspace: la identidad de un documento es su \
                     ruta relativa, tal y como la devuelven knowledge_search o graph_query",
                    r.path.as_str()
                ),
            ))
        }
    }

    /// Proyección de estado del workspace (E10-H08): config activa, capacidades del perfil,
    /// conformidad y recuento agregado — la primera tool que debe llamar un agente en cada
    /// sesión (`docs/history/REFACTOR.md §7`).
    ///
    /// Compone `DocumentSet::analyze` (una sola verdad computada, invariante #3) +
    /// `core::types::workspace_revision` (E10-H03) + `WorkspaceConfig::load` (I/O de `workspace`,
    /// nunca del core) — sin lógica de dominio propia.
    /// Completa o deshace una transacción interrumpida **antes** de leer el canónico, si la hay
    /// (E24-H03).
    ///
    /// No-op —y sin coste, sin lock y sin escribir— cuando no hay recuperación pendiente, que es
    /// el caso normal. Cuando la hay, toma el **mismo lock exclusivo de publicación** que
    /// `Workspace::apply_transaction` y delega en `Workspace::recover`, cuya decisión es
    /// determinista por el estado durable del journal.
    ///
    /// Se comprueba dos veces —antes y después de tomar el lock— porque entre ambas puede haber
    /// recuperado otro proceso: sin la segunda comprobación se recuperaría dos veces la misma
    /// transacción.
    ///
    /// **Esto es reparar, no publicar**: el canónico vuelve a uno de los dos bordes de la
    /// transacción interrumpida. Ninguna operación del plan que lo invoca se materializa aquí.
    fn recover_if_pending(&self) -> Result<(), AppError> {
        if !self.workspace.recovery_pending() {
            return Ok(());
        }
        let lock = self.workspace.acquire_lock()?;
        if self.workspace.recovery_pending() {
            self.workspace.recover()?;
            // E24-H06: recoger aquí, no solo en el camino de éxito. Justo después de un crash es
            // cuando hay basura en el plano de control —el staging de la transacción interrumpida
            // y los temporales a medio escribir—, y el GC solo se disparaba desde `change_apply` y
            // `change_revert` cuando terminaban bien: el flujo que produce la basura era
            // exactamente el que no la recogía. Best-effort: un fallo recogiendo no puede impedir
            // planificar.
            //
            // E25-H03: la variante que exige el lock como TESTIGO, porque aquí ya lo tenemos
            // (`lock`). `gc_receipts` lo adquiere él mismo y es fail-fast, así que llamarlo desde
            // dentro de este bloque lo convertiría en un no-op silencioso **para siempre**: el único
            // camino que barre justo después de un crash dejaría de barrer. Pasar el guard es lo que
            // hace que el compilador —y no un comentario— garantice que este barrido corre bajo el
            // lock; el `&lock` mantiene además el guard vivo hasta que el GC termina.
            let _ = self.workspace.gc_receipts_con_el_lock_tomado(&lock);
        }
        Ok(())
    }

    pub fn workspace_status(&self, profile: Profile) -> Result<WorkspaceStatus, AppError> {
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
            valid: analysis.hard_fail() == 0,
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
                // E23-H04: computado, no literal. `recovery_pending()` mira los journals no sellados
                // de `.lodestar/runtime/`, la única fuente durable de «hay algo a medias».
                pending_transaction: self.workspace.recovery_pending(),
            },
            // E23-H11: los recibos persistidos, acotados a lo justo para elegir cuál revertir. Sin
            // este listado, un agente que perdía el `receiptId` de `change_apply` no podía revertir
            // aunque el recibo siguiera en disco. No es una 11ª tool: la superficie converge en 10
            // (`§19.6`) y este es el sitio donde ya vive `recovery.pendingTransaction`.
            receipts: self
                .workspace
                .list_receipts()
                .into_iter()
                .map(|r| ReceiptSummary {
                    receipt_id: r.id,
                    change_set_id: r.change_set_id,
                    result_revision: r.result_revision,
                    changed_path_count: r.changed_paths.len(),
                })
                .collect(),
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
    /// El orden es el **único** que hay: E23-H11 retiró el parámetro `sort`, que se aceptaba y se
    /// **ignoraba en silencio** desde E10-H09 (`_sort` en esta misma firma). Además de mentirle al
    /// agente, un criterio alternativo rompería el cursor-offset, que se apoya justo en que el orden
    /// dependa solo del contenido. Reintroducirlo el día que se implemente es aditivo.
    ///
    /// **Proyección de frontmatter** (`include`, E23-H11): cada hit puede traer los campos de
    /// frontmatter que pida el llamador, para que ver el `status` de 30 resultados no cueste 30
    /// `knowledge_get` (el N+1 que dejó E19-H05 al retirar los campos privilegiados OKF). Las
    /// proyecciones llegan ya parseadas ([`FrontmatterProjection`], que es quien valida la forma
    /// `frontmatter.<fieldPath>` y rechaza lo demás con `INVALID_SCHEMA`) y se resuelven con
    /// [`ParsedFrontmatter::get`](lodestar_core::types::ParsedFrontmatter::get) — la única verdad de
    /// acceso a metadata (invariante #3), que además resuelve dot-paths anidados. Reglas:
    /// - la clave del mapa es el **field path pedido tal cual** (`"status"`, `"owner.name"`), sin el
    ///   prefijo `frontmatter.` y sin re-anidar;
    /// - los valores viajan **crudos**, con su tipo YAML (un número es número, una lista es lista);
    /// - un campo **ausente en ese documento no aparece** en su mapa — nunca un `null` disfrazado,
    ///   misma regla que el `include` de [`App::knowledge_get`];
    /// - sin `include`, el hit conserva exactamente su forma anterior (no aparece `frontmatter`).
    ///
    /// Cada resultado lleva `revision` = [`DocumentRevision`] del contenido en disco (blake3, E10-H03)
    /// y un `snippet` compacto NO vacío; la estructura [`SearchResult`] **no tiene** campo `body`, así
    /// que es imposible filtrar el cuerpo completo por esta vía.
    ///
    /// Un `where_expr`/`filter` **malformado** (no parseable) se surface con `INVALID_SCHEMA` y el
    /// diagnóstico del parser del core (E24-H10, E26-H07).
    ///
    /// **Un [`TypeError`] al evaluar ABORTA la consulta** (E26-H08): una expresión bien formada
    /// sobre datos de otro tipo (p. ej. `priority >= "high"` sobre un `priority` numérico, o
    /// `priority starts_with "3"` sobre ese mismo campo numérico desde E29-H04) devuelve
    /// `Err(AppError)` con `INVALID_SCHEMA` y un mensaje que nombra campo, operador, los tipos
    /// implicados y el documento (`error_de_tipo`). Hasta v0.4.0 ese documento se **excluía en
    /// silencio** —el
    /// criterio de E19-H04: «el corpus es heterogéneo y un tipo incompatible no debe tumbar la
    /// consulta sobre los demás»—, y el precio era una lista recortada, decidida documento a
    /// documento, indistinguible de la correcta; sobre un corpus homogéneo, un `[]` indistinguible
    /// de «no hay resultados». E26-H08 revisa ese criterio con el mismo principio con que E24-H07
    /// revisó el del parseo: una respuesta silenciosamente equivocada es peor que un error.
    ///
    /// **Determinismo y ámbito del error** (la decisión, porque el orden de los criterios la fija):
    /// el `where`/`filter` se evalúa **antes** que el `text`, sobre el orden total de
    /// [`Analysis::documents`], y se reporta el **primer** `Err` de ese orden. Consecuencias
    /// deliberadas:
    /// - un documento que el `text` habría descartado **sí** dispara su `TypeError`: el error es de
    ///   la CONSULTA («este `where` no es respondible sobre este workspace»), no del subconjunto que
    ///   el `text` deja pasar. Lo contrario —dejar que un `text` más estrecho tape el error— haría
    ///   que la misma consulta fuera legal o ilegal según un parámetro que no habla de tipos, y que
    ///   añadir resultados a una búsqueda la rompiera;
    /// - `limit`/`cursor` **tampoco** cambian el veredicto: la página se recorta después de recorrer
    ///   el orden entero. Cortocircuitar al llenar la página sería más barato, pero haría que
    ///   `limit: 1` tuviera éxito donde `limit: 100` falla — la corrección manda (E26-H08).
    pub fn knowledge_search(
        &self,
        text: &str,
        where_expr: Option<&str>,
        filter: Option<&Value>,
        include: &[FrontmatterProjection],
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<SearchResults, AppError> {
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

            // (1) Intersección con el lenguaje de consulta. Va ANTES del `text` a propósito
            //     (E26-H08): un `TypeError` es un error de la CONSULTA, no del subconjunto, así que
            //     se decide sobre el orden total y no sobre lo que el `text` haya dejado pasar. El
            //     `?` propaga el PRIMER `Err` del orden total —este bucle recorre
            //     `analysis.documents` de principio a fin y la paginación es posterior—, así que ni
            //     el `text` ni el `limit` pueden cambiar el veredicto. `Ok(false)` sigue siendo
            //     exclusión.
            if let Some(expr) = &expr {
                let doc = EvalDocument {
                    path,
                    frontmatter: parsed.frontmatter.as_ref(),
                    body: &parsed.body,
                };
                if !evalua_documento(expr, &doc, analysis)? {
                    continue;
                }
            }

            // (2) Intersección con el FTS de `text` (subcadena, verdad del core).
            if !text_trim.is_empty() && !loose_text_match(path, &fm, &parsed.body, &needle) {
                continue;
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

            // Proyección pedida (E23-H11): solo si el llamador pidió algo, y solo lo que pidió —
            // `knowledge_search` sigue siendo la tool de payload acotado (nada de volcar el
            // frontmatter entero). Un campo que este documento no tiene, sencillamente no entra.
            let frontmatter = (!include.is_empty()).then(|| {
                let mut proyectado = BTreeMap::new();
                for proyeccion in include {
                    if let Some(valor) = fm.get(proyeccion.field_path()) {
                        // YAML → JSON conservando el tipo (no hay coerción: `§20.2`): un número
                        // sigue siendo número, una lista lista y un mapa mapa. Dos casos los
                        // NORMALIZA `serde_json` en silencio, y conviene saber cuáles son porque no
                        // fallan:
                        // - un flotante **no finito** (`.nan`, `.inf`) no existe en JSON y viaja
                        //   como `null` (no se omite): `nan: .nan` se proyecta `"nan": null`;
                        // - una **clave escalar no-string** de un mapa se estringa:
                        //   `config: {1: uno}` viaja como `{"1": "uno"}`.
                        // El `Err` que descarta el campo queda para lo que de verdad no es
                        // representable —una clave que es lista o mapa, legal en YAML—: entonces se
                        // omite, como si el campo estuviera ausente, antes que tumbar la búsqueda
                        // entera por un documento exótico.
                        //
                        // Lo que NO se colapsa (es el criterio de la historia, «nunca un `null`
                        // disfrazado»): un campo **ausente** no aporta clave, mientras que un campo
                        // **presente con `null` explícito** sí aparece, con valor `null` —
                        // `ParsedFrontmatter::get` ya distingue los dos casos y aquí se respeta.
                        if let Ok(json) = serde_json::to_value(valor) {
                            proyectado.insert(proyeccion.key().to_string(), json);
                        }
                    }
                }
                proyectado
            });

            results.push(SearchResult {
                path: path.clone(),
                id: None,
                title,
                snippet,
                score: score_of(raw, &needle),
                revision,
                frontmatter,
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
        // E30-H01: la cota la aplica `pagina()`, el punto ÚNICO de mecánica de paginación. Hasta
        // v0.5.0 estas cuatro líneas estaban copiadas aquí, en `knowledge_check` y en `graph_query`,
        // cada una con su `decode_cursor` — por eso el defecto del cursor era cuádruple. El
        // `nextCursor` solo aparece si hubo progreso y quedan resultados (evita bucles con
        // `limit == 0`), que es la misma regla de antes.
        let (page, next_cursor) = pagina(
            results,
            limit,
            cursor,
            DEFAULT_SEARCH_LIMIT,
            MAX_SEARCH_LIMIT,
            &CursorScope::KnowledgeSearch,
        )?;

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
    /// `"backlinks"`, `"diagnostics"`; `"revision"` es aceptado pero no-op,
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
    /// El campo `diagnostics` de esta proyección viene **solo** de `Analysis::diagnostics`
    /// (invariante #3).
    ///
    /// # `externalReferences`, RETIRADO en E23-H12
    ///
    /// El valor `"externalReferences"` del `include` —que resolvía contra disco los campos de
    /// frontmatter `implemented_by`/`verified_by` y devolvía `{path, exists}`— se retiró **sin
    /// sustituto**: eran las últimas claves con semántica impuesta y no configurable, contra el
    /// invariante 3 de `ARCHITECTURE.md §20.2`. Hoy es un valor desconocido del `include`, y como
    /// tal se ignora (ningún campo extra viaja en la respuesta).
    ///
    /// **Ruta de migración**: apuntar a código NO desapareció — un enlace Markdown a un fichero de
    /// código viaja en `outgoingLinks` ya clasificado como
    /// [`LinkTarget::WorkspaceFile`](lodestar_core::types::LinkTarget::WorkspaceFile) (`§20.6`), con
    /// su diagnóstico si el destino no existe. Lo que desapareció es que un **nombre de campo** de
    /// frontmatter activase esa resolución.
    pub fn knowledge_get(
        &self,
        r: &DocumentRef,
        include: &[String],
        sections: Option<&[Vec<String>]>,
    ) -> Result<DocumentView, AppError> {
        let path = self.resolve_ref(r)?;
        let doc_set = self.workspace.document_set()?;
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
        // Mismo cómputo que `knowledge_search` y `graph_query` (invariante #3: una sola verdad
        // computada, nunca una segunda implementación del título).
        let title = model::derived_title(parsed.frontmatter.as_ref(), &parsed.body, &path);
        Ok(DocumentView {
            path,
            title,
            revision,
            frontmatter,
            body,
            outgoing_links,
            backlinks,
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
    ///   `Err(ErrorCode::InvalidSchema)`, **con un mensaje que dice cuál de las dos cosas pasó**
    ///   (E26-H07).
    ///
    /// Un `mode` sin reconocer → `Err(ErrorCode::InvalidSchema)` (nunca entra en pánico). Un
    /// workspace sin frontmatter en ningún documento NO es un error: el catálogo sale vacío.
    ///
    /// # Un solo dialecto de dot-paths (E26-H09)
    ///
    /// El `field` se normaliza con [`lodestar_core::parse::build_field_path`] —el **mismo** punto
    /// por el que pasan `where`, `filter` y `has`/`missing`— y no con `FieldPath::parse`, que hasta
    /// v0.4.0 hacía de esta tool un segundo dialecto: `frontmatter.status` buscaba una clave literal
    /// `frontmatter` y devolvía `presentIn: 0` sobre una base llena de `status`, y `graph.backlinks`
    /// inspeccionaba la clave del frontmatter mientras el mismo texto en un `where` consultaba el
    /// grafo. De ahí hereda `field` las tres reglas del lenguaje: la **abreviatura**
    /// (`frontmatter.status` ≡ `status`), el **anclaje** (`frontmatter.graph.backlinks` alcanza la
    /// clave del usuario) y el **rechazo** de una propiedad desconocida bajo namespace reservado
    /// (`graph.backlink`, con typo), con el mismo mensaje que da la consulta.
    ///
    /// Y dos reglas propias. La primera: un namespace reservado **válido** (`graph.backlinks`,
    /// `document.path`) tampoco es inspeccionable → `Err(ErrorCode::InvalidSchema)`.
    /// `metadata_inspect` describe **metadata**, y una propiedad calculada no vive en ningún
    /// frontmatter: no tiene presencia ni vocabulario que describir. El mensaje dice por dónde sí:
    /// `graph_query` para el grafo, y el anclaje `frontmatter.` para la clave homónima del usuario.
    ///
    /// La segunda: un `field` que empieza por el anclaje, **no encuentra nada** y sin embargo es un
    /// nombre que el **catálogo anuncia** es la colisión con una clave de primer nivel llamada
    /// literalmente `frontmatter` → `Err(ErrorCode::InvalidSchema)` explicando la ambigüedad, en vez
    /// de un `presentIn: 0` indistinguible de una ausencia legítima. El caso normal no cambia: si la
    /// resolución anclada encuentra algo, se responde con ella.
    ///
    /// # Paginación (E26-H10)
    ///
    /// `limit`/`cursor` acotan la **lista** de los dos modos —`fields` en `catalog`, `values` en
    /// `field`— con el mismo cursor-offset hex opaco y autosuficiente del resto de la superficie
    /// (`knowledge_search`/`knowledge_check`/`graph_query`): `limit` por defecto **100**, tope
    /// **1000** (`DEFAULT_METADATA_LIMIT`/`MAX_METADATA_LIMIT`), `next_cursor` `None` al agotar. Hasta
    /// v0.4.0 esta era la única de las diez tools sin cota, y sus dos modos devuelven respuestas de
    /// tamaño proporcional al workspace: el catálogo emite una fila por cada field path (mapas
    /// intermedios incluidos) y `values` una entrada por cada valor escalar distinto —N entradas para
    /// N documentos en un campo de alta cardinalidad (un `id`, una fecha, un `owner`)—.
    ///
    /// La cota la aplica **esta fachada**, no [`metadata::catalog`]/[`metadata::inspect_field`]: el
    /// core sigue puro y devolviendo la verdad completa (invariantes #2 y #3), y quien la trunca es
    /// quien sirve el wire. Y se pagina la **lista, no la estadística**: `present_in`/`missing_in`/
    /// `inferred_types` —y el `present_in` de cada fila del catálogo— se computan sobre **todo** el
    /// workspace, así que una página de 5 valores nunca implica un `presentIn: 5`.
    ///
    /// `Result<_, AppError>` (no `WorkspaceError`) — mismo patrón que [`App::knowledge_get`]: es un
    /// servicio de cara a la fachada MCP/CLI, y el código estable **con su mensaje** es lo que el
    /// llamante necesita para el wire de error.
    pub fn metadata_inspect(
        &self,
        mode: &str,
        field: Option<&str>,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<MetadataInspection, AppError> {
        let doc_set = self.workspace.document_set()?;

        match mode {
            "catalog" => {
                let catalog = metadata::catalog(&doc_set);
                let (fields, next_cursor) = pagina(
                    catalog.fields,
                    limit,
                    cursor,
                    DEFAULT_METADATA_LIMIT,
                    MAX_METADATA_LIMIT,
                    &CursorScope::metadata_catalog(),
                )?;
                Ok(MetadataInspection::Catalog(MetadataCatalogPage {
                    catalog: MetadataCatalog { fields },
                    next_cursor,
                }))
            }
            "field" => {
                let field = field.ok_or_else(|| {
                    AppError::invalid_schema(
                        "el modo «field» exige el parámetro «field» con el dot-path del campo a \
                         inspeccionar (p. ej. «status» u «owner.name»); el modo «catalog» lista los \
                         campos disponibles",
                    )
                })?;
                // E26-H09: el MISMO normalizador que `where`/`filter`/`has` (un solo dialecto por
                // construcción). Su `ParseError` ya distingue el dot-path malformado de la
                // propiedad desconocida bajo namespace reservado, y ese mensaje viaja entero.
                let field_path = lodestar_core::parse::build_field_path(field).map_err(|e| {
                    // El diagnóstico del core viaja ENTERO (invariante #3): distingue el dot-path
                    // malformado (`a..b`) de la propiedad desconocida bajo namespace reservado
                    // (`graph.backlink`), y en este segundo caso ya dice cómo alcanzar la clave
                    // homónima del usuario. La fachada solo añade qué parámetro falló.
                    AppError::invalid_schema(format!(
                        "«field» no es un dot-path inspeccionable: {}; se esperaba algo como \
                         «status» u «owner.name», recibido «{field}»",
                        e.message
                    ))
                })?;
                if let Some(props) = field_path.props_del_namespace() {
                    return Err(AppError::invalid_schema(
                        mensaje_namespace_no_inspeccionable(&field_path, props),
                    ));
                }
                let mut inspection = metadata::inspect_field(&doc_set, &field_path);
                // E26-H09: un `field` que empieza por el ANCLAJE y no encuentra nada puede ser la
                // colisión con una clave de primer nivel llamada literalmente `frontmatter`, que el
                // catálogo SÍ anuncia con ese texto (es su nombre literal, no un anclaje). Devolver
                // `presentIn: 0` sería el defecto que esta épica retira: una respuesta
                // silenciosamente equivocada sobre un dato que existe. Se comprueba contra el
                // catálogo —la misma verdad que el agente leyó— y solo en el caso vacío, que es el
                // único ambiguo: si la resolución anclada encuentra algo, manda ella.
                if inspection.present_in == 0 && empieza_por_el_anclaje(field) {
                    let anunciado = metadata::catalog(&doc_set)
                        .fields
                        .iter()
                        .any(|e| e.field.to_string() == field);
                    if anunciado {
                        return Err(AppError::invalid_schema(mensaje_colision_con_el_anclaje(
                            field,
                        )));
                    }
                }
                // La página se recorta AQUÍ, después de que los agregados
                // (`present_in`/`missing_in`/`inferred_types`) queden fijados sobre el workspace
                // entero: se pagina la lista, no la estadística (E26-H10).
                // E30-H01: el contexto se firma con el path NORMALIZADO, no con el texto tecleado:
                // «status» y «frontmatter.status» son la misma lista (mismo orden total), así que su
                // cursor tiene que ser el mismo — la identidad va con el campo que se inspecciona,
                // no con la forma en que se escribió.
                let (values, next_cursor) = pagina(
                    std::mem::take(&mut inspection.values),
                    limit,
                    cursor,
                    DEFAULT_METADATA_LIMIT,
                    MAX_METADATA_LIMIT,
                    &CursorScope::metadata_field(&inspection.field.to_string()),
                )?;
                inspection.values = values;
                Ok(MetadataInspection::Field(FieldInspectionPage {
                    inspection,
                    next_cursor,
                }))
            }
            _ => Err(AppError::invalid_schema(format!(
                "«mode» debe ser «catalog» (los campos que existen en la base) o «field» (el \
                 detalle de uno); recibido «{mode}»"
            ))),
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
    /// `summary` (errors/warnings/info) y `valid` (== `errors == 0`) se computan sobre **todo**
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
    ) -> Result<CheckReport, AppError> {
        let (doc_set, discovery_diagnostics) = self.workspace.document_set_with_discovery()?;
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

        // Summary/valid sobre TODO el scope (antes de minimum_severity y paginación).
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
        let valid = errors == 0;

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
        // E30-H01: misma convergencia que `knowledge_search` — la mecánica de la página vive en
        // `pagina()`, que es también quien firma y verifica el origen del cursor.
        let (page, next_cursor) = pagina(
            diagnostics_all,
            limit,
            cursor,
            DEFAULT_CHECK_LIMIT,
            MAX_CHECK_LIMIT,
            &CursorScope::KnowledgeCheck,
        )?;

        Ok(CheckReport {
            valid,
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
    ) -> Result<BTreeSet<RelPath>, AppError> {
        match scope {
            CheckScope::Workspace => Ok(analysis.documents.iter().cloned().collect()),
            CheckScope::Document { r#ref } => {
                let path = self.resolve_ref(r#ref)?;
                Ok(std::iter::once(path).collect())
            }
            CheckScope::Paths { paths } => {
                // E29-H05: cada path debe existir en el inventario, exactamente el mismo criterio
                // que ya aplican los brazos `Document`/`Affected` vía `resolve_ref` (invariante #3
                // — una sola verdad de existencia; se reusa en vez de duplicar el predicado). El
                // primero que no resuelva, en el orden RECIBIDO (no el del `BTreeSet` de salida),
                // decide el `DOCUMENT_NOT_FOUND`.
                let mut set: BTreeSet<RelPath> = BTreeSet::new();
                for path in paths {
                    let r#ref = DocumentRef {
                        path: path.clone(),
                        id: None,
                    };
                    let resolved = self.resolve_ref(&r#ref)?;
                    set.insert(resolved);
                }
                Ok(set)
            }
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
    /// (`--json`/`--sarif`/humano), por el **mismo camino** que [`App::knowledge_check`] scope
    /// `workspace` (E23-H01, invariante #3 de `CLAUDE.md`: *una sola verdad computada*).
    ///
    /// Hasta E23-H01 este método era `document_set().analyze()` a secas, y eso hacía que la CLI y el
    /// MCP dieran **veredictos contradictorios sobre el mismo workspace**: con
    /// `validation: {danglingDocumentLinks: ignore}` en `.lodestar/config.yaml`, `lodestar check`
    /// salía 1 («NO CONFORME») mientras `knowledge_check` respondía `valid: true`. Dos verdades
    /// sobre los mismos ficheros. Ahora se componen las **dos** mitades que le faltaban:
    ///
    /// 1. **La política de severidad** de `validation` (`§20.9`): cada diagnóstico pasa por
    ///    [`lodestar_workspace::config::ValidationSection::effective_severity`], que lo reclasifica
    ///    (`error`/`warning`) o lo
    ///    **suprime** (`ignore`). Con la config por defecto es la identidad, así que un workspace sin
    ///    `.lodestar/config.yaml` no cambia de veredicto.
    /// 2. **Los diagnósticos de descubrimiento** (`DOC-NOT-UTF8`, `DOC-TOO-LARGE`,
    ///    `SYMLINK-UNSUPPORTED`, `LINK-CASE-MISMATCH`…): describen ficheros que Lodestar **no pudo
    ///    incorporar al inventario**, así que no están en `analysis.documents` y el análisis puro no
    ///    los ve. `knowledge_check` ya los añadía; `lodestar check` los descartaba, o sea que la
    ///    mitad del catálogo de `§20.9` era invisible desde la puerta de CI.
    ///
    /// Los de descubrimiento se indexan en `diagnostics` **por su primer `target`** (el fichero que
    /// describen), que por definición **no** es uno de `analysis.documents`. Un diagnóstico **sin**
    /// `target` —`PATH-NOT-UTF8`, cuyo path no es representable como [`RelPath`], o el
    /// `WORKSPACE-EMPTY` de E29-H06, que describe la ausencia de todos— no tiene un fichero con el
    /// que entrar en el mapa. Hasta v0.5.0 se **descartaba** ahí mismo, así que solo lo veía
    /// `knowledge_check` (que los lleva en una lista aparte) y la puerta de CI se quedaba sin la
    /// mitad del catálogo. Desde E29-H06 entran bajo [`ANCHOR_WORKSPACE`], una clave **sintética**
    /// que no puede colisionar con ningún documento (el plano de control `.lodestar/` es el suelo
    /// duro del descubrimiento, `discovery::CONTROL_PLANE_EXCLUDE`), y por tanto tampoco puede
    /// pisar los diagnósticos de un fichero real.
    pub fn full_analysis(&self) -> Result<Analysis, AppError> {
        let (doc_set, discovery_diagnostics) = self.workspace.document_set_with_discovery()?;
        let mut analysis = doc_set.analyze().clone();
        let validation = &self.workspace.config().validation;

        // 1. Política de severidad sobre los diagnósticos de documento.
        for checks in analysis.diagnostics.values_mut() {
            checks.retain_mut(|check| match validation.effective_severity(check) {
                Some(level) => {
                    check.level = level;
                    true
                }
                // Familia a `ignore`: el diagnóstico no se reporta (ni cuenta para el veredicto).
                None => false,
            });
        }

        // 2. Diagnósticos de descubrimiento, bajo la clave de su primer `target` — y los que no
        //    tienen ninguno, bajo el ancla sintética del workspace (E29-H06).
        fusiona_diagnosticos_de_descubrimiento(&mut analysis, discovery_diagnostics, validation);

        Ok(analysis)
    }

    /// Consulta el grafo, consolidando en una sola tool lo que hoy son 4 tools separadas
    /// (`find_backlinks`/`neighborhood`/`find_orphans`/`find_dangling`, E11-H01,
    /// `ARCHITECTURE.md §19.6`, `REFACTOR §9.5/§15`).
    ///
    /// `operation` ∈ `"backlinks"`/`"outgoing"`/`"neighborhood"`/`"isolated"`/`"dangling"`:
    /// - `backlinks`/`outgoing`/`neighborhood` requieren `r` (resuelto con [`App::resolve_ref`]);
    ///   su **ausencia** es `Err(ErrorCode::InvalidSchema)`, con un mensaje que nombra el parámetro
    ///   que falta y la operación que lo exige (E26-H07). Hasta v0.4.0 era `DOCUMENT_NOT_FOUND`
    ///   —«no hay un código de falta-parámetro en el catálogo, y es el mismo error que produciría un
    ///   `ref` que no resuelve»—, y esa equivalencia era justo el defecto: quien **olvida** el `ref`
    ///   recibía el mismo error que quien apunta a un documento inexistente, y tomaba el camino de
    ///   recuperación equivocado (buscar el documento, en vez de mirar su llamada). Un `ref`
    ///   **presente** que no resuelve sigue siendo `DOCUMENT_NOT_FOUND`, que es lo que su nombre dice.
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
    ///   consecutivos `[a→..→b]`. La ausencia de cualquiera de los dos extremos es `INVALID_SCHEMA`
    ///   (E26-H07, mismo criterio que arriba). Si algún ref no resuelve → `Err(ErrorCode::DocumentNotFound)`; si
    ///   no hay camino, `nodes`/`edges` vacíos (nunca error). **Nota**: la paginación genérica
    ///   ordena `nodes` por `id`, así que el orden del camino se recupera de `edges`, no de `nodes`.
    /// - `cycles` no requiere `r`: reusa [`DocumentSet::cycles`]. `nodes` = la unión de los nodos que
    ///   participan en algún ciclo (SCC no trivial); `edges` = los enlaces del grafo internos a ese
    ///   conjunto. La partición en ciclos concretos la da el core; aquí se sirve el subgrafo cíclico
    ///   agregado (coherente con la forma `{nodes,edges}` de esta tool).
    /// - `components` no requiere `r`: reusa [`DocumentSet::components`]. Como las componentes conexas
    ///   particionan **todo** el grafo, se parte del grafo completo (`nodes`/`edges` de
    ///   [`DocumentSet::graph_model`]) **antes de paginar** (E26-H10: lo que viaja es una página de
    ///   ese grafo, no el grafo entero); el cliente reconstruye la partición con
    ///   [`DocumentSet::components`] o recorriendo las aristas.
    ///
    /// **Paginación**: orden total y estable de `nodes` por `id` (mismo criterio que
    /// `knowledge_search`/`knowledge_check`); `limit` trunca esa página con un cursor-offset opaco
    /// (mismo esquema hex, autosuficiente entre procesos). **E26-H10**: `limit` ausente vale **100**
    /// y se acota a **1000** (`DEFAULT_GRAPH_LIMIT`/`MAX_GRAPH_LIMIT`) —hasta v0.4.0 `None` significaba «el
    /// grafo entero», así que `components` sobre una base grande servía una respuesta del tamaño del
    /// workspace—; con `limit` mayor o igual al total no hay truncamiento y `nextCursor` es `None`.
    /// Las `edges` devueltas se acotan a los `nodes` que sobreviven a la página (origen y destino
    /// ambos presentes), así el subgrafo que se sirve es siempre coherente consigo mismo — nunca una
    /// arista "colgando" de un nodo que la paginación dejó fuera.
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
    ) -> Result<GraphQueryResult, AppError> {
        let doc_set = self.workspace.document_set()?;

        // Extremo (`ref` u `to`) de las operaciones que lo exigen (E26-H07). Que FALTE es
        // `INVALID_SCHEMA` —el agente tiene que mirar su llamada— y que no RESUELVA es
        // `DOCUMENT_NOT_FOUND` (lo dice `resolve_ref`) —el agente tiene que buscar el documento—:
        // dos caminos de recuperación distintos, que hasta v0.4.0 compartían código.
        let extremo = |valor: Option<&DocumentRef>, parametro: &str, papel: &str| {
            let r = valor.ok_or_else(|| {
                AppError::invalid_schema(format!(
                    "la operación «{operation}» exige el parámetro «{parametro}» ({papel}); las \
                     operaciones isolated/dangling/cycles/components no lo llevan"
                ))
            })?;
            self.resolve_ref(r)
        };

        let (mut nodes, mut edges): (Vec<GraphNode>, Vec<Edge>) = match operation {
            "backlinks" => {
                let path = extremo(r, "ref", "el documento consultado")?;
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
                let path = extremo(r, "ref", "el documento consultado")?;
                let nb = doc_set.neighborhood(&path, 1, Direction::Out);
                (nb.nodes, nb.edges)
            }
            "neighborhood" => {
                let path = extremo(r, "ref", "el centro del vecindario")?;
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
                let from = extremo(r, "ref", "el origen del camino")?;
                let dest = extremo(to, "to", "el destino del camino")?;
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
            // Una `operation` fuera de las anteriores es entrada inválida; mismo criterio que
            // `metadata_inspect` para un `mode` no reconocido. El mensaje enumera las válidas
            // (E26-H07): el código dice que el agente se equivocó, la lista le dice en qué.
            _ => {
                return Err(AppError::invalid_schema(format!(
                    "«operation» debe ser una de backlinks, outgoing, neighborhood, isolated, \
                     dangling, path_between, cycles o components; recibido «{operation}»"
                )))
            }
        };

        // Orden total y estable por `id` — paginación reproducible entre procesos frescos.
        nodes.sort_by(|a, b| a.id.cmp(&b.id));

        // E26-H10: `limit` ausente ya NO significa «el grafo entero». Hasta v0.4.0 `None => total`,
        // así que un `components` —que sirve el `graph_model` completo— volcaba una respuesta del
        // tamaño del workspace. Ahora la página por defecto es `DEFAULT_GRAPH_LIMIT` y el resto se
        // recorre por `nextCursor` (consecuencia declarada de la historia).
        // E30-H01: la mecánica —y con ella la firma de origen del cursor— la aplica `pagina()`, el
        // punto único. `truncated` sigue siendo «quedan nodos fuera de esta página», que es lo mismo
        // que «hay `nextCursor`»: la única diferencia con la fórmula inline de v0.5.0 es el caso
        // degenerado `limit == 0` (página vacía sobre un grafo no vacío), donde no emitir cursor es
        // lo correcto —emitirlo dejaba al agente en un bucle de páginas vacías— y `truncated` deja
        // de contradecirlo.
        let (page_nodes, next_cursor) = pagina(
            nodes,
            limit,
            cursor,
            DEFAULT_GRAPH_LIMIT,
            MAX_GRAPH_LIMIT,
            &CursorScope::GraphQuery,
        )?;
        let truncated = next_cursor.is_some();
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
    ) -> Result<ImpactReport, AppError> {
        // E21-H01: `kind` restringido a las operaciones de impacto del modelo universal (`§20.10`).
        // Los `kind` semánticos retirados caen aquí como esquema de entrada inválido.
        if kind != "move" && kind != "delete" {
            return Err(AppError::invalid_schema(format!(
                "«proposedOperation.kind» debe ser «move» o «delete»; recibido «{kind}». Los kind \
                 semánticos (deprecate, transition_status, change_relation, replace_document) se \
                 retiraron en E21-H01 con el modelo que los definía"
            )));
        }
        let path = self.resolve_ref(r)?;
        let doc_set = self.workspace.document_set()?;

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
    /// Devuelve un [`PlanResult`] (proyección de servicio) con el plan completo. `Err(AppError)`
    /// —código estable **y** mensaje— para el wire de error (mismo patrón que el resto de servicios
    /// de `App` desde E26-H07). El diagnóstico de un `selection.where`/`filter` malformado llega
    /// entero en ese mensaje, por la misma compilación de consulta que usa `knowledge_search`
    /// (`build_search_expression`, invariante #3).
    pub fn change_plan(
        &self,
        expected_workspace_revision: Option<WorkspaceRevision>,
        raw_ops: &Value,
        policy: PlanPolicy,
    ) -> Result<PlanResult, AppError> {
        // (0) Recuperación pendiente ANTES de leer nada (E24-H03). Si una transacción anterior
        //     quedó a medias, el disco tiene renames parciales: planificar sobre él captura una
        //     `base_revision` de un estado que `apply_transaction` va a deshacer en su paso (2),
        //     y el control optimista del paso (7) lo ve como un conflicto ajeno →
        //     `WRITE_CONFLICT` en la PRIMERA escritura del agente, siempre, con un código que
        //     además miente (lo alteró la propia recuperación de Lodestar, no otro escritor).
        //
        //     Recuperar aquí es reparar, no publicar: el plan sigue sin materializar su resultado
        //     en el canónico. Se hace bajo el MISMO lock de publicación que toma el apply, para
        //     que dos planificadores no recuperen a la vez, y solo cuando hay algo que recuperar
        //     —el camino normal no toma el lock ni escribe nada—.
        self.recover_if_pending()?;

        let doc_set = self.workspace.document_set()?;
        let cfg = self.workspace.config();
        let files = doc_set.files();
        let writable = &cfg.workspace.writable_roots;

        // (1) Revisión base del workspace + control optimista a nivel de workspace.
        let base_revision = workspace_revision(files, writable);
        if let Some(expected) = &expected_workspace_revision {
            if expected != &base_revision {
                return Err(AppError::new(
                    ErrorCode::RevisionConflict,
                    format!(
                        "el workspace ya no está en la revisión esperada: «expectedWorkspaceRevision» \
                         es {} y la actual es {}. Vuelve a leer el estado (workspace_status) y \
                         replanifica",
                        expected.0, base_revision.0
                    ),
                ));
            }
        }

        // (2)+(3) Normalización, acumulando en un ÚNICO change set. Dos formas de wire:
        //   · Array de ops sueltas `[ {op, …}, … ]` (E12-H08), con control optimista por op.
        //   · Selección masiva `{ selection: {where|filter}, operation: {<op>: {…}} }` (E21-H02):
        //     la consulta E19 elige documentos del workspace y la operación se expande a una
        //     `NormalizedOperation` por documento seleccionado, capturando su `DocumentRevision`.
        let (normalized, captured_revisions): (
            Vec<NormalizedOperation>,
            BTreeMap<RelPath, DocumentRevision>,
        ) = if raw_ops.get("selection").is_some() {
            expand_selection(&doc_set, raw_ops)?
        } else {
            let ops_arr = raw_ops.as_array().ok_or_else(|| {
                AppError::invalid_schema(
                    "se esperaba «operations» (un array de operaciones) o la forma de selección \
                     masiva «{selection, operation}»",
                )
            })?;
            let mut normalized: Vec<NormalizedOperation> = Vec::new();
            // E28-H04: la ocupación de paths con la que se juzgan las colisiones de existencia es
            // ACUMULADA — arranca del disco y cada op ya normalizada la actualiza (un `create`/
            // `move.to` ocupa, un `delete`/`move.from` libera). Hasta H04 todas las ops se
            // normalizaban contra el `doc_set` de partida, que deja de ser cierto a la segunda:
            // `[move a→final, move b→final]` aplicaba destruyendo `a` en silencio y
            // `[delete X, create X]` se rechazaba pese a ser legítimo. El criterio de colisión es el
            // mismo que el de una sola operación (`plan::EstadoOcupacion`, invariante #3): lo que
            // cambia es contra qué estado se pregunta.
            let mut ocupacion = plan::EstadoOcupacion::nueva(files);
            for raw in ops_arr {
                if let Some(expected) = raw.get("expectedRevision").and_then(Value::as_str) {
                    let target = op_target_path(raw)?;
                    let actual = files.get(&target).map(|raw_md| {
                        DocumentRevision::from_hash(*blake3::hash(raw_md.as_bytes()).as_bytes())
                    });
                    if actual.as_ref().map(|r| r.0.as_str()) != Some(expected) {
                        return Err(AppError::new(
                            ErrorCode::RevisionConflict,
                            format!(
                                "«{}» ya no está en la revisión «{expected}» que declara la \
                                 operación (ahora es {}). Vuelve a leerlo (knowledge_get) y \
                                 replanifica",
                                target.as_str(),
                                actual
                                    .map(|r| format!("«{}»", r.0))
                                    .unwrap_or_else(|| "inexistente".to_string())
                            ),
                        ));
                    }
                }
                let ops = normalize_raw_op(&doc_set, &ocupacion, raw)?;
                for op in &ops {
                    ocupacion.aplicar(op);
                }
                normalized.extend(ops);
            }
            (normalized, BTreeMap::new())
        };

        // (3-bis) Red de seguridad de E28-H04 sobre la secuencia YA normalizada, venga del array de
        //     ops o de la selección masiva: ninguna operación puede ocupar un path que otra del
        //     mismo plan tenga ocupado. Con el acumulado del bucle de arriba esto no dispara nunca
        //     —cada op ya se juzgó contra el estado que veía—, pero deja el veredicto verificado
        //     sobre el plan COMPLETO, que es lo que se persiste y se aplica: si alguna vía futura
        //     construyera `normalized` sin acumular, la colisión se ve aquí y no en disco. Comparte
        //     criterio con los guards (`plan::EstadoOcupacion`), así que no puede divergir de ellos.
        plan::assert_sin_colisiones_intra_plan(files, &normalized)
            .map_err(|e| AppError::from(&e))?;

        // (4) DocumentSet hipotético + análisis del plan (todo en memoria, sin escribir).
        let after_files =
            plan::apply_normalized_ops(files, &normalized).map_err(|e| AppError::from(&e))?;
        let after = DocumentSet::from_files(after_files);

        // (4-bis) Guard de descubrimiento (E15-H09): ningún path que el plan escribiría puede
        //     quedar fuera del inventario. Se comprueban los creados/modificados —los borrados
        //     estaban en el inventario por construcción— y se hace ANTES de persistir el plan, de
        //     modo que un plan rechazado no queda aplicable después. Nótese que aquí NO se llama a
        //     `assert_writable`: las raíces de la config se siguen juzgando en el apply.
        for (path, contenido) in after.files() {
            if files.get(path) != Some(contenido) {
                self.workspace.assert_discoverable(path)?;
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
            policy,
            expires_at: expires_at_string(),
            normalized_operations: normalized,
            risk,
            semantic_diff,
            impact,
            captured_revisions,
            diagnostics_before: before_report.summary,
            diagnostics_after: after_report.summary,
        };

        // (6) Persistencia en runtime (E12-H09, `ARCHITECTURE.md §19.4/§19.5`): un plan exitoso se
        // guarda entero en `.lodestar/runtime/plans/` para que `load_plan` (y, más adelante,
        // `change_apply`, E13) lo recupere por `changeSetId`. Es runtime — gitignored, fuera de
        // `WorkspaceRevision` (E9-H06/E10-H03) — así que NO usa el único-escritor atómico de
        // `lodestar_workspace::io` (ese protocolo protege el conocimiento canónico, no el scratch).
        persist_plan(&self.workspace, &result)?;

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
    pub fn load_plan(&self, id: &ChangeSetId) -> Result<PlanResult, AppError> {
        // Los tres motivos de PLAN_STALE comparten mensaje porque comparten remedio: el plan ya no
        // es utilizable y hay que volver a pedirlo (E26-H07 les da voz; el código no cambia).
        let no_utilizable = || {
            AppError::new(
                ErrorCode::PlanStale,
                format!(
                    "el plan «{}» ya no está disponible (no existe, se purgó el runtime o su \
                     fichero no es legible): vuelve a llamar a change_plan para obtener un \
                     changeSetId vigente",
                    id.0
                ),
            )
        };
        let path = plan_file_path(self.workspace.root(), id);
        let raw = std::fs::read(&path).map_err(|_| no_utilizable())?;
        let plan: PlanResult = serde_json::from_slice(&raw).map_err(|_| no_utilizable())?;

        let expires_at: u64 = plan.expires_at.parse().map_err(|_| no_utilizable())?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        if expires_at < now {
            return Err(AppError::new(
                ErrorCode::PlanExpired,
                format!(
                    "el plan «{}» caducó (expiresAt {expires_at}, ahora {now}): vuelve a llamar a \
                     change_plan para replanificar sobre el estado actual",
                    id.0
                ),
            ));
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
    ///    - **El veredicto del plan VINCULA** (E29-H07, `decisiones §18`): con la base ya
    ///      verificada, se **recomputa** [`plan::can_apply`] sobre el resultado hipotético del plan
    ///      con la [`PlanPolicy`] que el cliente mandó a `change_plan` (persistida en el plan). Si
    ///      es `false` → `Err(InvalidResult)` con un mensaje que **nombra la cláusula** que bloqueó
    ///      (`requireValidResult`/`allowWarnings`), **antes de tomar el lock** y sin escribir nada:
    ///      sin staging, sin journal, sin recibo y sin copias de recuperación. Hasta E29-H07
    ///      `canApply` era un consejo que nadie ejercía y el único filtro que quedaba era el gate de
    ///      staging —`transactions`, una política distinta— que no muerde con errores preexistentes
    ///      ni con warnings.
    /// 4. **Transacción**: [`Workspace::apply_transaction_con_recibo`] publica por el único escritor
    ///    (staging → lock → backup → journal + registro durable del recibo → renames atómicos),
    ///    devolviendo `(previous, result, changedPaths)`. Su `assert_writable` rechaza cualquier path
    ///    fuera de `writableRoots` → `PERMISSION_DENIED` ANTES de tocar el canónico.
    /// 5. **Receipt + GC**: persiste el [`ChangeReceipt`] de la aplicación completada (E13-H07) y
    ///    ejecuta la retención (`gc_receipts`).
    /// 6. Devuelve un [`ApplyResult`] (proyección de servicio) con `applied:true`, las revisiones
    ///    antes/después, los paths cambiados, el `semanticDiff` del plan y la conformidad post-apply.
    ///
    /// # El `receiptId` lo decide la transacción (E28-H03)
    /// El `receiptId` devuelto es el `txnId` **efectivo** con el que la publicación ocurrió, y viene
    /// del paso (4): como el `changeSetId` es determinista, replanificar el mismo cambio sobre la
    /// misma base repite el `txnId` candidato, y publicar bajo él borraba el `recovery/`/`receipts/`
    /// de la transacción anterior. La mecánica resuelve ahora la primera variante libre bajo el lock,
    /// así que **dos applies del mismo `changeSetId` devuelven `receiptId` distintos** y ninguno pisa
    /// al otro. Recalcularlo desde el `changeSetId` fuera de aquí ya no es fiable.
    ///
    /// # Publicar implica recibo (E25-H04)
    /// El **punto de no retorno** es el primer rename del paso (4). A partir de ahí el conocimiento ya
    /// cambió, así que este método no puede devolver `Err` por nada de lo que venga después: los pasos
    /// (5) y (6) —recibo, retención, conformidad— son **best-effort con aviso por stderr**. Un error ahí
    /// diría al agente que no se aplicó nada sobre algo que sí se aplicó, y no habría salida:
    /// `change_revert` responde `PLAN_EXPIRED` sin recibo y un segundo `change_apply` del mismo plan,
    /// `PLAN_STALE`, porque la base cambió. Degradarlos no basta por sí solo (no cubre el `SIGKILL`): lo
    /// que cierra el agujero es que el recibo se persista **con el journal**, dentro del paso (4), y que
    /// la recuperación por la vía COMPLETAR lo dé por bueno.
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
    ) -> Result<ApplyResult, AppError> {
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
    ) -> Result<ApplyResult, AppError> {
        // (1) Cargar el plan persistido (caducidad → PLAN_EXPIRED; ausente/ilegible → PLAN_STALE).
        let plan = self.load_plan(change_set_id)?;

        let cfg = self.workspace.config();
        let doc_set = self.workspace.document_set()?;
        let current_base = workspace_revision(doc_set.files(), &cfg.workspace.writable_roots);

        // (2) Control optimista a nivel de workspace (si el llamante fijó una expectativa).
        if let Some(expected) = &expected_workspace_revision {
            if expected != &current_base {
                return Err(AppError::new(
                    ErrorCode::RevisionConflict,
                    format!(
                        "el workspace ya no está en la revisión esperada: \
                         «expectedWorkspaceRevision» es {} y la actual es {}. Vuelve a leer el \
                         estado (workspace_status) y replanifica",
                        expected.0, current_base.0
                    ),
                ));
            }
        }

        // (3) Verificar `planHash` sobre la base ACTUAL: si el workspace cambió bajo el plan, el hash
        //     recomputado difiere del persistido → PLAN_STALE (no se escribe).
        let recomputed = compute_plan_hash(&current_base, &plan.normalized_operations);
        if recomputed != plan.plan_hash {
            return Err(AppError::new(
                ErrorCode::PlanStale,
                format!(
                    "el conocimiento cambió bajo el plan «{}»: se planificó sobre la revisión {} y \
                     el workspace está en {}, así que no se ha escrito nada. Vuelve a llamar a \
                     change_plan sobre el estado actual",
                    plan.change_set_id.0, plan.base_workspace_revision.0, current_base.0
                ),
            ));
        }

        // (3-veredicto) EL VEREDICTO DEL PLAN VINCULA (E29-H07, `decisiones §18`). Hasta esta historia
        //     `canApply` viajaba al cliente y nadie lo ejercía: un plan que la superficie declaraba
        //     «no aplicable bajo tu policy» se publicaba igual si el agente insistía, porque el
        //     único filtro de validez que quedaba era el gate de staging (`rejectNewErrors`/
        //     `allowExistingErrors` de `transactions`, E20-H04) — una política DISTINTA, que no
        //     muerde ni con errores PREEXISTENTES ni con warnings. Dos políticas, una publicada y
        //     otra ejercida.
        //
        //     Se RECOMPUTA en vez de leer el `canApply` persistido (la recomendación de la historia):
        //     el apply re-verifica todo lo demás —el `planHash` sobre la base actual, la revisión— y
        //     un booleano congelado sería el único veredicto que no se re-computa. Se llama a
        //     `plan::can_apply`, el MISMO predicado que usó `change_plan` (invariante #3: el
        //     predicado no se reimplementa aquí), sobre el resultado hipotético reconstruido con
        //     `plan::apply_normalized_ops` — que es idéntico al que vio `change_plan`, porque el
        //     paso (3) acaba de verificar que la base y las operaciones son las mismas.
        //
        //     Ocurre ANTES de tocar la transacción: sin lock, sin staging, sin journal, sin recibo y
        //     sin copias de recuperación. Es un veredicto sobre el PLAN, no sobre el disco.
        let after_plan = DocumentSet::from_files(
            plan::apply_normalized_ops(doc_set.files(), &plan.normalized_operations)
                .map_err(|e| AppError::from(&e))?,
        );
        let after_report = plan::validate_result(&after_plan);
        if !plan::can_apply(&after_report, &plan.policy) {
            return Err(AppError::new(
                ErrorCode::InvalidResult,
                plan_policy_rejection_message(&plan, &after_report),
            ));
        }

        // (3-bis) Red de colisión intra-plan EN EL LADO QUE ESCRIBE (E28-H04, reserva). El plan es un
        //     artefacto DURABLE con TTL propio, no un valor en memoria: que `change_plan` haya
        //     juzgado las operaciones entre sí no protege a este camino, porque el fichero pudo
        //     escribirlo un binario anterior al guard —o cualquier vía futura que construyera
        //     `normalizedOperations` sin acumular ocupación—. Sin esto, un plan persistido con
        //     `[move a→final, move b→final]` publica y `a.md` desaparece en silencio: los pasos (1)
        //     a (3) juzgan caducidad, revisión y `planHash`, y ninguno de los tres mira una
        //     operación contra otra. Es el MISMO juicio del core que usa `change_plan`
        //     (`plan::EstadoOcupacion`, invariante #3), no un criterio nuevo, y se hace ANTES de la
        //     primera escritura: el disco queda intacto.
        plan::assert_sin_colisiones_intra_plan(doc_set.files(), &plan.normalized_operations)
            .map_err(|e| AppError::from(&e))?;

        // (4) Publicar por el único escritor (staging → lock → backup → journal + REGISTRO DURABLE DEL
        //     RECIBO → renames). El guard `assert_writable` de la transacción rechaza fuera de
        //     `writableRoots` → PERMISSION_DENIED antes de tocar el canónico. Se presta el
        //     `semanticDiff` del plan porque es la única pieza del recibo que la mecánica de disco no
        //     puede conocer (E25-H04): con ella, el recibo queda persistido ANTES del primer rename y
        //     una publicación no puede volverse irreversible por morirse el proceso después.
        //
        //     E28-H03 — LA IDENTIDAD LA DECIDE LA TRANSACCIÓN, NO ESTA FACHADA. El `txnId` (y con él
        //     el `receiptId`) ya no se deriva aquí del `changeSetId`: la transacción lo resuelve bajo
        //     el lock contra el material vigente en disco y lo devuelve. Derivarlo fuera era correcto
        //     mientras un `changeSetId` identificara a lo sumo una transacción publicada, y deja de
        //     serlo en cuanto se replanifica el mismo cambio sobre la misma base: el hash es
        //     determinista, así que el segundo apply reutilizaba el id del primero y sobrescribía su
        //     `recovery/`/`receipts/` — las únicas copias con las que aquél se deshacía.
        let change_set = plan_to_change_set(&plan);
        let publicada = self
            .workspace
            .apply_transaction_con_recibo(&change_set, Some(&plan.semantic_diff))?;
        let receipt_id = ReceiptId(publicada.txn_id);
        let (previous, result, changed_paths) = (
            publicada.previous,
            publicada.result,
            publicada.changed_paths,
        );

        // Punto de caída de la FACHADA (E25-H04): entre el retorno de la transacción y el recibo. Es la
        // ventana en la que el canónico ya está publicado, el lock ya está soltado y —hasta esta
        // historia— no existía todavía ningún registro con el que deshacer el cambio. Sin
        // `--features test-failpoints` no genera ni una instrucción.
        #[cfg(feature = "test-failpoints")]
        if lodestar_workspace::failpoints::disparado(
            lodestar_workspace::failpoints::FailPoint::TrasLaTransaccionAntesDelRecibo,
        ) {
            return Err(AppError::new(
                ErrorCode::InternalIoError,
                "failpoint de prueba: caída simulada tras publicar la transacción y antes del recibo",
            ));
        }

        // (5) Receipt de la aplicación completada + retención (E13-H07). El `receiptId` es el `txnId`
        //     EFECTIVO con el que la transacción publicó (E28-H03), así el receipt localiza sus copias
        //     de recuperación por convención de nombre. Hasta E28-H03 se recalculaba aquí desde el
        //     `changeSetId`; hoy eso nombraría el candidato y no la transacción, porque la identidad
        //     se resuelve bajo el lock contra el material vigente.
        //
        //     DESDE AQUÍ TODO ES BEST-EFFORT (E25-H04): estos pasos corren con el canónico ya publicado,
        //     así que un `Err` suyo diría al agente que no se aplicó nada sobre algo que sí se aplicó —y
        //     sin salida, porque `change_revert` respondería `PLAN_EXPIRED` y un segundo `change_apply`
        //     del mismo plan, `PLAN_STALE`—. La transacción ya dejó el recibo persistido y promovido; esta
        //     escritura es la red de seguridad de que existe también si su promoción no pudo completarse.
        let receipt = ChangeReceipt {
            id: receipt_id.clone(),
            change_set_id: plan.change_set_id.clone(),
            previous_revision: previous.clone(),
            result_revision: result.clone(),
            changed_paths: changed_paths.clone(),
            semantic_diff: plan.semantic_diff.clone(),
        };
        if let Err(e) = self.workspace.write_receipt(&receipt) {
            eprintln!(
                "lodestar: aviso: no se pudo re-escribir el recibo de `{}` tras publicar: {e}",
                plan.change_set_id.0
            );
        }
        if let Err(e) = self.workspace.gc_receipts() {
            eprintln!("lodestar: aviso: la retención de recibos falló tras publicar: {e}");
        }

        // (6) Conformidad del workspace ya publicado (una sola verdad computada, invariante #3).
        //     También best-effort: si el canónico no se puede releer, la publicación sigue siendo un
        //     hecho. Lo único que se degrada es lo que se puede AFIRMAR de ella, y se degrada al lado
        //     conservador (`valid: false`): no se declara válido lo que no se ha podido comprobar.
        let validation = match self.workspace.analyze() {
            Ok(analysis) => ApplyValidation {
                valid: analysis.hard_fail() == 0,
                errors: analysis.hard_fail(),
                warnings: analysis.warn_count(),
            },
            Err(e) => {
                eprintln!(
                    "lodestar: aviso: el cambio se publicó pero su conformidad no se pudo computar: \
                     {e}. Se reporta como no verificado (`valid: false`); `knowledge_check` la \
                     recomputa"
                );
                ApplyValidation {
                    valid: false,
                    errors: 0,
                    warnings: 0,
                }
            }
        };

        Ok(ApplyResult {
            receipt_id,
            applied: true,
            previous_workspace_revision: previous,
            workspace_revision: result,
            changed_paths,
            semantic_diff: plan.semantic_diff,
            validation,
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
    /// 4. **Restauración recuperable**: delega en `Workspace::revert_transaction_con_recibo`, que
    ///    **re-verifica bajo el lock** la revisión observada en el paso 3 (E25-H05: entre esa
    ///    comprobación y el lock cabe otro escritor → [`ErrorCode::WriteConflict`]), verifica que las
    ///    copias de recuperación (E13-H04) existen, registra el [`ChangeReceipt`] de la inversa con su
    ///    journal y restaura por el único escritor, promoviendo el recibo al sellar. Lo que queda aquí
    ///    —re-escribir ese recibo y la retención (`gc_receipts`)— es **best-effort**: corre con la
    ///    reversión ya publicada, así que no puede convertirla en un `Err`.
    ///
    /// Devuelve un [`RevertResult`] con `reverted:true`, las revisiones antes/después de la
    /// transacción INVERSA (`previousWorkspaceRevision` == `resultRevision` del apply;
    /// `workspaceRevision` == `previousRevision` del apply, el estado restaurado) y los paths
    /// restaurados.
    ///
    /// # Revertir una reversión (E28-H01, E28-H03)
    ///
    /// Es una operación como cualquier otra, y **componible**: el recibo que se revierte puede ser el
    /// de una reversión previa. La identidad de la inversa se deriva del `receiptId` que se deshace
    /// —no del `changeSetId` que ese recibo lleva dentro, que las reversiones **heredan** de la
    /// transacción original— por [`lodestar_workspace::revert_transaction_id`], de modo que cada
    /// eslabón de la cadena (`X` → `X-revert` → `X-revert-2` → …) tiene su propio journal, sus propias
    /// copias y su propio recibo. Hasta E28-H01, derivar del `changeSetId` heredado hacía que revertir
    /// un `-revert` restaurase desde el árbol pre-apply —ya vigente: un no-op declarado exitoso— y
    /// sobrescribiese su propio material de recuperación, destruyendo el estado *redo* de forma
    /// permanente y silenciosa (defecto M-01 del testbench homelab).
    ///
    /// Ese id derivado es un **candidato** (E28-H03): la mecánica lo resuelve bajo el lock contra el
    /// material vigente y publica bajo la primera variante libre, así que un `X-revert` ya ocupado
    /// —por ejemplo por la reversión de un apply anterior con el mismo `changeSetId`— ya no deja la
    /// transacción sin salida. El `receiptId` que devuelve [`RevertResult`] es el **efectivo**: es el
    /// que localiza el recibo, y puede no coincidir con lo que el llamante derivaría por su cuenta.
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
    ) -> Result<RevertResult, AppError> {
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
    ) -> Result<RevertResult, AppError> {
        // (1) Cargar el receipt persistido. Ausente/purgado ⇒ transacción no disponible → PLAN_EXPIRED.
        let receipt = self.workspace.load_receipt(receipt_id).map_err(|_| {
            AppError::new(
                ErrorCode::PlanExpired,
                format!(
                    "no hay recibo «{}»: esa transacción ya no es reversible (el recibo nunca \
                     existió o la retención lo purgó)",
                    receipt_id.0
                ),
            )
        })?;

        // (2) Revisión actual del conocimiento escribible.
        let cfg = self.workspace.config();
        let doc_set = self.workspace.document_set()?;
        let current = workspace_revision(doc_set.files(), &cfg.workspace.writable_roots);

        // (3) Control optimista a nivel de workspace (si el llamante fijó una expectativa).
        if let Some(expected) = &expected_workspace_revision {
            if expected != &current {
                return Err(AppError::new(
                    ErrorCode::RevisionConflict,
                    format!(
                        "el workspace ya no está en la revisión esperada: \
                         «expectedWorkspaceRevision» es {} y la actual es {}",
                        expected.0, current.0
                    ),
                ));
            }
        }

        // (4) Ficheros afectados no alterados: el workspace sigue en la `resultRevision` del apply.
        //     Si difiere, algo cambió tras el apply → WRITE_CONFLICT y NO se revierte.
        if current != receipt.result_revision {
            return Err(AppError::new(
                ErrorCode::WriteConflict,
                format!(
                    "el conocimiento cambió después del apply «{}» (quedó en {} y ahora está en \
                     {}), así que revertir pisaría ese cambio: no se ha restaurado nada",
                    receipt.change_set_id.0, receipt.result_revision.0, current.0
                ),
            ));
        }

        // (5) Restaurar por el único escritor (transacción inversa recuperable con journal propio).
        //     `current` viaja con la llamada para que se **re-verifique BAJO EL LOCK** (E25-H05): el
        //     paso (4) mira sin lock —lo toma `revert_transaction_con_recibo`—, así que en esa ventana otro
        //     escritor puede tocar un `.md` afectado y la reversión le escribiría la copia respaldada
        //     encima. Y el `semanticDiff` del recibo original se presta para que la inversa registre su
        //     propio recibo con su journal, ANTES de su punto de no retorno.
        //
        //     E28-H01 — LA IDENTIDAD SE DERIVA DEL RECIBO, NO DEL `changeSetId` QUE LLEVA DENTRO.
        //     El `txnId` de la transacción que se deshace **es** el `receiptId` de su recibo: así lo
        //     nombran tanto `change_apply` (paso 5) como esta misma función, y de ahí que un mismo id
        //     localice su journal, su staging, sus copias y su recibo. Derivarlo del `changeSetId`
        //     —lo que se hacía hasta aquí— era correcto solo para el primer escalón de la cadena: un
        //     recibo `X-revert` **hereda** el `changeSetId` original, así que revertirlo recalculaba
        //     `orig_txn_id = X` y restauraba desde `recovery/X/` (el árbol pre-apply, ya vigente: un
        //     no-op) mientras `revert_txn_id` volvía a dar `X-revert` y la inversa se sobrescribía a
        //     sí misma, destruyendo el estado *redo*. Ese era el defecto M-01 del testbench homelab.
        //
        //     Se usa `receipt.id`, no el `receiptId` que llegó por parámetro: son el mismo id, pero el
        //     del recibo es el que el propio recibo declara (y el que `load_receipt` acaba de
        //     verificar), así que no depende de cómo lo escribiera el llamante.
        //
        //     E28-H03 — EL ID DERIVADO ES UN CANDIDATO, Y LA TRANSACCIÓN DEVUELVE EL EFECTIVO. Con
        //     `apply` resolviendo ya su propia identidad (arriba), un `X-revert` puede estar ocupado
        //     por la reversión de OTRA transacción de la misma cadena; la mecánica busca entonces la
        //     primera variante libre en vez de rechazar, y el `txnId` con el que publicó —el que
        //     nombra su journal, sus copias y su recibo— es el que viaja de vuelta.
        let orig_txn_id = receipt.id.0.clone();
        let revert_txn_id = revert_transaction_id(&orig_txn_id);
        let revertida = self.workspace.revert_transaction_con_recibo(
            &orig_txn_id,
            &revert_txn_id,
            &current,
            Some((&receipt.change_set_id, &receipt.semantic_diff)),
        )?;
        let revert_txn_id = revertida.txn_id;
        let (previous, result, changed_paths) = (
            revertida.previous,
            revertida.result,
            revertida.changed_paths,
        );

        // Punto de caída de la FACHADA (E25-H05, espejo del de `change_apply`): entre el retorno de la
        // transacción inversa y su recibo. Es la ventana en la que el canónico ya volvió atrás, el lock
        // ya está soltado y —hasta esta historia— no existía todavía ningún registro de que la
        // reversión hubiera ocurrido. Sin `--features test-failpoints` no genera ni una instrucción.
        #[cfg(feature = "test-failpoints")]
        if lodestar_workspace::failpoints::disparado(
            lodestar_workspace::failpoints::FailPoint::TrasLaTransaccionAntesDelRecibo,
        ) {
            return Err(AppError::new(
                ErrorCode::InternalIoError,
                "failpoint de prueba: caída simulada tras revertir la transacción y antes del recibo",
            ));
        }

        // (6) Receipt de la reversión (inversa: previous/result intercambiados respecto al apply) +
        //     retención. Su id nombra por convención las copias de recuperación de la inversa
        //     (`recovery/<revert_txn_id>/`), que el GC purgará junto al recibo.
        //
        //     El `changeSetId` se HEREDA de la transacción deshecha —una reversión no nace de un
        //     `change_plan`, así que no tiene uno propio— y es trazabilidad, no identidad: desde
        //     E28-H01 el `txnId` (y con él el `receiptId` y todo el material) se deriva del `receiptId`
        //     que se revierte, precisamente porque este campo NO distingue los eslabones de la cadena;
        //     y desde E28-H03 ese id derivado lo resuelve la mecánica contra el material vigente, así
        //     que el `revert_txn_id` de aquí es el EFECTIVO que devolvió la transacción, no el
        //     candidato con el que se la llamó.
        let revert_receipt_id = ReceiptId(revert_txn_id);
        let revert_receipt = ChangeReceipt {
            id: revert_receipt_id.clone(),
            change_set_id: receipt.change_set_id.clone(),
            previous_revision: previous.clone(),
            result_revision: result.clone(),
            changed_paths: changed_paths.clone(),
            semantic_diff: receipt.semantic_diff.clone(),
        };
        // DESDE AQUÍ TODO ES BEST-EFFORT (E25-H05, regla heredada de E25-H04): estos pasos corren con
        // la reversión ya publicada, así que un `Err` suyo diría al agente que no se revirtió nada
        // sobre algo que sí se revirtió. La transacción inversa ya dejó su recibo persistido y
        // promovido (`revert_transaction_con_recibo`); esta escritura es la red de seguridad de que
        // existe también si su promoción no pudo completarse.
        if let Err(e) = self.workspace.write_receipt(&revert_receipt) {
            eprintln!(
                "lodestar: aviso: no se pudo re-escribir el recibo de la reversión `{}` tras \
                 revertir: {e}",
                revert_receipt.id.0
            );
        }
        // Best-effort, por el mismo motivo que en `change_apply` (E25-H04): la retención corre con la
        // reversión ya publicada, así que un fallo suyo no puede convertirla en un error.
        if let Err(e) = self.workspace.gc_receipts() {
            eprintln!("lodestar: aviso: la retención de recibos falló tras revertir: {e}");
        }

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
    /// criterio que [`Workspace::ensure_managed_gitignore`] en `lodestar-workspace`. Es runtime
    /// puro: gitignored, fuera de `WorkspaceRevision` (E9-H06/E10-H03), no indexado y no expuesto
    /// por ninguna tool MCP (solo diagnóstico local).
    fn audit(&self, entry: AuditEntry) {
        if let Err(e) = try_append_audit(&self.workspace, &entry) {
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
///
/// El veredicto es **posterior a la publicación**, así que no puede negarla: si el análisis no se puede
/// computar, el apply sigue siendo un éxito y lo que se degrada es lo que se afirma de él (E25-H04, ver
/// [`ApplyValidation::valid`]).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct ApplyValidation {
    /// `true` si el workspace publicado no tiene ningún check `Err` (`hardFail == 0`).
    ///
    /// **También `false` cuando la conformidad post-apply no se pudo ejecutar** (E25-H04): el análisis
    /// corre con el canónico ya publicado, así que un fallo suyo no puede convertir el apply en un error
    /// —el cambio está hecho—, y lo único honesto que queda es no declarar válido lo que no se ha podido
    /// comprobar. Ese caso se distingue por `errors == 0 && warnings == 0 && valid == false`, que es
    /// imposible en un veredicto realmente computado (`valid` es exactamente `errors == 0`), y va
    /// acompañado de un aviso por stderr. Para obtener el veredicto de verdad, `knowledge_check` lo
    /// recomputa.
    pub valid: bool,
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
    outcome: &Result<ApplyResult, AppError>,
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
        Err(err) => (None, None, Vec::new(), err.code.as_str().to_string()),
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
    outcome: &Result<RevertResult, AppError>,
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
        Err(err) => (None, None, Vec::new(), err.code.as_str().to_string()),
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

/// Ruta completa de `.lodestar/runtime/audit.jsonl` bajo `root` (E13-H10). Nadie garantiza ya ese
/// directorio al abrir (E23-H12 retiró el scaffold de runtime): `try_append_audit` lo crea con
/// `create_dir_all` justo antes de escribir (mismo patrón que `persist_plan`).
fn audit_file_path(root: &Path) -> PathBuf {
    root.join(".lodestar").join("runtime").join("audit.jsonl")
}

/// Anexa `entry` como una línea JSON (+ `\n`) a `.lodestar/runtime/audit.jsonl` del workspace
/// (E13-H10): crea `.lodestar/runtime/` si falta y abre en modo `append` — JSONL que solo crece,
/// nunca reescribe líneas previas. Devuelve el error de I/O sin envolver; `App::audit` es quien
/// decide que un fallo aquí es best-effort (no debe tumbar la operación auditada).
///
/// Es uno de los cuatro chokepoints de escritura de E23-H12: escribir la auditoría hace nacer
/// runtime desechable, así que aquí se ajusta el `.gitignore` gestionado. Por eso recibe el
/// [`Workspace`] y no un `&Path`.
fn try_append_audit(ws: &Workspace, entry: &AuditEntry) -> std::io::Result<()> {
    use std::io::Write;
    ws.ensure_managed_gitignore();
    let path = audit_file_path(ws.root());
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
fn op_target_path(op: &Value) -> Result<RelPath, AppError> {
    let candidate = op
        .get("ref")
        .and_then(|r| r.get("path"))
        .and_then(Value::as_str)
        .or_else(|| op.get("path").and_then(Value::as_str))
        .or_else(|| op.get("from").and_then(Value::as_str))
        .or_else(|| op.get("source").and_then(Value::as_str))
        .ok_or_else(|| {
            AppError::invalid_schema(
                "una operación con «expectedRevision» necesita identificar su documento objetivo \
                 en «ref.path», «path» o «from»",
            )
        })?;
    RelPath::new(candidate).map_err(|e| AppError::from(&e))
}

/// `ref.path` o `path` de una op cruda como [`RelPath`]. `Err(InvalidSchema)` si falta, o el error
/// mapeado de [`RelPath::new`] (path-traversal → `PermissionDenied`) si es inválido.
fn op_ref_path(op: &Value) -> Result<RelPath, AppError> {
    let s = op
        .get("ref")
        .and_then(|r| r.get("path"))
        .and_then(Value::as_str)
        .or_else(|| op.get("path").and_then(Value::as_str))
        .ok_or_else(|| {
            AppError::invalid_schema(format!(
                "la operación «{}» necesita el documento objetivo en «ref.path» o en «path» \
                 (una ruta relativa al workspace)",
                op_kind_de(op)
            ))
        })?;
    RelPath::new(s).map_err(|e| AppError::from(&e))
}

/// Un campo string obligatorio de una op cruda como [`RelPath`].
fn op_rel_field(op: &Value, key: &str) -> Result<RelPath, AppError> {
    let s = op.get(key).and_then(Value::as_str).ok_or_else(|| {
        AppError::invalid_schema(format!(
            "la operación «{}» exige el campo «{key}» con una ruta relativa al workspace",
            op_kind_de(op)
        ))
    })?;
    RelPath::new(s).map_err(|e| AppError::from(&e))
}

/// El discriminador `op` de una op cruda, para citarlo en los mensajes de error (E26-H07);
/// `«sin op»` si ni siquiera viene.
fn op_kind_de(op: &Value) -> &str {
    op.get("op").and_then(Value::as_str).unwrap_or("sin op")
}

/// Despacha UNA op cruda (`{op, …}`) a su normalizador del core, devolviendo las
/// [`NormalizedOperation`] resultantes (una op de estructura puede producir varias, E12-H06).
/// El discriminador `op` usa el mismo vocabulario snake_case que [`NormalizedOperation`]. Un `op`
/// desconocido o un parámetro inválido → `Err(ErrorCode::InvalidSchema)`; los errores del core se
/// mapean con [`error_code`].
///
/// `estado` es la ocupación de paths **acumulada** por las operaciones anteriores del mismo change
/// set (E28-H04): contra ella juzgan su colisión de existencia `create` y el destino de `move`, para
/// que una op vea lo que las de delante ocuparon o liberaron. El `doc_set` sigue siendo el del
/// workspace de partida y es de donde sale todo el **contenido** (cuerpos, entrantes, frontmatter).
fn normalize_raw_op(
    doc_set: &DocumentSet,
    estado: &plan::EstadoOcupacion<'_>,
    op: &Value,
) -> Result<Vec<NormalizedOperation>, AppError> {
    let kind = op.get("op").and_then(Value::as_str).ok_or_else(|| {
        AppError::invalid_schema(
            "cada operación necesita su discriminador «op»: create, patch_frontmatter, \
             replace_body, replace_text, edit_section, move o delete",
        )
    })?;
    let one = |n: NormalizedOperation| vec![n];
    match kind {
        "create" => {
            let path = op_rel_field(op, "path")?;
            // E23-H02: frontmatter ARBITRARIO y opcional. Ya no se leen `type`/`title` como campos
            // privilegiados — el motor no impone ninguna clave (`§20.2` invariante 3) y el título se
            // deriva (`§20.4`). Un `frontmatter` presente pero que no sea un objeto es una op mal
            // formada, igual que en `patch_frontmatter`.
            let frontmatter = match op.get("frontmatter") {
                None | Some(Value::Null) => None,
                Some(v) if v.is_object() => {
                    Some(serde_json::from_value(v.clone()).map_err(|e| {
                        AppError::invalid_schema(format!(
                        "el «frontmatter» de «create» no es un mapa de claves interpretable: {e}"
                    ))
                    })?)
                }
                Some(v) => {
                    return Err(AppError::invalid_schema(format!(
                        "el «frontmatter» de «create» debe ser un objeto de claves YAML \
                         arbitrarias; recibido {v}"
                    )))
                }
            };
            let body = op.get("body").and_then(Value::as_str).map(str::to_string);
            plan::normalize_create_en(estado, &path, frontmatter, body)
                .map(one)
                .map_err(|e| AppError::from(&e))
        }
        "patch_frontmatter" => {
            let path = op_ref_path(op)?;
            let patch = op_patch(op)?;
            plan::normalize_patch_frontmatter(doc_set, &path, patch)
                .map(one)
                .map_err(|e| AppError::from(&e))
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
                .map_err(|e| AppError::from(&e))
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
                .map_err(|e| AppError::from(&e))
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
                .map_err(|e| AppError::from(&e))
        }
        "move" => {
            let from = op_rel_field(op, "from")?;
            let to = op_rel_field(op, "to")?;
            let rewrite = op
                .get("rewriteInboundLinks")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            plan::normalize_move_en(estado, doc_set, &from, &to, rewrite)
                .map_err(|e| AppError::from(&e))
        }
        "delete" => {
            let path = op_ref_path(op)?;
            let policy = match op.get("inboundLinksPolicy").and_then(Value::as_str) {
                Some("reject") => InboundLinksPolicy::Reject,
                Some("remove_links") => InboundLinksPolicy::RemoveLinks,
                // Política ausente: `§Fase 12` exige elegirla EXPLÍCITAMENTE cuando hay algo que
                // decidir — es decir, cuando el documento tiene enlaces entrantes. No se defaultea
                // a `reject` en silencio: un `delete` con backlinks y sin política es una op mal
                // formada (`INVALID_SCHEMA`: falta un campo requerido), DISTINTO del
                // `INBOUND_LINKS_EXIST` que produce un `reject` explícito con backlinks. Sin
                // backlinks no hay nada que decidir, así que se permite (borrado limpio).
                None => {
                    let entrantes = doc_set.backlinks(&path).inbound.len();
                    if entrantes == 0 {
                        InboundLinksPolicy::Reject
                    } else {
                        return Err(AppError::invalid_schema(format!(
                            "«{}» tiene {entrantes} enlaces entrantes, así que «delete» exige \
                             elegir explícitamente «inboundLinksPolicy»: {:?}",
                            path.as_str(),
                            InboundLinksPolicy::WIRE_VALUES
                        )));
                    }
                }
                // Un valor de política no reconocido es una op mal formada. Desde E23-H05 esto
                // incluye `retarget` y `create_stub`, que se aceptaban sin ejecutarse: ahora se
                // rechazan aquí, y la fachada MCP añade al mensaje cuáles son las válidas
                // (`InboundLinksPolicy::WIRE_VALUES`).
                Some(otra) => {
                    return Err(AppError::invalid_schema(format!(
                        "«{otra}» no es una política válida ante enlaces entrantes; usa una de \
                         {:?}",
                        InboundLinksPolicy::WIRE_VALUES
                    )))
                }
            };
            plan::normalize_delete(doc_set, &path, policy).map_err(|e| AppError::from(&e))
        }
        // NOTA E23-H11: `"apply_fix"` ya NO tiene brazo propio — cae en el de por defecto, o sea
        // que un `apply_fix` es hoy una op desconocida (`INVALID_SCHEMA`), el mismo trato que
        // `transition_status` desde E21-H01. Antes llegaba a `normalize_apply_fix`, que devolvía
        // siempre `FixNotFound` → `DOCUMENT_NOT_FOUND` (código que apuntaba al sitio equivocado).
        _ => Err(AppError::invalid_schema(format!(
            "«{kind}» no es una operación conocida; usa create, patch_frontmatter, replace_body, \
             replace_text, edit_section, move o delete"
        ))),
    }
}

/// Expande la forma-objeto de **selección masiva** de `change_plan` (E21-H02, `§Fase 12`):
///
/// ```json
/// { "selection": { "where": "<consulta E19>" | "filter": { … } },
///   "operation":  { "<op universal>": { <parámetros> } } }
/// ```
///
/// La consulta E19 (`selection.where` textual o `selection.filter` JSON, traducidas al MISMO
/// [`Expression`] que `knowledge_search`) se evalúa contra **cada** documento del workspace; los que
/// casan reciben **una** [`NormalizedOperation`] cada uno, expandiendo la `operation` (que codifica el
/// tipo como CLAVE, `{patch_frontmatter: {…}}`) con ese `path`. Solo se admiten las ops con sentido en
/// masa (`patch_frontmatter`/`replace_text`/`delete` — `apply_fix` salió con la op en E23-H11);
/// `create` no aplica a documentos existentes. Cada documento seleccionado captura además su
/// [`DocumentRevision`] actual (el mismo blake3 que reporta `knowledge_get`) — el *snapshot de
/// revisiones* de `§Fase 12`.
///
/// Una selección que no casa ningún documento devuelve un plan **vacío**, sin error. Un `where`/
/// `filter` malformado, una `operation` que no es un objeto de una sola clave, o una op no admitida →
/// `Err` con [`ErrorCode::InvalidSchema`] y el mensaje que dice cuál de los tres casos fue (E26-H07;
/// para el `where`/`filter`, el diagnóstico del parser del core tal cual).
///
/// **Un [`TypeError`] al evaluar ABORTA el plan** (E26-H08), con el mismo `INVALID_SCHEMA` y el
/// mismo texto que daría `knowledge_search` para esa consulta ([`evalua_documento`], `§20.10`).
/// Hasta v0.4.0 el documento que erraba se **excluía en silencio** de la selección, así que una
/// selección masiva saltaba documentos sin decirlo y el plan afectaba a menos ficheros de los que el
/// agente creía haber seleccionado — la superficie donde el defecto era más caro, porque el
/// resultado se escribe.
///
/// **Determinismo**: la consulta se evalúa sobre el orden total de [`Analysis::documents`] **antes**
/// de expandir ninguna operación, y se reporta el primer `Err` de ese orden; ni el orden en que el
/// planificador toque los documentos ni un fallo de normalización de una op posterior pueden
/// cambiarlo. `Ok(false)` sigue siendo exclusión: no casar no es un error.
fn expand_selection(
    doc_set: &DocumentSet,
    raw: &Value,
) -> Result<
    (
        Vec<NormalizedOperation>,
        BTreeMap<RelPath, DocumentRevision>,
    ),
    AppError,
> {
    let selection = raw.get("selection").ok_or_else(|| {
        AppError::invalid_schema(
            "una selección masiva necesita «selection» con «where» (consulta textual) o «filter» \
             (filtro JSON)",
        )
    })?;
    let operation = raw.get("operation").ok_or_else(|| {
        AppError::invalid_schema(
            "una selección masiva necesita «operation»: el objeto de UNA clave con la operación a \
             expandir (p. ej. {\"patch_frontmatter\": {…}})",
        )
    })?;

    let where_expr = selection.get("where").and_then(Value::as_str);
    let filter = selection.get("filter");
    let expr = build_selection_expression(where_expr, filter)?;

    let (op_kind, op_params) = single_operation(operation)?;

    let analysis = doc_set.analyze();
    let files = doc_set.files();

    // (1) La CONSULTA decide el conjunto, recorriendo el orden total de `Analysis::documents`
    //     entero y antes de expandir nada. Un `TypeError` aborta el plan con el error del PRIMER
    //     documento de ese orden que yerra (E26-H08): el criterio no puede ser «el primero que
    //     tocó el planificador», y expandir por el camino dejaría que el fallo de una op sobre un
    //     documento que casa se adelantara al error de tipo de otro anterior.
    let mut seleccionados: Vec<&RelPath> = Vec::new();
    for path in &analysis.documents {
        let Some(raw_md) = files.get(path) else {
            continue;
        };
        let parsed = model::parse_file(path.as_str(), raw_md);
        let doc = EvalDocument {
            path,
            frontmatter: parsed.frontmatter.as_ref(),
            body: &parsed.body,
        };
        if evalua_documento(&expr, &doc, analysis)? {
            seleccionados.push(path);
        }
    }

    // (2) …y solo entonces se expande la operación sobre los documentos elegidos.
    //
    // E28-H04: la selección masiva queda FUERA del estado de ocupación acumulado, a propósito. Cada
    // documento seleccionado genera como mucho una operación, y `single_operation` ya excluye de
    // esta vía las dos únicas que ocupan un path (`create` y `move`), así que no hay secuencia
    // intra-selección que acumular: la ocupación de partida es la del workspace y no se mueve.
    let ocupacion = plan::EstadoOcupacion::nueva(files);
    let mut normalized: Vec<NormalizedOperation> = Vec::new();
    let mut captured: BTreeMap<RelPath, DocumentRevision> = BTreeMap::new();
    for path in seleccionados {
        let Some(raw_md) = files.get(path) else {
            continue;
        };
        let raw_op = build_selected_op(op_kind, op_params, path)?;
        normalized.extend(normalize_raw_op(doc_set, &ocupacion, &raw_op)?);
        captured.insert(
            path.clone(),
            DocumentRevision::from_hash(*blake3::hash(raw_md.as_bytes()).as_bytes()),
        );
    }

    Ok((normalized, captured))
}

/// Traduce la consulta de una [`expand_selection`] (`where` textual o `filter` JSON) al [`Expression`]
/// del core (E19) **por la misma función** que `knowledge_search` ([`build_search_expression`]), de
/// modo que la misma consulta malformada dé el mismo código Y el mismo mensaje por las dos tools que
/// la aceptan (E26-H07, invariante #3: una sola verdad computada). Lo único propio de la selección es
/// que exige **al menos** un criterio: una selección sin `where` ni `filter` seleccionaría el
/// workspace entero por descuido.
///
/// Hasta v0.4.0 esta función parseaba por su cuenta y tiraba el diagnóstico del parser con
/// `map_err(|_| ErrorCode::InvalidSchema)`, así que el mismo `where: "status ="` se diagnosticaba
/// por `knowledge_search` y se callaba por `change_plan`.
fn build_selection_expression(
    where_expr: Option<&str>,
    filter: Option<&Value>,
) -> Result<Expression, AppError> {
    build_search_expression(where_expr, filter)?.ok_or_else(|| {
        AppError::invalid_schema(
            "«selection» necesita «where» (consulta textual) o «filter» (filtro JSON): sin \
             criterio, la selección masiva alcanzaría a todos los documentos",
        )
    })
}

/// Extrae de la `operation` de una selección masiva su `(tipo, parámetros)`: la op codifica el tipo
/// como CLAVE (`{patch_frontmatter: {…}}`), así que debe ser un objeto de **exactamente una** clave, y
/// esa clave una op con sentido en masa. `create` no aplica a una selección de documentos existentes;
/// `move` tampoco (un solo `to` no puede servir para N documentos). `Err(ErrorCode::InvalidSchema)`
/// en cualquier otro caso.
fn single_operation(operation: &Value) -> Result<(&str, &Value), AppError> {
    const EN_MASA: &str = "patch_frontmatter, replace_text o delete";
    let obj = operation.as_object().ok_or_else(|| {
        AppError::invalid_schema(format!(
            "«operation» debe ser un objeto que codifique la operación como CLAVE \
             ({{\"patch_frontmatter\": {{…}}}}); en masa solo tienen sentido {EN_MASA}"
        ))
    })?;
    let mut claves = obj.keys();
    let (Some(kind), None) = (claves.next(), claves.next()) else {
        return Err(AppError::invalid_schema(format!(
            "«operation» debe tener EXACTAMENTE una clave (la operación a expandir); recibidas {:?}",
            obj.keys().collect::<Vec<_>>()
        )));
    };
    let params = &obj[kind];
    match kind.as_str() {
        // E23-H11: `apply_fix` salió de la lista blanca con la op. Una selección masiva que la pida
        // es `INVALID_SCHEMA` (op no admitida en masa), igual que si pidiera `create`.
        "patch_frontmatter" | "replace_text" | "delete" => Ok((kind, params)),
        otra => Err(AppError::invalid_schema(format!(
            "«{otra}» no es una operación admitida en una selección masiva; usa {EN_MASA} \
             («create» no aplica a documentos existentes y «move» necesita un destino por documento)"
        ))),
    }
}

/// Construye la op cruda `{op, ref:{path}, …}` para un documento seleccionado, de forma que
/// [`normalize_raw_op`] la despache igual que si la hubiera enviado el agente sueltamente. El valor de
/// `patch_frontmatter` ES el merge-patch (va bajo la clave `patch`); las demás ops llevan sus
/// parámetros sueltos (`find`/`replace`, `inboundLinksPolicy`…).
fn build_selected_op(kind: &str, params: &Value, path: &RelPath) -> Result<Value, AppError> {
    let mut obj = serde_json::Map::new();
    obj.insert("op".to_string(), Value::String(kind.to_string()));
    obj.insert(
        "ref".to_string(),
        serde_json::json!({ "path": path.as_str() }),
    );
    if kind == "patch_frontmatter" {
        if !params.is_object() {
            return Err(AppError::invalid_schema(format!(
                "el valor de «patch_frontmatter» ES el merge-patch de frontmatter, así que debe ser \
                 un objeto; recibido {params}"
            )));
        }
        obj.insert("patch".to_string(), params.clone());
    } else {
        let extra = params.as_object().ok_or_else(|| {
            AppError::invalid_schema(format!(
                "los parámetros de «{kind}» deben venir en un objeto; recibido {params}"
            ))
        })?;
        for (k, v) in extra {
            obj.insert(k.clone(), v.clone());
        }
    }
    Ok(Value::Object(obj))
}

/// Convierte el campo `patch` de una op cruda en un [`FrontmatterPatch`] (merge-patch RFC 7386:
/// `null` borra la clave, cualquier otro valor la escribe). `Err(InvalidSchema)` si `patch` falta o
/// no es un objeto.
fn op_patch(op: &Value) -> Result<FrontmatterPatch, AppError> {
    let patch = op.get("patch").ok_or_else(|| {
        AppError::invalid_schema(
            "«patch_frontmatter» exige el campo «patch» con el merge-patch (RFC 7386: un valor \
             escribe la clave, «null» la borra)",
        )
    })?;
    if !patch.is_object() {
        return Err(AppError::invalid_schema(format!(
            "«patch» debe ser un objeto de claves de frontmatter; recibido {patch}"
        )));
    }
    serde_json::from_value(patch.clone()).map_err(|e| {
        AppError::invalid_schema(format!("«patch» no es un merge-patch interpretable: {e}"))
    })
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
/// `root` del workspace. Nadie garantiza ya ese directorio al abrir (E23-H12 retiró el scaffold de
/// runtime): [`persist_plan`] lo crea con `create_dir_all` justo antes de escribir.
fn plan_file_path(root: &Path, id: &ChangeSetId) -> PathBuf {
    root.join(".lodestar")
        .join("runtime")
        .join("plans")
        .join(plan_file_name(id))
}

/// Mensaje del rechazo de `change_apply` cuando el plan **no es aplicable bajo su propia
/// `PlanPolicy`** (E29-H07, `decisiones §18`).
///
/// Nombra la(s) **cláusula(s)** de la policy que bloquearon —`requireValidResult` y/o
/// `allowWarnings`— y el remedio (replanificar, o relajar la policy). Esa precisión es lo que
/// sostiene reusar `INVALID_RESULT` en vez de abrir una fila decimoctava en el catálogo: el gate de
/// **staging** rechaza con el mismo código, pero su mensaje habla de `rejectNewErrors`/
/// `allowExistingErrors` (la política `transactions`), de modo que los dos vocabularios son
/// disjuntos y el agente sabe cuál de los dos gates le habló. Este mensaje **no** menciona la
/// política de staging, ni el de staging la del plan.
fn plan_policy_rejection_message(plan: &PlanResult, report: &ValidationReport) -> String {
    let mut clausulas: Vec<String> = Vec::new();
    if plan.policy.require_valid_result && !report.valid {
        clausulas.push(format!(
            "«requireValidResult» es true y el resultado simulado no es válido ({} error(es))",
            report.summary.errors
        ));
    }
    if !plan.policy.allow_warnings && report.summary.warnings > 0 {
        clausulas.push(format!(
            "«allowWarnings» es false y el resultado simulado tiene {} warning(s)",
            report.summary.warnings
        ));
    }
    if clausulas.is_empty() {
        // Inalcanzable: este mensaje solo se construye cuando `plan::can_apply` dijo `false`, y ese
        // predicado no tiene más causas que las dos de arriba. Se cubre para que un futuro campo de
        // `PlanPolicy` sin rama aquí produzca un mensaje pobre pero honesto, nunca una lista vacía.
        clausulas.push("la policy del plan no admite este resultado".to_string());
    }
    format!(
        "el plan «{}» no es aplicable bajo la policy con la que se planificó ({}): {}. \
         No se ha escrito nada. Replanifica sobre el estado actual (change_plan) o vuelve a \
         planificar con una policy que lo admita",
        plan.change_set_id.0,
        // La policy entera, para que el agente vea el contexto de la cláusula citada.
        format_args!(
            "requireValidResult={}, allowWarnings={}",
            plan.policy.require_valid_result, plan.policy.allow_warnings
        ),
        clausulas.join("; "),
    )
}

/// Persiste el `PlanResult` completo (operaciones normalizadas, revisión base, hash, caducidad,
/// diff, impacto, validación) en `.lodestar/runtime/plans/<hash>.json` (E12-H09,
/// `ARCHITECTURE.md §19.4/§19.5`).
///
/// Runtime, no canónico: gitignored y excluido de `WorkspaceRevision` (E9-H06/E10-H03), por lo que
/// se escribe con `std::fs::write` normal — el protocolo temp+rename del único-escritor
/// (`lodestar_workspace::io::write_atomic`) protege el conocimiento `.md` canónico, no el scratch
/// de runtime, que ni el watcher ni el walker observan.
///
/// Es uno de los cuatro chokepoints de escritura de E23-H12: `change_plan` persiste **sin** tomar
/// el lock, así que es aquí (y no en `acquire_lock`) donde le toca ajustar el `.gitignore`
/// gestionado — abrir el workspace ya no lo hace. Por eso recibe el [`Workspace`] y no un `&Path`:
/// el ajuste no puede quedar a criterio del llamador.
fn persist_plan(ws: &Workspace, plan: &PlanResult) -> Result<(), AppError> {
    let io = |e: std::io::Error| {
        AppError::new(
            ErrorCode::InternalIoError,
            format!(
                "no se pudo persistir el plan «{}» en .lodestar/runtime/plans/: {e}",
                plan.change_set_id.0
            ),
        )
    };
    ws.ensure_managed_gitignore();
    let dir = ws.root().join(".lodestar").join("runtime").join("plans");
    std::fs::create_dir_all(&dir).map_err(io)?;
    let path = dir.join(plan_file_name(&plan.change_set_id));
    let json = serde_json::to_vec_pretty(plan).map_err(|e| {
        AppError::new(
            ErrorCode::InternalIoError,
            format!(
                "el plan «{}» no serializa a JSON: {e}",
                plan.change_set_id.0
            ),
        )
    })?;
    std::fs::write(&path, json).map_err(io)
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
    /// La [`PlanPolicy`] con la que se computó `canApply` — E29-H07 (`decisiones §18`).
    ///
    /// Se persiste con el plan porque el veredicto **vincula** al apply: `change_apply` recomputa
    /// [`plan::can_apply`] con esta policy (invariante #3: el predicado no se reimplementa, y no se
    /// congela un booleano que el apply no pueda re-verificar como re-verifica el `planHash` y la
    /// revisión). Un plan persistido por un binario ANTERIOR a E29-H07 no lleva el campo:
    /// `#[serde(default)]` lo completa con [`PlanPolicy::default`] —la policy más estricta de las
    /// dos que el wire admite—, de modo que el gate nunca es más laxo de lo que el cliente pidió.
    /// El desajuste posible (un plan planificado con `requireValidResult:false` que tras actualizar
    /// el binario se rechaza) se resuelve replanificando, y lo acota el TTL corto del plan.
    #[serde(default)]
    pub policy: PlanPolicy,
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
    /// Revisiones capturadas por documento seleccionado en una **selección masiva** (E21-H02):
    /// `path → DocumentRevision` (`"blake3:…"`), una entrada por documento que casó la consulta —
    /// el *snapshot de revisiones* de `§Fase 12` (query → documentos → snapshot → … → change plan).
    /// Vacío (`{}`) para la forma de array de operaciones sueltas, que no nace de una selección.
    #[serde(default)]
    pub captured_revisions: BTreeMap<RelPath, DocumentRevision>,
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
    pub valid: bool,
    /// Recuento por severidad sobre TODO el scope (independiente de `minimumSeverity`/paginación).
    pub summary: CheckSummary,
    /// La página de diagnósticos (cada uno con su `id` estable), tras filtrar por severidad y paginar.
    pub diagnostics: Vec<Check>,
    /// Revisión determinista del workspace en el momento de la auditoría (`WorkspaceRevision`).
    pub workspace_revision: WorkspaceRevision,
    /// Cursor opaco a la siguiente página, o `None` si no quedan más diagnósticos.
    pub next_cursor: Option<String>,
}

/// Clave **sintética** con la que los diagnósticos de descubrimiento **sin `target`** entran en
/// [`Analysis::diagnostics`], que es un `BTreeMap<RelPath, Vec<Check>>` y por tanto exige una clave
/// (E29-H06).
///
/// Es `.lodestar`, el **plano de control** del workspace, y **jamás** puede ser un documento del
/// inventario ni pisar los diagnósticos de un fichero real. La garantía la sostienen **dos** piezas
/// distintas, porque el suelo duro por sí solo no cubre el literal:
///
/// 1. **Lo que hay bajo `.lodestar/`**: el suelo duro del descubrimiento
///    (`lodestar_workspace::discovery::CONTROL_PLANE_EXCLUDE`) excluye `.lodestar/**` sin que
///    ninguna config pueda re-incluirlo, así que ningún `.lodestar/loquesea.md` entra al inventario.
/// 2. **La entrada literal `.lodestar`** (un **fichero** llamado así, sin barra): ese glob **no** la
///    cubre —`.lodestar/**` casa con lo de dentro de un directorio, no con una entrada suelta—, y la
///    cierra el arranque: `.lodestar` como fichero hace que leer `<root>/.lodestar/config.yaml` dé
///    `ENOTDIR`, y desde **E29-H01** un `config.yaml` que existe pero no se puede leer es error de
///    apertura (`App::open` falla, `exit 3`) en vez de degradar a los valores por defecto. Es decir:
///    en un workspace que **abre**, esa entrada no existe, y donde existiera no hay `Analysis` que
///    indexar.
///
/// No es un `RelPath` de la raíz —no existe: `RelPath::new("")` es
/// `Err` por diseño, invariante #6— sino una etiqueta estable que dice «esto es del workspace, no
/// de un fichero tuyo»: los diagnósticos que la usan (`PATH-NOT-UTF8`, `WORKSPACE-EMPTY`) siguen
/// viajando con `targets` **vacío**, que es la verdad, y su severidad cuenta igual para
/// `Analysis::hard_fail`/`warn_count` y para el gate de `lodestar check`.
pub const ANCHOR_WORKSPACE: &str = ".lodestar";

/// El [`RelPath`] de [`ANCHOR_WORKSPACE`].
///
/// # Pánico
/// Nunca: `.lodestar` es un literal válido para [`RelPath::new`] (relativo, un solo segmento, sin
/// `..` ni backslashes), y el test `ancla_de_workspace_es_relpath_valido` lo clava.
fn anchor_workspace() -> RelPath {
    RelPath::new(ANCHOR_WORKSPACE).expect("«.lodestar» es un RelPath válido por construcción")
}

/// Fusiona los diagnósticos de **descubrimiento** dentro de un [`Analysis`] ya calculado, aplicando
/// la política de severidad y anclando cada uno bajo la clave que le corresponde (E29-H06).
///
/// Es la mitad del cuerpo de [`App::full_analysis`] que **no** depende del disco: recibe el
/// `Analysis` de los documentos, la lista de diagnósticos de descubrimiento y la sección
/// `validation`, y no lee nada más. Se extrajo a función propia en **E30-H03** (seguimiento 9)
/// precisamente para que sea ejercitable con un [`Check`] **sintético**: los diagnósticos que la
/// motivan —`PATH-NOT-UTF8` y `WORKSPACE-EMPTY`— nacen de condiciones del sistema de ficheros que
/// no se pueden fabricar en un test portable (en APFS no hay forma de crear un nombre de fichero
/// que no sea UTF-8 válido), así que sin este seam el camino de anclaje solo se podía observar de
/// refilón.
///
/// Las tres reglas que fija, y que un test puede clavar una a una:
/// 1. Una familia reclasificada a `ignore` por la config **no entra** (se descarta, como los
///    diagnósticos de documento).
/// 2. Un diagnóstico **con** `targets` se ancla bajo su **primer** target, mezclándose con los
///    diagnósticos que ese documento ya tuviera.
/// 3. Un diagnóstico **sin** `targets` se ancla bajo [`anchor_workspace`], nunca se descarta: es
///    exactamente el caso que hasta E29-H06 desaparecía en silencio por no tener clave con la que
///    entrar al mapa.
///
/// La severidad efectiva se escribe en el propio `check.level` antes de insertarlo, así que el
/// diagnóstico anclado cuenta ya reclasificado para `hard_fail`/`warn_count`.
fn fusiona_diagnosticos_de_descubrimiento(
    analysis: &mut Analysis,
    discovery_diagnostics: Vec<Check>,
    validation: &lodestar_workspace::config::ValidationSection,
) {
    for mut check in discovery_diagnostics {
        let Some(level) = validation.effective_severity(&check) else {
            continue;
        };
        check.level = level;
        let anchor = match check.targets.first().cloned() {
            Some(target) => target,
            None => anchor_workspace(),
        };
        analysis.diagnostics.entry(anchor).or_default().push(check);
    }
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

/// Límite por defecto de nodos por página de `graph_query` (E26-H10). Hasta v0.4.0 no había
/// default: `limit` ausente servía el grafo **entero**.
const DEFAULT_GRAPH_LIMIT: usize = 100;
/// Tope duro de nodos por página de `graph_query` (E26-H10; el `inputSchema` lo declara como
/// `maximum` y la fachada MCP rechaza lo que lo exceda).
const MAX_GRAPH_LIMIT: usize = 1000;

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
    /// Título **derivado** (`frontmatter.title` → primer H1 → nombre del fichero, `§20.2`).
    /// Siempre presente (E24-H11).
    ///
    /// Es heurística de presentación, no una propiedad reservada del frontmatter. Viaja aquí por la
    /// misma razón que en `SearchResult` y en `GraphNode`, y lo computa **la misma** función del
    /// core ([`model::derived_title`]): hasta v0.3.0 la tool que lee UN documento era la única de
    /// las tres que no lo traía, así que un agente que seguía el flujo recomendado
    /// (`knowledge_search` → `knowledge_get`) perdía el título al leer, y el `include` cerrado
    /// tampoco le dejaba pedirlo.
    pub title: String,
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
    /// Los campos de frontmatter **pedidos** en `include` (E23-H11), tecleados por el field path
    /// tal y como se pidió (`"status"`, `"owner.name"`) y con su **valor YAML crudo**.
    ///
    /// `None` —campo ausente del wire— si el llamador no pidió ninguno: sin `include`, el hit
    /// conserva byte a byte su forma anterior. Un campo pedido que este documento **no tiene** no
    /// aparece como clave (nunca un `null` disfrazado), así que el mapa puede ser vacío; un campo
    /// **presente con `null` explícito** sí aparece, con valor `null`. Son dos estados distintos a
    /// propósito, y [`ParsedFrontmatter::get`](lodestar_core::types::ParsedFrontmatter::get) ya los
    /// distingue.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmatter: Option<BTreeMap<String, Value>>,
}

/// Una proyección de frontmatter pedida en el `include` de `knowledge_search` (E23-H11):
/// la entrada de wire `"frontmatter.<fieldPath>"` ya validada y parseada.
///
/// Existe como tipo —y no como un `&str` que cada capa reinterprete— para que el parseo ocurra
/// **una vez**, en la frontera, y para que [`App::knowledge_search`] reciba algo que no puede estar
/// mal formado. La clave que viajará en la respuesta es el sufijo **tal cual se pidió**, de modo que
/// quien escribió `frontmatter.owner.name` lee la respuesta con esa misma cadena.
#[derive(Debug, Clone)]
pub struct FrontmatterProjection {
    /// El sufijo pedido, sin el prefijo `frontmatter.` — la clave del mapa de la respuesta.
    key: String,
    /// El mismo sufijo ya parseado a [`FieldPath`], que es quien resuelve el dot-path.
    field_path: FieldPath,
}

/// Prefijo obligatorio de cada entrada del `include` de `knowledge_search`.
const SEARCH_INCLUDE_PREFIX: &str = "frontmatter.";

impl FrontmatterProjection {
    /// Parsea **una** entrada del `include`.
    ///
    /// # Por qué NO pasa por `parse::build_field_path` (E26-H09)
    ///
    /// Esta no es una tercera normalización del dot-path del lenguaje de consulta, sino la
    /// **semántica del anclaje con el prefijo obligatorio**: `frontmatter.` no es aquí un anclaje
    /// opcional que compita con namespaces reservados, sino el namespace **exigido** de cada
    /// entrada, y el sufijo direcciona siempre el frontmatter del usuario. No hay abreviatura que
    /// aplicar (el prefijo nunca puede faltar) ni namespace calculado que desambiguar
    /// (`frontmatter.graph.backlinks` proyecta la clave `graph.backlinks` del usuario, que es lo
    /// que `build_field_path` también produce, por el anclaje).
    ///
    /// Delegar tendría además un **coste observable** en el único caso donde las dos vías difieren:
    /// `frontmatter.frontmatter.x` proyecta hoy —correctamente— la clave del usuario
    /// `frontmatter.x`, mientras que `build_field_path` consumiría el primer `frontmatter.` como
    /// abreviatura y la resolución acabaría en `x`: una respuesta silenciosamente equivocada, justo
    /// la clase de defecto que E24-H08/E26-H09 retiran. Esta superficie es, de hecho, la **única**
    /// que sabe leer una clave que vive bajo un `frontmatter` literal (ver
    /// `App::metadata_inspect`, que por eso remite aquí cuando detecta esa colisión).
    ///
    /// # Errores
    /// [`ErrorCode::InvalidSchema`] si la entrada no empieza por `frontmatter.` o si su sufijo no es
    /// un [`FieldPath`] válido (vacío, o con algún segmento vacío como `a..b`).
    pub fn parse(entrada: &str) -> Result<Self, ErrorCode> {
        let key = entrada
            .strip_prefix(SEARCH_INCLUDE_PREFIX)
            .ok_or(ErrorCode::InvalidSchema)?;
        let field_path = FieldPath::parse(key).map_err(|_| ErrorCode::InvalidSchema)?;
        Ok(FrontmatterProjection {
            key: key.to_string(),
            field_path,
        })
    }

    /// Parsea la lista entera: **todo o nada**. Una entrada válida no redime a la inválida que la
    /// acompaña — aceptar y descartar en silencio es justamente el defecto que E23-H11 retira del
    /// parámetro `sort`.
    ///
    /// # Errores
    /// [`ErrorCode::InvalidSchema`] en cuanto una entrada no sea válida (ver [`Self::parse`]).
    pub fn parse_all(entradas: &[String]) -> Result<Vec<Self>, ErrorCode> {
        entradas.iter().map(|e| Self::parse(e)).collect()
    }

    /// La clave con la que este campo viaja en la respuesta (el sufijo pedido).
    pub fn key(&self) -> &str {
        &self.key
    }

    /// El [`FieldPath`] con el que se resuelve el valor sobre el frontmatter del documento.
    pub fn field_path(&self) -> &FieldPath {
        &self.field_path
    }
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
/// Es la **única** compilación de consulta del motor: la selección masiva de `change_plan` la
/// consume vía [`build_selection_expression`] (E26-H07), así que el mismo `where`/`filter`
/// malformado produce el mismo código y el mismo texto por las dos tools que lo aceptan
/// (invariante #3).
///
/// Un `where`/`filter` **malformado** se surface con el código estable **`INVALID_SCHEMA`**
/// (E24-H10). Hasta v0.3.0 se envolvía en [`WorkspaceError::Core`], que `workspace_error_code`
/// mapea a `INTERNAL_IO_ERROR`: un typo del agente en su consulta se le reportaba como error
/// interno de I/O. Y la MISMA consulta malformada daba **dos códigos distintos según la tool**,
/// porque `build_selection_expression` (la selección masiva de `change_plan`) ya devolvía
/// `INVALID_SCHEMA`.
///
/// El mensaje se queda con el texto del `ParseError`/`FilterError` del core y NO propaga el
/// `Display` de serde: `"data did not match any variant of untagged enum WireNode"` es un interno
/// de implementación que no ayuda a nadie a arreglar su consulta.
/// Mensaje de un `FilterError` apto para el wire: se queda con la parte útil y **descarta** el
/// `Display` de serde cuando aparece (E24-H10).
///
/// `filter::from_json` usa `#[serde(untagged)]`, y un JSON que no casa ninguna variante produce
/// literalmente `"data did not match any variant of untagged enum WireNode"` — un interno de
/// implementación que no le dice a nadie qué arreglar en su filtro.
fn mensaje_de_filtro(e: &lodestar_core::filter::FilterError) -> String {
    if e.message.contains("did not match any variant") {
        return "no es un nodo de filtro válido: se esperaba {field, operator, value} o una \
                envoltura and/or/not/has/missing"
            .to_string();
    }
    e.message.clone()
}

/// El mensaje de por qué un namespace reservado **válido** (`graph.backlinks`, `document.path`) no
/// es inspeccionable por `metadata_inspect` (E26-H09), y por dónde sí se pregunta lo que el agente
/// quería saber.
///
/// Dos salidas, siempre las dos que existen: la **tool** que responde por esa propiedad calculada
/// (`graph_query` para el grafo; el `where` de `knowledge_search` para las de documento, que no
/// tienen tool propia) y el **anclaje** `frontmatter.` para la clave homónima del usuario, que es lo
/// que el agente pedía si su frontmatter tiene una clave `graph:`/`document:`. Sin la segunda mitad
/// el rechazo sería un callejón sin salida: la clave del catálogo existe, y hay una forma de
/// alcanzarla.
fn mensaje_namespace_no_inspeccionable(field: &FieldPath, props: &[&str]) -> String {
    let ns = field
        .segments()
        .first()
        .map_or_else(String::new, String::clone);
    let validas = props
        .iter()
        .map(|p| format!("`{ns}.{p}`"))
        .collect::<Vec<_>>()
        .join(", ");
    let por_donde = if ns == "graph" {
        "Para preguntar por el grafo real usa `graph_query` (o un `where` sobre esas propiedades \
         en knowledge_search)"
    } else {
        "Para filtrar por esas propiedades usa un `where` en knowledge_search"
    };
    format!(
        "«{field}» no es un campo de metadata: el namespace `{ns}` son propiedades CALCULADAS \
         ({validas}) y no viven en el frontmatter de ningún documento, así que no tienen presencia \
         ni vocabulario que inspeccionar. {por_donde}; para inspeccionar la clave de TU \
         frontmatter con ese nombre, ánclala: «{}»",
        field.anclado()
    )
}

/// `true` si el texto de un `field` empieza por el **anclaje** (`frontmatter.`), es decir, si el
/// normalizador lo va a reinterpretar en vez de tomarlo al pie de la letra.
fn empieza_por_el_anclaje(field: &str) -> bool {
    field
        .strip_prefix(FRONTMATTER_ANCHOR)
        .is_some_and(|resto| resto.starts_with('.'))
}

/// El mensaje de la **colisión con el anclaje** (E26-H09): el catálogo anuncia este nombre, pero
/// viene de una clave de primer nivel llamada literalmente `frontmatter`, y el lenguaje lee ese
/// mismo texto como el anclaje al frontmatter del usuario. No hay sintaxis para distinguirlas —el
/// mismo límite que una clave con **punto literal**—, así que la tool lo dice en voz alta en vez de
/// contestar `presentIn: 0` sobre un dato que existe.
///
/// El escape que se ofrece es real: el `include` de `knowledge_search` exige el prefijo
/// `frontmatter.` y parsea el **sufijo literalmente**, así que `frontmatter.{texto}` sí lee el
/// valor de esa clave (lo que no existe es forma de *inspeccionarla* como campo).
fn mensaje_colision_con_el_anclaje(texto: &str) -> String {
    format!(
        "«{texto}» no es inspeccionable: el prefijo «frontmatter.» es el ANCLAJE del lenguaje de \
         consulta (E24-H08), así que este texto se lee como una clave del frontmatter que no \
         aparece en ningún documento; el nombre que anuncia el catálogo viene de una clave de \
         primer nivel llamada literalmente «frontmatter», y el lenguaje no tiene comillas para \
         distinguir las dos cosas. Su VALOR sí se puede leer, con \
         knowledge_search{{include: [\"{FRONTMATTER_ANCHOR}.{texto}\"]}}, cuyo prefijo es \
         obligatorio y cuyo sufijo es literal; para inspeccionarla como campo habría que renombrar \
         esa clave"
    )
}

fn build_search_expression(
    where_expr: Option<&str>,
    filter: Option<&Value>,
) -> Result<Option<Expression>, AppError> {
    // Un `where` en blanco (solo espacios) se trata como ausente: no es una consulta malformada.
    let del_where =
        match where_expr.map(str::trim).filter(|s| !s.is_empty()) {
            Some(w) => Some(lodestar_core::parse::parse(w).map_err(|e| {
                AppError::invalid_schema(format!("«where» inválido: {}", e.message))
            })?),
            None => None,
        };
    let del_filter = match filter {
        Some(f) => Some(lodestar_core::filter::from_json(f).map_err(|e| {
            AppError::invalid_schema(format!("«filter» inválido: {}", mensaje_de_filtro(&e)))
        })?),
        None => None,
    };
    Ok(match (del_where, del_filter) {
        (None, None) => None,
        (Some(e), None) | (None, Some(e)) => Some(e),
        (Some(w), Some(f)) => Some(Expression::And(vec![w, f])),
    })
}

/// Evalúa `expr` contra **un** documento y traduce el [`TypeError`] del evaluador al [`AppError`] de
/// la superficie (E26-H08): un error de tipo **aborta la consulta** en vez de excluir el documento.
///
/// Es el **único** puente entre [`evaluate`] y las dos superficies que aceptan el lenguaje
/// (`knowledge_search` y la selección masiva de `change_plan`), de modo que la misma consulta mal
/// tipada dé el mismo código **y** el mismo texto por las dos (`§20.10`, invariante #3). El core no
/// cambia: sigue devolviendo `Result<bool, TypeError>` por documento —el dato que permite a la
/// fachada decidir—; lo que cambia es qué hace la fachada con el `Err`.
///
/// `Ok(false)` **sigue siendo exclusión**: no casar no es un error, y un campo ausente no llega
/// nunca aquí como `Err` (la ausencia cortocircuita antes de comprobar tipos, E19-H01).
fn evalua_documento(
    expr: &Expression,
    doc: &EvalDocument<'_>,
    analysis: &Analysis,
) -> Result<bool, AppError> {
    evaluate(expr, doc, analysis).map_err(|e| error_de_tipo(&e, doc.path))
}

/// Traduce un [`TypeError`] del evaluador —el que produce una consulta bien formada sobre datos de
/// otro tipo— al [`AppError`] de superficie: `INVALID_SCHEMA` con un mensaje que nombra **el campo,
/// el operador, el/los tipo(s) implicados y el documento** donde chocaron, más cómo salir del error.
///
/// Son **tres** las variantes desde E29-H04 —orden cruzado, operador de lista sobre un no-lista y
/// operador de texto (`starts_with`/`ends_with`, y desde E30-H03 también `contains` con literal
/// no-string sobre un campo string) sobre un no-string—, y el `match` de abajo es
/// **exhaustivo a propósito**: es el mecanismo que garantiza que ningún `TypeError` nuevo del core
/// llegue al wire sin mensaje propio.
///
/// Hasta v0.4.0 este `Err` se descartaba en los dos consumidores con el mismo `continue` que un
/// `Ok(false)`, así que `priority >= "high"` sobre un `priority` numérico devolvía `[]` —o una lista
/// recortada, decidida documento a documento— sin un solo aviso: la respuesta silenciosamente
/// equivocada que E24-H07 declaró peor que un error, aquí en la evaluación (E26-H08).
///
/// El **documento** viaja en el mensaje porque el mismo `where` puede ser perfectamente respondible
/// sobre casi todo el corpus: sin él, el agente no sabe dónde mirar. **Quién** es ese documento lo
/// fija el llamador, recorriendo el orden total de [`Analysis::documents`] y quedándose con el
/// primer `Err` (determinismo, E26-H08): los dos consumidores iteran ese mismo orden de principio a
/// fin, así que el error no depende de dónde paró el motor.
fn error_de_tipo(err: &TypeError, path: &RelPath) -> AppError {
    let detalle = match err {
        TypeError::OrderNotDefined {
            field,
            operator,
            field_type,
            value_type,
        } => format!(
            "en «{}» el campo «{field}» es de tipo {} y la consulta lo compara con un literal de \
             tipo {} mediante el operador de orden «{}». El orden solo está definido entre dos \
             number o entre dos string (lexicográfico), y el lenguaje no coerce tipos (§20.8)",
            path.as_str(),
            nombre_de_wire(field_type),
            nombre_de_wire(value_type),
            nombre_de_wire(operator),
        ),
        TypeError::NotAList {
            field,
            operator,
            found,
        } => format!(
            "en «{}» el campo «{field}» es de tipo {} y el operador «{}» exige una list (o un \
             string, en el caso de contains, donde significa subcadena)",
            path.as_str(),
            nombre_de_wire(found),
            nombre_de_wire(operator),
        ),
        TypeError::NotAString {
            field,
            operator,
            found,
        } => format!(
            "en «{}» la comparación entre el campo «{field}» y su literal tiene un operando de tipo \
             {}, y el operador de texto «{}» exige un string a los DOS lados: lo que no es texto no \
             tiene prefijo, sufijo ni subcadena que comprobar, y el lenguaje no coerce tipos \
             (§20.8). Comprueba \
             los dos lados — el tipo que falla puede ser el del campo o el del literal",
            path.as_str(),
            nombre_de_wire(found),
            nombre_de_wire(operator),
        ),
        // El match es EXHAUSTIVO a propósito (E26-H08): una variante nueva del enum del core rompe
        // la compilación aquí, que es justo lo que se quiere — un `TypeError` sin mensaje propio
        // sería el defecto de vuelta. Son tres desde E29-H04.
    };
    // La salida sugerida tiene que RESOLVER el error que se reporta. `has(campo)` no vale: la
    // ausencia nunca produce un `TypeError` (cortocircuita antes de mirar el tipo), así que el
    // documento que yerra TIENE el campo y `has()` no lo excluye — sugerirlo mandaría al agente a
    // reescribir la consulta para volver a chocar con lo mismo.
    AppError::invalid_schema(format!(
        "la consulta no es respondible sobre estos datos: {detalle}. Ajusta la consulta al tipo \
         real del campo: compara con un literal de ese tipo, o usa un operador definido para él \
         («=»/«!=» nunca son error — el cruce de tipos es false); \
         metadata_inspect{{\"mode\":\"field\"}} enumera los tipos que ese campo toma en el workspace"
    ))
}

/// Rinde un tipo o un operador del lenguaje con su **grafía de wire** —la de `filter.operator`
/// (`greater_than_or_equal`, `contains_any`) y la de `metadata_inspect.inferredTypes` (`number`,
/// `string`)—, que es la que el agente ya ve en el contrato.
///
/// Se deriva de `Serialize` en vez de tabularse aquí a propósito: una tabla propia en la fachada
/// sería un vocabulario paralelo al del wire (invariante #4) y podría desincronizarse del core en
/// silencio. El `unwrap_or` no es alcanzable con los enums de unidad de `core::types` (serializan
/// siempre a string), pero evita un `expect` en un camino de error.
fn nombre_de_wire<T: Serialize>(valor: &T) -> String {
    serde_json::to_value(valor)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_else(|| "desconocido".to_string())
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

/// **E30-H01**. Identidad de origen que un cursor de paginación
/// lleva firmada: la **tool** que lo emitió y, cuando la tool pagina más de una lista, el **contexto
/// de listado** dentro de ella (el `mode`/`field` de `metadata_inspect`).
///
/// Es lo que hace que decodificar un cursor ajeno falle de forma determinista en vez de «colar» un
/// offset numéricamente válido (`decisiones §23/A-03`, ROB-06). La **forma concreta** de la firma en
/// el wire la elige la fase verde; lo que fijan los tests es el comportamiento observable.
///
/// La identidad **no** incluye el criterio de selección (`text`/`where`/`filter`/`scope`/`ref`): ver
/// la decisión declarada en la cabecera de la sección E30-H01 de
/// `crates/lodestar-mcp/tests/mcp.rs` y el test
/// `cursor_de_otra_consulta_de_la_misma_tool_es_hallazgo_de_seguimiento`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CursorScope {
    /// Cursor de `knowledge_search` (lista `results`).
    KnowledgeSearch,
    /// Cursor de `knowledge_check` (lista `diagnostics`).
    KnowledgeCheck,
    /// Cursor de `graph_query` (lista `nodes`).
    GraphQuery,
    /// Cursor de `metadata_inspect`, con el contexto de listado que distingue sus dos modos: `None`
    /// es el catálogo (`mode: "catalog"`), `Some(field)` el vocabulario de ese campo.
    MetadataInspect(Option<String>),
}

impl CursorScope {
    /// Contexto del catálogo de `metadata_inspect` (`mode: "catalog"`).
    pub(crate) fn metadata_catalog() -> Self {
        CursorScope::MetadataInspect(None)
    }

    /// Contexto del vocabulario de un campo de `metadata_inspect` (`mode: "field"`), que es una
    /// lista distinta —y con otro orden total— por cada `field`.
    pub(crate) fn metadata_field(field: &str) -> Self {
        CursorScope::MetadataInspect(Some(field.to_string()))
    }

    /// Etiqueta estable del contexto, la que viaja **firmada** dentro del cursor y por la que se
    /// reconoce a su emisor. Es dato de wire: cambiarla invalida los cursores ya emitidos.
    fn etiqueta(&self) -> String {
        match self {
            CursorScope::KnowledgeSearch => "knowledge_search".to_string(),
            CursorScope::KnowledgeCheck => "knowledge_check".to_string(),
            CursorScope::GraphQuery => "graph_query".to_string(),
            CursorScope::MetadataInspect(None) => "metadata_inspect#catalog".to_string(),
            CursorScope::MetadataInspect(Some(f)) => format!("metadata_inspect#field:{f}"),
        }
    }

    /// Cómo se nombra este contexto en el mensaje de error: la tool y, si la tool pagina más de una
    /// lista, cuál de ellas. El nombre de la tool viaja como **palabra suelta** (`knowledge_search`,
    /// no `«knowledge_search»` pegado a otro token) para que el agente pueda buscarlo tal cual.
    fn descripcion(&self) -> String {
        match self {
            CursorScope::MetadataInspect(None) => "metadata_inspect en modo «catalog»".to_string(),
            CursorScope::MetadataInspect(Some(f)) => {
                format!("metadata_inspect en modo «field» sobre «{f}»")
            }
            otro => otro.etiqueta(),
        }
    }
}

/// **E30-H01**. Codifica un offset de paginación como cursor opaco **firmado con su origen**.
///
/// Debe sustituir a [`encode_cursor`] (v0.5.0: `format!("{offset:x}")`, un offset hex desnudo, sin
/// marca de quién lo emitió). Mantiene la propiedad que el rustdoc de aquella ya declaraba
/// —**autosuficiente**: como el orden de resultados es determinista y solo depende del contenido, un
/// offset reanuda idénticamente en cualquier servidor fresco— para la tool y el contexto correctos;
/// no introduce estado de sesión ni TTL.
///
/// # Forma del cursor (dato de wire)
///
/// `«<hex(etiqueta|offset)>.<firma>»`, donde la firma son los 8 primeros hex de
/// `blake3(etiqueta|offset)`. Dos consecuencias buscadas: (a) la etiqueta del emisor se **recupera**
/// al decodificar, así que un rechazo puede decir de qué contexto venía el cursor y no solo que no
/// vale; (b) la firma cubre etiqueta **y** offset, así que un cursor **retocado** —cambiar un dígito
/// del offset, mover un cursor de una tool a otra— no pasa (`decisiones §23/A-03`).
///
/// # Lo que la firma NO es
///
/// **No es defensa contra forja deliberada**, y el contrato no debe prometerlo: `blake3` va aquí
/// **sin clave** y el esquema está documentado, así que quien quiera fabricar un cursor válido para
/// cualquier tool y offset puede hacerlo. Es un mecanismo **anti-confusión** —detecta el retoque
/// accidental y el cruce de cursores entre tools, que es el defecto que E30-H01 cierra—, no un
/// control de autenticidad. Un cursor forjado con la firma correcta se acepta, y eso es inocuo: lo
/// único que un cliente consigue así es pedir un offset, que es exactamente lo que el parámetro
/// expresa. Si algún día el cursor llegara a codificar algo que no sea posición, esta propiedad
/// habría que revisarla (haría falta clave, y entonces dejaría de ser autosuficiente).
///
/// Sigue sin haber estado de sesión ni TTL: la etiqueta y el offset son todo lo que hace falta, y
/// `blake3` es determinista, así que dos procesos distintos emiten el **mismo** cursor para el mismo
/// offset y contexto.
fn encode_cursor_firmado(offset: usize, scope: &CursorScope) -> String {
    let payload = format!("{}|{offset:x}", scope.etiqueta());
    let firma = blake3::hash(payload.as_bytes()).to_hex();
    let cuerpo: String = payload.bytes().map(|b| format!("{b:02x}")).collect();
    format!("{cuerpo}.{}", &firma.as_str()[..FIRMA_CURSOR])
}

/// Hex de firma que lleva cada cursor: suficiente para que retocar un cursor a mano no cuele por
/// azar, corto para que el cursor siga siendo manejable en una traza.
const FIRMA_CURSOR: usize = 8;

/// **E30-H01**. Decodifica un cursor a su offset, verificando que fue emitido para `scope`.
///
/// **Deja de ser infalible**: [`decode_cursor`] es hoy
/// `usize::from_str_radix(cursor, 16).unwrap_or(0)`, así que un cursor basura (`decisiones §23/A-02`,
/// ROB-05) o de otra tool (`§23/A-03`, ROB-06) se reinterpreta en silencio como offset 0 o como una
/// página ajena. Esta devuelve [`ErrorCode::InvalidSchema`] con un mensaje que **nombra el cursor
/// recibido** y, cuando es determinable, qué tool lo esperaba frente a cuál lo produjo; la fachada
/// MCP lo sirve tal cual.
///
/// Dos rechazos distintos, con mensajes distintos porque son errores distintos del agente:
/// **no decodifica** (basura, o un cursor retocado cuya firma no cuadra) y **de otro origen** (una
/// firma legítima de otro contexto, que se nombra: es la información que le dice al agente que lo
/// que hizo fue mezclar dos paginaciones).
fn decode_cursor_firmado(cursor: &str, scope: &CursorScope) -> Result<usize, AppError> {
    let ilegible = || {
        AppError::invalid_schema(format!(
            "«cursor» no es un cursor de paginación de esta superficie: «{cursor}». Un cursor se \
             toma del «nextCursor» de una respuesta anterior de {} — no se deriva de un número de \
             página; omite el parámetro para empezar por el principio",
            scope.descripcion()
        ))
    };

    let (cuerpo, firma) = cursor.split_once('.').ok_or_else(ilegible)?;
    // Un cursor emitido por esta capa es hex ASCII puro en sus dos mitades. Comprobarlo ANTES de
    // trocear no es un lujo: el troceo de abajo va por bytes, y un cursor con un carácter multibyte
    // («🔥.807e307a») partía el proceso en un índice que no era frontera de carácter —panic, no
    // error—, y con él la sesión JSON-RPC entera. Un cursor no-ASCII es malformado como cualquier
    // otro: `INVALID_SCHEMA`, nunca un panic.
    let ascii_hex = |s: &str| s.bytes().all(|b| b.is_ascii_hexdigit());
    if !ascii_hex(cuerpo)
        || !ascii_hex(firma)
        || cuerpo.len() % 2 != 0
        || firma.len() != FIRMA_CURSOR
    {
        return Err(ilegible());
    }
    let bytes: Vec<u8> = cuerpo
        .as_bytes()
        .chunks_exact(2)
        .map(|par| {
            // `par` es hex ASCII verificado arriba, así que ni el `from_utf8` ni el parseo fallan.
            u8::from_str_radix(std::str::from_utf8(par).unwrap_or("zz"), 16)
        })
        .collect::<Result<_, _>>()
        .map_err(|_| ilegible())?;
    let payload = String::from_utf8(bytes).map_err(|_| ilegible())?;
    // La firma recomputada es hex ASCII de blake3, así que este corte sí es seguro por construcción.
    if &blake3::hash(payload.as_bytes()).to_hex().as_str()[..FIRMA_CURSOR] != firma {
        return Err(ilegible());
    }

    let (etiqueta, offset) = payload.rsplit_once('|').ok_or_else(ilegible)?;
    let offset = usize::from_str_radix(offset, 16).map_err(|_| ilegible())?;
    if etiqueta != scope.etiqueta() {
        return Err(AppError::invalid_schema(format!(
            "«cursor» pertenece a otra paginación: «{cursor}» lo emitió {}, y esta llamada es de \
             {}. Un cursor no es intercambiable entre tools ni entre contextos de listado — cada \
             lista tiene su propio orden total, así que reanudar con el offset de otra devolvería \
             una página ajena; sigue el «nextCursor» de esta misma llamada",
            emisor_legible(etiqueta),
            scope.descripcion()
        )));
    }
    Ok(offset)
}

/// Cómo se nombra en el mensaje de error el contexto que **emitió** un cursor ajeno: se reconstruye
/// desde la etiqueta firmada, así que sirve incluso para una etiqueta que esta versión ya no emite.
fn emisor_legible(etiqueta: &str) -> String {
    match etiqueta.split_once('#') {
        Some(("metadata_inspect", "catalog")) => "metadata_inspect en modo «catalog»".to_string(),
        Some(("metadata_inspect", resto)) => format!(
            "metadata_inspect en modo «field» sobre «{}»",
            resto.trim_start_matches("field:")
        ),
        _ => etiqueta.to_string(),
    }
}

/// Traduce el `cursor` recibido por el wire al offset de arranque de la página.
///
/// **E30-H01**: aquí vive la decisión declarada de que `cursor: ""` cuenta como **ausente**, no como
/// malformado — es lo que hacía v0.5.0 y lo que el `inputSchema` da por bueno al anunciar el
/// parámetro (`descubribilidad.rs` manda exactamente ese valor de ejemplo). Un cursor de wire nunca
/// se emite vacío, así que la cadena vacía solo puede venir de un cliente que quiso decir «desde el
/// principio». Cualquier **otra** cadena que no decodifique para este contexto es `INVALID_SCHEMA`
/// (hasta v0.5.0 caía a offset 0 en silencio: `decisiones §23/A-02`).
fn offset_de_cursor(cursor: Option<&str>, scope: &CursorScope) -> Result<usize, AppError> {
    match cursor {
        None | Some("") => Ok(0),
        Some(c) => decode_cursor_firmado(c, scope),
    }
}

/// Aplica la **cota de página** sobre una lista ya ordenada por su orden total: recorta a
/// `[start, start + limit)` —con `start` leído del cursor-offset— y devuelve el trozo más el
/// `next_cursor` al siguiente (o `None` al agotar).
///
/// Es la mecánica que `knowledge_search`/`knowledge_check`/`graph_query` traían **copiada** inline
/// (las mismas cuatro líneas en tres sitios, con su propio `decode_cursor`); desde E30-H01 las
/// cuatro tools paginadas pasan por aquí, que es lo que hace que la firma de origen del cursor sea
/// **una** decisión y no cuatro (invariante #3).
///
/// `scope` es la identidad que se firma y se verifica: un cursor de otra tool —o de otro contexto de
/// listado de la misma tool— no decodifica aquí, y el `Err(InvalidSchema)` sube tal cual a la
/// fachada. `limit` ausente → `default_limit`; por encima de `max_limit` se acota (la fachada MCP ya
/// rechaza antes lo que exceda el máximo declarado en el `inputSchema`, así que esto es la red de
/// seguridad de cualquier otro llamante).
fn pagina<T>(
    items: Vec<T>,
    limit: Option<usize>,
    cursor: Option<&str>,
    default_limit: usize,
    max_limit: usize,
    scope: &CursorScope,
) -> Result<(Vec<T>, Option<String>), AppError> {
    let total = items.len();
    let limit = limit.unwrap_or(default_limit).min(max_limit);
    let start = offset_de_cursor(cursor, scope)?.min(total);
    let end = start.saturating_add(limit).min(total);
    let next_cursor = (end > start && end < total).then(|| encode_cursor_firmado(end, scope));
    let page = items
        .into_iter()
        .skip(start)
        .take(end.saturating_sub(start))
        .collect();
    Ok((page, next_cursor))
}

// ---------------------------------------------------------------------------
// `metadata_inspect` — envoltorio de respuesta de la tool (E20-H03, sustituye a `schema_inspect`).
//
// Es el discriminador de modo más el `nextCursor` de la página (E26-H10): un enum `untagged` que
// **aplana** los tipos de wire del CORE (`MetadataCatalog`/`FieldInspection`, `core::types`, con su
// serde ya fijado en E20-H03) y les añade el cursor. No es una capa DTO paralela (invariante #4): la
// forma de los datos sigue viviendo una sola vez en `core::types` —aquí se reexpone con `flatten`,
// sin redeclarar un solo campo—; esto es framing de tool (qué proyección devuelve cada `mode` y
// dónde se quedó la página), igual que `GraphQueryResult` o `KnowledgeGetResponse`.
//
// El cursor vive AQUÍ y no en el core por la misma razón que la cota (E26-H10): paginar es servir el
// wire, y el core sigue devolviendo la verdad completa (invariantes #2 y #3).
// ---------------------------------------------------------------------------

/// Límite por defecto de entradas por página de `metadata_inspect` (E26-H10): campos en modo
/// `catalog`, valores en modo `field`. Mismo par que `knowledge_check`, la otra tool que enumera un
/// catálogo.
const DEFAULT_METADATA_LIMIT: usize = 100;
/// Tope duro de entradas por página de `metadata_inspect` (E26-H10; evita respuestas de tamaño
/// proporcional al workspace: el catálogo emite una fila por field path y `values` una por valor
/// escalar distinto).
const MAX_METADATA_LIMIT: usize = 1000;

/// Respuesta de la tool `metadata_inspect` (`ARCHITECTURE.md §20.10`, `REFACTOR_PHASE_2 §Fase 6`).
///
/// `untagged`: serializa como el valor interno directo, así que `Catalog` da
/// `{ "fields": [ … ], "nextCursor": … }` (la forma de [`MetadataCatalog`] más el cursor) y `Field`
/// da `{ "field": …, "presentIn": …, …, "nextCursor": … }` (la de [`FieldInspection`] más el
/// cursor) — sin envoltorio ni discriminador extra. Solo `Serialize` (+ `JsonSchema` para el
/// `outputSchema`): la tool PRODUCE esta respuesta, nunca la consume del wire.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(untagged)]
pub enum MetadataInspection {
    /// Modo `"catalog"`: la página del catálogo de propiedades del workspace.
    Catalog(MetadataCatalogPage),
    /// Modo `"field"`: la página de la inspección de una propiedad concreta.
    Field(FieldInspectionPage),
}

/// Una página del catálogo de propiedades (E26-H10): el [`MetadataCatalog`] del core —**aplanado**,
/// con sus `fields` ya recortados a la página— y el cursor a la siguiente.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct MetadataCatalogPage {
    /// El catálogo con la página de `fields` (el orden total es el determinista del core, por field
    /// path). Se aplana: en el wire sus claves son las de [`MetadataCatalog`], sin anidar.
    #[serde(flatten)]
    pub catalog: MetadataCatalog,
    /// Cursor opaco a la siguiente página de `fields`, o `None` si no quedan más campos.
    pub next_cursor: Option<String>,
}

/// Una página de la inspección de una propiedad (E26-H10): la [`FieldInspection`] del core
/// —**aplanada**, con sus `values` ya recortados a la página— y el cursor a la siguiente.
///
/// Los agregados (`presentIn`/`missingIn`/`inferredTypes`) describen **todo** el workspace, no la
/// página: lo que se pagina es la lista, no la estadística.
#[derive(Debug, Clone, Serialize, schemars::JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct FieldInspectionPage {
    /// La inspección con la página de `values` (el orden total es el determinista del core: por
    /// conteo descendente y, a igual conteo, por el texto del valor). Se aplana: en el wire sus
    /// claves son las de [`FieldInspection`], sin anidar.
    #[serde(flatten)]
    pub inspection: FieldInspection,
    /// Cursor opaco a la siguiente página de `values`, o `None` si no queda más vocabulario.
    pub next_cursor: Option<String>,
}

// ---------------------------------------------------------------------------
// `outputSchema` (E10-H13, `ARCHITECTURE.md §19.6`, decisión **D6b**, `docs/history/REFACTOR.md §13`).
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

    /// `outputSchema` de `metadata_inspect` (== [`MetadataInspection`]), con el `type: "object"` de
    /// la raíz FIJADO a mano.
    ///
    /// Es el único tipo de salida de la superficie que es un `enum` (`untagged`): schemars lo
    /// deriva como un `anyOf` de las dos variantes y **no** emite `type` en la raíz —en el caso
    /// general las ramas de un `untagged` podrían ser de tipos JSON distintos, así que no lo
    /// infiere—. Pero el spec MCP exige que todo `outputSchema` sea un JSON Schema **de tipo
    /// `object`**, y un cliente estricto que rechaza una tool inválida deja de registrar **las
    /// diez**: el `anyOf` pelado inutilizaba el servidor entero.
    ///
    /// Fijarlo aquí no excluye ninguna respuesta hoy válida, porque las dos variantes
    /// ([`super::MetadataCatalogPage`] y [`super::FieldInspectionPage`]) ya son `type: "object"`:
    /// solo declara en la raíz lo que el `anyOf` no sabe expresar. El wire no cambia — envuelve el
    /// schema DERIVADO, no rediseña el tipo, que sigue sirviendo `{ fields, nextCursor }` o
    /// `{ field, values, … }` sin discriminador.
    pub fn metadata_inspect_schema() -> Value {
        let mut schema = schema_of::<MetadataInspection>();
        if let Some(raiz) = schema.as_object_mut() {
            raiz.insert("type".to_string(), Value::String("object".to_string()));
        }
        schema
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

#[cfg(test)]
mod tests {
    use super::*;

    /// El ancla sintética de los diagnósticos sin `target` (E29-H06) debe ser un [`RelPath`]
    /// válido: `anchor_workspace()` hace `expect` sobre ella y un cambio del literal que la
    /// invalidara (una barra inicial, un `..`) tumbaría `full_analysis` en caliente.
    #[test]
    fn ancla_de_workspace_es_relpath_valido() {
        let anchor = RelPath::new(ANCHOR_WORKSPACE)
            .expect("ANCHOR_WORKSPACE debe ser construible como RelPath");
        assert_eq!(anchor.as_str(), ANCHOR_WORKSPACE);
        assert_eq!(anchor_workspace(), anchor);
        assert!(
            !anchor.is_markdown(),
            "el ancla no puede parecer un documento: es el plano de control, no un `.md`"
        );
    }

    // -----------------------------------------------------------------------
    // E30-H03 (seguimiento 9) — `PATH-NOT-UTF8` sin `targets` llega a `full_analysis`
    //
    // `fusiona_diagnosticos_de_descubrimiento` es la mitad del cuerpo de `App::full_analysis` que
    // NO toca el disco, extraída en E30-H03 precisamente para poder ejercerla con un `Check`
    // SINTÉTICO: los diagnósticos que la motivan —`PATH-NOT-UTF8`, `WORKSPACE-EMPTY`— nacen de
    // condiciones del sistema de ficheros que no se fabrican de forma portable (en APFS no hay
    // manera de crear un nombre de fichero que no sea UTF-8 válido).
    //
    // El test de abajo clava las TRES reglas que su rustdoc enumera, cada una con su propia
    // aserción, para que mutar cualquiera de las tres haga fallar el test:
    //   1. familia reclasificada a `ignore` por config → se DESCARTA;
    //   2. `Check` CON `targets` → se ancla bajo su PRIMER target, junto a lo que ese documento ya
    //      tuviera;
    //   3. `Check` SIN `targets` → se ancla bajo `anchor_workspace()`, NUNCA se descarta.
    // -----------------------------------------------------------------------

    /// Un [`Check`] sintético mínimo con el código, la severidad y los `targets` pedidos.
    fn check_sintetico(
        level: Severity,
        code: lodestar_core::types::CheckCode,
        targets: &[&str],
    ) -> Check {
        Check::new(
            level,
            code,
            format!("diagnóstico sintético {}", code.as_str()),
            targets
                .iter()
                .map(|t| RelPath::new(t).expect("target del test debe ser un RelPath válido"))
                .collect(),
        )
    }

    /// **E30-H03 (seguimiento 9)** · `path_not_utf8_sin_targets_llega_a_full_analysis`: **Dado** un
    /// [`Check`] sintético con código `PATH-NOT-UTF8` y **sin** `targets`, **Cuando** se inyecta en
    /// el camino de `full_analysis` (su costura pura,
    /// [`fusiona_diagnosticos_de_descubrimiento`]), **Entonces** aparece en el resultado final —
    /// bajo [`ANCHOR_WORKSPACE`] y contando para [`Analysis::warn_count`]— en vez de desaparecer
    /// en silencio por no tener clave con la que entrar al mapa.
    ///
    /// Clava además, en la misma pasada, las otras dos reglas de la función (anclaje por primer
    /// target y descarte de la familia puesta a `ignore`), que son las que dan sentido a la
    /// tercera: sin ellas, «no se descarta nunca» sería trivialmente cierto.
    #[test]
    fn path_not_utf8_sin_targets_llega_a_full_analysis() {
        use lodestar_core::types::CheckCode;
        use lodestar_workspace::config::{
            ValidationSection, ValidationSeverity, FAMILY_MALFORMED_FRONTMATTER,
        };

        // El documento `alfa.md` ya trae un diagnóstico propio: la regla 2 debe MEZCLAR el
        // diagnóstico de descubrimiento con él, no reemplazarlo.
        let alfa = RelPath::new("alfa.md").unwrap();
        let previo = check_sintetico(Severity::Info, CheckCode::LinkCaseMismatch, &["alfa.md"]);
        let mut analysis = Analysis {
            documents: vec![alfa.clone()],
            diagnostics: BTreeMap::from([(alfa.clone(), vec![previo.clone()])]),
            ..Analysis::default()
        };

        // Config: la familia `malformedFrontmatter` (la de `FM-UNCLOSED`) queda a `ignore`.
        let validation = ValidationSection {
            families: BTreeMap::from([(
                FAMILY_MALFORMED_FRONTMATTER.to_string(),
                ValidationSeverity::Ignore,
            )]),
        };

        let sin_targets = check_sintetico(Severity::Warn, CheckCode::PathNotUtf8, &[]);
        let con_target = check_sintetico(Severity::Err, CheckCode::DocTooLarge, &["alfa.md"]);
        let suprimido = check_sintetico(Severity::Err, CheckCode::FmUnclosed, &["alfa.md"]);

        fusiona_diagnosticos_de_descubrimiento(
            &mut analysis,
            vec![sin_targets, con_target, suprimido],
            &validation,
        );

        // REGLA 3 — el `PATH-NOT-UTF8` sin `targets` sobrevive, bajo el ancla del workspace.
        let del_ancla = analysis
            .diagnostics
            .get(&anchor_workspace())
            .unwrap_or_else(|| {
                panic!(
                    "un diagnóstico de descubrimiento SIN `targets` debe anclarse bajo \
                     «{ANCHOR_WORKSPACE}», nunca descartarse: {:?}",
                    analysis.diagnostics
                )
            });
        assert!(
            del_ancla.iter().any(|c| c.code == CheckCode::PathNotUtf8),
            "`PATH-NOT-UTF8` sin `targets` debe llegar al `Analysis` final: {del_ancla:?}"
        );
        assert!(
            del_ancla
                .iter()
                .all(|c| c.targets.is_empty() || c.code != CheckCode::PathNotUtf8),
            "el anclaje no puede FALSEAR los `targets`: el diagnóstico sigue viajando con la \
             verdad («no hay fichero»), la clave del mapa es solo la etiqueta: {del_ancla:?}"
        );

        // REGLA 2 — el que SÍ tiene `targets` va bajo su primer target, junto al que ya estaba.
        let de_alfa = analysis
            .diagnostics
            .get(&alfa)
            .expect("`alfa.md` conserva su entrada en el mapa");
        assert!(
            de_alfa.contains(&previo),
            "el anclaje por target no puede pisar los diagnósticos que el documento ya tenía: \
             {de_alfa:?}"
        );
        assert!(
            de_alfa.iter().any(|c| c.code == CheckCode::DocTooLarge),
            "un diagnóstico de descubrimiento CON `targets` se ancla bajo su primer target, no \
             bajo «{ANCHOR_WORKSPACE}»: {de_alfa:?}"
        );
        assert!(
            !analysis
                .diagnostics
                .get(&anchor_workspace())
                .is_some_and(|v| v.iter().any(|c| c.code == CheckCode::DocTooLarge)),
            "un diagnóstico CON `targets` NO puede acabar bajo el ancla del workspace: {:?}",
            analysis.diagnostics
        );

        // REGLA 1 — la familia puesta a `ignore` se descarta, venga de donde venga.
        assert!(
            analysis
                .diagnostics
                .values()
                .flatten()
                .all(|c| c.code != CheckCode::FmUnclosed),
            "una familia reclasificada a `ignore` por la config no entra al `Analysis`: {:?}",
            analysis.diagnostics
        );

        // La severidad EFECTIVA se escribe en el propio `check.level` antes de insertarlo, así que
        // el diagnóstico anclado cuenta ya reclasificado para `hard_fail`/`warn_count`.
        assert_eq!(
            analysis.warn_count(),
            1,
            "el `PATH-NOT-UTF8` anclado cuenta como aviso en el recuento final: {:?}",
            analysis.diagnostics
        );
        assert_eq!(
            analysis.hard_fail(),
            1,
            "solo `alfa.md` tiene un error (el `DOC-TOO-LARGE` anclado por target): {:?}",
            analysis.diagnostics
        );
    }

    // -----------------------------------------------------------------------
    // E30-H01 — Cursores estrictos: el núcleo de codificación
    //
    // El comportamiento observable por el wire lo fijan los tests de
    // `crates/lodestar-mcp/tests/mcp.rs` (sección E30-H01). Aquí se fija la MECÁNICA que lo
    // sostiene, que es privada y no tiene otra puerta: el par `encode_cursor`/`decode_cursor` y su
    // uso desde `pagina()`.
    //
    // FASE ROJA — los cuatro tests fallan con el `todo!()` de los STUBS `encode_cursor_firmado`/
    // `decode_cursor_firmado`, declarados en la fase roja como firma + `todo!()` y NADA más. La
    // firma la fija esta fase porque el criterio no se puede expresar sin ella: la historia manda
    // que `decode_cursor` «deje de ser infalible» (pase a `Result`) y que el cursor lleve la
    // identidad de su origen (`CursorScope`).
    //
    // POR QUÉ EL PAR VIEJO SIGUE AHÍ: `encode_cursor`/`decode_cursor` los llaman CUATRO sitios de
    // producción; renombrarlos o cambiarles la firma es reescribir producción, que no es trabajo
    // del autor de tests. Los stubs conviven con ellos y solo los referencian estos tests, así que
    // el resto del crate compila y corre igual que antes.
    //
    // NOTA para el implementador: `pagina()` es hoy el punto único **solo para
    // `metadata_inspect`**. `knowledge_search` (L720), `knowledge_check` (L1125) y `graph_query`
    // (L1459) tienen la mecánica COPIADA inline y llaman a `decode_cursor` por su cuenta. El
    // alcance de E30-H01 («`pagina()` sigue siendo el único punto de mecánica de paginación») exige
    // converger las cuatro ahí antes de añadirle el parámetro de identidad, o la firma habrá que
    // propagarla a mano por cuatro sitios y el siguiente defecto de cursor volverá a ser cuádruple.
    // Al converger, el par viejo desaparece y el firmado se queda con su nombre.
    // -----------------------------------------------------------------------

    /// **E30-H01** · Criterio `roundtrip()` del núcleo de codificación:
    /// **Dado** el par `encode_cursor`/`decode_cursor` tras el arreglo, **Cuando** se codifica un
    /// offset y se decodifica de vuelta con la identidad de tool/consulta **correcta**, **Entonces**
    /// el offset recuperado es exactamente el original.
    ///
    /// Es la propiedad que la historia manda **conservar**: el cursor sigue siendo autosuficiente
    /// (un offset reanuda idénticamente en un servidor fresco), solo que ahora lleva firmado de
    /// dónde salió. Se prueban los bordes que la mecánica de `pagina()` produce de verdad: 0, el
    /// primer corte y un offset grande.
    #[test]
    fn roundtrip() {
        for scope in [
            CursorScope::KnowledgeSearch,
            CursorScope::KnowledgeCheck,
            CursorScope::GraphQuery,
            CursorScope::metadata_catalog(),
            CursorScope::metadata_field("uid"),
        ] {
            for offset in [0usize, 1, 20, 100, 152, 99_999] {
                let cursor = encode_cursor_firmado(offset, &scope);
                assert_eq!(
                    decode_cursor_firmado(&cursor, &scope).unwrap_or_else(|e| panic!(
                        "un cursor recién emitido para su propio scope debe decodificar: {e}"
                    )),
                    offset,
                    "codificar y decodificar con el MISMO scope debe devolver el offset original \
                     («{cursor}»)"
                );
            }
        }
    }

    /// **E30-H01** · Mitad de servicio de `cursor_malformado_es_invalid_schema` (A-02 / ROB-05):
    /// **Dado** una cadena que no es un cursor emitido por esta capa, **Cuando** se decodifica,
    /// **Entonces** falla — **no** cae a offset 0.
    ///
    /// Hasta v0.5.0 `decode_cursor` era `usize::from_str_radix(cursor, 16).unwrap_or(0)`: cualquier
    /// cadena que no parseara como hex se trataba como «empieza desde el principio», indistinguible
    /// de un cliente que omite `cursor` a propósito. El error debe además **nombrar el valor
    /// recibido**, que es lo que la fachada MCP sirve como mensaje del `INVALID_SCHEMA`.
    #[test]
    fn un_cursor_que_no_decodifica_no_cae_a_cero() {
        let scope = CursorScope::KnowledgeSearch;
        for basura in ["zzz-no-hex", "!!", "  ", "-1", "0x20"] {
            let err = decode_cursor_firmado(basura, &scope).expect_err(
                "un cursor que no decodifica debe fallar, no reinterpretarse como offset 0 \
                 (ROB-05): la respuesta silenciosamente equivocada es peor que el error",
            );
            assert!(
                err.message.contains(basura),
                "…y el error debe deletrear el valor recibido («{basura}»), que es lo que el agente \
                 necesita para corregir su llamada: «{}»",
                err.message
            );
            assert_eq!(
                err.code,
                ErrorCode::InvalidSchema,
                "…con el código del catálogo que `decisiones §16(j)` decidió: «{}»",
                err.message
            );
        }
    }

    /// **E30-H01** · Mitad de servicio de `cursor_de_otra_tool_es_invalid_schema` (A-03 / ROB-06):
    /// **Dado** un cursor **bien formado** emitido para un scope, **Cuando** se decodifica con otro
    /// scope, **Entonces** falla — no decodifica a un offset numéricamente válido y ajeno.
    ///
    /// Es el defecto por construcción que la ficha nombra: el cursor no llevaba marca de origen, así
    /// que cualquier hex de cualquier tool decodificaba a un offset que las cuatro aceptaban. Se
    /// cruzan **todos** los pares distintos, incluidos los dos contextos de `metadata_inspect` y dos
    /// campos distintos de mode «field» (la decisión declarada en la fase roja: la identidad se ata
    /// a la tool y a su contexto de listado).
    #[test]
    fn un_cursor_de_otro_scope_no_decodifica() {
        let scopes = [
            CursorScope::KnowledgeSearch,
            CursorScope::KnowledgeCheck,
            CursorScope::GraphQuery,
            CursorScope::metadata_catalog(),
            CursorScope::metadata_field("uid"),
            CursorScope::metadata_field("status"),
        ];
        for (i, emisor) in scopes.iter().enumerate() {
            let cursor = encode_cursor_firmado(100, emisor);
            for (j, receptor) in scopes.iter().enumerate() {
                if i == j {
                    continue;
                }
                let err = decode_cursor_firmado(&cursor, receptor)
                    .err()
                    .unwrap_or_else(|| {
                        panic!(
                        "el cursor «{cursor}» se emitió para {emisor:?} y NO puede decodificar \
                         para {receptor:?}: hasta v0.5.0 era un offset hex desnudo que las cuatro \
                         tools compartían, así que colaba como página válida en forma y ajena en \
                         significado (ROB-06)"
                    )
                    });
                assert_eq!(
                    err.code,
                    ErrorCode::InvalidSchema,
                    "…con el mismo código que el cursor malformado: «{}»",
                    err.message
                );
            }
        }
    }

    /// **E30-H01** · Control anti-vacuo del cursor firmado: **Dado** dos offsets distintos del mismo
    /// scope, **Cuando** se codifican, **Entonces** producen cursores distintos, y un cursor
    /// **sigue siendo autosuficiente** (no depende de nada del proceso que lo emitió).
    ///
    /// Sin esta guarda, «firmar el origen» podría degenerar en un cursor constante (que decodificara
    /// siempre a 0 y pasara `un_cursor_de_otro_scope_no_decodifica` por accidente) o en un handle de
    /// sesión, que es justo lo que la historia prohíbe: el cursor debe reanudar idéntico en cualquier
    /// servidor fresco. La codificación se ejerce dos veces para fijar que es **determinista**: dos
    /// procesos distintos emiten el mismo cursor para el mismo offset y scope.
    #[test]
    fn el_cursor_firmado_sigue_siendo_determinista_y_autosuficiente() {
        let scope = CursorScope::KnowledgeSearch;
        let a = encode_cursor_firmado(20, &scope);
        let b = encode_cursor_firmado(40, &scope);
        assert_ne!(
            a, b,
            "dos offsets distintos del mismo scope deben producir cursores distintos, o el cursor \
             no codifica el offset"
        );
        assert_eq!(
            a,
            encode_cursor_firmado(20, &scope),
            "la codificación debe ser DETERMINISTA (misma entrada → mismo cursor): es lo que hace \
             que un cursor emitido en un proceso reanude en otro fresco (autosuficiencia, \
             `§20.10`)"
        );
        assert_eq!(decode_cursor_firmado(&a, &scope).ok(), Some(20));
        assert_eq!(decode_cursor_firmado(&b, &scope).ok(), Some(40));
    }

    /// **`decisiones §16(l)`** (superviviente nuevo, hallado en la pasada de mutantes) — `pagina()`
    /// **acota** el `limit` a `max_limit`, y aplica `default_limit` cuando no llega ninguno.
    ///
    /// El mutante que sobrevivía: quitar el `.min(max_limit)` de `pagina()`. Ningún test fallaba,
    /// porque las cuatro tools llegan hoy por la fachada MCP, que ya rechaza antes lo que exceda el
    /// `maximum` del `inputSchema`. Pero `pagina()` es la **red de seguridad** de cualquier otro
    /// llamante —el propio rustdoc lo dice— y una red que nadie comprueba no es una red: sin la
    /// cota, un `limit` desmedido devuelve la lista entera en una respuesta, que es exactamente el
    /// desbordamiento de payload que E26-H10 cerró.
    ///
    /// Se asevera sobre la **mecánica de paginación** (cuántos elementos trae la página y si hay
    /// `nextCursor`), no sobre la forma del código.
    #[test]
    fn pagina_acota_el_limit_al_maximo() {
        let scope = CursorScope::KnowledgeSearch;
        let items: Vec<usize> = (0..500).collect();

        // (1) Un `limit` por encima del máximo se acota al máximo, y por tanto queda continuación.
        let (page, next) = pagina(items.clone(), Some(9_999), None, 20, 100, &scope)
            .expect("paginar sin cursor no puede fallar");
        assert_eq!(
            page.len(),
            100,
            "un `limit` de 9999 con `max_limit` 100 debe servir 100 elementos, no los 500: sin la \
             cota, `pagina()` devuelve la lista entera en una respuesta — el desbordamiento de \
             payload que E26-H10 cerró — y deja de ser la red de seguridad que su contrato promete"
        );
        assert!(
            next.is_some(),
            "y al haber acotado quedan elementos por servir, así que tiene que emitir `nextCursor`: \
             sin él la página acotada sería una truncadura silenciosa, que es peor que no acotar"
        );

        // (2) Control anti-vacuo: un `limit` POR DEBAJO del máximo se respeta tal cual.
        let (page, _) = pagina(items.clone(), Some(7), None, 20, 100, &scope)
            .expect("paginar sin cursor no puede fallar");
        assert_eq!(
            page.len(),
            7,
            "la cota solo recorta por arriba: un `limit` legítimo se sirve entero (si no, el \
             arreglo sería «devolver siempre max_limit», que rompe la paginación)"
        );

        // (3) Sin `limit` manda el `default_limit`, no el máximo.
        let (page, _) =
            pagina(items, None, None, 20, 100, &scope).expect("paginar sin cursor no puede fallar");
        assert_eq!(
            page.len(),
            20,
            "sin `limit` explícito manda `default_limit` (20), no `max_limit`: son dos parámetros \
             distintos y confundirlos multiplica por 5 el payload de toda llamada sin `limit`"
        );
    }
}
