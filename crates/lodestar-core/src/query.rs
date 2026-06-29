//! Query: un único tokenizer + matcher con semántica de **subcadena** (`ARCHITECTURE.md §4.3`).
//!
//! Port fiel de `tokenizeQuery`/`matchToken`/`isPredicate`/`fieldMatch`/`valueIncludes`/`fmGet`/`fmPresent`.
//! Conserva los quirks: gating de fichero reservado ANTES de negar, flip `!val`, campo ASCII `[\w\-]+`.

use serde_yaml::Value as Yaml;

use crate::bundle::Bundle;
use crate::types::{Analysis, Frontmatter, RelPath, Severity};

/// Un token de la DSL.
#[derive(Debug, Clone)]
pub struct Token {
    pub neg: bool,
    pub field: Option<String>,
    pub op: Option<char>,
    pub val: String,
}

/// Port de `tokenizeQuery`.
pub fn tokenize_query(q: &str) -> Vec<Token> {
    let chars: Vec<char> = q.chars().collect();
    let n = chars.len();
    let mut out = Vec::new();
    let mut i = 0;
    while i < n {
        while i < n && chars[i].is_whitespace() {
            i += 1;
        }
        if i >= n {
            break;
        }
        let mut neg = false;
        if chars[i] == '-' {
            neg = true;
            i += 1;
        }
        let mut field = None;
        let mut op = None;
        let mut j = i;
        while j < n && (chars[j].is_ascii_alphanumeric() || chars[j] == '_' || chars[j] == '-') {
            j += 1;
        }
        if j < n && (chars[j] == ':' || chars[j] == '=') {
            field = Some(chars[i..j].iter().collect());
            op = Some(chars[j]);
            i = j + 1;
        }
        let mut val = String::new();
        if i < n && chars[i] == '"' {
            i += 1;
            while i < n && chars[i] != '"' {
                val.push(chars[i]);
                i += 1;
            }
            if i < n {
                i += 1;
            }
        } else {
            while i < n && !chars[i].is_whitespace() {
                val.push(chars[i]);
                i += 1;
            }
        }
        if val.starts_with('!') {
            neg = !neg;
            val = val[1..].to_string();
        }
        out.push(Token {
            neg,
            field,
            op,
            val,
        });
    }
    out
}

/// Filtra los paths del bundle que casan con TODOS los tokens (vacío = todos).
pub fn query(bundle: &Bundle, dsl: &str) -> Vec<RelPath> {
    let tokens = tokenize_query(dsl.trim());
    let analysis = bundle.analyze();
    bundle
        .files()
        .keys()
        .filter(|p| match_file(bundle, p, &tokens, analysis))
        .cloned()
        .collect()
}

fn match_file(bundle: &Bundle, path: &RelPath, tokens: &[Token], a: &Analysis) -> bool {
    if tokens.is_empty() {
        return true;
    }
    let fm = bundle
        .parsed(path)
        .and_then(|p| p.fm.clone())
        .unwrap_or_default();
    let body = bundle.parsed(path).map(|p| p.body.as_str()).unwrap_or("");
    tokens.iter().all(|t| match_token(t, path, &fm, body, a))
}

fn match_token(t: &Token, path: &RelPath, fm: &Frontmatter, body: &str, a: &Analysis) -> bool {
    let reserved = path.is_reserved();
    let val = t.val.to_lowercase();
    let field_name = t.field.as_ref().map(|f| f.to_lowercase());
    let is_field_token = t.field.is_some()
        && !matches!(
            field_name.as_deref(),
            Some("has") | Some("no") | Some("is") | Some("body")
        );

    // Quirk: gating de fichero reservado ANTES de negar.
    if reserved
        && (is_field_token
            || field_name.as_deref() == Some("has")
            || field_name.as_deref() == Some("no")
            || (field_name.as_deref() == Some("is") && val != "reserved"))
    {
        return false;
    }

    let res = if let Some(field) = &t.field {
        match field_name.as_deref() {
            Some("has") => fm_present(fm, &t.val),
            Some("no") => !fm_present(fm, &t.val),
            Some("is") => is_predicate(&val, path, fm, a),
            Some("body") => body.to_lowercase().contains(&val),
            _ => field_match(fm_get(fm, field), &t.val, t.op),
        }
    } else {
        // Texto suelto: basename, luego cualquier valor de frontmatter, luego el cuerpo (subcadena).
        path.basename().to_lowercase().contains(&val)
            || fm.as_pairs().iter().any(|(_, v)| value_includes(v, &val))
            || body.to_lowercase().contains(&val)
    };

    if t.neg {
        !res
    } else {
        res
    }
}

fn fm_get(fm: &Frontmatter, key: &str) -> Option<Yaml> {
    let pairs = fm.as_pairs();
    if let Some((_, v)) = pairs.iter().find(|(k, _)| k == key) {
        return Some(v.clone());
    }
    let lower = key.to_lowercase();
    pairs
        .iter()
        .find(|(k, _)| *k == lower)
        .map(|(_, v)| v.clone())
}

fn fm_present(fm: &Frontmatter, key: &str) -> bool {
    match fm_get(fm, key) {
        None => false,
        Some(Yaml::String(s)) => !s.is_empty(),
        Some(Yaml::Sequence(s)) => !s.is_empty(),
        Some(Yaml::Null) => false,
        Some(_) => true,
    }
}

fn field_match(raw: Option<Yaml>, value: &str, op: Option<char>) -> bool {
    let raw = match raw {
        Some(r) => r,
        None => return false,
    };
    let val = value.to_lowercase();
    let exact = op == Some('=');
    match raw {
        Yaml::Sequence(items) => items.iter().any(|x| {
            let s = scalar_to_string(x).to_lowercase();
            if exact {
                s == val
            } else {
                s.contains(&val)
            }
        }),
        other => {
            let s = scalar_to_string(&other).to_lowercase();
            if exact {
                s == val
            } else {
                s.contains(&val)
            }
        }
    }
}

fn value_includes(raw: &Yaml, val: &str) -> bool {
    match raw {
        Yaml::Null => false,
        Yaml::Sequence(items) => items
            .iter()
            .any(|x| scalar_to_string(x).to_lowercase().contains(val)),
        other => scalar_to_string(other).to_lowercase().contains(val),
    }
}

fn is_predicate(name: &str, path: &RelPath, fm: &Frontmatter, a: &Analysis) -> bool {
    match name {
        "orphan" => a.orphans.contains(path),
        "invalid" => a
            .per_file
            .get(path)
            .map(|cs| cs.iter().any(|c| c.level == Severity::Err))
            .unwrap_or(false),
        "reserved" => path.is_reserved(),
        "linked" => a.inn.get(path).map(|v| !v.is_empty()).unwrap_or(false),
        "accepted" | "draft" | "review" | "deprecated" => fm
            .status
            .as_deref()
            .map(|s| s.to_lowercase() == name)
            .unwrap_or(false),
        _ => false,
    }
}

/// Representación de un escalar YAML como string (port de `String(raw)` de JS).
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
