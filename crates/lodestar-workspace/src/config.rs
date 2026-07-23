//! Configuración **por-workspace**: `<root>/.lodestar/config.yaml` (`ARCHITECTURE.md §20.5`, `§20.9`;
//! `DECISIONES.md §0` D4/D5).
//!
//! Desde E15-H08 es el **único** fichero de configuración del motor: el `lodestar.toml` legado
//! (`Config`/`GateConfig`) se borró —dos ficheros de config para lo mismo era deuda, y su otro
//! habitante (`identity`) murió en E15-H01—, de modo que un `lodestar.toml` en la raíz es hoy un
//! fichero más del proyecto: ni se lee, ni su sintaxis importa (cierra `DECISIONES.md §8`).
//!
//! La regla que gobierna todo lo que hay aquí es **la config LIMITA, nunca habilita**
//! (`ARCHITECTURE.md §20.1`): su ausencia no impide usar Lodestar (defaults seguros = los de
//! `§20.5`), lo que declara solo puede restringir, y un YAML malformado es un **error explícito**
//! —nunca una caída silenciosa a defaults, que relajaría las restricciones del usuario sin avisar.

use std::collections::BTreeMap;
use std::path::Path;

use lodestar_core::types::{Analysis, RelPath};
use serde::Deserialize;

use crate::discovery::{DiscoveryPolicy, CONTROL_PLANE_EXCLUDE};

/// Ruta del fichero de configuración, relativa al root del workspace.
pub const WORKSPACE_CONFIG_FILE: &str = ".lodestar/config.yaml";

/// Configuración efectiva de un workspace (`.lodestar/config.yaml`, YAML).
///
/// El mapeo YAML usa claves `camelCase` (`writableRoots`, `respectGitignore`, `blockWarnings`, …)
/// que se deserializan a los campos `snake_case` de estas structs. Todas las secciones son
/// opcionales y traen defaults seguros.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceConfig {
    /// Raíces de escritura/lectura del workspace (la *write policy* de `§20.1`).
    pub workspace: WorkspaceSection,
    /// Política de descubrimiento (`§20.5`): qué documentos forman el inventario.
    pub discovery: DiscoverySection,
    /// Política de validación (`§20.9`): severidad por familia de diagnóstico. **Solo se carga**;
    /// aplicarla es E20.
    pub validation: ValidationSection,
    /// Puerta de conformidad (strictness de `lodestar check`).
    pub gate: GateSection,
    /// Política transaccional y retención del histórico de recibos (E13; la política de cambios de
    /// `§20.9` **solo se carga** aquí, su mecánica es E20).
    pub transactions: TransactionsSection,
}

/// Raíces de escritura/lectura del workspace (`ARCHITECTURE.md §20.1`).
///
/// > **`workspace.root` NO se implementa** (E15-H08, `§20.5`). `REFACTOR_PHASE_2 §Fase 2` lo
/// > sugería como configuración opcional, pero es **circular**: este fichero vive en
/// > `<root>/.lodestar/config.yaml`, luego hay que conocer ya la raíz para poder leerlo. La raíz
/// > sale **exclusivamente** de `--root` (o `--path`) o del cwd, y es fija durante toda la sesión.
/// > La clave se ignora si aparece en el YAML: no redirige nada.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct WorkspaceSection {
    /// Raíces donde Lodestar puede escribir (validado en E11-H04; aquí solo se carga el dato).
    ///
    /// **`Vec` vacío significa "todo el workspace es escribible"** (sin restricción) — no es una
    /// lista de cero raíces permitidas. No existe un valor centinela para "la raíz del workspace"
    /// porque `RelPath::new(".")` es inválido (`.` se normaliza a "sin componentes" y `RelPath`
    /// rechaza la cadena vacía resultante); representar "todo" como ausencia de restricción evita
    /// ese valor imposible.
    pub writable_roots: Vec<RelPath>,
    /// Raíces visibles para validación pero **nunca** escribibles por Lodestar (p. ej. `src`,
    /// `tests` de un repo de código adoptado). Vacío por defecto. Se retira en E20 con las refs
    /// externas por frontmatter.
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

