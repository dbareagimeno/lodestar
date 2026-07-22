//! Configuración **por-bundle**: `<root>/lodestar.toml` (legado, `ARCHITECTURE.md §12`, `§10`) y
//! `<root>/.lodestar/config.yaml` (nueva, `ARCHITECTURE.md §19.4`, `DECISIONES.md §0` D4/D5).
//!
//! Ambas son aditivas y con defaults seguros: un bundle sin fichero de config se comporta como
//! hasta ahora (solo `Err` bloquea; identidad por defecto; todo el bundle escribible). Los
//! ficheros se versionan con el bundle (no son cache).

use std::path::Path;

use lodestar_core::types::{Analysis, Author, RelPath};
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
#[derive(Debug, Clone, Deserialize, PartialEq)]
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

// ---------------------------------------------------------------------------
// `WorkspaceConfig` — `.lodestar/config.yaml` (E9-H05, ARCHITECTURE.md §19.4, DECISIONES.md §0 D4/D5)
// ---------------------------------------------------------------------------

/// Ruta del fichero de configuración nuevo, relativa al root del bundle.
pub const WORKSPACE_CONFIG_FILE: &str = ".lodestar/config.yaml";

/// Configuración efectiva de un bundle en el formato nuevo (`.lodestar/config.yaml`, YAML).
///
/// Reemplaza a `Config`/`lodestar.toml` como destino de migración (D4); convive con él mientras
/// dure la transición (`Config` sigue siendo lo que consume `Workspace::open`/`lodestar-cli`). El
/// mapeo YAML usa claves `camelCase` (`writableRoots`, `blockWarnings`, …) que se deserializan a
/// los campos `snake_case` de estas structs.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceConfig {
    /// Raíces de escritura/lectura del bundle.
    pub workspace: WorkspaceSection,
    /// Puerta de conformidad (mismo rol que `GateConfig`, formato nuevo).
    pub gate: GateSection,
    /// Retención del histórico de recibos transaccionales (E13; solo config aquí, sin mecánica).
    pub transactions: TransactionsSection,
    /// Identidad de commits — sección **dormida**: git queda fuera de la superficie headless
    /// (`ARCHITECTURE.md §19.1`); se conserva por si el vcs vuelve a exponerse, pero
    /// `WorkspaceConfig` no la usa hoy (a diferencia de `Config::author`).
    pub identity: Option<IdentityConfig>,
}

/// Raíces de escritura/lectura del bundle (`ARCHITECTURE.md §19.4`).
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceSection {
    /// Raíces donde Lodestar puede escribir (validado en E11-H04; aquí solo se carga el dato).
    ///
    /// **`Vec` vacío significa "todo el bundle es escribible"** (sin restricción) — no es una
    /// lista de cero raíces permitidas. No existe un valor centinela para "la raíz del bundle"
    /// porque `RelPath::new(".")` es inválido (`.` se normaliza a "sin componentes" y `RelPath`
    /// rechaza la cadena vacía resultante); representar "todo" como ausencia de restricción evita
    /// ese valor imposible.
    pub writable_roots: Vec<RelPath>,
    /// Raíces visibles para validación pero **nunca** escribibles por Lodestar (p. ej. `src`,
    /// `tests` de un repo de código adoptado). Vacío por defecto. Uso diferido a E11-H04.
    pub reference_roots: Vec<RelPath>,
    /// Rutas (relativas al root, no necesariamente `RelPath` válidos si describen directorios
    /// arbitrarios de un repo adoptado) que el walker ignora. `#[serde(default)]` **reemplaza**
    /// la lista entera cuando el YAML trae `ignored` propio (no hace merge), así que el
    /// deserializado en crudo puede no traer los obligatorios. [`WorkspaceConfig::load`] los
    /// inyecta siempre tras deserializar (merge + dedupe) — el campo `ignored` que ve cualquier
    /// consumidor de `WorkspaceConfig` (tras `load`) SIEMPRE incluye `.lodestar/runtime` y
    /// `.git`, se hayan especificado o no en el YAML.
    pub ignored: Vec<String>,
}

impl Default for WorkspaceSection {
    fn default() -> Self {
        WorkspaceSection {
            writable_roots: Vec::new(),
            reference_roots: Vec::new(),
            ignored: default_ignored(),
        }
    }
}

fn default_ignored() -> Vec<String> {
    vec![".lodestar/runtime".to_string(), ".git".to_string()]
}

/// Puerta de conformidad (formato nuevo; mismo rol que `GateConfig`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct GateSection {
    /// Si `true`, los avisos (`Warn`) también hacen fallar la puerta (además de `Err`).
    pub block_warnings: bool,
}

/// Retención del histórico de recibos transaccionales (mecánica en E13; aquí solo el dato de
/// config). Tipos deliberadamente simples (`String`/`usize`): la unidad de `retain_receipts_for`
/// (p. ej. `"24h"`) la interpreta quien implemente la retención, no este loader.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct TransactionsSection {
    /// Durante cuánto tiempo se retiene un recibo antes de purgarlo (p. ej. `"24h"`).
    pub retain_receipts_for: String,
    /// Número máximo de recibos retenidos simultáneamente.
    pub maximum_receipts: usize,
}

impl Default for TransactionsSection {
    fn default() -> Self {
        TransactionsSection {
            retain_receipts_for: "24h".to_string(),
            maximum_receipts: 20,
        }
    }
}

impl WorkspaceConfig {
    /// Carga `<root>/.lodestar/config.yaml` si existe; si no, devuelve los defaults seguros
    /// (mismo patrón que `Config::load`: la ausencia de fichero no es un error). YAML malformado,
    /// o un `writableRoots`/`referenceRoots` con un componente inválido (p. ej. `..`, rechazado
    /// por `RelPath`), sí es un error explícito — nunca se silencia a defaults.
    ///
    /// Tras deserializar, inyecta siempre los obligatorios (`.lodestar/runtime`, `.git`) en
    /// `workspace.ignored` (merge + dedupe): `#[serde(default)]` reemplaza la lista entera cuando
    /// el YAML trae la suya, así que sin esta inyección un `ignored` explícito del usuario se
    /// comería los obligatorios.
    pub fn load(root: &Path) -> Result<WorkspaceConfig, String> {
        let path = root.join(WORKSPACE_CONFIG_FILE);
        let mut cfg = match std::fs::read_to_string(&path) {
            Ok(text) => serde_yaml::from_str::<WorkspaceConfig>(&text)
                .map_err(|e| format!("{WORKSPACE_CONFIG_FILE} inválido: {e}"))?,
            Err(_) => WorkspaceConfig::default(),
        };
        for obligatorio in default_ignored() {
            if !cfg.workspace.ignored.contains(&obligatorio) {
                cfg.workspace.ignored.push(obligatorio);
            }
        }
        Ok(cfg)
    }
}
