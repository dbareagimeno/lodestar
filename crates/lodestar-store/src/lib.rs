//! `lodestar-store` — cache derivada (SQLite/FTS5) + watcher. **Scaffold de E3.**
//!
//! Dueño único del DDL en `<bundle>/.lodestar/index.db` (WAL, gitignored, reconstruible).
//! `rusqlite`/`notify`/`crossbeam` se añaden al implementar la épica E3. De momento solo
//! reserva la superficie para que el grafo de crates del `§3` compile.

#![doc(html_no_source)]
