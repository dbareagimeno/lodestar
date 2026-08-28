//! API estable de configuración del workspace.
//!
//! El contrato y el loader viven en `lodestar-discovery`, el owner inferior compartido por
//! `Workspace` y `Store`. Esta reexportación conserva los tipos públicos históricos sin mantener
//! un segundo schema ni una segunda ruta de validación.

pub use lodestar_discovery::config::*;

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
                policy
                    .exclude
                    .iter()
                    .any(|g| g == lodestar_discovery::CONTROL_PLANE_EXCLUDE),
                "el suelo duro debe estar en la política efectiva de «{yaml}»: {:?}",
                policy.exclude
            );
            // …y sin duplicarlo cuando ya viene de los defaults.
            assert_eq!(
                policy
                    .exclude
                    .iter()
                    .filter(|g| *g == lodestar_discovery::CONTROL_PLANE_EXCLUDE)
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
