//! `WorkspaceSnapshot`: el snapshot unificado que empujan las fachadas (`§6`, `§8`).

use lodestar_core::types::{Analysis, FileMap, GraphModel};
use serde::Serialize;

/// Files + analysis + graph, todo junto. Es lo que empujaba la fachada Tauri como evento
/// `bundle:changed` (nombre de la UI retirada a `experimental/ui-desktop`, no de esta API).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkspaceSnapshot {
    pub files: FileMap,
    pub analysis: Analysis,
    pub graph: GraphModel,
}
