//! El agregado [`DocumentSet`] (`ARCHITECTURE.md §4.2`): construcción desde un `FileMap`,
//! `analyze()` cacheado y la superficie de lectura/escritura semántica.

use std::collections::{BTreeMap, BTreeSet};

use crate::conform;
use crate::links;
use crate::model::{self, Parsed};
use crate::types::{
    Analysis, Backlinks, Check, DanglingLink, Direction, DocumentSummary, FrontmatterPatch,
    GraphModel, GraphNode, Inventory, LinkReference, LinkTarget, Neighborhood, ParsedFrontmatter,
    RelPath, ResolvedLink, Severity, WriteOutcome,
};

/// Workspace: un mapa de ficheros + el análisis derivado (cacheado).
pub struct DocumentSet {
    files: crate::types::FileMap,
    parsed: BTreeMap<RelPath, Parsed>,
    inventory: Inventory,
    analysis: once_cell::sync::OnceCell<Analysis>,
}

impl DocumentSet {
    /// Construye un `DocumentSet` desde un `FileMap`. Parsea cada fichero una vez (puro, sin I/O).
    ///
    /// El inventario que ve la resolución de enlaces son **solo** estos documentos: un enlace a un
    /// fichero del proyecto que no sea `.md` se clasificará [`LinkTarget::Missing`]. Para declarar
    /// esos ficheros, [`DocumentSet::with_other_files`].
    pub fn from_files(files: crate::types::FileMap) -> Self {
        let inventory = Inventory::from_documents(&files);
        Self::build(files, inventory)
    }

    /// Como [`DocumentSet::from_files`], pero declarando además los **ficheros del proyecto que no
    /// son documentos** (código, imágenes, `.md` excluidos del descubrimiento…).
    ///
    /// Sin ellos, [`crate::links::resolve`] no puede distinguir un [`LinkTarget::WorkspaceFile`]
    /// —el fichero existe, aunque no sea nodo del grafo— de un [`LinkTarget::Missing`], y un enlace
    /// a código produciría un `LINK-TARGET-MISSING` espurio (`§20.6`, precisión 2).
    pub fn with_other_files<I>(files: crate::types::FileMap, other_files: I) -> Self
    where
        I: IntoIterator<Item = RelPath>,
    {
        let inventory = Inventory::new(files.keys().cloned(), other_files);
        Self::build(files, inventory)
    }

    /// Construye el agregado: parsea cada fichero una vez y guarda el inventario con el que se
    /// resolverán los enlaces.
    fn build(files: crate::types::FileMap, inventory: Inventory) -> Self {
        let parsed = files
            .iter()
            .map(|(p, raw)| (p.clone(), model::parse_file(p.as_str(), raw)))
            .collect();
        DocumentSet {
            files,
            parsed,
            inventory,
            analysis: once_cell::sync::OnceCell::new(),
        }
    }

    /// El mapa de ficheros subyacente.
    pub fn files(&self) -> &crate::types::FileMap {
        &self.files
    }

    /// El inventario con el que se resuelven los enlaces: los documentos de este `FileMap` más los
    /// ficheros del proyecto declarados en [`DocumentSet::with_other_files`].
    pub fn inventory(&self) -> &Inventory {
        &self.inventory
    }

    /// Análisis del workspace, cacheado con `OnceCell` (recomputar es idempotente).
    pub fn analyze(&self) -> &Analysis {
        self.analysis.get_or_init(|| self.compute_analysis())
    }

