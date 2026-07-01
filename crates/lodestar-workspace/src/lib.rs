//! `lodestar-workspace` — el handle unificado (`ARCHITECTURE.md §6`).
//!
//! Compone `lodestar-core` (puro) + `lodestar-vcs`. Es lo que ven las fachadas. Es el **único
//! escritor**: los comandos nunca escriben la cache; escriben el `.md` (atómico temp+rename).
//!
//! Nota de fase: la cache incremental (`lodestar-store`: SQLite/FTS5 + watcher, E3) es la capa de
//! aceleración. Mientras no esté cableada, la workspace **recarga desde disco** bajo demanda — el
//! core es la autoridad y la cache es derivada/desechable (`§2.3`, `§10` fila 1), así que el resultado
//! es correcto, solo que no incremental.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use crossbeam_channel::Receiver;
use lodestar_core::diff::OkfDiff;
use lodestar_core::types::{
    Analysis, Author, Backlinks, Branch, CommitConformance, CommitRow, Direction, FileMap,
    FrontmatterPatch, GraphModel, Mutation, Neighborhood, RelPath, Sha, SyncOutcome, WriteOutcome,
};
use lodestar_core::Bundle;
use lodestar_store::{IndexEvent, Store, Watcher};
use lodestar_vcs::{MergeOutcome, Vcs};

pub mod config;
mod error;
mod io;
mod snapshot;

pub use config::Config;
pub use error::WorkspaceError;
pub use snapshot::BundleSnapshot;

/// Handle unificado de un bundle abierto.
pub struct Workspace {
    root: PathBuf,
    vcs: Option<Mutex<Vcs>>,
    identity: Author,
    /// Cache incremental (SQLite/FTS5). `None` en modo efímero (CLI one-shot).
    cache: Option<Arc<Store>>,
    /// Watcher vivo (mantiene la observación de disco mientras exista).
    _watcher: Option<Watcher>,
}

impl Workspace {
    /// Abre un bundle: descubre git (puede no haber). Identidad por defecto (override en E8-H01).
    /// **No** activa la cache incremental (usa [`Workspace::open_live`] o [`Workspace::enable_cache`]).
    pub fn open(root: &Path) -> Result<Self, WorkspaceError> {
        let vcs = Vcs::discover(root)?.map(Mutex::new);
        // La identidad de `lodestar.toml` (si existe) tiene prioridad sobre el defecto.
        let identity = Config::load(root)
            .ok()
            .and_then(|c| c.author())
            .unwrap_or_else(default_identity);
        Ok(Workspace {
            root: root.to_path_buf(),
            vcs,
            identity,
            cache: None,
            _watcher: None,
        })
    }

    /// La configuración efectiva del bundle (`lodestar.toml` + defaults).
    pub fn config(&self) -> Config {
        Config::load(&self.root).unwrap_or_default()
    }

    /// Abre sin git (modo hermético, p. ej. CLI efímera).
    pub fn open_ephemeral(root: &Path) -> Result<Self, WorkspaceError> {
        Ok(Workspace {
            root: root.to_path_buf(),
            vcs: None,
            identity: default_identity(),
            cache: None,
            _watcher: None,
        })
    }

    /// Abre un bundle **en vivo**: git + cache incremental construida + watcher arrancado.
    /// Es lo que usan las fachadas interactivas (Tauri/MCP) para recibir `IndexEvent`.
    pub fn open_live(root: &Path) -> Result<Self, WorkspaceError> {
        let mut ws = Workspace::open(root)?;
        ws.enable_cache()?;
        Ok(ws)
    }

    /// Activa (si no lo está) la cache incremental: abre `.lodestar/index.db`, la reconstruye
    /// desde disco y arranca el watcher (único escritor de la cache).
    pub fn enable_cache(&mut self) -> Result<(), WorkspaceError> {
        if self.cache.is_some() {
            return Ok(());
        }
        let store = Arc::new(Store::open_and_build(&self.root)?);
        let watcher = store.watch()?;
        self.cache = Some(store);
        self._watcher = Some(watcher);
        Ok(())
    }

    /// Suscribe un receptor de [`IndexEvent`] del bus de la cache. Error si la cache no está activa.
    pub fn subscribe(&self) -> Result<Receiver<IndexEvent>, WorkspaceError> {
        self.cache
            .as_ref()
            .map(|s| s.subscribe())
            .ok_or(WorkspaceError::NoCache)
    }

    /// Acceso a la cache incremental (para consultas aceleradas: backlinks/orphans/FTS).
    pub fn cache(&self) -> Option<&Arc<Store>> {
        self.cache.as_ref()
    }

