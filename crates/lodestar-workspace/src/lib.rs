//! `lodestar-workspace` — glue que compone core + store + vcs. **Scaffold de E5.**
//!
//! Será el handle unificado `Workspace`: único escritor (escritura atómica temp+rename),
//! dueño del único watcher por proceso y del bus de eventos. Se implementa en la épica E5.
