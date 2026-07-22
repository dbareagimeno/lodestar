//! `lodestar-app` — servicios de caso de uso finos sobre `lodestar-workspace`.
//!
//! Capa compartida por las dos fachadas de superficie (`lodestar-mcp`, `lodestar-cli`): compone
//! el `Envelope<T>` de protocolo (framing, no dominio — decisión **D3**, `docs/REFACTOR_DISENO_PROPUESTA.md`)
//! y la fachada `App`, que envuelve un [`lodestar_workspace::Workspace`] y expone los métodos de
//! caso de uso (`workspace_status`, `knowledge_search`, … — se irán poblando en E10-H08+).
//!
//! Este crate depende solo de `lodestar-core` + `lodestar-workspace` + `serde`/`serde_json` — nunca
//! directamente de `rusqlite`/`git2`/`tokio` (invariante #2 de `CLAUDE.md`, verificado por
//! `cargo tree -p lodestar-app`).

use std::path::Path;

use serde::{Deserialize, Serialize};

use lodestar_core::types::{Check, WorkspaceRevision};
use lodestar_workspace::{Workspace, WorkspaceError};

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

/// Fachada fina de servicios de caso de uso sobre un [`Workspace`] abierto.
///
/// `App` es lo que consumen `lodestar-mcp` y `lodestar-cli`: un punto de entrada único que
/// traduce peticiones de protocolo a operaciones del `Workspace` y envuelve las respuestas en
/// [`Envelope`]. De momento solo abre el workspace subyacente — los métodos de caso de uso
/// (`workspace_status`, `knowledge_search`, `knowledge_get`, `schema_inspect`, `knowledge_check`,
/// …) se irán añadiendo en E10-H08 y siguientes.
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

    /// El `Workspace` subyacente, para los servicios que se implementen sobre `App`.
    pub fn workspace(&self) -> &Workspace {
        &self.workspace
    }

    // Los métodos de caso de uso (`workspace_status`, `knowledge_search`, `knowledge_get`,
    // `schema_inspect`, `knowledge_check`, …) llegan en E10-H08+.
}
