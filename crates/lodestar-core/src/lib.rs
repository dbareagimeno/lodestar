//! `lodestar-core` — núcleo **puro** de lodestar.
//!
//! Contiene toda la semántica OKF (modelo, conformidad, links, query, grafo, plan de cambios y
//! diff semántico) sin I/O, sin DB, sin git y sin runtime. Es testeable sin DB/runtime y es la
//! **única verdad computada** que comparten las fachadas (CLI, MCP). Ver `ARCHITECTURE.md §2`,
//! `§4`.
//!
//! Invariantes (no negociables):
//! - Los `.md` en disco son la única fuente de verdad; este crate solo computa.
//! - El contrato de tipos (`Check`/`Severity`/`CheckCode`/`Analysis`/…) se define **una vez**
//!   en [`types`]; las fachadas hacen `use` de él, sin capa DTO paralela.
//! - [`types::RelPath`] es un newtype validado: único chokepoint de path-traversal.

#![forbid(unsafe_code)]

pub mod diff;
pub mod error;
pub mod eval;
pub mod filter;
pub mod links;
pub mod metadata;
pub mod model;
pub mod parse;
pub mod plan;
pub mod text;
pub mod types;

mod conform;
mod document_set;
mod graph;
mod store_trait;

pub use document_set::DocumentSet;
pub use error::CoreError;
pub use store_trait::DocumentStore;
pub use types::*;

#[cfg(feature = "render")]
pub mod render;
