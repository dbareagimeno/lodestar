//! `core::schema` — el catálogo de tipos de un bundle (`DocType`, campos, relaciones tipadas,
//! plantillas), **puro** (`ARCHITECTURE.md §19.2`, `docs/REFACTOR.md §4/§9.4`).
//!
//! Este módulo solo modela y deserializa: **nunca abre ficheros**. La lectura de
//! `<root>/.lodestar/schema.yaml` es I/O de `lodestar-workspace` (patrón `Config::load`,
//! `crates/lodestar-workspace/src/config.rs`), que deserializa el texto a [`Schema`] y se lo
//! entrega ya construido a quien lo consuma. El wire YAML usa claves `camelCase`
//! (`requiredFields`/`allowedStatuses`/`bodyTemplate`/`targetTypes`) mapeadas a los campos
//! `snake_case` de estos tipos — mismo convenio que `WorkspaceConfig`.
//!
//! Un bundle **sin** `schema.yaml` se modela como [`Schema::default()`]: `types` vacío, lo que
//! deja el bundle sin restricciones adicionales (compat con bundles OKF actuales que no declaran
//! esquema). La validación schema-driven (`SCHEMA-REQFIELD`/`SCHEMA-STATUS`) vive en
//! [`validate_schema`] (E10-H07): función **pura y separada** de `analyze`/`validate_file` — no
//! se integra ahí (aditiva por composición del llamante, no por acoplamiento del core).

use std::collections::{BTreeMap, BTreeSet};

use crate::model;
use crate::types::{Check, CheckCode, Fix, ParsedFrontmatter, Range, RelPath, Severity};
use crate::Bundle;

use serde::{Deserialize, Serialize};

/// Catálogo de esquemas de un bundle: versión + `DocType`s indexados por nombre de tipo.
///
/// `Schema::default()` es el esquema **vacío y permisivo** (sin `DocType`s declarados): el que
/// se usa cuando un bundle no tiene `.lodestar/schema.yaml`.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Schema {
    /// Versión del formato de esquema (independiente de `okf_version` del frontmatter).
    #[serde(default = "default_schema_version")]
    pub version: String,
    /// `DocType`s declarados, indexados por su nombre de tipo (la clave del mapa YAML).
    pub types: BTreeMap<String, DocType>,
}

impl Default for Schema {
    fn default() -> Self {
        Schema {
            version: default_schema_version(),
            types: BTreeMap::new(),
        }
    }
}

fn default_schema_version() -> String {
    "1".to_string()
}

/// Definición de un tipo de documento (`docs/REFACTOR.md §9.4`, salida de `schema_inspect`).
///
/// Todos los campos llevan `#[serde(default)]`: un YAML parcial (solo `requiredFields`, por
/// ejemplo) deserializa sin error, con el resto en su valor por defecto.
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct DocType {
    /// Nombre del tipo. Puede omitirse en el YAML (la clave de `Schema::types` ya lo da); si el
    /// wire lo trae explícito, debe coincidir con la clave — el loader de `workspace` no lo
    /// fuerza aquí (el core no valida contra sí mismo), pero sí lo rellena cuando falta
    /// (`WorkspaceSchema::load`).
    pub name: String,
    /// Descripción humana del tipo (para `schema_inspect`, UI, etc.).
    pub description: String,
    /// Campos de frontmatter obligatorios para este tipo (`SCHEMA-REQFIELD` en E10-H07).
    pub required_fields: Vec<String>,
    /// Valores permitidos para `status` de este tipo (`SCHEMA-STATUS` en E10-H07). Vacío =
    /// cualquier `status` es válido (sin restricción).
    pub allowed_statuses: Vec<String>,
    /// Campos adicionales declarados por el tipo, indexados por nombre (forma simple: solo la
    /// descripción del campo; la validación de tipo/forma queda fuera de alcance de esta
    /// historia).
    pub fields: BTreeMap<String, FieldDef>,
    /// Relaciones tipadas declaradas por el tipo, indexadas por el nombre del campo de relación
    /// (p. ej. `implemented_by`). Validadas en E11-H03 (`REL-TARGET`/`REL-CARD`/`REL-TYPE`).
    pub relations: BTreeMap<String, RelationDef>,
    /// Reglas adicionales en lenguaje libre (documentales; sin mecánica de validación asociada
    /// todavía).
    pub rules: Vec<String>,
    /// Plantilla de cuerpo para `create_concept` de este tipo (aplicación en E12-H05).
    pub body_template: Option<String>,
}

/// Definición simple de un campo adicional de un [`DocType`].
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct FieldDef {
    /// Descripción humana del campo.
    pub description: String,
}

