//! Diff semántico OKF (`ARCHITECTURE.md §4.4`, `§13.3`). Port de `diffSnap`/`fmDiff`/`lineDiff`/`collapseDiff`.
//!
//! Es la **única verdad computada** del diff; lo renderizan igual las fachadas y el frontend.
//! El LCS lleva una **guarda de tamaño** (fallback grueso por umbral) para no reventar la memoria
//! con ficheros enormes; la versión Hirschberg/dos-filas es una mejora aditiva futura.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

use crate::model;
use crate::types::{FileMap, RelPath};

/// Umbral de celdas (n×m) del LCS antes de caer a un diff grueso. ~2M celdas ≈ pocos MB.
const MAX_LCS_CELLS: usize = 2_000_000;

macro_rules! schema_derive {
    ($item:item) => {
        #[cfg_attr(feature = "schemars", derive(schemars::JsonSchema))]
        $item
    };
}

schema_derive! {
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ChangeKind {
    Add,
    Mod,
    Remove,
}
}

schema_derive! {
/// Cambio de un campo de frontmatter. Orden status-first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FieldChange {
    pub key: String,
    pub from: Option<String>,
    pub to: Option<String>,
}
}

schema_derive! {
/// Un trozo del diff del cuerpo (LCS + plegado de contexto).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", content = "v", rename_all = "lowercase")]
pub enum BodyHunk {
    Context(String),
    Add(String),
    Remove(String),
    Gap(u32),
}
}

schema_derive! {
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FileDiff {
    pub path: RelPath,
    pub kind: ChangeKind,
    pub fm: Vec<FieldChange>,
    pub body: Vec<BodyHunk>,
    pub links_added: Vec<RelPath>,
    pub links_removed: Vec<RelPath>,
}
}

schema_derive! {
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedChange {
    pub path: RelPath,
    pub kind: ChangeKind,
}
}

schema_derive! {
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StatusChange {
    pub path: RelPath,
    pub from: Option<String>,
    pub to: Option<String>,
}
}

schema_derive! {
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiffStats {
    pub added: usize,
    pub modified: usize,
    pub removed: usize,
}
}

schema_derive! {
/// Pista de mensaje de commit (i18n vía catálogo en la fachada).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum MessageHint {
    AddSingle { title: String },
    StatusSingle { to: String, title: String },
    Update { added: usize, modified: usize, removed: usize },
}
}

schema_derive! {
/// El diff semántico OKF completo.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OkfDiff {
    pub files: Vec<FileDiff>,
    pub generated: Vec<GeneratedChange>,
    pub stats: DiffStats,
    pub status_changes: Vec<StatusChange>,
    pub suggested: MessageHint,
}
}

/// `true` si el path es un artefacto generado (index/log/tags). Port de `isGenerated`.
pub fn is_generated(p: &RelPath) -> bool {
    p.is_reserved() || p.as_str().starts_with("tags/")
}

/// Diff entre dos file-maps (árbol vs árbol, o HEAD vs working). Port de `diffSnap`.
pub fn diff_snap(a: &FileMap, b: &FileMap) -> OkfDiff {
    // El proto ordena las claves con `sortPaths` (numeric-aware: `doc-2` < `doc-10`), no léxico.
    let keys: Vec<RelPath> = {
        let set: BTreeSet<RelPath> = a.keys().chain(b.keys()).cloned().collect();
        let mut v: Vec<RelPath> = set.into_iter().collect();
        v.sort_by(|x, y| model::sort_paths_cmp(x.as_str(), y.as_str()));
        v
    };
    let mut files: Vec<FileDiff> = Vec::new();
    let mut generated: Vec<GeneratedChange> = Vec::new();
    let mut stats = DiffStats::default();
    let mut status_changes: Vec<StatusChange> = Vec::new();

    for p in &keys {
        let av = a.get(p);
        let bv = b.get(p);
        if av == bv {
            continue;
        }
        let kind = match (av, bv) {
            (None, _) => ChangeKind::Add,
            (_, None) => ChangeKind::Remove,
            _ => ChangeKind::Mod,
        };
        if is_generated(p) {
            generated.push(GeneratedChange {
                path: p.clone(),
                kind,
            });
            continue;
        }
        match kind {
            ChangeKind::Add => stats.added += 1,
            ChangeKind::Remove => stats.removed += 1,
            ChangeKind::Mod => stats.modified += 1,
        }
        let fm = fm_diff(
            av.map(String::as_str).unwrap_or(""),
            bv.map(String::as_str).unwrap_or(""),
        );
        if let Some(sc) = fm.iter().find(|c| c.key == "status") {
            status_changes.push(StatusChange {
                path: p.clone(),
                from: sc.from.clone(),
                to: sc.to.clone(),
            });
        }
        let a_body = av.map(|r| model::split_front(r).body).unwrap_or_default();
        let b_body = bv.map(|r| model::split_front(r).body).unwrap_or_default();
        let body = collapse_diff(line_diff(&a_body, &b_body));
        let la: BTreeSet<RelPath> = av
            .map(|r| out_link_paths(p, &model::split_front(r).body))
            .unwrap_or_default();
        let lb: BTreeSet<RelPath> = bv
            .map(|r| out_link_paths(p, &model::split_front(r).body))
            .unwrap_or_default();
        let links_added = lb.difference(&la).cloned().collect();
        let links_removed = la.difference(&lb).cloned().collect();
        files.push(FileDiff {
            path: p.clone(),
            kind,
            fm,
            body,
            links_added,
            links_removed,
        });
    }

    let suggested = suggest_msg(a, b, &files, &stats, &status_changes);
    OkfDiff {
        files,
        generated,
        stats,
        status_changes,
        suggested,
    }
}

