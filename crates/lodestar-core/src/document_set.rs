//! El agregado [`DocumentSet`] (`ARCHITECTURE.md §4.2`): construcción desde un `FileMap`,
//! `analyze()` cacheado y la superficie de lectura/escritura semántica.

use std::collections::{BTreeMap, BTreeSet};

use crate::conform::{self, ConformCtx};
use crate::model::{self, Parsed};
use crate::types::{
    Analysis, Backlinks, Check, Direction, DocumentSummary, FrontmatterPatch, GraphModel,
    GraphNode, LinkRef, Neighborhood, ParsedFrontmatter, RelPath, Severity, WriteOutcome,
};

/// Workspace: un mapa de ficheros + el análisis derivado (cacheado).
pub struct DocumentSet {
    files: crate::types::FileMap,
    parsed: BTreeMap<RelPath, Parsed>,
    analysis: once_cell::sync::OnceCell<Analysis>,
}

impl DocumentSet {
    /// Construye un `DocumentSet` desde un `FileMap`. Parsea cada fichero una vez (puro, sin I/O).
    pub fn from_files(files: crate::types::FileMap) -> Self {
        let parsed = files
            .iter()
            .map(|(p, raw)| (p.clone(), model::parse_file(p.as_str(), raw)))
            .collect();
        DocumentSet {
            files,
            parsed,
            analysis: once_cell::sync::OnceCell::new(),
        }
    }

    /// El mapa de ficheros subyacente.
    pub fn files(&self) -> &crate::types::FileMap {
        &self.files
    }

    /// Análisis del workspace, cacheado con `OnceCell` (recomputar es idempotente).
    pub fn analyze(&self) -> &Analysis {
        self.analysis.get_or_init(|| self.compute_analysis())
    }