    /// Computa el **grafo universal** de `§20.7` (E17-H04): nodos = todos los documentos
    /// descubiertos, aristas = los enlaces resueltos entre ellos. Ningún basename se salta el
    /// grafo ni recibe trato aparte (E16-H02).
    ///
    /// Un solo recorrido resuelve cada enlace **una vez** (invariante #3): de ahí salen
    /// `outgoing`, su inversa `incoming`, los colgantes, los aislados y los diagnósticos de
    /// enlace, sin que ninguna de esas vistas recalcule nada por su cuenta.
    fn compute_analysis(&self) -> Analysis {
        let documents: Vec<RelPath> = self.files.keys().cloned().collect();

        // Salientes: TODOS los enlaces del cuerpo, en orden de aparición y ya clasificados.
        let mut outgoing: BTreeMap<RelPath, Vec<ResolvedLink>> = BTreeMap::new();
        for path in &documents {
            let body = &self.parsed[path].body;
            let resueltos: Vec<ResolvedLink> = links::extract_links(body)
                .iter()
                .map(|raw| links::resolve(raw, path, &self.inventory))
                .collect();
            outgoing.insert(path.clone(), resueltos);
        }

        // Inversa + colgantes. Un enlace a un documento del inventario es un entrante suyo, venga
        // de donde venga; uno a un destino contenido que no existe es un colgante con su origen.
        let mut incoming: BTreeMap<RelPath, Vec<LinkReference>> =
            documents.iter().map(|p| (p.clone(), Vec::new())).collect();
        let mut dangling: Vec<DanglingLink> = Vec::new();
        for from in &documents {
            for link in &outgoing[from] {
                match &link.target {
                    LinkTarget::Document(t) => {
                        // `Document` significa «está en el inventario»; solo los del `FileMap` son
                        // nodos con entrada propia (un `.md` excluido del descubrimiento no lo es).
                        if let Some(refs) = incoming.get_mut(t) {
                            refs.push(LinkReference {
                                from: from.clone(),
                                link: link.clone(),
                            });
                        }
                    }
                    LinkTarget::Missing(t) => dangling.push(DanglingLink {
                        from: from.clone(),
                        target: t.clone(),
                        link: link.clone(),
                    }),
                    _ => {}
                }
            }
        }

        // Diagnósticos por documento: los del documento en sí (`§20.9`) más los de sus enlaces
        // (E17-H03), que necesitan el inventario completo y por eso se emiten aquí.
        let mut diagnostics: BTreeMap<RelPath, Vec<Check>> = BTreeMap::new();
        for (path, raw) in &self.files {
            let mut checks = conform::validate_file(path, &self.parsed[path], raw);
            checks.extend(links::diagnose(path, raw, &outgoing[path], &self.inventory));
            diagnostics.insert(path.clone(), checks);
        }

        // Aislados (`§20.7`): sin enlaces INTERNOS entrantes ni salientes. Interno = `Document`
        // (arista real) o `Missing` (arista a un fantasma: el documento participa en el grafo). Un
        // enlace externo, un anchor propio o uno a código no conectan con ningún documento.
        let isolated: Vec<RelPath> = documents
            .iter()
            .filter(|p| {
                incoming[*p].is_empty() && !outgoing[*p].iter().any(|l| l.target.is_internal())
            })
            .cloned()
            .collect();

        Analysis {
            documents,
            outgoing,
            incoming,
            isolated,
            dangling,
            diagnostics,
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
                    .diagnostics
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

    /// Vecindad de enlaces de un documento: su porción de [`Analysis::incoming`]/
    /// [`Analysis::outgoing`] tal cual, **sin recalcular nada** (invariante #3).
    ///
    /// Desde E16-H02 **no** hay `index_refs`: quien te enlaza entra por `inbound`, sea un
    /// `index.md` o cualquier otro documento. Desde E17-H05 `inbound` lleva una entrada por
    /// **enlace** (un origen que enlaza dos veces aparece dos veces, con sus dos hrefs) y `out`
    /// son los enlaces resueltos completos, colgantes y externos incluidos.
    pub fn backlinks(&self, p: &RelPath) -> Backlinks {
        let a = self.analyze();
        Backlinks {
            inbound: a.incoming.get(p).cloned().unwrap_or_default(),
            out: a.outgoing.get(p).cloned().unwrap_or_default(),
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
            .diagnostics
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
                checks: self
                    .analyze()
                    .diagnostics
                    .get(p)
                    .cloned()
                    .unwrap_or_default(),
                workspace_hard_fail: self.analyze().hard_fail(),
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
        let checks = analysis.diagnostics.get(p).cloned().unwrap_or_default();
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
            workspace_hard_fail: analysis.hard_fail(),
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