fn out_link_paths(p: &RelPath, body: &str) -> BTreeSet<RelPath> {
    model::out_links(p.as_str(), body)
        .into_iter()
        .filter_map(|t| RelPath::new(&t).ok())
        .collect()
}

/// Port de `fmDiff`: cambios por campo de frontmatter, orden status-first.
pub fn fm_diff(a_raw: &str, b_raw: &str) -> Vec<FieldChange> {
    let a = fm_pairs(a_raw);
    let b = fm_pairs(b_raw);
    // Unión de claves en orden de aparición (Set de JS), no alfabético: el sort status-first
    // es estable y el proto conserva ese orden para las claves sin rango.
    let mut keys: Vec<String> = a.iter().map(|(k, _)| k.clone()).collect();
    for (k, _) in &b {
        if !keys.contains(k) {
            keys.push(k.clone());
        }
    }
    let get = |m: &Vec<(String, serde_yaml::Value)>, k: &str| -> Option<serde_yaml::Value> {
        m.iter().find(|(kk, _)| kk == k).map(|(_, v)| v.clone())
    };
    let mut out: Vec<FieldChange> = Vec::new();
    for k in keys {
        let av = get(&a, &k);
        let bv = get(&b, &k);
        let af = av.as_ref().map(fm_fmt);
        let bf = bv.as_ref().map(fm_fmt);
        // El proto compara los FORMATEADOS (`fmFmt(undefined) === ""`): clave ausente y clave
        // con valor que formatea a "" son indistinguibles → sin cambio fantasma.
        if af.clone().unwrap_or_default() == bf.clone().unwrap_or_default() {
            continue;
        }
        out.push(FieldChange {
            key: k,
            from: af,
            to: bf,
        });
    }
    let order = [
        "status",
        "type",
        "title",
        "description",
        "tags",
        "timestamp",
        "resource",
    ];
    let rank = |k: &str| {
        order
            .iter()
            .position(|x| *x == k)
            .map(|i| i + 1)
            .unwrap_or(99)
    };
    out.sort_by_key(|c| rank(&c.key));
    out
}

fn fm_pairs(raw: &str) -> Vec<(String, serde_yaml::Value)> {
    let sf = model::split_front(raw);
    let text = match sf.fm_text {
        Some(t) if !t.is_empty() => t,
        _ => return Vec::new(),
    };
    match model::parse_yaml(&text) {
        Ok(fm) => fm.as_pairs(),
        Err(_) => Vec::new(),
    }
}

/// Port de `fmFmt`: representación textual de un valor de frontmatter.
fn fm_fmt(v: &serde_yaml::Value) -> String {
    match v {
        serde_yaml::Value::Null => String::new(),
        serde_yaml::Value::String(s) => s.clone(),
        serde_yaml::Value::Bool(b) => b.to_string(),
        serde_yaml::Value::Number(n) => n.to_string(),
        serde_yaml::Value::Sequence(seq) => seq
            .iter()
            .map(|x| match x {
                serde_yaml::Value::String(s) => s.clone(),
                other => fm_fmt(other),
            })
            .collect::<Vec<_>>()
            .join(", "),
        other => serde_yaml::to_string(other)
            .unwrap_or_default()
            .trim()
            .to_string(),
    }
}

#[derive(Clone)]
enum Row {
    Ctx(String),
    Del(String),
    Ins(String),
}

