//! Generadores puros: `gen_index` y `gen_tag_indexes` (`ARCHITECTURE.md §4.2`, `§10` fila 12).
//!
//! Devuelven un [`Mutation`] (plan); la workspace lo aplica por el único camino de escritura.
//! Port de `genIndex`/`generateTagIndex`/`slugifyTag`. Cabeceras canónicas fijas (i18n `§12`).

use std::collections::{BTreeMap, BTreeSet};

use once_cell::sync::Lazy;
use regex::Regex;

use crate::bundle::Bundle;
use crate::model;
use crate::types::{Mutation, RelPath};

static TAG_INDEX_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"^tags/[^/]+/index\.md$").unwrap());
static SLUG_SEP_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r#"[/\\'"]+"#).unwrap());
static SLUG_WS_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"\s+").unwrap());
static SLUG_BAD_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"[^\p{L}\p{N}._-]+").unwrap());
static SLUG_DASH_RE: Lazy<Regex> = Lazy::new(|| Regex::new(r"-+").unwrap());

/// Genera el `index.md` de un directorio. `dir` es `""` (root) o `"sub/"`. Port de `genIndex`.
pub fn gen_index(bundle: &Bundle, dir: &str) -> Mutation {
    let files = bundle.files();
    let here: Vec<&RelPath> = {
        let mut v: Vec<&RelPath> = files
            .keys()
            .filter(|p| p.dir() == dir && !p.is_reserved())
            .collect();
        v.sort_by(|a, b| a.as_str().cmp(b.as_str()));
        v
    };

    let mut subdirs: BTreeSet<String> = BTreeSet::new();
    for p in files.keys() {
        let ps = p.as_str();
        if ps.starts_with(dir) && ps != dir {
            let rest = &ps[dir.len()..];
            if let Some(seg) = rest.split('/').next() {
                if rest.contains('/') {
                    subdirs.insert(format!("{seg}/"));
                }
            }
        }
    }

    let mut out = if dir.is_empty() {
        String::from("---\nokf_version: \"0.1\"\n---\n\n# Bundle\n\n")
    } else {
        format!("# {}\n\n", dir.trim_end_matches('/'))
    };

    // Agrupa por tipo.
    let mut by_type: BTreeMap<String, Vec<&RelPath>> = BTreeMap::new();
    for p in &here {
        // `fm.type || "Concept"` del proto: el fallback captura también el string vacío (falsy).
        let ty = bundle
            .parsed(p)
            .and_then(|x| x.fm.as_ref())
            .and_then(|f| f.r#type.clone())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "Concept".to_string());
        by_type.entry(ty).or_default().push(p);
    }
    for (ty, ps) in &by_type {
        out.push_str(&format!("# {ty}\n\n"));
        for p in ps {
            let fm = bundle.parsed(p).and_then(|x| x.fm.as_ref());
            let title = fm
                .and_then(|f| f.title.clone())
                .filter(|s| !s.is_empty())
                .unwrap_or_else(|| model::concept_id(p.basename()));
            let desc = fm.and_then(|f| f.description.clone()).unwrap_or_default();
            let tail = if desc.is_empty() {
                String::new()
            } else {
                format!(" - {desc}")
            };
            out.push_str(&format!("* [{}]({}){}\n", title, p.basename(), tail));
        }
        out.push('\n');
    }

    if !subdirs.is_empty() {
        out.push_str("# Subdirectorios\n\n");
        for s in &subdirs {
            out.push_str(&format!("* [{s}]({s})\n"));
        }
        out.push('\n');
    }

    // replace(/\n+$/,"\n")
    let trimmed = format!("{}\n", out.trim_end_matches('\n'));
    let path = RelPath::new(&format!("{dir}index.md")).expect("path de index válido");
    Mutation {
        writes: BTreeMap::from([(path, trimmed)]),
        deletes: Vec::new(),
    }
}

/// Genera/purga los índices de tags. Port de `generateTagIndex` (la parte pura → `Mutation`).
pub fn gen_tag_indexes(bundle: &Bundle) -> Mutation {
    let a = bundle.analyze();
    // tag → conjunto de paths.
    let mut tag_map: BTreeMap<String, BTreeSet<RelPath>> = BTreeMap::new();
    for p in &a.concepts {
        let fm = match bundle.parsed(p).and_then(|x| x.fm.as_ref()) {
            Some(f) => f,
            None => continue,
        };
        let tags = match &fm.tags {
            Some(t) => t,
            None => continue,
        };
        let items: Vec<String> = match tags {
            serde_yaml::Value::Sequence(seq) => seq.iter().filter_map(yaml_scalar_string).collect(),
            other => yaml_scalar_string(other).into_iter().collect(),
        };
        for raw in items {
            let t = raw.trim().to_string();
            if !t.is_empty() {
                tag_map.entry(t).or_default().insert(p.clone());
            }
        }
    }

    // El proto ordena con `localeCompare` (no bytes): el orden decide además qué tag gana el
    // slug base en una colisión (`Foo` vs `foo` → `foo`/`foo-2`).
    let mut tags: Vec<String> = tag_map.keys().cloned().collect();
    tags.sort_by(|a, b| model::locale_cmp(a, b));
    let tags = tags;
    let existing: Vec<RelPath> = bundle
        .files()
        .keys()
        .filter(|p| p.as_str() == "tags/index.md" || TAG_INDEX_RE.is_match(p.as_str()))
        .cloned()
        .collect();

    if tags.is_empty() {
        return Mutation {
            writes: BTreeMap::new(),
            deletes: existing,
        };
    }

    // slug por tag con dedup de colisiones.
    let mut slug_by_tag: BTreeMap<String, String> = BTreeMap::new();
    let mut used: BTreeSet<String> = BTreeSet::new();
    for t in &tags {
        let base = slugify_tag(t);
        let mut s = base.clone();
        let mut i = 2;
        while used.contains(&s) {
            s = format!("{base}-{i}");
            i += 1;
        }
        used.insert(s.clone());
        slug_by_tag.insert(t.clone(), s);
    }

    let mut writes: BTreeMap<RelPath, String> = BTreeMap::new();
    for t in &tags {
        let slug = &slug_by_tag[t];
        let mut paths: Vec<&RelPath> = tag_map[t].iter().collect();
        // El prototipo ordena los items de cada tag con `sortPaths` (numeric-aware), no léxico.
        paths.sort_by(|a, b| model::sort_paths_cmp(a.as_str(), b.as_str()));
        let items: Vec<String> = paths
            .iter()
            .map(|p| {
                let fm = bundle.parsed(p).and_then(|x| x.fm.as_ref());
                let title = fm
                    .and_then(|f| f.title.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| model::concept_id(p.basename()));
                let desc = fm.and_then(|f| f.description.clone()).unwrap_or_default();
                let tail = if desc.is_empty() {
                    String::new()
                } else {
                    format!(" - {desc}")
                };
                format!("* [{}](/{}){}", title, p.as_str(), tail)
            })
            .collect();
        let path = RelPath::new(&format!("tags/{slug}/index.md")).expect("path de tag válido");
        writes.insert(path, format!("# {}\n\n{}\n", t, items.join("\n")));
    }

    let root_items: Vec<String> = tags
        .iter()
        .map(|t| {
            let n = tag_map[t].len();
            let plural = if n != 1 { "s" } else { "" };
            format!("* [{}]({}/) - {} concept{}", t, slug_by_tag[t], n, plural)
        })
        .collect();
    let root_path = RelPath::new("tags/index.md").expect("path raíz de tags válido");
    writes.insert(root_path, format!("# Tags\n\n{}\n", root_items.join("\n")));

    let deletes: Vec<RelPath> = existing
        .into_iter()
        .filter(|p| !writes.contains_key(p))
        .collect();
    Mutation { writes, deletes }
}

/// Port de `slugifyTag`: `lower → trim → NFC → reemplazos → colapsa → recorta`.
pub fn slugify_tag(t: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let s = t.to_lowercase();
    let s = s.trim();
    // NFC como en el prototipo (`.normalize("NFC")`): compone p. ej. `e`+U+0301 → `é` para que
    // `\p{L}` no descarte la marca combinante.
    let s: String = s.nfc().collect();
    let s = SLUG_SEP_RE.replace_all(&s, "-");
    let s = SLUG_WS_RE.replace_all(&s, "-");
    let s = SLUG_BAD_RE.replace_all(&s, "-");
    let s = SLUG_DASH_RE.replace_all(&s, "-");
    let s = s.trim_matches(|c| c == '-' || c == '.');
    if s.is_empty() {
        "tag".to_string()
    } else {
        s.to_string()
    }
}

fn yaml_scalar_string(v: &serde_yaml::Value) -> Option<String> {
    match v {
        serde_yaml::Value::String(s) => Some(s.clone()),
        serde_yaml::Value::Number(n) => Some(n.to_string()),
        serde_yaml::Value::Bool(b) => Some(b.to_string()),
        _ => None,
    }
}