    /// Update **optimista** de la cache tras una escritura por el único escritor (`§10` fila 19):
    /// la UI ve el cambio al instante; el watcher reconcilia después (no-op por el gate de hash).
    fn cache_upsert(&self, path: &RelPath, content: &str) {
        if let Some(store) = &self.cache {
            let _ = store.upsert(path, content, 0, content.len() as i64);
        }
    }

    fn cache_remove(&self, path: &RelPath) {
        if let Some(store) = &self.cache {
            let _ = store.remove(path);
        }
    }

    /// Fija la identidad de los commits (autor/committer).
    pub fn set_identity(&mut self, author: Author) {
        self.identity = author;
    }

    /// `true` si el bundle tiene un repo git.
    pub fn has_vcs(&self) -> bool {
        self.vcs.is_some()
    }

    /// Inicializa git en el bundle (first-run / "activar git").
    pub fn init_vcs(&mut self) -> Result<(), WorkspaceError> {
        let vcs = Vcs::init(&self.root, &self.identity)?;
        self.vcs = Some(Mutex::new(vcs));
        Ok(())
    }

    // --- lectura ----------------------------------------------------------

    /// Carga el bundle desde disco (el core es la autoridad).
    pub fn bundle(&self) -> Result<Bundle, WorkspaceError> {
        Ok(Bundle::from_files(io::load_bundle(&self.root)?))
    }

    /// Snapshot unificado: files + analysis + graph, todo junto.
    pub fn snapshot(&self) -> Result<BundleSnapshot, WorkspaceError> {
        let bundle = self.bundle()?;
        Ok(BundleSnapshot {
            files: bundle.files().clone(),
            analysis: bundle.analyze().clone(),
            graph: bundle.graph_model(),
        })
    }

    /// Análisis (conformidad/grafo derivados).
    pub fn analyze(&self) -> Result<Analysis, WorkspaceError> {
        Ok(self.bundle()?.analyze().clone())
    }

    /// Vecindad de enlaces de un concept.
    pub fn backlinks(&self, p: &RelPath) -> Result<Backlinks, WorkspaceError> {
        Ok(self.bundle()?.backlinks(p))
    }

    /// Subgrafo dirigido alrededor de un concept.
    pub fn neighborhood(
        &self,
        p: &RelPath,
        depth: u32,
        dir: Direction,
    ) -> Result<Neighborhood, WorkspaceError> {
        Ok(self.bundle()?.neighborhood(p, depth, dir))
    }

    /// Grafo completo.
    pub fn graph_model(&self) -> Result<GraphModel, WorkspaceError> {
        Ok(self.bundle()?.graph_model())
    }

    /// Query estructurada (devuelve paths).
    pub fn query(&self, dsl: &str) -> Result<Vec<RelPath>, WorkspaceError> {
        Ok(self.bundle()?.query(dsl))
    }

    // --- escritura validada (por el ÚNICO escritor) -----------------------

    /// Crea un concept validado y lo escribe por el único escritor (si es conforme).
    pub fn create_concept(
        &self,
        p: &RelPath,
        ty: &str,
        title: Option<&str>,
        body: &str,
        allow_nonconformant: bool,
    ) -> Result<WriteOutcome, WorkspaceError> {
        let bundle = self.bundle()?;
        let outcome = bundle.create_concept(p, ty, title, body, allow_nonconformant);
        if outcome.written {
            io::write_atomic(&self.root, &outcome.path, &outcome.raw)?;
            self.cache_upsert(&outcome.path, &outcome.raw);
        }
        Ok(outcome)
    }

    /// Escribe contenido **crudo** en un concept (editor multi-escritor), validado por el core.
    /// Rechazo = `written:false` (no un `Err`). Escribe por el único escritor si es conforme.
    pub fn write_concept(
        &self,
        p: &RelPath,
        raw: &str,
        allow_nonconformant: bool,
    ) -> Result<WriteOutcome, WorkspaceError> {
        let bundle = self.bundle()?;
        let outcome = bundle.write_concept_raw(p, raw, allow_nonconformant);
        if outcome.written {
            io::write_atomic(&self.root, &outcome.path, &outcome.raw)?;
            self.cache_upsert(&outcome.path, &outcome.raw);
        }
        Ok(outcome)
    }

    /// Lee el contenido crudo de un concept desde disco.
    pub fn read_concept(&self, p: &RelPath) -> Result<String, WorkspaceError> {
        std::fs::read_to_string(self.root.join(p.as_str()))
            .map_err(|e| WorkspaceError::Io(e.to_string()))
    }