/// Sección `discovery` (`ARCHITECTURE.md §20.5`): la política de descubrimiento declarada por el
/// usuario, antes de aplicarle el **suelo duro**.
///
/// Sus defaults son, campo a campo, los de [`DiscoveryPolicy::default`] —se derivan de ella, no se
/// reescriben— para que escribir la política por defecto documentada en `§20.5` dentro del
/// `config.yaml` dé exactamente el mismo comportamiento que no escribir nada. Si divergieran,
/// declarar los valores «de fábrica» cambiaría el descubrimiento: una config que *habilita* en vez
/// de limitar.
///
/// La política **efectiva** se obtiene con [`DiscoverySection::policy`], que es donde se inyecta el
/// suelo duro `.lodestar/**`.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DiscoverySection {
    /// Globs de lo que **entra** en el inventario (por defecto `**/*.md`).
    pub include: Vec<String>,
    /// Globs de lo que queda **fuera**, con prioridad sobre `include`.
    ///
    /// Lo que el usuario escriba aquí **reemplaza** la lista por defecto (no hace merge), con una
    /// única excepción innegociable: `.lodestar/**` (ver [`DiscoverySection::policy`]).
    pub exclude: Vec<String>,
    /// Aplicar los `.gitignore` del árbol (por defecto `true`).
    pub respect_gitignore: bool,
    /// Aplicar los `.lodestarignore` del árbol (por defecto `true`).
    pub respect_lodestar_ignore: bool,
    /// Seguir symlinks (por defecto `false`: se reportan con `SYMLINK-UNSUPPORTED`).
    pub follow_symlinks: bool,
    /// Tamaño máximo por documento en bytes; por encima se reporta `DOC-TOO-LARGE`.
    pub max_document_bytes: usize,
}

impl Default for DiscoverySection {
    fn default() -> Self {
        // Derivada de la política del motor: una sola fuente de verdad para los defaults de `§20.5`.
        let p = DiscoveryPolicy::default();
        DiscoverySection {
            include: p.include,
            exclude: p.exclude,
            respect_gitignore: p.respect_gitignore,
            respect_lodestar_ignore: p.respect_lodestar_ignore,
            follow_symlinks: p.follow_symlinks,
            max_document_bytes: p.max_document_bytes,
        }
    }
}

impl DiscoverySection {
    /// La [`DiscoveryPolicy`] **efectiva**: lo declarado por el usuario con el **suelo duro**
    /// [`CONTROL_PLANE_EXCLUDE`] (`.lodestar/**`) inyectado siempre.
    ///
    /// El suelo duro vive aquí —en la construcción de la política, no en el default de la
    /// sección— porque un default es sobreescribible por definición: un usuario que escriba
    /// `exclude: []`, o que liste sus propias exclusiones sin repetir las de fábrica (lo natural),
    /// se llevaría por delante la exclusión que sostiene un invariante del motor. Inyectándolo al
    /// construir la política, **toda** vía de obtención (config deserializada, `default()`,
    /// construida a mano) la lleva.
    ///
    /// El invariante que protege (`§20.5`, corrección E15-H07): *todo documento del inventario
    /// tiene que contar para la [`lodestar_core::types::workspace_revision`]*. Un `.md` bajo
    /// `.lodestar/` sería nodo del grafo, analizable y escribible, pero **ciego al control
    /// optimista** —la revisión excluye `.lodestar/` por decisión **D5** y no puede dejar de
    /// hacerlo: `StagingDir` materializa ahí copias `.md` de los documentos cuya escritura está
    /// guardando, así que si contaran, `reverify_base_revision` fallaría *a causa del apply en
    /// curso*. `.lodestar/` es el plano de control de Lodestar (config, cache, runtime), nunca
    /// conocimiento del usuario.
    ///
    /// La config puede, por tanto, **añadir** exclusiones; nunca quitar esa.
    pub fn policy(&self) -> DiscoveryPolicy {
        let mut exclude = self.exclude.clone();
        if !exclude.iter().any(|g| g == CONTROL_PLANE_EXCLUDE) {
            exclude.push(CONTROL_PLANE_EXCLUDE.to_string());
        }
        DiscoveryPolicy {
            include: self.include.clone(),
            exclude,
            respect_gitignore: self.respect_gitignore,
            respect_lodestar_ignore: self.respect_lodestar_ignore,
            follow_symlinks: self.follow_symlinks,
            max_document_bytes: self.max_document_bytes,
        }
    }
}

