//! Casado de **texto libre por subcadena** (`ARCHITECTURE.md §20.12`): la búsqueda full-text del
//! `text` de `knowledge_search` y la aceleración FTS del store se apoyan en la **misma** verdad.
//!
//! Es el único resto vivo de la extinta DSL de tokens (`query.rs`, retirada en E19-H05): el filtrado
//! por metadata pasó al lenguaje tipado (`crate::parse`/`crate::filter`/`crate::eval`), pero el
//! `text` sigue siendo una **subcadena** case-insensitive sobre basename + valores de frontmatter +
//! cuerpo. Se conserva aquí, en un módulo propio, para que la cache (`lodestar-store`) y la fachada
//! (`lodestar-app`) compartan LA MISMA función en lugar de reimplementarla en SQL (invariante #3 de
//! `CLAUDE.md`: una sola verdad computada; el FTS solo acelera).

use serde_yaml::Value as Yaml;

use crate::types::{ParsedFrontmatter, RelPath};

/// Semántica de **texto suelto** (subcadena, case-insensitive): casa si `needle_lower` aparece en el
/// basename, en cualquier valor de frontmatter o en el cuerpo. `needle_lower` debe venir ya en
/// minúsculas. Pública porque la comparten la cache (`lodestar-store`) y la fachada (`lodestar-app`)
/// como única verdad del casado textual (en vez de un `LIKE` de SQL que divergiría en el lowercase
/// Unicode y en no cubrir basename/frontmatter).
pub fn loose_text_match(
    path: &RelPath,
    fm: &ParsedFrontmatter,
    body: &str,
    needle_lower: &str,
) -> bool {
    path.basename().to_lowercase().contains(needle_lower)
        || fm
            .entries()
            .iter()
            .any(|(_, v)| value_includes(v, needle_lower))
        || body.to_lowercase().contains(needle_lower)
}

/// `true` si `val` (ya en minúsculas) es subcadena de la representación textual de `raw`. Una lista
/// casa si algún elemento casa; un `null` nunca casa.
fn value_includes(raw: &Yaml, val: &str) -> bool {
    match raw {
        Yaml::Null => false,
        Yaml::Sequence(items) => items
            .iter()
            .any(|x| scalar_to_string(x).to_lowercase().contains(val)),
        other => scalar_to_string(other).to_lowercase().contains(val),
    }
}

/// Representación de un escalar YAML como string (port de `String(raw)` de JS): strings tal cual,
/// booleanos/números por su `to_string`, `null` como cadena vacía y cualquier estructura por su
/// serialización YAML recortada.
fn scalar_to_string(v: &Yaml) -> String {
    match v {
        Yaml::String(s) => s.clone(),
        Yaml::Bool(b) => b.to_string(),
        Yaml::Number(n) => n.to_string(),
        Yaml::Null => String::new(),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}