    /// Computa el análisis: **todos** los `.md` son nodos (E16-H02, `§20.7`) — ningún basename
    /// se salta el grafo ni recibe trato aparte.
    fn compute_analysis(&self) -> Analysis {
        let mut documents: Vec<RelPath> = Vec::new();
        let mut out: BTreeMap<RelPath, Vec<RelPath>> = BTreeMap::new();
        let mut inn: BTreeMap<RelPath, Vec<RelPath>> = BTreeMap::new();
        let mut dangling_set: BTreeSet<RelPath> = BTreeSet::new();

        // Adyacencia saliente: el cuerpo de cada documento, sin excepciones por nombre.
        for path in self.files.keys() {
            documents.push(path.clone());
            let body = &self.parsed[path].body;
            let targets: Vec<RelPath> = model::out_links(path.as_str(), body)
                .into_iter()
                .filter_map(|t| RelPath::new(&t).ok())
                .collect();
            out.insert(path.clone(), targets);
        }

        for p in &documents {
            inn.entry(p.clone()).or_default();
        }
        // Inversión de aristas + dangling. Un enlace a un documento que existe es un entrante
        // suyo, venga de donde venga; uno a un `.md` que no existe es colgante.
        for p in &documents {
            for t in out.get(p).cloned().unwrap_or_default() {
                if self.files.contains_key(&t) {
                    inn.entry(t.clone()).or_default().push(p.clone());
                } else {
                    dangling_set.insert(t.clone());
                }
            }
        }

        // Conformidad por fichero.
        let ctx = ConformCtx {
            files: &self.files,
            out: &out,
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

        // Aislados (`§20.7`): ni entrantes ni salientes. Un enlace saliente cuenta aunque su
        // destino no exista todavía — el documento participa en el grafo (tiene un nodo ghost por
        // vecino), que es lo que «aislado» niega.
        let isolated: Vec<RelPath> = documents
            .iter()
            .filter(|p| {
                inn.get(*p).map(|v| v.is_empty()).unwrap_or(true)
                    && out.get(*p).map(|v| v.is_empty()).unwrap_or(true)
            })
            .cloned()
            .collect();

        let dangling: Vec<RelPath> = dangling_set.into_iter().collect();

        Analysis {
            documents,
            out,
            inn,
            dangling,
            isolated,
            per_file,
            hard_fail,
            warn_count,
        }
    }

    // --- lectura semántica ------------------------------------------------

    /// Filas del árbol de documentos con `isolated`/`invalid` resueltos.
    pub fn list_documents(&self) -> Vec<DocumentSummary> {
        let a = self.analyze();
        let isolated_set: BTreeSet<&RelPath> = a.isolated.iter().collect();
        a.documents
            .iter()
            .map(|p| {
                let parsed = &self.parsed[p];
                let fm = parsed.frontmatter.as_ref();
                let title = model::derived_title(fm, &parsed.body, p);
                let invalid = a
                    .per_file
                    .get(p)
                    .map(|cs| cs.iter().any(|c| c.level == Severity::Err))
                    .unwrap_or(false);
                DocumentSummary {
                    path: p.clone(),
                    title,
                    r#type: fm.and_then(|f| f.get_text("type")),
                    status: fm.and_then(|f| f.get_text("status")),
                    isolated: isolated_set.contains(p),
                    invalid,
                }
            })
            .collect()
    }

    /// Vecindad de enlaces de un documento.
    ///
    /// Desde E16-H02 **no** hay `index_refs`: quien te enlaza entra por `inbound`, sea un
    /// `index.md` o cualquier otro documento.
    pub fn backlinks(&self, p: &RelPath) -> Backlinks {
        let mut inbound: Vec<LinkRef> = Vec::new();
        // Quién enlaza aquí, con el href usado.
        for q in self.files.keys() {
            if q == p {
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
        // Salientes resueltos vs colgantes: `analysis.out` dedupea y excluye el self-enlace, y
        // los hrefs colgantes no se repiten.
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
                                if rp != *p && seen_out.insert(rp.clone()) {
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
        Backlinks {
            inbound,
            out: out_resolved,
            dangling,
        }
    }

    /// Subgrafo dirigido alrededor de un documento.
    pub fn neighborhood(&self, p: &RelPath, depth: u32, dir: Direction) -> Neighborhood {
        crate::graph::neighborhood(self, p, depth, dir)
    }

    /// Modelo de grafo completo del workspace.
    pub fn graph_model(&self) -> GraphModel {
        crate::graph::graph_model(self)
    }

    /// [`GraphNode`] de `id` (ghost/type/status), reusando `graph::node_for` — la única definición
    /// de "qué es un nodo del grafo" (invariante #3). Envoltorio público para que las fachadas
    /// (`lodestar-app`) no reimplementen ese criterio ni necesiten acceso a `parsed` (`pub(crate)`).
    pub fn node(&self, id: &RelPath) -> GraphNode {
        crate::graph::node_for(self, id)
    }

    /// Camino más corto **dirigido** de `a` a `b` (`[a, .., b]`), o `vec![]` si no hay camino
    /// (E11-H02). Nunca error. Ver `graph::path_between`.
    pub fn path_between(&self, a: &RelPath, b: &RelPath) -> Vec<RelPath> {
        crate::graph::path_between(self, a, b)
    }

    /// Ciclos dirigidos del grafo de enlaces; cada ciclo es el conjunto de nodos de una SCC no
    /// trivial (E11-H02). Ver `graph::cycles`.
    pub fn cycles(&self) -> Vec<Vec<RelPath>> {
        crate::graph::cycles(self)
    }

    /// Componentes conexas (conectividad no dirigida) del grafo de enlaces; cada componente es el
    /// conjunto de sus nodos (E11-H02). Ver `graph::components`.
    pub fn components(&self) -> Vec<Vec<RelPath>> {
        crate::graph::components(self)
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
    pub fn validate_draft(&self, fm: Option<&ParsedFrontmatter>, body: &str) -> Vec<Check> {
        let raw = model::build_raw(fm, body);
        let draft_path = RelPath::new("__draft__.md").expect("path constante válido");
        let mut files = self.files.clone();
        files.insert(draft_path.clone(), raw);
        let tmp = DocumentSet::from_files(files);
        tmp.analyze()
            .per_file
            .get(&draft_path)
            .cloned()
            .unwrap_or_default()
    }

    /// Crea un documento validado. Rechaza por defecto si introduciría un `Err` (regla dura: `type`).
    ///
    /// `timestamp` es el instante de creación en ISO-8601 (paridad con el prototipo, que escribe
    /// `timestamp: now()`). El core es **puro**: no computa el reloj; el llamante (la workspace,
    /// único escritor con I/O) inyecta el valor. `None` omite la clave.
    ///
    /// Si `body` está vacío (tras `trim`) se genera un heading por defecto único en el core: con
    /// `ty` no vacío `# {ty} - {título}\n`, y `# {título}\n` cuando `ty` está vacío. El `título` es
    /// el `title` recibido o, en su defecto, el último eslabón de
    /// [`model::derived_title`] — el nombre del fichero sin `.md` (el documento aún no tiene ni
    /// frontmatter ni cuerpo de los que derivarlo). Las fachadas no inyectan plantilla: pasan `""`
    /// para delegar aquí el default.
    pub fn create_document(
        &self,
        p: &RelPath,
        ty: &str,
        title: Option<&str>,
        body: &str,
        timestamp: Option<&str>,
        allow_nonconformant: bool,
    ) -> WriteOutcome {
        let resolved_title = title
            .map(|s| s.to_string())
            .unwrap_or_else(|| model::derived_title(None, "", p));
        let default_body = if ty.is_empty() {
            format!("# {resolved_title}\n")
        } else {
            format!("# {ty} - {resolved_title}\n")
        };
        // Orden de inserción = orden en el `.md` (el `Mapping` de serde_yaml preserva el de
        // aparición): `type`, `title`, `timestamp` y `status`, como hacía la canonicalización OKF.
        let mut map = serde_yaml::Mapping::new();
        let mut set = |k: &str, v: serde_yaml::Value| {
            map.insert(serde_yaml::Value::String(k.to_string()), v);
        };
        set("type", serde_yaml::Value::String(ty.to_string()));
        set("title", serde_yaml::Value::String(resolved_title));
        if let Some(ts) = timestamp {
            set("timestamp", serde_yaml::Value::String(ts.to_string()));
        }
        set("status", serde_yaml::Value::String("draft".to_string()));
        let fm = ParsedFrontmatter::from_mapping(map);
        let body = if body.trim().is_empty() {
            &default_body
        } else {
            body
        };
        let raw = model::build_raw(Some(&fm), body);
        self.outcome_for_write(p, raw, allow_nonconformant)
    }

    /// Valida y prepara la escritura de contenido **crudo** en `p` (el editor guarda lo que el
    /// usuario tecleó, sin canonicalizar). Rechaza por defecto si introduciría un `Err`.
    pub fn write_document_raw(
        &self,
        p: &RelPath,
        raw: &str,
        allow_nonconformant: bool,
    ) -> WriteOutcome {
        self.outcome_for_write(p, raw.to_string(), allow_nonconformant)
    }

    /// Aplica un patch de frontmatter (merge-patch RFC 7386: `Some` escribe, `None` borra).
    ///
    /// Delega en [`model::patch_frontmatter`] (E16-H04, invariante #3: una sola verdad de
    /// patcheo), así que la edición es **quirúrgica** siempre que se puede: las líneas que el
    /// patch no toca llegan al `.md` byte a byte. Si el frontmatter del documento no es
    /// interpretable, la escritura se **rechaza** (`written: false`) en vez de reconstruir el
    /// bloque encima y borrar la metadata del usuario.
    pub fn merge_frontmatter(&self, p: &RelPath, patch: FrontmatterPatch) -> WriteOutcome {
        let previo = self.files.get(p).cloned().unwrap_or_default();
        match model::patch_frontmatter(&previo, &patch) {
            Ok(patched) => self.outcome_for_write(p, patched.raw, false),
            Err(e) => WriteOutcome {
                path: p.clone(),
                raw: previo.clone(),
                hash: *blake3::hash(previo.as_bytes()).as_bytes(),
                written: false,
                rejected: Some(e.to_string()),
                checks: self.analyze().per_file.get(p).cloned().unwrap_or_default(),
                workspace_hard_fail: self.analyze().hard_fail,
            },
        }
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
        let projected = DocumentSet::from_files(files);
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
            workspace_hard_fail: analysis.hard_fail,
        }
    }
}

/// Aplica un `FrontmatterPatch` (merge-patch RFC 7386) sobre el mapa YAML de un frontmatter:
/// `Some(v)` escribe/reemplaza el valor **tal cual** (sin coercionarlo a string), `None` borra la
/// clave, y una clave ausente del patch no se toca.
///
/// Escribir sobre una clave existente conserva su posición y borrar usa `shift_remove` (no
/// `swap_remove`): el orden de aparición del resto de claves queda intacto.
///
/// `pub(crate)`: además de [`DocumentSet::merge_frontmatter`], lo reutiliza `crate::plan`
/// (E12-H08, `apply_normalized_ops`) para materializar en memoria un `Create`/`PatchFrontmatter`
/// sobre el `FileMap` hipotético — una sola lógica de merge-patch en todo el core (invariante #3
/// de `CLAUDE.md`), nunca reimplementada.
pub(crate) fn apply_patch(map: &mut serde_yaml::Mapping, patch: FrontmatterPatch) {
    for (key, val) in patch.0 {
        let key = serde_yaml::Value::String(key);
        match val {
            Some(v) => {
                map.insert(key, v);
            }
            None => {
                map.shift_remove(&key);
            }
        }
    }
}