/// Definición de una relación tipada (`docs/REFACTOR.md §9.4`; validación en E11-H03).
#[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RelationDef {
    /// Tipos de `DocType` que puede referenciar esta relación. Vacío = cualquier tipo destino.
    pub target_types: Vec<String>,
    /// Cardinalidad de la relación (`"one"`/`"many"`), en forma simple de `String` — el enum
    /// cerrado se decide junto con su validación (E11-H03), no aquí.
    #[serde(default = "default_cardinality")]
    pub cardinality: String,
}

impl Default for RelationDef {
    fn default() -> Self {
        RelationDef {
            target_types: Vec::new(),
            cardinality: default_cardinality(),
        }
    }
}

fn default_cardinality() -> String {
    "many".to_string()
}

/// Valida los conceptos de `bundle` contra el catálogo `schema` (E10-H07, `ARCHITECTURE.md
/// §19.2/§19.3`): para cada concepto cuyo `type` está declarado en `schema.types`, comprueba que
/// estén presentes sus `required_fields` (ausente → [`CheckCode::SchemaReqfield`]) y que
/// `status`, si no está vacío, esté en `allowed_statuses` cuando este último no está vacío
/// (fuera → [`CheckCode::SchemaStatus`]). Ambos con severidad [`Severity::Err`].
///
/// Función **pura y separada** de `Bundle::analyze`/`conform::validate_file`: no se llama desde
/// ninguna de las dos, así que un bundle sin `schema.yaml` (`Schema::default()`, `types` vacío)
/// no cambia su veredicto de conformidad actual (aditiva por composición del llamante). Un
/// concepto cuyo `type` no está declarado en el schema se ignora (el catálogo es permisivo, no
/// exhaustivo) — mismo criterio que `sin_schema_permisivo` (E10-H05).
pub fn validate_schema(bundle: &Bundle, schema: &Schema) -> Vec<Check> {
    if schema.types.is_empty() {
        return Vec::new();
    }

    let mut out = Vec::new();
    for path in &bundle.analyze().concepts {
        let Some(raw) = bundle.files().get(path) else {
            continue;
        };
        let parsed = model::parse_file(path.as_str(), raw);
        let Some(fm) = parsed.frontmatter else {
            continue;
        };
        let Some(tipo) = fm.get_text("type") else {
            continue;
        };
        let Some(doctype) = schema.types.get(&tipo) else {
            continue;
        };

        for campo in &doctype.required_fields {
            if !field_present(&fm, campo) {
                out.push(Check::new(
                    Severity::Err,
                    CheckCode::SchemaReqfield,
                    format!("Falta el campo obligatorio «{campo}» para el tipo «{tipo}»."),
                    vec![path.clone()],
                ));
            }
        }

        if let Some(status) = fm.get_text("status") {
            if !status.is_empty()
                && !doctype.allowed_statuses.is_empty()
                && !doctype.allowed_statuses.iter().any(|s| s == &status)
            {
                out.push(Check::new(
                    Severity::Err,
                    CheckCode::SchemaStatus,
                    format!("El estado «{status}» no está permitido para el tipo «{tipo}»."),
                    vec![path.clone()],
                ));
            }
        }
    }
    out
}

