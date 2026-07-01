//! Configuración **por-bundle** en `<root>/lodestar.toml` (`ARCHITECTURE.md §12`, `§10`).
//!
//! Es aditiva y con defaults seguros: un bundle sin `lodestar.toml` se comporta como hasta ahora
//! (solo `Err` bloquea; identidad por defecto). El fichero se versiona con el bundle (no es cache).

use std::path::Path;

use lodestar_core::types::{Analysis, Author};
use serde::Deserialize;

/// Nombre del fichero de configuración por-bundle.
pub const CONFIG_FILE: &str = "lodestar.toml";

/// Configuración efectiva de un bundle (con defaults aplicados).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Puerta de conformidad (qué severidades bloquean).
    pub gate: GateConfig,
    /// Identidad para autor/committer de los commits (override del defecto).
    pub identity: Option<IdentityConfig>,
}

/// Puerta de conformidad. Por defecto solo `Err` bloquea (`§4.1`): `block_warnings = false`.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct GateConfig {
    /// Si `true`, los avisos (`Warn`) también hacen fallar la puerta (además de `Err`).
    pub block_warnings: bool,
}

/// Identidad de commits configurada.
#[derive(Debug, Clone, Deserialize)]
pub struct IdentityConfig {
    pub name: String,
    pub email: String,
}

impl Config {
    /// Carga `<root>/lodestar.toml` si existe; si no, devuelve los defaults. TOML inválido → error.
    pub fn load(root: &Path) -> Result<Config, String> {
        let path = root.join(CONFIG_FILE);
        match std::fs::read_to_string(&path) {
            Ok(text) => toml::from_str(&text).map_err(|e| format!("lodestar.toml inválido: {e}")),
            Err(_) => Ok(Config::default()),
        }
    }

    /// `true` si la puerta debe fallar para este análisis según la strictness configurada.
    pub fn gate_blocked(&self, a: &Analysis) -> bool {
        a.hard_fail > 0 || (self.gate.block_warnings && a.warn_count > 0)
    }

    /// La identidad configurada como `Author`, si la hay.
    pub fn author(&self) -> Option<Author> {
        self.identity.as_ref().map(|i| Author {
            name: i.name.clone(),
            email: i.email.clone(),
        })
    }
}