    /// Lista las filas del árbol de concepts (título/orphan/invalid resueltos por el core).
    pub fn list_concepts(
        &self,
    ) -> Result<Vec<lodestar_core::types::ConceptSummary>, WorkspaceError> {
        Ok(self.bundle()?.list_concepts())
    }

    /// Aplica un patch de frontmatter (null-borra) y lo escribe si es conforme.
    pub fn merge_frontmatter(
        &self,
        p: &RelPath,
        patch: FrontmatterPatch,
    ) -> Result<WriteOutcome, WorkspaceError> {
        let bundle = self.bundle()?;
        let outcome = bundle.merge_frontmatter(p, patch);
        if outcome.written {
            io::write_atomic(&self.root, &outcome.path, &outcome.raw)?;
            self.cache_upsert(&outcome.path, &outcome.raw);
        }
        Ok(outcome)
    }

    /// Aplica una `Mutation` por el único escritor y devuelve `{written, removed, unchanged}`.
    pub fn apply_mutation(&self, mutation: &Mutation) -> Result<ApplyReport, WorkspaceError> {
        let mut written = 0;
        let mut unchanged = 0;
        for (path, content) in &mutation.writes {
            let on_disk = std::fs::read_to_string(self.root.join(path.as_str())).ok();
            if on_disk.as_deref() == Some(content.as_str()) {
                unchanged += 1;
            } else {
                io::write_atomic(&self.root, path, content)?;
                self.cache_upsert(path, content);
                written += 1;
            }
        }
        let mut removed = 0;
        for path in &mutation.deletes {
            if self.root.join(path.as_str()).exists() {
                io::delete(&self.root, path)?;
                self.cache_remove(path);
                removed += 1;
            }
        }
        Ok(ApplyReport {
            written,
            removed,
            unchanged,
        })
    }

    /// Genera y aplica el `index.md` de un directorio.
    pub fn generate_index(&self, dir: &str) -> Result<ApplyReport, WorkspaceError> {
        let mutation = self.bundle()?.gen_index(dir);
        self.apply_mutation(&mutation)
    }

    /// Genera y aplica los índices de tags (purga obsoletos).
    pub fn generate_tags(&self) -> Result<ApplyReport, WorkspaceError> {
        let mutation = self.bundle()?.gen_tag_indexes();
        self.apply_mutation(&mutation)
    }

    /// Exporta el bundle a un `.zip`.
    pub fn export<W: std::io::Write + std::io::Seek>(&self, w: W) -> Result<(), WorkspaceError> {
        self.bundle()?.export_zip(w).map_err(WorkspaceError::from)
    }

    // --- git (vía lodestar-vcs) -------------------------------------------

    /// Conformidad del HEAD actual (usa la cache por tree-oid si está activa).
    pub fn conformance(&self) -> Result<Option<CommitConformance>, WorkspaceError> {
        let head = {
            let guard = match &self.vcs {
                Some(v) => v.lock().unwrap(),
                None => return Ok(None),
            };
            guard.log(1)?.first().map(|r| r.id.clone())
        };
        match head {
            Some(sha) => Ok(Some(self.conformance_of(&sha)?)),
            None => Ok(None),
        }
    }

    /// Conformidad de un commit concreto, cacheada por `tree_oid` en el store (`§10` fila 20):
    /// solo recomputa (analyze sobre el árbol) en el primer acceso; luego sirve de la cache.
    pub fn conformance_of(&self, sha: &Sha) -> Result<CommitConformance, WorkspaceError> {
        let guard = self
            .vcs
            .as_ref()
            .ok_or(WorkspaceError::NoVcs)?
            .lock()
            .unwrap();
        let tree_oid = guard.tree_oid(sha)?;
        if let Some(store) = &self.cache {
            if let Some(cached) = store.get_conformance(&tree_oid)? {
                return Ok(cached);
            }
            let computed = guard.conformance(sha)?;
            store.put_conformance(&tree_oid, &computed)?;
            return Ok(computed);
        }
        Ok(guard.conformance(sha)?)
    }

    /// Crea una rama local (opcionalmente desde un `Sha`). No toca el working tree.
    pub fn create_branch(&self, name: &str, from: Option<&Sha>) -> Result<(), WorkspaceError> {
        self.vcs
            .as_ref()
            .ok_or(WorkspaceError::NoVcs)?
            .lock()
            .unwrap()
            .create_branch(name, from)?;
        Ok(())
    }