/// Port de `lineDiff`: LCS por líneas con guarda de tamaño.
fn line_diff(a: &str, b: &str) -> Vec<Row> {
    let av: Vec<&str> = a.split('\n').collect();
    let bv: Vec<&str> = b.split('\n').collect();
    let n = av.len();
    let m = bv.len();

    // Guarda: si la tabla es demasiado grande, diff grueso (todo borrado + todo añadido).
    if n.saturating_mul(m) > MAX_LCS_CELLS {
        let mut out = Vec::with_capacity(n + m);
        out.extend(av.iter().map(|s| Row::Del((*s).to_string())));
        out.extend(bv.iter().map(|s| Row::Ins((*s).to_string())));
        return out;
    }

    // dp[i][j] = LCS de A[i..] y B[j..].
    let mut dp = vec![vec![0i32; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[i][j] = if av[i] == bv[j] {
                dp[i + 1][j + 1] + 1
            } else {
                dp[i + 1][j].max(dp[i][j + 1])
            };
        }
    }
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if av[i] == bv[j] {
            out.push(Row::Ctx(av[i].to_string()));
            i += 1;
            j += 1;
        } else if dp[i + 1][j] >= dp[i][j + 1] {
            out.push(Row::Del(av[i].to_string()));
            i += 1;
        } else {
            out.push(Row::Ins(bv[j].to_string()));
            j += 1;
        }
    }
    while i < n {
        out.push(Row::Del(av[i].to_string()));
        i += 1;
    }
    while j < m {
        out.push(Row::Ins(bv[j].to_string()));
        j += 1;
    }
    out
}

/// Port de `collapseDiff`: pliega rachas largas de contexto en `Gap`.
fn collapse_diff(rows: Vec<Row>) -> Vec<BodyHunk> {
    let mut out: Vec<BodyHunk> = Vec::new();
    let mut i = 0;
    let len = rows.len();
    while i < len {
        if matches!(rows[i], Row::Ctx(_)) {
            let mut j = i;
            while j < len && matches!(rows[j], Row::Ctx(_)) {
                j += 1;
            }
            let run = j - i;
            let keep_top = if i > 0 { 2 } else { 0 };
            let keep_bot = if j < len { 2 } else { 0 };
            if run > 4 && run.saturating_sub(keep_top).saturating_sub(keep_bot) > 0 {
                for k in 0..keep_top {
                    out.push(ctx_hunk(&rows[i + k]));
                }
                out.push(BodyHunk::Gap((run - keep_top - keep_bot) as u32));
                for k in (1..=keep_bot).rev() {
                    out.push(ctx_hunk(&rows[j - k]));
                }
            } else {
                for r in &rows[i..j] {
                    out.push(ctx_hunk(r));
                }
            }
            i = j;
        } else {
            out.push(match &rows[i] {
                Row::Del(s) => BodyHunk::Remove(s.clone()),
                Row::Ins(s) => BodyHunk::Add(s.clone()),
                Row::Ctx(s) => BodyHunk::Context(s.clone()),
            });
            i += 1;
        }
    }
    out
}

fn ctx_hunk(r: &Row) -> BodyHunk {
    match r {
        Row::Ctx(s) => BodyHunk::Context(s.clone()),
        Row::Del(s) => BodyHunk::Remove(s.clone()),
        Row::Ins(s) => BodyHunk::Add(s.clone()),
    }
}

/// Port de `suggestMsg`: deriva la pista de mensaje de commit.
fn suggest_msg(
    a: &FileMap,
    b: &FileMap,
    files: &[FileDiff],
    stats: &DiffStats,
    status_changes: &[StatusChange],
) -> MessageHint {
    if stats.added == 1 && stats.modified == 0 && stats.removed == 0 {
        let title = files
            .iter()
            .find(|f| f.kind == ChangeKind::Add)
            .map(|f| page_title(a, b, &f.path))
            .unwrap_or_else(|| "una página".to_string());
        return MessageHint::AddSingle { title };
    }
    if status_changes.len() == 1 {
        // El proto exige `to` truthy: `status → ""` cae al mensaje genérico.
        if let Some(to) = status_changes[0].to.as_ref().filter(|t| !t.is_empty()) {
            let title = page_title(a, b, &status_changes[0].path);
            return MessageHint::StatusSingle {
                to: to.clone(),
                title,
            };
        }
    }
    MessageHint::Update {
        added: stats.added,
        modified: stats.modified,
        removed: stats.removed,
    }
}

fn page_title(a: &FileMap, b: &FileMap, p: &RelPath) -> String {
    let raw = b.get(p).or_else(|| a.get(p));
    if let Some(raw) = raw {
        let pairs = fm_pairs(raw);
        if let Some((_, serde_yaml::Value::String(t))) = pairs.iter().find(|(k, _)| k == "title") {
            if !t.is_empty() {
                return t.clone();
            }
        }
    }
    model::title_from_path(p.as_str())
}
