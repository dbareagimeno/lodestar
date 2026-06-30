//! `lodestar-vcs` — dueño único de git (`ARCHITECTURE.md §13`).
//!
//! Transporte híbrido: **libgit2** para todo lo local (discover/init/status/log/diff/commit/branch),
//! **binario `git`** confinado a la red (push/pull/fetch). `git2::Oid` NUNCA cruza la frontera:
//! se expone `Sha`/`Branch`/`CommitRow` de `lodestar_core::types`. **vcs no escribe el working tree**
//! en operaciones locales: `tree_files` devuelve file-maps que la workspace aplica por el único escritor.

use std::path::{Path, PathBuf};

use git2::{Repository, StatusOptions};
use lodestar_core::types::{
    Author, Branch, CommitConformance, CommitRow, FileMap, RelPath, RepoState, Sha, SyncKind,
    SyncOutcome,
};
use lodestar_core::Bundle;

mod error;
mod net;

pub use error::VcsError;

/// Handle de git de un bundle. `git2::Repository` es `!Sync` → la workspace lo guarda tras `Mutex<Vcs>`.
pub struct Vcs {
    repo: Repository,
    root: PathBuf,
}

impl Vcs {
    /// Descubre el repo del bundle SIN enganchar un repo ancestro (techo en el root).
    ///
    /// `Repository::open` no busca hacia arriba (a diferencia de `discover`), así que un `.git` en
    /// `~/` nunca se engancha. Tres estados: `None` = sin repo · vacío · con historial.
    pub fn discover(root: &Path) -> Result<Option<Vcs>, VcsError> {
        match Repository::open(root) {
            Ok(repo) => Ok(Some(Vcs {
                repo,
                root: root.to_path_buf(),
            })),
            Err(e) if e.code() == git2::ErrorCode::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Inicializa un repo nuevo: `git init` + `.gitignore` (incluye `.lodestar/`) + commit inicial.
    pub fn init(root: &Path, author: &Author) -> Result<Vcs, VcsError> {
        let repo = Repository::init(root)?;
        let gitignore = root.join(".gitignore");
        if !gitignore.exists() {
            std::fs::write(&gitignore, "/.lodestar/\n*.db\n*.db-shm\n*.db-wal\n")
                .map_err(|e| VcsError::Io(e.to_string()))?;
        }
        let vcs = Vcs {
            repo,
            root: root.to_path_buf(),
        };
        vcs.commit("Commit inicial", author)?;
        Ok(vcs)
    }

    /// El root del bundle (working tree).
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Estado del repo: detecta merge/rebase en curso (`§13.6.3`).
    pub fn repo_state(&self) -> RepoState {
        match self.repo.state() {
            git2::RepositoryState::Merge => RepoState::Merging,
            git2::RepositoryState::Rebase
            | git2::RepositoryState::RebaseInteractive
            | git2::RepositoryState::RebaseMerge => RepoState::Rebasing,
            git2::RepositoryState::CherryPick | git2::RepositoryState::CherryPickSequence => {
                RepoState::CherryPicking
            }
            git2::RepositoryState::Revert | git2::RepositoryState::RevertSequence => {
                RepoState::Reverting
            }
            _ => RepoState::Clean,
        }
    }

    /// Conjunto de paths con cambios sin commitear (working tree dirty vs HEAD).
    pub fn dirty_paths(&self) -> Result<Vec<RelPath>, VcsError> {
        let mut opts = StatusOptions::new();
        opts.include_untracked(true).include_ignored(false);
        let statuses = self.repo.statuses(Some(&mut opts))?;
        let mut out = Vec::new();
        for entry in statuses.iter() {
            if let Some(p) = entry.path() {
                if let Ok(rp) = RelPath::new(p) {
                    out.push(rp);
                }
            }
        }
        Ok(out)
    }

    /// Historial de commits (metadatos baratos por revwalk; `conformance = None`).
    pub fn log(&self, limit: usize) -> Result<Vec<CommitRow>, VcsError> {
        let mut revwalk = self.repo.revwalk()?;
        if revwalk.push_head().is_err() {
            return Ok(Vec::new()); // repo sin commits
        }
        let mut out = Vec::new();
        for oid in revwalk.take(limit) {
            let oid = oid?;
            let commit = self.repo.find_commit(oid)?;
            out.push(commit_row(&commit));
        }
        Ok(out)
    }

    /// Materializa el árbol de un commit a un `FileMap` (solo `.md` UTF-8; binarios se saltan). No toca el working tree.
    pub fn tree_files(&self, sha: &Sha) -> Result<FileMap, VcsError> {
        let oid = git2::Oid::from_str(sha.as_str()).map_err(VcsError::from)?;
        let commit = self.repo.find_commit(oid)?;
        let tree = commit.tree()?;
        let mut files = FileMap::new();
        tree.walk(git2::TreeWalkMode::PreOrder, |dir, entry| {
            if entry.kind() == Some(git2::ObjectType::Blob) {
                let name = entry.name().unwrap_or("");
                if name.ends_with(".md") {
                    let full = format!("{dir}{name}");
                    if let Ok(rp) = RelPath::new(&full) {
                        if let Ok(blob) = entry.to_object(&self.repo).and_then(|o| o.peel_to_blob())
                        {
                            if let Ok(text) = std::str::from_utf8(blob.content()) {
                                files.insert(rp, text.to_string());
                            }
                        }
                    }
                }
            }
            git2::TreeWalkResult::Ok
        })?;
        Ok(files)
    }

    /// Conformidad de un commit = `analyze` sobre su árbol (la pieza estrella, `§13.4`).
    /// Se cachea cruda (sin strictness); aquí se computa, la cache por tree-oid vive en `store`.
    pub fn conformance(&self, sha: &Sha) -> Result<CommitConformance, VcsError> {
        let files = self.tree_files(sha)?;
        let bundle = Bundle::from_files(files);
        let a = bundle.analyze();
        Ok(CommitConformance {
            hard_fail: a.hard_fail,
            warn_count: a.warn_count,
            conform: a.hard_fail == 0,
        })
    }

    /// Último commit cuyo árbol pasa la puerta OKF (barrido early-exit hacia atrás).
    pub fn last_conforming(&self) -> Result<Option<Sha>, VcsError> {
        for row in self.log(1000)? {
            if self.conformance(&row.id)?.conform {
                return Ok(Some(row.id));
            }
        }
        Ok(None)
    }

    /// Stage del working tree + commit. Por libgit2 → **no** corre hooks ni firma (`§13.8`).
    pub fn commit(&self, msg: &str, author: &Author) -> Result<Sha, VcsError> {
        let mut index = self.repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_oid = index.write_tree()?;
        let tree = self.repo.find_tree(tree_oid)?;
        let sig = git2::Signature::now(&author.name, &author.email)?;
        let parents = match self.repo.head().ok().and_then(|h| h.target()) {
            Some(oid) => vec![self.repo.find_commit(oid)?],
            None => Vec::new(),
        };
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        let oid = self
            .repo
            .commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)?;
        Sha::new(&oid.to_string()).map_err(VcsError::from)
    }

    /// Ramas locales con ahead/behind vs upstream.
    pub fn branches(&self) -> Result<Vec<Branch>, VcsError> {
        let head_name = self.current_branch();
        let mut out = Vec::new();
        for b in self.repo.branches(Some(git2::BranchType::Local))? {
            let (branch, _) = b?;
            let name = match branch.name()? {
                Some(n) => n.to_string(),
                None => continue,
            };
            let (upstream, ahead, behind) = self.tracking(&branch);
            out.push(Branch {
                is_head: Some(name.as_str()) == head_name.as_deref(),
                name,
                upstream,
                ahead,
                behind,
            });
        }
        Ok(out)
    }

    fn tracking(&self, branch: &git2::Branch) -> (Option<String>, usize, usize) {
        let upstream = match branch.upstream() {
            Ok(up) => up,
            Err(_) => return (None, 0, 0),
        };
        let up_name = upstream.name().ok().flatten().map(|s| s.to_string());
        let local_oid = branch.get().target();
        let up_oid = upstream.get().target();
        match (local_oid, up_oid) {
            (Some(l), Some(u)) => {
                let (a, b) = self.repo.graph_ahead_behind(l, u).unwrap_or((0, 0));
                (up_name, a, b)
            }
            _ => (up_name, 0, 0),
        }
    }

    /// La rama actual; HEAD desacoplado = `None`.
    pub fn current_branch(&self) -> Option<String> {
        let head = self.repo.head().ok()?;
        if head.is_branch() {
            head.shorthand().map(|s| s.to_string())
        } else {
            None
        }
    }

    /// Crea una rama (no toca el working tree).
    pub fn create_branch(&self, name: &str, from: Option<&Sha>) -> Result<(), VcsError> {
        let target_oid = match from {
            Some(sha) => git2::Oid::from_str(sha.as_str())?,
            None => self.repo.head()?.target().ok_or(VcsError::NoHead)?,
        };
        let commit = self.repo.find_commit(target_oid)?;
        self.repo.branch(name, &commit, false)?;
        Ok(())
    }

    /// `git pull --ff-only` vía binario `git` (red confinada). Nunca conflicta in-app.
    pub fn pull(&self) -> Result<SyncOutcome, VcsError> {
        net::run_git(&self.root, &["pull", "--ff-only"], SyncKind::Pull)
    }

    /// `git push` al upstream configurado vía binario `git`. Rechazo (non-ff) → `ok:false`.
    pub fn push(&self) -> Result<SyncOutcome, VcsError> {
        net::run_git(&self.root, &["push"], SyncKind::Push)
    }
}

/// Construye un `CommitRow` (sin conformidad) a partir de un commit de git2.
fn commit_row(commit: &git2::Commit) -> CommitRow {
    let id = Sha::new(&commit.id().to_string()).expect("oid es hex válido");
    let author = commit.author();
    CommitRow {
        short: id.short(),
        id,
        message: commit.message().unwrap_or("").trim_end().to_string(),
        author: Author {
            name: author.name().unwrap_or("").to_string(),
            email: author.email().unwrap_or("").to_string(),
        },
        time_unix: commit.time().seconds(),
        parents: commit
            .parent_ids()
            .map(|oid| Sha::new(&oid.to_string()).expect("oid es hex válido"))
            .collect(),
        conformance: None,
    }
}