    /// Cambia de rama por el único escritor: checkpoint si hay trabajo sin commitear, mueve `HEAD`,
    /// aplica el árbol destino y regenera index/tags (`§16`).
    pub fn switch(&self, name: &str) -> Result<ApplyReport, WorkspaceError> {
        let target_files = {
            let guard = self
                .vcs
                .as_ref()
                .ok_or(WorkspaceError::NoVcs)?
                .lock()
                .unwrap();
            if !guard.dirty_paths()?.is_empty() {
                guard.commit(
                    "Checkpoint automático antes de cambiar de rama",
                    &self.identity,
                )?;
            }
            guard.switch(name)?
        };
        let current = io::load_bundle(&self.root)?;
        let report = self.apply_mutation(&restore_mutation(&current, &target_files))?;
        self.generate_index("")?;
        self.generate_tags()?;
        Ok(report)
    }

    /// Merge (local) de una rama en la actual, por el único escritor. Checkpoint previo; en
    /// conflicto deja marcadores (`OKF-CONFLICT`) y `MERGE_HEAD` (bloquea el commit hasta resolver).
    pub fn merge(&self, name: &str) -> Result<MergeReport, WorkspaceError> {
        let outcome: MergeOutcome = {
            let guard = self
                .vcs
                .as_ref()
                .ok_or(WorkspaceError::NoVcs)?
                .lock()
                .unwrap();
            if !guard.dirty_paths()?.is_empty() {
                guard.commit("Checkpoint automático antes de hacer merge", &self.identity)?;
            }
            guard.merge(name)?
        };
        let report = if outcome.up_to_date {
            ApplyReport {
                written: 0,
                removed: 0,
                unchanged: 0,
            }
        } else {
            let current = io::load_bundle(&self.root)?;
            self.apply_mutation(&restore_mutation(&current, &outcome.files))?
        };
        // Solo regenera artefactos si el merge quedó limpio (con conflictos, primero se resuelve).
        if outcome.conflicted.is_empty() && !outcome.up_to_date {
            self.generate_index("")?;
            self.generate_tags()?;
        }
        Ok(MergeReport {
            report,
            conflicted: outcome.conflicted,
            fast_forward: outcome.fast_forward,
            up_to_date: outcome.up_to_date,
        })
    }

    /// Instala el hook `pre-commit` que corre `lodestar check`.
    pub fn install_hooks(&self) -> Result<std::path::PathBuf, WorkspaceError> {
        Ok(self
            .vcs
            .as_ref()
            .ok_or(WorkspaceError::NoVcs)?
            .lock()
            .unwrap()
            .install_hooks()?)
    }

    /// Historial de commits.
    pub fn vcs_log(&self, limit: usize) -> Result<Vec<CommitRow>, WorkspaceError> {
        match &self.vcs {
            Some(v) => Ok(v.lock().unwrap().log(limit)?),
            None => Ok(Vec::new()),
        }
    }

    /// Ramas locales.
    pub fn branches(&self) -> Result<Vec<Branch>, WorkspaceError> {
        match &self.vcs {
            Some(v) => Ok(v.lock().unwrap().branches()?),
            None => Ok(Vec::new()),
        }
    }

    /// Último commit conforme.
    pub fn last_conforming(&self) -> Result<Option<Sha>, WorkspaceError> {
        match &self.vcs {
            Some(v) => Ok(v.lock().unwrap().last_conforming()?),
            None => Ok(None),
        }
    }

    /// Commit del working tree. Niega el commit si hay un merge/rebase en curso (`§13.6.3`);
    /// regenera index/tags antes de commitear y devuelve la conformidad post-commit.
    pub fn commit(&self, msg: &str) -> Result<CommitOutcome, WorkspaceError> {
        // Regenera artefactos para que el commit sea coherente.
        self.generate_index("")?;
        self.generate_tags()?;
        let guard = self
            .vcs
            .as_ref()
            .ok_or(WorkspaceError::NoVcs)?
            .lock()
            .unwrap();
        if guard.repo_state() != lodestar_core::types::RepoState::Clean {
            return Err(WorkspaceError::RepoBusy);
        }
        let sha = guard.commit(msg, &self.identity)?;
        let conformance = guard.conformance(&sha)?;
        Ok(CommitOutcome { sha, conformance })
    }

