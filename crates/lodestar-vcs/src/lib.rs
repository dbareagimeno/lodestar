//! `lodestar-vcs` — dueño único de git. **Scaffold de E4.**
//!
//! Transporte híbrido: libgit2 para lo local (status/log/diff/commit/branch/merge/restore/init)
//! + binario `git` confinado a la red (push/pull/fetch). `git2` se añade al implementar E4;
//! `git2::Oid` nunca cruza la frontera (se expone `Sha`/`Branch` de `lodestar_core::types`).