/// Sección `validation` (`ARCHITECTURE.md §20.9`): severidad por **familia de diagnóstico**
/// (`malformedFrontmatter: error`, `isolatedDocuments: ignore`, …).
///
/// Es un mapa abierto a propósito: las familias no son una lista cerrada en esta historia
/// —aplicar la política es E20—, así que aquí solo se **carga sin perder datos**, conservando
/// literalmente las claves del YAML. Lo único que se valida es la severidad, cuyo catálogo sí es
/// cerrado ([`ValidationSeverity`]): un `warn` mal escrito es exactamente el typo que la regla
/// «una config rota es un error, no un default» quiere cazar.
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(transparent)]
pub struct ValidationSection {
    /// Familia de diagnóstico (tal cual aparece en el YAML) → severidad configurada.
    pub families: BTreeMap<String, ValidationSeverity>,
}

/// Severidad configurable de una familia de diagnóstico (`§20.9`). **Solo dato**: quien la aplique
/// es E20.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ValidationSeverity {
    /// El diagnóstico es un error.
    Error,
    /// El diagnóstico es un aviso.
    Warning,
    /// El diagnóstico no se reporta.
    Ignore,
}

/// Puerta de conformidad: strictness de `lodestar check` (`ARCHITECTURE.md §7.3`).
#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct GateSection {
    /// Si `true`, los avisos (`Warn`) también hacen fallar la puerta (además de los errores).
    pub block_warnings: bool,
}

/// Política transaccional (`§20.9`) y retención del histórico de recibos (mecánica de la retención
/// en E13; la de `rejectNewErrors`/`allowExistingErrors`, en E20 — aquí solo el dato de config).
///
/// Tipos deliberadamente simples (`String`/`usize`): la unidad de `retain_receipts_for` (p. ej.
/// `"24h"`) la interpreta quien implemente la retención, no este loader.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct TransactionsSection {
    /// Durante cuánto tiempo se retiene un recibo antes de purgarlo (p. ej. `"24h"`).
    pub retain_receipts_for: String,
    /// Número máximo de recibos retenidos simultáneamente.
    pub maximum_receipts: usize,
    /// Un cambio no puede introducir errores nuevos ni empeorar los existentes (por defecto
    /// `true`). **Solo se carga**: la mecánica es E20.
    pub reject_new_errors: bool,
    /// Lodestar puede trabajar en un repositorio que ya tiene problemas, y una reparación parcial
    /// se puede aplicar (por defecto `true`). **Solo se carga**: la mecánica es E20.
    pub allow_existing_errors: bool,
}

impl Default for TransactionsSection {
    fn default() -> Self {
        TransactionsSection {
            retain_receipts_for: "24h".to_string(),
            maximum_receipts: 20,
            reject_new_errors: true,
            allow_existing_errors: true,
        }
    }
}

impl WorkspaceConfig {
    /// Carga `<root>/.lodestar/config.yaml` si existe; si no, devuelve los defaults seguros (la
    /// ausencia de fichero **no** es un error: `§20.1`, arranque sin ceremonia). YAML malformado,
    /// o un `writableRoots`/`referenceRoots` con un componente inválido (p. ej. `..`, rechazado
    /// por `RelPath`), sí es un error explícito — nunca se silencia a defaults.
    ///
    /// Tras deserializar, inyecta siempre los obligatorios (`.lodestar/runtime`, `.git`) en
    /// `workspace.ignored` (merge + dedupe): `#[serde(default)]` reemplaza la lista entera cuando
    /// el YAML trae la suya, así que sin esta inyección un `ignored` explícito del usuario se
    /// comería los obligatorios. El suelo duro del **descubrimiento** no se inyecta aquí sino en
    /// [`DiscoverySection::policy`], para que lo lleve toda vía de construcción de la política.
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