    /// Restaura el working tree al árbol de un commit, por el único escritor.
    /// Si hay cambios sin commitear, primero hace un **commit de checkpoint** (`§13.6.1`).
    pub fn restore(&self, sha: &Sha) -> Result<ApplyReport, WorkspaceError> {
        let target_files = {
            let guard = self
                .vcs
                .as_ref()
                .ok_or(WorkspaceError::NoVcs)?
                .lock()
                .unwrap();
            // Checkpoint si hay trabajo sin commitear (no perder trabajo).
            if !guard.dirty_paths()?.is_empty() {
                guard.commit("Checkpoint automático antes de restaurar", &self.identity)?;
            }
            guard.tree_files(sha)?
        };
        // Computa la mutación (diff vs working tree actual) y aplica por el único escritor.
        let current = io::load_bundle(&self.root)?;
        let mutation = restore_mutation(&current, &target_files);
        let report = self.apply_mutation(&mutation)?;
        // Regenera index/tags tras aplicar.
        self.generate_index("")?;
        self.generate_tags()?;
        Ok(report)
    }

    /// Diff semántico del working tree vs HEAD (`OkfDiff` perezoso para el modo "Cambios").
    pub fn diff_working(&self) -> Result<OkfDiff, WorkspaceError> {
        let head_files = match &self.vcs {
            Some(v) => {
                let guard = v.lock().unwrap();
                match guard.log(1)?.first() {
                    Some(row) => guard.tree_files(&row.id)?,
                    None => FileMap::new(),
                }
            }
            None => FileMap::new(),
        };
        let working = io::load_bundle(&self.root)?;
        Ok(lodestar_core::diff::diff_snap(&head_files, &working))
    }

    /// `git pull --ff-only` (red por binario `git`).
    pub fn pull(&self) -> Result<SyncOutcome, WorkspaceError> {
        Ok(self
            .vcs
            .as_ref()
            .ok_or(WorkspaceError::NoVcs)?
            .lock()
            .unwrap()
            .pull()?)
    }

    /// `git push` al upstream configurado (red por binario `git`).
    pub fn push(&self) -> Result<SyncOutcome, WorkspaceError> {
        Ok(self
            .vcs
            .as_ref()
            .ok_or(WorkspaceError::NoVcs)?
            .lock()
            .unwrap()
            .push()?)
    }
}

/// Conteo de una aplicación de `Mutation`: el `--check` de CI sale de aquí.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ApplyReport {
    pub written: usize,
    pub removed: usize,
    pub unchanged: usize,
}

/// Resultado de un commit: el `Sha` + su conformidad post-commit.
#[derive(Debug, Clone)]
pub struct CommitOutcome {
    pub sha: Sha,
    pub conformance: CommitConformance,
}

/// Resultado de un merge por la workspace.
#[derive(Debug, Clone)]
pub struct MergeReport {
    /// Ficheros aplicados por el único escritor.
    pub report: ApplyReport,
    /// Paths con conflicto (llevan marcadores; hay que resolverlos antes de commitear).
    pub conflicted: Vec<RelPath>,
    /// `true` si fue fast-forward.
    pub fast_forward: bool,
    /// `true` si ya estaba al día.
    pub up_to_date: bool,
}

impl Workspace {
    /// Analiza el árbol de una revisión git (para `lodestar check --rev <REV>`).
    pub fn analyze_rev(&self, rev: &str) -> Result<Analysis, WorkspaceError> {
        let files = {
            let guard = self
                .vcs
                .as_ref()
                .ok_or(WorkspaceError::NoVcs)?
                .lock()
                .unwrap();
            let sha = guard.resolve_rev(rev)?;
            guard.tree_files(&sha)?
        };
        Ok(Bundle::from_files(files).analyze().clone())
    }

    /// Analiza el árbol **staged** (para `lodestar check --staged`).
    pub fn analyze_staged(&self) -> Result<Analysis, WorkspaceError> {
        let files = self
            .vcs
            .as_ref()
            .ok_or(WorkspaceError::NoVcs)?
            .lock()
            .unwrap()
            .staged_files()?;
        Ok(Bundle::from_files(files).analyze().clone())
    }
}

/// Computa la `Mutation` para llevar `current` al estado de `target` (restore/switch/merge).
fn restore_mutation(current: &FileMap, target: &FileMap) -> Mutation {
    let mut writes = std::collections::BTreeMap::new();
    for (path, content) in target {
        if current.get(path) != Some(content) {
            writes.insert(path.clone(), content.clone());
        }
    }
    let deletes = current
        .keys()
        .filter(|p| !target.contains_key(*p))
        .cloned()
        .collect();
    Mutation { writes, deletes }
}

fn default_identity() -> Author {
    Author {
        name: "lodestar".to_string(),
        email: "lodestar@localhost".to_string(),
    }
}