/// Valida las relaciones tipadas del frontmatter contra su [`RelationDef`] (E11-H03,
/// `ARCHITECTURE.md §19.2/§19.3`): para cada concepto cuyo `type` está declarado en
/// `schema.types`, y por cada relación declarada en `doctype.relations`, lee el campo
/// homónimo del frontmatter (una secuencia YAML de paths target, o un único `String`) y
/// comprueba:
///
/// 1. cada target existe como concepto del bundle → si no, [`CheckCode::RelTarget`];
/// 2. el `type` del target, si el target existe, está en `RelationDef::target_types` (vacío =
///    cualquier tipo) → si no, [`CheckCode::RelType`];
/// 3. el nº de targets respeta `RelationDef::cardinality` (`"one"` ⇒ máx. 1) → si no,
///    [`CheckCode::RelCard`].
///
/// Todos los `Check` son [`Severity::Err`], con `targets = [path del concepto origen]` y, cuando
/// se localiza la línea del campo en el frontmatter crudo, `range` relleno.
///
/// Función **pura y separada** de `Bundle::analyze`/`conform::validate_file`/[`validate_schema`]:
/// no se llama desde ninguna, así que un bundle sin relaciones tipadas (o sin `schema.yaml`) no
/// cambia su veredicto de conformidad actual (aditiva por composición del llamante). Un campo de
/// relación ausente del frontmatter no se valida (nada que comprobar); un concepto cuyo `type` no
/// está en el schema se ignora — mismo criterio que [`validate_schema`].
pub fn validate_relations(bundle: &Bundle, schema: &Schema) -> Vec<Check> {
    if schema.types.is_empty() {
        return Vec::new();
    }

    let concepts: BTreeSet<&RelPath> = bundle.analyze().concepts.iter().collect();

    let mut out = Vec::new();
    for path in &bundle.analyze().concepts {
        let Some(raw) = bundle.files().get(path) else {
            continue;
        };
        let parsed = model::parse_file(path.as_str(), raw);
        let Some(fm) = parsed.frontmatter else {
            continue;
        };
        let Some(tipo) = fm.get_text("type") else {
            continue;
        };
        let Some(doctype) = schema.types.get(&tipo) else {
            continue;
        };

        for (rel_name, reldef) in &doctype.relations {
            let Some(targets) = relation_targets(&fm, rel_name) else {
                continue;
            };
            if targets.is_empty() {
                continue;
            }

            let range = find_field_range(raw, rel_name);

            if reldef.cardinality == "one" && targets.len() > 1 {
                let mut check = Check::new(
                    Severity::Err,
                    CheckCode::RelCard,
                    format!(
                        "La relación «{rel_name}» de «{}» admite como máximo un target \
                         (cardinalidad «one») pero declara {}.",
                        path.as_str(),
                        targets.len()
                    ),
                    vec![path.clone()],
                );
                check.range = range;
                out.push(check);
            }

            for target_str in &targets {
                let Ok(target_path) = RelPath::new(target_str) else {
                    let mut check = Check::new(
                        Severity::Err,
                        CheckCode::RelTarget,
                        format!(
                            "La relación «{rel_name}» de «{}» apunta a «{target_str}», que no es una ruta válida.",
                            path.as_str()
                        ),
                        vec![path.clone()],
                    );
                    check.range = range;
                    check.fixes = vec![rel_target_fix(path, rel_name, target_str)];
                    out.push(check);
                    continue;
                };

                if !concepts.contains(&target_path) {
                    let mut check = Check::new(
                        Severity::Err,
                        CheckCode::RelTarget,
                        format!(
                            "La relación «{rel_name}» de «{}» apunta a «{target_str}», que no existe.",
                            path.as_str()
                        ),
                        vec![path.clone()],
                    );
                    check.range = range;
                    check.fixes = vec![rel_target_fix(path, rel_name, target_str)];
                    out.push(check);
                    continue;
                }

                if !reldef.target_types.is_empty() {
                    if let Some(target_type) = target_type_of(bundle, &target_path) {
                        if !reldef.target_types.iter().any(|t| t == &target_type) {
                            let mut check = Check::new(
                                Severity::Err,
                                CheckCode::RelType,
                                format!(
                                    "La relación «{rel_name}» de «{}» apunta a «{target_str}» de \
                                     tipo «{target_type}», no permitido (admite: {}).",
                                    path.as_str(),
                                    reldef.target_types.join(", ")
                                ),
                                vec![path.clone()],
                            );
                            check.range = range;
                            out.push(check);
                        }
                    }
                }
            }
        }
    }
    out
}

/// Lee el campo `rel_name` del frontmatter como lista de paths target: acepta una secuencia YAML
/// de `String` o un único `String` (envuelto en un vector de un elemento). `None` si el campo no
/// está presente en `extra` o su forma no es ninguna de las dos anteriores.
pub(crate) fn relation_targets(fm: &ParsedFrontmatter, rel_name: &str) -> Option<Vec<String>> {
    match fm.get_key(rel_name)? {
        serde_yaml::Value::Sequence(seq) => Some(
            seq.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect(),
        ),
        serde_yaml::Value::String(s) => Some(vec![s.clone()]),
        _ => None,
    }
}

/// El `type` del concepto en `target`, si existe en el bundle y parsea con frontmatter válido.
pub(crate) fn target_type_of(bundle: &Bundle, target: &RelPath) -> Option<String> {
    let raw = bundle.files().get(target)?;
    let parsed = model::parse_file(target.as_str(), raw);
    parsed.frontmatter.and_then(|fm| fm.get_text("type"))
}

/// Datos para materializar el `Fix` `safe` de una relación tipada ROTA (`REL-TARGET`, E12-H07):
/// quitar `target` del campo `rel_name` del frontmatter de `source`. Lo consume
/// [`crate::plan::normalize_apply_fix`], que re-localiza el fix por su `fix_id` estable.
pub(crate) struct RelTargetRepair {
    /// Id estable del fix (idéntico al del `Fix` que adjunta [`validate_relations`]).
    pub fix_id: String,
    /// Concepto origen (dueño del campo de relación) sobre el que recae el patch correctivo.
    pub source: RelPath,
    /// Nombre del campo de relación en el frontmatter (p. ej. `mentor`).
    pub rel_name: String,
    /// Target roto tal cual aparece en el campo, que el arreglo quita.
    pub target: String,
}

