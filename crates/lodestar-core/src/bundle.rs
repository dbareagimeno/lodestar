//! El agregado [`Bundle`] (`ARCHITECTURE.md §4.2`): construcción desde un `FileMap`,
//! `analyze()` cacheado y la superficie de lectura/escritura semántica.

use std::collections::{BTreeMap, BTreeSet};

use once_cell::sync::Lazy;
use regex::Regex;

use crate::conform::{self, ConformCtx};
use crate::model::{self, Parsed};
use crate::types::{
    Analysis, Backlinks, Check, ConceptSummary, Direction, Frontmatter, FrontmatterPatch,
    GraphModel, LinkRef, Neighborhood, RelPath, Severity, WriteOutcome,
};

static OKF_VER_VAL_RE: Lazy<Regex> =
    Lazy::new(|| Regex::new(r#"(?m)^\s*okf_version\s*:\s*['"]?([^'"\r\n]+?)['"]?\s*$"#).unwrap());

/// Bundle OKF: un mapa de ficheros + el análisis derivado (cacheado).
pub struct Bundle {
    files: crate::types::FileMap,
    parsed: BTreeMap<RelPath, Parsed>,
    analysis: once_cell::sync::OnceCell<Analysis>,
}

impl Bundle {
    /// Construye un `Bundle` desde un `FileMap`. Parsea cada fichero una vez (puro, sin I/O).
    pub fn from_files(files: crate::types::FileMap) -> Self {
        let parsed = files
            .iter()
            .map(|(p, raw)| (p.clone(), model::parse_file(p.as_str(), raw)))
            .collect();
        Bundle {
            files,
            parsed,
            analysis: once_cell::sync::OnceCell::new(),
        }
    }

    /// El mapa de ficheros subyacente.
    pub fn files(&self) -> &crate::types::FileMap {
        &self.files
    }

    /// Análisis del bundle, cacheado con `OnceCell` (recomputar es idempotente).
    pub fn analyze(&self) -> &Analysis {
        self.analysis.get_or_init(|| self.compute_analysis())
    }

    fn compute_analysis(&self) -> Analysis {
        let mut concepts: Vec<RelPath> = Vec::new();
        let mut out: BTreeMap<RelPath, Vec<RelPath>> = BTreeMap::new();
        let mut inn: BTreeMap<RelPath, Vec<RelPath>> = BTreeMap::new();
        let mut in_index: BTreeSet<RelPath> = BTreeSet::new();
        let mut dangling_set: BTreeSet<RelPath> = BTreeSet::new();

        // Adyacencia saliente + in_index (port de analyzeBundle).
        for (path, raw) in &self.files {
            let bn = path.basename();
            if bn == "index.md" {
                for t in model::out_links(path.as_str(), raw) {
                    if let Ok(rp) = RelPath::new(&t) {
                        in_index.insert(rp);
                    }
                }
                continue;
            }
            if bn == "log.md" {
                continue;
            }
            concepts.push(path.clone());
            let body = &self.parsed[path].body;
            let targets: Vec<RelPath> = model::out_links(path.as_str(), body)
                .into_iter()
                .filter_map(|t| RelPath::new(&t).ok())
                .collect();
            out.insert(path.clone(), targets);
        }

        for p in &concepts {
            inn.entry(p.clone()).or_default();
        }
        // Inversión de aristas + dangling.
        for p in &concepts {
            for t in out.get(p).cloned().unwrap_or_default() {
                let exists = self.files.contains_key(&t);
                if exists && !t.is_reserved() {
                    inn.entry(t.clone()).or_default().push(p.clone());
                } else if exists && t.basename() == "index.md" {
                    // enlaces a index.md no cuentan como backlink ni como dangling
                } else {
                    dangling_set.insert(t.clone());
                }
            }
        }

        // Conformidad por fichero.
        let ctx = ConformCtx {
            files: &self.files,
            out: &out,
            inn: &inn,
            in_index: &in_index,
        };
        let mut per_file: BTreeMap<RelPath, Vec<Check>> = BTreeMap::new();
        let mut hard_fail = 0usize;
        let mut warn_count = 0usize;
        for (path, raw) in &self.files {
            let checks = conform::validate_file(path, &self.parsed[path], raw, &ctx);
            if checks.iter().any(|c| c.level == Severity::Err) {
                hard_fail += 1;
            }
            warn_count += checks.iter().filter(|c| c.level == Severity::Warn).count();
            per_file.insert(path.clone(), checks);
        }

        // Huérfanos (isRootish siempre false en el prototipo).
        let orphans: Vec<RelPath> = concepts
            .iter()
            .filter(|p| inn.get(*p).map(|v| v.is_empty()).unwrap_or(true) && !in_index.contains(*p))
            .cloned()
            .collect();

        let dangling: Vec<RelPath> = dangling_set.into_iter().collect();

        Analysis {
            concepts,
            out,
            inn,
            in_index,
            dangling,
            orphans,
            per_file,
            hard_fail,
            warn_count,
            okf_version: self.root_okf_version(),
        }
    }

    fn root_okf_version(&self) -> Option<String> {
        let idx = RelPath::new("index.md").ok()?;
        let raw = self.files.get(&idx)?;
        OKF_VER_VAL_RE
            .captures(raw)
            .map(|c| c[1].trim().to_string())
    }

    // --- lectura semántica ------------------------------------------------

    /// Filas del árbol de concepts con `orphan`/`invalid` resueltos (port de `fileRow`).
    pub fn list_concepts(&self) -> Vec<ConceptSummary> {
        let a = self.analyze();
        let orphan_set: BTreeSet<&RelPath> = a.orphans.iter().collect();
        a.concepts
            .iter()
            .map(|p| {
                let parsed = &self.parsed[p];
                let fm = parsed.fm.as_ref();
                let title = fm
                    .and_then(|f| f.title.clone())
                    .filter(|s| !s.is_empty())
                    .unwrap_or_else(|| model::title_from_path(p.as_str()));
                let invalid = a
                    .per_file
                    .get(p)
                    .map(|cs| cs.iter().any(|c| c.level == Severity::Err))
                    .unwrap_or(false);
                ConceptSummary {
                    path: p.clone(),
                    title,
                    r#type: fm.and_then(|f| f.r#type.clone()),
                    status: fm.and_then(|f| f.status.clone()),
                    orphan: orphan_set.contains(p),
                    invalid,
                }
            })
            .collect()
    }

    /// Vecindad de enlaces de un concept (port del panel de backlinks).
    pub fn backlinks(&self, p: &RelPath) -> Backlinks {
        let mut inbound: Vec<LinkRef> = Vec::new();
        // Quién enlaza aquí, con el href usado.
        for q in self.files.keys() {
            if q == p || q.is_reserved() {
                continue;
            }
            let body = &self.parsed[q].body;
            for cap in model::LINK_RE.captures_iter(body) {
                if let Some(href) = cap.get(1) {
                    if let Some(t) = model::resolve_link(href.as_str(), q.as_str()) {
                        if RelPath::new(&t).ok().as_ref() == Some(p) {
                            inbound.push(LinkRef {
                                path: q.clone(),
                                href: href.as_str().to_string(),
                            });
                            break;
                        }
                    }
                }
            }
        }
        // index.md que lo listan.
        let a = self.analyze();
        let index_refs: Vec<RelPath> = self
            .files
            .keys()
            .filter(|k| k.basename() == "index.md")
            .filter(|k| {
                let raw = &self.files[*k];
                model::out_links(k.as_str(), raw)
                    .into_iter()
                    .filter_map(|t| RelPath::new(&t).ok())
                    .any(|t| &t == p)
            })
            .cloned()
            .collect();
        // Salientes resueltos vs colgantes. Como el panel del proto (usa `analysis.out`, que
        // dedupea y excluye self), sin destinos reservados y sin hrefs colgantes repetidos.
        let mut out_resolved: Vec<RelPath> = Vec::new();
        let mut dangling: Vec<String> = Vec::new();
        let mut seen_out: BTreeSet<RelPath> = BTreeSet::new();
        let mut seen_dangling: BTreeSet<String> = BTreeSet::new();
        if let Some(parsed) = self.parsed.get(p) {
            for cap in model::LINK_RE.captures_iter(&parsed.body) {
                if let Some(href) = cap.get(1) {
                    if let Some(t) = model::resolve_link(href.as_str(), p.as_str()) {
                        match RelPath::new(&t) {
                            Ok(rp) if self.files.contains_key(&rp) => {
                                if rp != *p && !rp.is_reserved() && seen_out.insert(rp.clone()) {
                                    out_resolved.push(rp);
                                }
                            }
                            _ => {
                                let h = href.as_str().to_string();
                                if seen_dangling.insert(h.clone()) {
                                    dangling.push(h);
                                }
                            }
                        }
                    }
                }
            }
        }
        let _ = a;
        Backlinks {
            inbound,
            index_refs,
            out: out_resolved,
            dangling,
        }
    }

    /// Subgrafo dirigido alrededor de un concept.
    pub fn neighborhood(&self, p: &RelPath, depth: u32, dir: Direction) -> Neighborhood {
        crate::graph::neighborhood(self, p, depth, dir)
    }

    /// Modelo de grafo completo del bundle.
    pub fn graph_model(&self) -> GraphModel {
        crate::graph::graph_model(self)
    }

    /// Filtro de paths por la DSL de query (port fiel; devuelve paths).
    pub fn query(&self, dsl: &str) -> Vec<RelPath> {
        crate::query::query(self, dsl)
    }

    /// Acceso interno al fichero parseado (para los submódulos del core).
    pub(crate) fn parsed(&self, p: &RelPath) -> Option<&Parsed> {
        self.parsed.get(p)
    }

    // --- escritura validada (lógica OKF; la workspace aplica) --------------

    /// Valida un draft (contenido sin guardar) reutilizando el pipeline de análisis.
    pub fn validate_draft(&self, fm: &Frontmatter, body: &str) -> Vec<Check> {
        let raw = model::build_raw(fm, body);
        let draft_path = RelPath::new("__draft__.md").expect("path constante válido");
        let mut files = self.files.clone();
        files.insert(draft_path.clone(), raw);
        let tmp = Bundle::from_files(files);
        tmp.analyze()
            .per_file
            .get(&draft_path)
            .cloned()
            .unwrap_or_default()
    }

    /// Crea un concept validado. Rechaza por defecto si introduciría un `Err` (regla dura: `type`).
    pub fn create_concept(
        &self,
        p: &RelPath,
        ty: &str,
        title: Option<&str>,
        body: &str,
        allow_nonconformant: bool,
    ) -> WriteOutcome {
        let mut fm = Frontmatter {
            r#type: Some(ty.to_string()),
            status: Some("draft".to_string()),
            ..Default::default()
        };
        let resolved_title = title
            .map(|s| s.to_string())
            .unwrap_or_else(|| model::title_from_path(p.as_str()));
        fm.title = Some(resolved_title);
        let raw = model::build_raw(&fm, body);
        self.outcome_for_write(p, raw, allow_nonconformant)
    }

    /// Valida y prepara la escritura de contenido **crudo** en `p` (el editor guarda lo que el
    /// usuario tecleó, sin canonicalizar). Rechaza por defecto si introduciría un `Err`.
    pub fn write_concept_raw(
        &self,
        p: &RelPath,
        raw: &str,
        allow_nonconformant: bool,
    ) -> WriteOutcome {
        self.outcome_for_write(p, raw.to_string(), allow_nonconformant)
    }

    /// Aplica un patch de frontmatter (merge-patch RFC 7386: `Some` escribe, `None` borra).
    pub fn merge_frontmatter(&self, p: &RelPath, patch: FrontmatterPatch) -> WriteOutcome {
        let parsed = self.parsed.get(p);
        let mut fm = parsed.and_then(|x| x.fm.clone()).unwrap_or_default();
        let body = parsed.map(|x| x.body.clone()).unwrap_or_default();
        apply_patch(&mut fm, patch);
        let raw = model::build_raw(&fm, &body);
        self.outcome_for_write(p, raw, false)
    }

    /// Computa el `WriteOutcome` de escribir `raw` en `p`: hash, checks y rechazo si introduce `Err`.
    fn outcome_for_write(
        &self,
        p: &RelPath,
        raw: String,
        allow_nonconformant: bool,
    ) -> WriteOutcome {
        let hash = *blake3::hash(raw.as_bytes()).as_bytes();
        let mut files = self.files.clone();
        files.insert(p.clone(), raw.clone());
        let projected = Bundle::from_files(files);
        let analysis = projected.analyze();
        let checks = analysis.per_file.get(p).cloned().unwrap_or_default();
        let has_err = checks.iter().any(|c| c.level == Severity::Err);
        let rejected = if has_err && !allow_nonconformant {
            Some(format!(
                "La página no es conforme: {}",
                checks
                    .iter()
                    .filter(|c| c.level == Severity::Err)
                    .map(|c| c.code.as_str())
                    .collect::<Vec<_>>()
                    .join(", ")
            ))
        } else {
            None
        };
        WriteOutcome {
            path: p.clone(),
            raw,
            hash,
            written: rejected.is_none(),
            rejected,
            checks,
            bundle_hard_fail: analysis.hard_fail,
        }
    }
}

impl Bundle {
    /// Plan de generación del `index.md` de un directorio (`""` = root, `"sub/"` = subdir).
    pub fn gen_index(&self, dir: &str) -> crate::types::Mutation {
        crate::generate::gen_index(self, dir)
    }

    /// Plan de generación/purga de los índices de tags.
    pub fn gen_tag_indexes(&self) -> crate::types::Mutation {
        crate::generate::gen_tag_indexes(self)
    }

    /// Exporta el bundle a un `.zip` (sin zip-slip: las claves son `RelPath` validados).
    pub fn export_zip<W: std::io::Write + std::io::Seek>(&self, w: W) -> crate::error::Result<()> {
        use zip::write::SimpleFileOptions;
        let mut zip = zip::ZipWriter::new(w);
        let opts =
            SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        for (path, content) in &self.files {
            zip.start_file(path.as_str(), opts)
                .map_err(|e| crate::CoreError::Export(e.to_string()))?;
            std::io::Write::write_all(&mut zip, content.as_bytes())
                .map_err(|e| crate::CoreError::Export(e.to_string()))?;
        }
        zip.finish()
            .map_err(|e| crate::CoreError::Export(e.to_string()))?;
        Ok(())
    }
}

/// Aplica un `FrontmatterPatch` sobre un `Frontmatter` (conoce los KNOWN_FM tipados + extras).
fn apply_patch(fm: &mut Frontmatter, patch: FrontmatterPatch) {
    use serde_yaml::Value as Yaml;
    for (key, val) in patch.0 {
        let as_string = |v: &Yaml| match v {
            Yaml::String(s) => s.clone(),
            other => serde_yaml::to_string(other)
                .unwrap_or_default()
                .trim()
                .to_string(),
        };
        // Escribir o borrar un known string invalida su marca de null explícito.
        let clear_null = |fm: &mut Frontmatter, k: &str| fm.known_null.retain(|n| n != k);
        match key.as_str() {
            "type" => {
                clear_null(fm, "type");
                fm.r#type = val.as_ref().map(as_string);
            }
            "title" => {
                clear_null(fm, "title");
                fm.title = val.as_ref().map(as_string);
            }
            "description" => {
                clear_null(fm, "description");
                fm.description = val.as_ref().map(as_string);
            }
            "resource" => {
                clear_null(fm, "resource");
                fm.resource = val.as_ref().map(as_string);
            }
            "status" => {
                clear_null(fm, "status");
                fm.status = val.as_ref().map(as_string);
            }
            "tags" => fm.tags = val,
            "timestamp" => fm.timestamp = val,
            _ => match val {
                Some(v) => {
                    fm.extra.insert(key, v);
                }
                None => {
                    // `shift_remove` (no `swap_remove`) para conservar el orden de las claves restantes.
                    fm.extra.shift_remove(&key);
                }
            },
        }
    }
}