    /// `true` si la puerta de conformidad debe fallar para este análisis según la strictness
    /// configurada (`gate.blockWarnings`).
    ///
    /// Es lo que consume `lodestar check` sobre el veredicto del motor: la config solo puede
    /// **endurecer** la puerta (que los avisos también bloqueen), nunca relajarla.
    pub fn gate_blocked(&self, a: &Analysis) -> bool {
        a.hard_fail > 0 || (self.gate.block_warnings && a.warn_count > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Las secciones que esta historia **solo carga** (`validation`, la política de cambios de
    /// `transactions`) se deserializan sin perder datos, con sus claves camelCase — y
    /// `workspace.root` se ignora sin tumbar el parseo (es circular: `§20.5`).
    #[test]
    fn secciones_solo_de_carga_se_deserializan_sin_perder_datos() {
        let yaml = "\
workspace:
  root: /otro/sitio
  writableRoots: [knowledge]
validation:
  malformedFrontmatter: error
  isolatedDocuments: ignore
  caseMismatch: warning
transactions:
  rejectNewErrors: false
  allowExistingErrors: true
";
        let cfg: WorkspaceConfig = serde_yaml::from_str(yaml).expect("YAML válido");

        // `workspace.root` no redirige nada: se ignora y el resto de la sección se carga igual.
        assert_eq!(cfg.workspace.writable_roots.len(), 1);

        assert_eq!(
            cfg.validation.families.get("malformedFrontmatter"),
            Some(&ValidationSeverity::Error)
        );
        assert_eq!(
            cfg.validation.families.get("isolatedDocuments"),
            Some(&ValidationSeverity::Ignore)
        );
        assert_eq!(
            cfg.validation.families.get("caseMismatch"),
            Some(&ValidationSeverity::Warning)
        );

        assert!(!cfg.transactions.reject_new_errors);
        assert!(cfg.transactions.allow_existing_errors);
        // Lo no declarado conserva su default (la sección no se reemplaza entera).
        assert_eq!(cfg.transactions.maximum_receipts, 20);
        assert_eq!(cfg.transactions.retain_receipts_for, "24h");
    }

    /// El suelo duro no depende de que el usuario lo declare, ni de qué más excluya.
    #[test]
    fn el_suelo_duro_sobrevive_a_cualquier_exclude() {
        for yaml in [
            "discovery:\n  exclude: []\n",
            "discovery:\n  exclude: [\"notas/**\"]\n",
            "discovery: {}\n",
            "{}\n",
        ] {
            let cfg: WorkspaceConfig = serde_yaml::from_str(yaml).expect("YAML válido");
            let policy = cfg.discovery.policy();
            assert!(
                policy.exclude.iter().any(|g| g == CONTROL_PLANE_EXCLUDE),
                "el suelo duro debe estar en la política efectiva de «{yaml}»: {:?}",
                policy.exclude
            );
            // …y sin duplicarlo cuando ya viene de los defaults.
            assert_eq!(
                policy
                    .exclude
                    .iter()
                    .filter(|g| *g == CONTROL_PLANE_EXCLUDE)
                    .count(),
                1
            );
        }
    }

    /// Una severidad fuera del catálogo de `§20.9` es un error de config, no un default silencioso.
    #[test]
    fn severidad_desconocida_es_error() {
        let res: Result<WorkspaceConfig, _> =
            serde_yaml::from_str("validation:\n  malformedFrontmatter: catastrofe\n");
        assert!(res.is_err(), "«catastrofe» no es una severidad válida");
    }
}
