//! `BundleSnapshot`: el snapshot unificado que empujan las fachadas (`§6`, `§8`).

use lodestar_core::types::{Analysis, FileMap, GraphModel};
use serde::Serialize;

/// Files + analysis + graph, todo junto. Es lo que la fachada Tauri empuja como `bundle:changed`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleSnapshot {
    pub files: FileMap,
    pub analysis: Analysis,
    pub graph: GraphModel,
}