/// `fix_id` estable y determinista de una relación rota (`REL-TARGET`): `fix:blake3:<hex>` con
/// `hex = blake3(source ‖ 0x00 ‖ rel_name ‖ 0x00 ‖ target)`. Derivado **solo** del diagnóstico
/// (nunca de timestamps, orden ni caché), así el mismo bundle produce el mismo `fix_id` entre
/// procesos frescos → [`crate::plan::normalize_apply_fix`] puede re-localizarlo recomputando los
/// checks.
fn rel_target_fix_id(source: &str, rel_name: &str, target: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(source.as_bytes());
    hasher.update(b"\0");
    hasher.update(rel_name.as_bytes());
    hasher.update(b"\0");
    hasher.update(target.as_bytes());
    format!("fix:blake3:{}", hasher.finalize().to_hex())
}

/// Construye el `Fix { safe: true }` que se adjunta a un check `REL-TARGET` de relación rota: su
/// arreglo es «quitar la relación rota del campo». `safe` porque no destruye información conforme
/// (el target no existe/no es válido).
fn rel_target_fix(source: &RelPath, rel_name: &str, target: &str) -> Fix {
    Fix {
        fix_id: rel_target_fix_id(source.as_str(), rel_name, target),
        title: format!(
            "Quitar la relación rota «{rel_name} → {target}» de «{}».",
            source.as_str()
        ),
        safe: true,
    }
}

/// Recomputa los arreglos de relaciones tipadas ROTAS del bundle bajo `schema` — E12-H07. Espeja
/// exactamente la detección de `REL-TARGET` de [`validate_relations`] (target con ruta inválida o
/// que no existe como concepto), y devuelve, por cada uno, el [`RelTargetRepair`] con su `fix_id`
/// estable. Es la contraparte estructurada de los `Fix` que adjunta `validate_relations`: comparten
/// `rel_target_fix_id`, así que los `fix_id` coinciden. **Pura**.
pub(crate) fn rel_target_repairs(bundle: &Bundle, schema: &Schema) -> Vec<RelTargetRepair> {
    if schema.types.is_empty() {
        return Vec::new();
    }

    let concepts: BTreeSet<&RelPath> = bundle.analyze().concepts.iter().collect();

    let mut out = Vec::new();
    for path in &bundle.analyze().concepts {
        let Some(raw) = bundle.files().get(path) else {
            continue;
        };
        let parsed = model::parse_file(path.as_str(), raw);
        let Some(fm) = parsed.frontmatter else {
            continue;
        };
        let Some(tipo) = fm.get_text("type") else {
            continue;
        };
        let Some(doctype) = schema.types.get(&tipo) else {
            continue;
        };

        for rel_name in doctype.relations.keys() {
            let Some(targets) = relation_targets(&fm, rel_name) else {
                continue;
            };
            for target_str in &targets {
                let roto = match RelPath::new(target_str) {
                    Ok(target_path) => !concepts.contains(&target_path),
                    Err(_) => true,
                };
                if roto {
                    out.push(RelTargetRepair {
                        fix_id: rel_target_fix_id(path.as_str(), rel_name, target_str),
                        source: path.clone(),
                        rel_name: rel_name.clone(),
                        target: target_str.clone(),
                    });
                }
            }
        }
    }
    out
}

/// Busca la línea del campo `{field}:` dentro del bloque de frontmatter (entre el primer y el
/// segundo `---`) de `raw`, ignorando indentación. Devuelve su nº de línea 1-based como
/// `Range{start_line,end_line}` (un solo renglón); `None` si no se encuentra o el fichero no
/// tiene bloque de frontmatter.
fn find_field_range(raw: &str, field: &str) -> Option<Range> {
    let prefix = format!("{field}:");
    let mut in_front = false;
    for (idx, line) in raw.lines().enumerate() {
        let trimmed = line.trim_start();
        if trimmed == "---" {
            if in_front {
                break;
            }
            in_front = true;
            continue;
        }
        if in_front && trimmed.starts_with(&prefix) {
            let line_no = (idx + 1) as u32;
            return Some(Range {
                start_line: line_no,
                end_line: line_no,
            });
        }
    }
    None
}

/// `true` si `campo` está presente en el frontmatter. Desde E16-H01 no hay campos privilegiados:
/// cualquier clave de primer nivel cuenta igual, y una presente con `null` explícito cuenta como
/// presente (declarar la clave es declararla, aunque no tenga valor todavía).
fn field_present(fm: &ParsedFrontmatter, campo: &str) -> bool {
    fm.contains_key(campo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_vacio_es_permisivo() {
        let s = Schema::default();
        assert_eq!(s.version, "1");
        assert!(s.types.is_empty());
    }
}
