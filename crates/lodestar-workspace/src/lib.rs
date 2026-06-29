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
use std::sync::Mutex;

use lodestar_core::diff::OkfDiff;
use lodestar_core::types::{
    Analysis, Author, Backlinks, Branch, CommitConformance, CommitRow, Direction, FileMap,
    FrontmatterPatch, GraphModel, Mutation, Neighborhood, RelPath, Sha, SyncOutcome, WriteOutcome,
};
use lodestar_core::Bundle;
use lodestar_vcs::Vcs;

mod error;
mod io;
mod snapshot;

pub use error::WorkspaceError;
pub use snapshot::BundleSnapshot;

/// Handle unificado de un bundle abierto.
pub struct Workspace {
    root: PathBuf,
    vcs: Option<Mutex<Vcs>>,
    identity: Author,
}

impl Workspace {
    /// Abre un bundle: descubre git (puede no haber). Identidad por defecto (override en E8-H01).
    pub fn open(root: &Path) -> Result<Self, WorkspaceError> {
        let vcs = Vcs::discover(root)?.map(Mutex::new);
        Ok(Workspace {
            root: root.to_path_buf(),
            vcs,
            identity: default_identity(),
        })
    }

    /// Abre sin git (modo hermético, p. ej. CLI efímera).
    pub fn open_ephemeral(root: &Path) -> Result<Self, WorkspaceError> {
        Ok(Workspace {
            root: root.to_path_buf(),
            vcs: None,
            identity: default_identity(),
        })
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
        }
        Ok(outcome)
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
                written += 1;
            }
        }
        let mut removed = 0;
        for path in &mutation.deletes {
            if self.root.join(path.as_str()).exists() {
                io::delete(&self.root, path)?;
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

    /// Conformidad del HEAD actual.
    pub fn conformance(&self) -> Result<Option<CommitConformance>, WorkspaceError> {
        let guard = match &self.vcs {
            Some(v) => v.lock().unwrap(),
            None => return Ok(None),
        };
        match guard.log(1)?.first() {
            Some(row) => Ok(Some(guard.conformance(&row.id)?)),
            None => Ok(None),
        }
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
