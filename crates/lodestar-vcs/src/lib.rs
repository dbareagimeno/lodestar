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

    /// Resuelve una revisión (`HEAD`, nombre de rama, `SHA` corto/largo, `HEAD~2`…) a un `Sha` de commit.
    /// Habilita `lodestar check --rev <REV>` (`§7.3`).
    pub fn resolve_rev(&self, rev: &str) -> Result<Sha, VcsError> {
        let obj = self.repo.revparse_single(rev)?;
        let commit = obj.peel_to_commit()?;
        Sha::new(&commit.id().to_string()).map_err(VcsError::from)
    }

    /// El `tree_oid` (hex) del árbol de un commit — clave de la cache de conformidad (`§10` fila 20).
    pub fn tree_oid(&self, sha: &Sha) -> Result<String, VcsError> {
        let oid = git2::Oid::from_str(sha.as_str())?;
        let commit = self.repo.find_commit(oid)?;
        Ok(commit.tree_id().to_string())
    }

    /// El `FileMap` del **índice** (árbol staged), para `lodestar check --staged` (`§7.3`).
    /// Lee los blobs staged directamente del index; no toca HEAD ni el working tree.
    /// Con conflictos sin resolver devuelve error: un veredicto sobre «theirs gana» sería
    /// una luz verde falsa en pleno merge (esquivaría `OKF-CONFLICT`).
    pub fn staged_files(&self) -> Result<FileMap, VcsError> {
        let index = self.repo.index()?;
        if index.has_conflicts() {
            return Err(VcsError::IndexConflicts);
        }
        let mut files = FileMap::new();
        for entry in index.iter() {
            // Solo stage 0 (entradas normales); 1/2/3 son restos de conflicto.
            if (entry.flags >> 12) & 0x3 != 0 {
                continue;
            }
            let path = match std::str::from_utf8(&entry.path) {
                Ok(p) => p,
                Err(_) => continue,
            };
            if !path.ends_with(".md") {
                continue;
            }
            let Ok(rp) = RelPath::new(path) else { continue };
            if let Ok(blob) = self.repo.find_blob(entry.id) {
                if let Ok(text) = std::str::from_utf8(blob.content()) {
                    files.insert(rp, text.to_string());
                }
            }
        }
        Ok(files)
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

    /// Último commit cuyo árbol pasa la puerta OKF (barrido early-exit hacia atrás, historial completo).
    pub fn last_conforming(&self) -> Result<Option<Sha>, VcsError> {
        for row in self.log(usize::MAX)? {
            if self.conformance(&row.id)?.conform {
                return Ok(Some(row.id));
            }
        }
        Ok(None)
    }

    /// Stage del working tree + commit. Por libgit2 → **no** corre hooks ni firma (`§13.8`).
    ///
    /// Consciente del merge: si hay `MERGE_HEAD` (merge 3-vías pendiente), el commit lleva
    /// **dos padres** (ratifica §13.6.3) y limpia el estado de merge. Y detecta no-ops: si el
    /// árbol no cambió y no hay merge que concluir, devuelve el HEAD actual sin crear un
    /// commit vacío (evita los «checkpoints» espurios que ensucian el historial).
    pub fn commit(&self, msg: &str, author: &Author) -> Result<Sha, VcsError> {
        let mut index = self.repo.index()?;
        index.add_all(["*"].iter(), git2::IndexAddOption::DEFAULT, None)?;
        index.write()?;
        let tree_oid = index.write_tree()?;
        let tree = self.repo.find_tree(tree_oid)?;
        let sig = git2::Signature::now(&author.name, &author.email)?;
        let head_oid = self.repo.head().ok().and_then(|h| h.target());
        let mut parents = match head_oid {
            Some(oid) => vec![self.repo.find_commit(oid)?],
            None => Vec::new(),
        };
        // Segundo padre: el MERGE_HEAD de un merge 3-vías pendiente.
        let merge_parent = self
            .repo
            .find_reference("MERGE_HEAD")
            .ok()
            .and_then(|r| r.target());
        if let Some(oid) = merge_parent {
            if Some(oid) != head_oid {
                parents.push(self.repo.find_commit(oid)?);
            }
        }
        // No-op: árbol idéntico al del HEAD y sin merge que concluir → no crear commit vacío.
        if merge_parent.is_none() {
            if let Some(oid) = head_oid {
                let head_commit = self.repo.find_commit(oid)?;
                if head_commit.tree_id() == tree_oid {
                    return Sha::new(&oid.to_string()).map_err(VcsError::from);
                }
            }
        }
        let parent_refs: Vec<&git2::Commit> = parents.iter().collect();
        let oid = self
            .repo
            .commit(Some("HEAD"), &sig, &sig, msg, &tree, &parent_refs)?;
        if merge_parent.is_some() {
            // Concluye el merge: borra MERGE_HEAD/MERGE_MSG (si no, el repo queda «Merging» eterno).
            self.repo.cleanup_state()?;
        }
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

    /// Cambia de rama **sin escribir el working tree**: mueve `HEAD` a `refs/heads/<name>` y
    /// devuelve el `FileMap` de destino para que la workspace lo aplique por el único escritor
    /// (`§16`: switch/merge no pierden trabajo → la workspace hace checkpoint antes).
    pub fn switch(&self, name: &str) -> Result<FileMap, VcsError> {
        let branch = self.repo.find_branch(name, git2::BranchType::Local)?;
        let oid = branch.get().target().ok_or(VcsError::NoHead)?;
        let sha = Sha::new(&oid.to_string())?;
        let files = self.tree_files(&sha)?;
        self.repo
            .set_head(&format!("refs/heads/{name}"))
            .map_err(VcsError::from)?;
        // Sincroniza el index de git con el árbol destino (SIN tocar el working tree): si no,
        // todo fichero que difiera entre ramas aparecería «dirty» tras el switch aunque el
        // working tree == HEAD, y el siguiente switch dispararía un checkpoint espurio.
        self.sync_index_to(oid)?;
        Ok(files)
    }

    /// Alinea el index de git con el árbol de un commit (no escribe el working tree).
    fn sync_index_to(&self, commit_oid: git2::Oid) -> Result<(), VcsError> {
        let tree = self.repo.find_commit(commit_oid)?.tree()?;
        let mut index = self.repo.index()?;
        index.read_tree(&tree)?;
        index.write()?;
        Ok(())
    }

    /// Merge (local) de la rama `name` en la rama actual. **No escribe el working tree**: devuelve
    /// el `FileMap` resultante para que la workspace lo aplique. En conflicto, los ficheros llevan
    /// marcadores `<<<<<<<`/`=======`/`>>>>>>>` (los captura `OKF-CONFLICT`) y deja `MERGE_HEAD`
    /// escrito → `repo_state()` = `Merging` bloquea el commit hasta resolver (`§13.6.3`).
    pub fn merge(&self, name: &str) -> Result<MergeOutcome, VcsError> {
        let their_branch = self.repo.find_branch(name, git2::BranchType::Local)?;
        let their_oid = their_branch.get().target().ok_or(VcsError::NoHead)?;
        let our_oid = self.repo.head()?.target().ok_or(VcsError::NoHead)?;

        let their_annotated = self.repo.find_annotated_commit(their_oid)?;
        let (analysis, _) = self.repo.merge_analysis(&[&their_annotated])?;

        if analysis.is_up_to_date() {
            let sha = Sha::new(&our_oid.to_string())?;
            return Ok(MergeOutcome {
                files: self.tree_files(&sha)?,
                conflicted: Vec::new(),
                fast_forward: false,
                up_to_date: true,
            });
        }

        if analysis.is_fast_forward() {
            // Mueve la rama actual a theirs (ff) y devuelve su árbol. Con HEAD desacoplado no
            // hay rama que mover: reportar «éxito» sin mover nada dejaría al usuario creyendo
            // que mergeó cuando la historia no registró nada.
            let Some(branch_name) = self.current_branch() else {
                return Err(VcsError::DetachedHead);
            };
            self.repo.reference(
                &format!("refs/heads/{branch_name}"),
                their_oid,
                true,
                "merge fast-forward",
            )?;
            self.sync_index_to(their_oid)?;
            let sha = Sha::new(&their_oid.to_string())?;
            return Ok(MergeOutcome {
                files: self.tree_files(&sha)?,
                conflicted: Vec::new(),
                fast_forward: true,
                up_to_date: false,
            });
        }

        // Merge de tres vías a nivel de árbol (no escribe el working tree).
        let our_commit = self.repo.find_commit(our_oid)?;
        let their_commit = self.repo.find_commit(their_oid)?;
        let base_oid = self.repo.merge_base(our_oid, their_oid)?;
        let base_commit = self.repo.find_commit(base_oid)?;
        let merged_index = self.repo.merge_trees(
            &base_commit.tree()?,
            &our_commit.tree()?,
            &their_commit.tree()?,
            None,
        )?;

        let mut files = FileMap::new();
        // Entradas sin conflicto (stage 0). Los stages 1/2/3 se manejan como conflicto abajo.
        for entry in merged_index.iter() {
            let stage = (entry.flags >> 12) & 0x3;
            if stage != 0 {
                continue;
            }
            let path = match std::str::from_utf8(&entry.path) {
                Ok(p) if p.ends_with(".md") => p,
                _ => continue,
            };
            let Ok(rp) = RelPath::new(path) else { continue };
            if let Ok(blob) = self.repo.find_blob(entry.id) {
                if let Ok(text) = std::str::from_utf8(blob.content()) {
                    files.insert(rp, text.to_string());
                }
            }
        }

        // Conflictos: sintetiza contenido con marcadores para que OKF-CONFLICT lo detecte.
        let mut conflicted = Vec::new();
        for c in merged_index.conflicts()? {
            let c = c?;
            let our = c.our.as_ref();
            let their = c.their.as_ref();
            let path_bytes = our
                .map(|e| e.path.clone())
                .or_else(|| their.map(|e| e.path.clone()))
                .unwrap_or_default();
            let path = match std::str::from_utf8(&path_bytes) {
                Ok(p) if p.ends_with(".md") => p.to_string(),
                _ => continue,
            };
            let Ok(rp) = RelPath::new(&path) else {
                continue;
            };
            let read = |e: Option<&git2::IndexEntry>| -> String {
                e.and_then(|e| self.repo.find_blob(e.id).ok())
                    .and_then(|b| std::str::from_utf8(b.content()).ok().map(String::from))
                    .unwrap_or_default()
            };
            let content = format!(
                "<<<<<<< HEAD\n{}=======\n{}>>>>>>> {}\n",
                ensure_nl(&read(our)),
                ensure_nl(&read(their)),
                name
            );
            files.insert(rp.clone(), content);
            conflicted.push(rp);
        }

        // Deja MERGE_HEAD: el commit que concluya el merge llevará 2 padres (§13.6.3). La ruta
        // sale de `repo.path()` (no de `root/.git`): con worktrees o `.git`-fichero difieren.
        let merge_head = self.repo.path().join("MERGE_HEAD");
        std::fs::write(&merge_head, format!("{their_oid}\n"))
            .map_err(|e| VcsError::Io(e.to_string()))?;

        conflicted.sort();
        Ok(MergeOutcome {
            files,
            conflicted,
            fast_forward: false,
            up_to_date: false,
        })
    }

    /// Instala un hook `pre-commit` que corre `lodestar check --staged` (`§13.5`: juzga el
    /// índice staged, no el working sucio). Idempotente; sobrescribe el hook gestionado.
    pub fn install_hooks(&self) -> Result<PathBuf, VcsError> {
        let hooks_dir = self.repo.path().join("hooks");
        std::fs::create_dir_all(&hooks_dir).map_err(|e| VcsError::Io(e.to_string()))?;
        let hook = hooks_dir.join("pre-commit");
        let script = "#!/bin/sh\n\
             # Hook gestionado por lodestar: bloquea commits no conformes (OKF).\n\
             exec lodestar check --staged\n";
        std::fs::write(&hook, script).map_err(|e| VcsError::Io(e.to_string()))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&hook)
                .map_err(|e| VcsError::Io(e.to_string()))?
                .permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&hook, perms).map_err(|e| VcsError::Io(e.to_string()))?;
        }
        Ok(hook)
    }

    /// Garantiza que la cache `.lodestar/` esté ignorada. En un repo **adoptado** (con
    /// `.gitignore` propio sin `.lodestar/`), los commits/checkpoints acabarían metiendo
    /// `index.db` + `-wal` en la historia. Se añade a `.git/info/exclude` (no versionado,
    /// no invasivo con el `.gitignore` del usuario).
    pub fn ensure_cache_ignored(&self) -> Result<(), VcsError> {
        if self
            .repo
            .is_path_ignored(".lodestar/probe")
            .unwrap_or(false)
        {
            return Ok(());
        }
        let info = self.repo.path().join("info");
        std::fs::create_dir_all(&info).map_err(|e| VcsError::Io(e.to_string()))?;
        let exclude = info.join("exclude");
        let current = std::fs::read_to_string(&exclude).unwrap_or_default();
        if !current.lines().any(|l| l.trim() == "/.lodestar/") {
            let sep = if current.is_empty() || current.ends_with('\n') {
                ""
            } else {
                "\n"
            };
            std::fs::write(
                &exclude,
                format!("{current}{sep}# cache de lodestar (derivada/desechable)\n/.lodestar/\n"),
            )
            .map_err(|e| VcsError::Io(e.to_string()))?;
        }
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

/// Resultado de un merge local. `files` es el árbol a aplicar por el único escritor.
#[derive(Debug, Clone)]
pub struct MergeOutcome {
    /// El `FileMap` resultante (con marcadores de conflicto donde los haya).
    pub files: FileMap,
    /// Paths con conflicto (llevan marcadores; los captura `OKF-CONFLICT`).
    pub conflicted: Vec<RelPath>,
    /// `true` si fue un fast-forward.
    pub fast_forward: bool,
    /// `true` si ya estaba al día (nada que hacer).
    pub up_to_date: bool,
}

/// Garantiza que un fragmento acabe en `\n` (para los marcadores de conflicto).
fn ensure_nl(s: &str) -> String {
    if s.is_empty() || s.ends_with('\n') {
        s.to_string()
    } else {
        format!("{s}\n")
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
