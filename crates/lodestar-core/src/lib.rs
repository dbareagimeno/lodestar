//! `lodestar-core` — núcleo **puro** de lodestar.
//!
//! Contiene toda la semántica OKF (modelo, conformidad, links, query, grafo, generación,
//! export y diff semántico) sin I/O, sin DB, sin git y sin runtime. Es testeable sin
//! GUI/DB/runtime y es la **única verdad computada** que comparten las tres fachadas
//! (Tauri, CLI, MCP). Ver `ARCHITECTURE.md §2`, `§4`.
//!
//! Invariantes (no negociables):
//! - Los `.md` en disco son la única fuente de verdad; este crate solo computa.
//! - El contrato de tipos (`Check`/`Severity`/`CheckCode`/`Analysis`/…) se define **una vez**
//!   en [`types`]; las fachadas hacen `use` de él, sin capa DTO paralela.
//! - [`types::RelPath`] es un newtype validado: único chokepoint de path-traversal.

#![forbid(unsafe_code)]

pub mod diff;
pub mod error;
pub mod generate;
pub mod model;
pub mod query;
pub mod types;

mod bundle;
mod conform;
mod graph;
mod store_trait;

pub use bundle::Bundle;
pub use error::CoreError;
pub use store_trait::ConceptStore;
pub use types::*;

#[cfg(feature = "render")]
pub mod render;
