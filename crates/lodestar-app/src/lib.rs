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
use std::path::Path;

use serde::{Deserialize, Serialize};

use lodestar_core::model;
use lodestar_core::types::{
    workspace_revision, Backlinks, Check, ConceptRef, ConceptRevision, ErrorCode, Frontmatter,
    RelPath, WorkspaceRevision,
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
/// [`Envelope`]. Expone `workspace_status` (E10-H08), `knowledge_search` (E10-H09) y
/// `knowledge_get` (E10-H10); `schema_inspect`, `knowledge_check`, … se irán añadiendo en
/// historias siguientes.
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

    /// Localiza conceptos por texto y filtros, con snippets y paginación por cursor, **sin devolver
    /// cuerpos completos** (E10-H09, `ARCHITECTURE.md §19.6`, `REFACTOR §9.2/§15`).
    ///
    /// La **verdad** del casado la da el core (invariante #3): el conjunto de conceptos que casan
    /// `text` se computa con la misma semántica de subcadena de la DSL del prototipo
    /// (`Bundle::query` → `tokenize_query`/`match_token`), intersectada con la lista autoritativa de
    /// conceptos (`Analysis::concepts`, así los reservados `index.md`/`log.md` nunca aparecen). Un
    /// `text` vacío casa todos los conceptos.
    ///
    /// Los `filters` baratos se aplican aquí (`types`/`statuses`/`tags`/`pathPrefix`); los filtros
    /// avanzados del contrato (`references`/`referencedBy`/`linkedTo`/`is:*`/`has:*`) quedan
    /// **admitidos pero sin criterio** en esta historia (se ignoran silenciosamente si llegan, no se
    /// inventan con semántica dudosa — E10-H09 fuera de alcance).
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
    /// Cada resultado lleva `revision` = [`ConceptRevision`] del contenido en disco (blake3, E10-H03)
    /// y un `snippet` compacto NO vacío; la estructura [`SearchResult`] **no tiene** campo `body`, así
    /// que es imposible filtrar el cuerpo completo por esta vía.
    pub fn knowledge_search(
        &self,
        text: &str,
        filters: &SearchFilters,
        _sort: Option<&str>,
        limit: Option<usize>,
        cursor: Option<&str>,
    ) -> Result<SearchResults, WorkspaceError> {
        let bundle = self.workspace.bundle()?;
        let analysis = bundle.analyze();
        let files = bundle.files();

        let text_trim = text.trim();
        let needle = text_trim.to_lowercase();
        // Casado de texto reusando la verdad del core (subcadena); intersección con conceptos.
        let matched_text: BTreeSet<RelPath> = bundle.query(text_trim).into_iter().collect();

        let mut results: Vec<SearchResult> = Vec::new();
        for path in &analysis.concepts {
            if !matched_text.contains(path) {
                continue;
            }
            let Some(raw) = files.get(path) else { continue };
            let parsed = model::parse_file(path.as_str(), raw);
            let fm = parsed.fm.unwrap_or_default();

            if !passes_filters(path, &fm, filters) {
                continue;
            }

            let title = fm
                .title
                .clone()
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| model::title_from_path(path.as_str()));
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
            let revision = ConceptRevision::from_hash(*blake3::hash(raw.as_bytes()).as_bytes());

            results.push(SearchResult {
                path: path.clone(),
                id: None,
                r#type: fm.r#type.clone(),
                title,
                status: fm.status.clone(),
                description: fm.description.clone(),
                tags: tags_to_vec(&fm.tags),
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

    /// Obtiene un concepto concreto, con `include` selectivo y selección de secciones por
    /// `headingPath` (E10-H10, `ARCHITECTURE.md §19.6`, `REFACTOR §9.3`).
    ///
    /// Resuelve con [`App::resolve_ref`] (E10-H04) — `Err(ErrorCode::ConceptNotFound)` si el path
    /// no está en la lista autoritativa de conceptos. `revision` (== [`ConceptRevision`], E10-H03)
    /// se calcula **siempre**, sin depender de `include`: es la identidad de contenido, no un
    /// campo opcional.
    ///
    /// `include` es la lista de campos wire pedidos (`"frontmatter"`, `"body"`, `"outgoingLinks"`,
    /// `"backlinks"`, `"diagnostics"`, `"externalReferences"`; `"revision"` es aceptado pero no-op,
    /// ya que ese campo siempre se puebla). Un campo **no** pedido queda en `None` en el
    /// [`ConceptView`] — nunca en su valor por defecto "vacío" disfrazado de "no pedido", para que
    /// el `include` selectivo sea significativo (criterio `get_incluye_revision`).
    ///
    /// `sections`, si está presente y no vacío, acota el `body` devuelto (solo aplica si `body` fue
    /// pedido en `include`): cada `headingPath` (p. ej. `["Security","Token rotation"]`) localiza
    /// esa subsección anidada del Markdown (ver la función privada `extract_sections` más abajo) y
    /// el resultado final es la concatenación de todos los `headingPath` pedidos. Sin `sections`,
    /// `body` es el cuerpo completo.
    ///
    /// `externalReferences` queda **vacío** en esta historia (E11-H04 fuera de alcance: no hay
    /// todavía criterio de qué frontmatter de productor cuenta como referencia externa) — se puebla
    /// como `Vec::new()` cuando se pide, para respetar la selectividad de `include` sin inventar
    /// semántica.
    pub fn knowledge_get(
        &self,
        r: &ConceptRef,
        include: &[String],
        sections: Option<&[Vec<String>]>,
    ) -> Result<ConceptView, ErrorCode> {
        let path = self.resolve_ref(r)?;
        let bundle = self
            .workspace
            .bundle()
            .map_err(|e| workspace_error_code(&e))?;
        let files = bundle.files();
        // `resolve_ref` ya comprobó que `path` está en `Analysis::concepts`, que se computa a
        // partir de este mismo `FileMap` (invariante #3) — así que el fichero existe.
        let raw = files
            .get(&path)
            .expect("resolve_ref garantiza presencia en el FileMap");
        let parsed = model::parse_file(path.as_str(), raw);
        let revision = ConceptRevision::from_hash(*blake3::hash(raw.as_bytes()).as_bytes());

        let wants = |field: &str| include.iter().any(|s| s == field);

        let frontmatter = wants("frontmatter").then(|| parsed.fm.clone().unwrap_or_default());
        let body = wants("body").then(|| match sections {
            Some(secs) if !secs.is_empty() => extract_sections(&parsed.body, secs),
            _ => parsed.body.clone(),
        });
        let outgoing_links = wants("outgoingLinks")
            .then(|| bundle.analyze().out.get(&path).cloned().unwrap_or_default());
        let backlinks = wants("backlinks").then(|| bundle.backlinks(&path));
        let diagnostics = wants("diagnostics").then(|| {
            bundle
                .analyze()
                .per_file
                .get(&path)
                .cloned()
                .unwrap_or_default()
        });
        let external_references = wants("externalReferences").then(Vec::new);

        Ok(ConceptView {
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
}

// ---------------------------------------------------------------------------
// `knowledge_get` — tipos de proyección de servicio y extracción de secciones (E10-H10).
//
// Proyección de servicio (framing), NO dominio: vive en `lodestar-app`, no en `core::types`. No
// hay función equivalente en `prototype/index.html` (la selección por `headingPath` es superficie
// nueva de esta épica, no un port) — implementación propia. Wire en camelCase.
// ---------------------------------------------------------------------------

/// Proyección de un concepto para `knowledge_get`. `path`/`revision` siempre presentes; el resto
/// es `None` cuando no se pidió en `include` (selectividad significativa, no vacua).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConceptView {
    /// Ruta relativa del concepto (su identidad en v2).
    pub path: RelPath,
    /// Identidad de contenido (`blake3:…`, == [`ConceptRevision`] de E10-H03). Siempre presente.
    pub revision: ConceptRevision,
    /// Frontmatter tipado, si se pidió `"frontmatter"` en `include`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub frontmatter: Option<Frontmatter>,
    /// Cuerpo Markdown (completo o acotado por `sections`), si se pidió `"body"` en `include`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// Enlaces salientes resueltos (`Analysis::out`), si se pidió `"outgoingLinks"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outgoing_links: Option<Vec<RelPath>>,
    /// Vecindad de enlaces entrantes (`Bundle::backlinks`), si se pidió `"backlinks"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub backlinks: Option<Backlinks>,
    /// Referencias externas (siempre vacío en esta historia; ver nota de `knowledge_get`), si se
    /// pidió `"externalReferences"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub external_references: Option<Vec<String>>,
    /// Checks de conformidad del concepto (`Analysis::per_file`), si se pidió `"diagnostics"`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub diagnostics: Option<Vec<Check>>,
}

/// Un heading Markdown detectado en un `body`, con el rango de bytes de la sección que abarca:
/// desde el final de su propia línea de heading hasta el siguiente heading de nivel **menor o
/// igual** al suyo (o el final del cuerpo). Ese rango contiene exactamente sus subsecciones
/// anidadas (nivel estrictamente mayor) y nada de sus hermanas ni de secciones de nivel superior —
/// la propiedad que usa [`locate_section`] para no necesitar validar jerarquía explícitamente.
struct Heading<'a> {
    /// Texto del heading, recortado.
    title: &'a str,
    /// Offset de byte donde empieza la línea del heading (para comprobar pertenencia a un rango).
    line_start: usize,
    /// Offset de byte donde empieza el contenido de su sección (justo tras su línea).
    content_start: usize,
    /// Offset de byte donde termina el contenido de su sección (exclusivo).
    content_end: usize,
}

/// Detecta los headings ATX (`#` a `######`) de `body` línea a línea y calcula el rango de
/// contenido de cada uno. **Limitación conocida**: no distingue bloques de código con fences
/// (` ``` `) — una línea que empiece por `#` dentro de un fence se detectaría igualmente como
/// heading. No hay caso de prueba que lo ejercite en esta historia; documentado para quien amplíe
/// esta función.
fn parse_headings(body: &str) -> Vec<Heading<'_>> {
    let mut raw: Vec<(usize, &str, usize, usize)> = Vec::new();
    let mut offset = 0usize;
    for line in body.split_inclusive('\n') {
        let trimmed = line.trim_end_matches('\n').trim_end_matches('\r');
        let hashes = trimmed.chars().take_while(|c| *c == '#').count();
        if (1..=6).contains(&hashes) {
            let rest = &trimmed[hashes..];
            if rest.starts_with(' ') || rest.starts_with('\t') {
                raw.push((hashes, rest.trim(), offset, offset + line.len()));
            }
        }
        offset += line.len();
    }
    let body_len = body.len();
    raw.iter()
        .enumerate()
        .map(|(i, &(level, title, line_start, content_start))| {
            let content_end = raw[i + 1..]
                .iter()
                .find(|&&(l, ..)| l <= level)
                .map(|&(_, _, ls, _)| ls)
                .unwrap_or(body_len);
            Heading {
                title,
                line_start,
                content_start,
                content_end,
            }
        })
        .collect()
}

/// Localiza el rango de bytes de la subsección apuntada por un `headingPath` (p. ej.
/// `["Security","Token rotation"]`): recorre el path segmento a segmento, en cada paso busca el
/// primer heading cuyo título coincida (comparación exacta, recortada) **dentro del rango actual**
/// y estrecha el rango a su sección. Como el rango de una sección solo contiene a sus
/// descendientes (ver [`Heading`]), no hace falta comprobar niveles explícitamente: el segundo
/// segmento del path solo puede casar con un heading anidado bajo el primero. `None` si algún
/// segmento no casa (headingPath inexistente).
fn locate_section(
    headings: &[Heading<'_>],
    body_len: usize,
    path: &[String],
) -> Option<(usize, usize)> {
    let mut range = (0usize, body_len);
    for segment in path {
        let found = headings
            .iter()
            .find(|h| h.line_start >= range.0 && h.line_start < range.1 && h.title == segment)?;
        range = (found.content_start, found.content_end);
    }
    Some(range)
}

/// Extrae y concatena (separadas por una línea en blanco) las subsecciones apuntadas por cada
/// `headingPath` de `sections`, en el orden pedido. Un `headingPath` que no casa con ningún
/// heading se omite silenciosamente (sin `sections` no vacío, el llamante ya filtra este caso).
fn extract_sections(body: &str, sections: &[Vec<String>]) -> String {
    let headings = parse_headings(body);
    sections
        .iter()
        .filter(|path| !path.is_empty())
        .filter_map(|path| locate_section(&headings, body.len(), path))
        .map(|(start, end)| body[start..end].to_string())
        .collect::<Vec<_>>()
        .join("\n\n")
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

/// Filtros de `knowledge_search` (`ARCHITECTURE.md §19.6`). Todos opcionales; un campo ausente no
/// filtra. Wire en camelCase (`pathPrefix`).
///
/// En esta historia se implementan los filtros **baratos y sin ambigüedad** (`types`/`statuses`/
/// `tags`/`pathPrefix`). Los filtros avanzados del contrato (`references`/`referencedBy`/`linkedTo`/
/// `is:*`/`has:*`) quedan **admitidos pero no ejercitados**: como el deserializador ignora las claves
/// desconocidas, un cliente puede enviarlos sin error, pero no alteran el resultado todavía (se
/// añadirán con su criterio en una historia posterior, para no inventar semántica dudosa).
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchFilters {
    /// Restringe a conceptos cuyo `type` (frontmatter) esté en esta lista (comparación
    /// case-insensitive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub types: Option<Vec<String>>,
    /// Restringe a conceptos cuyo `status` esté en esta lista (case-insensitive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub statuses: Option<Vec<String>>,
    /// Restringe a conceptos que tengan al menos uno de estos `tags` (case-insensitive).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    /// Restringe a conceptos cuyo `path` empiece por este prefijo.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path_prefix: Option<String>,
}

/// Un resultado de `knowledge_search` — proyección de un concepto para localizarlo, **nunca su
/// cuerpo completo** (invariante de la historia). Wire en camelCase.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResult {
    /// Ruta relativa del concepto (su identidad en v2, E10-H04).
    pub path: RelPath,
    /// Id estable del concepto, cuando exista (no-goal en v2 → siempre ausente).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// `type` del frontmatter.
    #[serde(rename = "type", default, skip_serializing_if = "Option::is_none")]
    pub r#type: Option<String>,
    /// Título resuelto (`title` del frontmatter o derivado del path).
    pub title: String,
    /// `status` del frontmatter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    /// `description` del frontmatter.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `tags` del frontmatter, normalizados a lista de strings.
    pub tags: Vec<String>,
    /// Extracto compacto NO vacío alrededor del match (o del inicio del cuerpo). **No** es el cuerpo.
    pub snippet: String,
    /// Puntuación de relevancia (mayor = más relevante). Base simple por frecuencia del texto.
    pub score: f64,
    /// Revisión de contenido del concepto (`blake3:…`, == [`ConceptRevision`] de E10-H03).
    pub revision: ConceptRevision,
}

/// Respuesta de `knowledge_search`: la página de resultados, el cursor a la siguiente página (o
/// `None` al agotar) y el total aproximado de coincidencias. Wire en camelCase (`nextCursor`,
/// `totalApproximate`).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SearchResults {
    /// La página actual de resultados (nunca contiene cuerpos).
    pub results: Vec<SearchResult>,
    /// Cursor opaco a la siguiente página, o `None` si no quedan más resultados.
    pub next_cursor: Option<String>,
    /// Número total de conceptos que casan (todas las páginas juntas).
    pub total_approximate: usize,
}

/// `true` si el concepto pasa todos los filtros baratos activos.
fn passes_filters(path: &RelPath, fm: &Frontmatter, filters: &SearchFilters) -> bool {
    if let Some(types) = &filters.types {
        let ty = fm.r#type.as_deref().unwrap_or("").to_lowercase();
        if !types.iter().any(|t| t.to_lowercase() == ty) {
            return false;
        }
    }
    if let Some(statuses) = &filters.statuses {
        let st = fm.status.as_deref().unwrap_or("").to_lowercase();
        if !statuses.iter().any(|s| s.to_lowercase() == st) {
            return false;
        }
    }
    if let Some(want) = &filters.tags {
        let have: BTreeSet<String> = tags_to_vec(&fm.tags)
            .iter()
            .map(|t| t.to_lowercase())
            .collect();
        if !want.iter().any(|t| have.contains(&t.to_lowercase())) {
            return false;
        }
    }
    if let Some(prefix) = &filters.path_prefix {
        if !path.as_str().starts_with(prefix.as_str()) {
            return false;
        }
    }
    true
}

/// Normaliza los `tags` crudos del frontmatter (`serde_yaml::Value`) a una lista de strings.
fn tags_to_vec(tags: &Option<serde_yaml::Value>) -> Vec<String> {
    use serde_yaml::Value as Y;
    match tags {
        Some(Y::Sequence(seq)) => seq.iter().filter_map(scalar_string).collect(),
        Some(Y::String(s)) if !s.is_empty() => vec![s.clone()],
        Some(other) => scalar_string(other).into_iter().collect(),
        None => Vec::new(),
    }
}

/// Representa un escalar YAML como string (para normalizar tags); `None` para no-escalares.
fn scalar_string(v: &serde_yaml::Value) -> Option<String> {
    use serde_yaml::Value as Y;
    match v {
        Y::String(s) => Some(s.clone()),
        Y::Number(n) => Some(n.to_string()),
        Y::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}

/// Puntuación simple: nº de apariciones del texto (minúsculas) en el contenido crudo; `1.0` para un
/// texto vacío (todos los conceptos empatan y el orden lo decide el `path`).
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
