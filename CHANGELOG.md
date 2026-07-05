# Changelog

Todos los cambios notables de este proyecto se documentan en este archivo.

El formato se basa en [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/)
y el proyecto sigue [Versionado Semántico](https://semver.org/lang/es/).

## [No publicado]

## [0.1.0] - 2026-07-05

Primera versión con el producto completo de extremo a extremo: backend, escritorio
y pipeline de release multiplataforma.

### Añadido

- **Épicas E0–E8 completas**: workspace de Cargo con 7 crates + `src-tauri`,
  siguiendo las direcciones de dependencia ratificadas.
- **`lodestar-core` (puro)**: modelo OKF, conformidad (15 checks + `OKF-CONFLICT`),
  analyze, query, grafo, generadores (index/tags), export/import y diff semántico.
  `#![forbid(unsafe_code)]`. Arnés diferencial JS-vs-Rust como oráculo de paridad
  frente al prototipo (6 fixtures).
- **`lodestar-store`**: cache SQLite/FTS5 (dueña única del DDL de `.lodestar/index.db`),
  cold rebuild, watcher `notify` con gate por hash blake3, síntesis SQL de
  backlinks/orphans/dangling/blast-radius y bus de eventos (`IndexEvent`).
- **`lodestar-vcs`**: git con transporte híbrido — libgit2 vendored para lo local
  (sin correr hooks) y binario `git` confinado a la red (push/pull/fetch); ramas
  locales, merge a nivel de árbol (`merge_trees`) con marcadores de conflicto,
  hooks (`pre-commit` → `lodestar check`) y cache de conformidad por tree-oid.
- **`lodestar-workspace`**: glue que compone core+store+vcs, handle unificado,
  **único escritor** (escritura atómica temp+rename), snapshot, commit/restore,
  switch/merge y bus de eventos en vivo (`open_live`/`enable_cache`/`subscribe`).
- **`lodestar-cli`**: `check` (humano/`--json`/`--sarif`, la puerta de CI con exit
  codes congelados 0/1/2/3/4), `init`, `index`/`tags` (`--check` → drift), `export`/
  `import`, `reindex` y git (`log`/`last-conforming`/`branch`/`switch`/`merge`/
  `pull`/`push`/`hooks`).
- **`lodestar-mcp`**: servidor MCP JSON-RPC por stdio (stdout puro) con 13 tools
  y test golden cross-fachada (salida de cada tool == `Workspace` directo).
- **Escritorio (Tauri v2 + Svelte 5)**: fachada con la tabla de comandos congelados
  sobre `Workspace` + forwarder del bus `IndexEvent` → evento `bundle:changed`
  (UI en vivo). Frontend funcional: layout de tres columnas colapsables, árbol
  filtrable, editor multi-escritor con diagnósticos localizados, panel de enlaces,
  isla imperativa del grafo (`createStarMap`) y modo «Cambios» (diff + commit).
- **Editor CodeMirror 6**: resaltado de sintaxis y autocompletado de enlaces
  (sustituye al textarea plano).
- **Vista Welcome**: reapertura del último workspace, tipo libre al crear conceptos
  y timestamp en `create_concept`.
- **Icono de escritorio** con la estrella dorada de la marca.
- **Pipeline de release multiplataforma** (`release.yml`): compila macOS Apple
  Silicon (arm64), Windows y Linux, y publica un GitHub Release en borrador con los
  bundles (dmg/deb/appimage/nsis) y los binarios de CLI/MCP. Bundles **sin firmar**
  (la firma/notarización queda diferida — ver `DECISIONES.md`).
- **CI multiplataforma**: el job de Rust (fmt/clippy/build/test/doc) corre en Linux,
  macOS y Windows; se mantienen los jobs `core-purity` y `frontend`.

### Cambiado

- **Heading por defecto de los conceptos**: ahora `# {Tipo} - {Nombre}` (antes
  `# Resumen`).

[No publicado]: https://github.com/dbareagimeno/lodestar/compare/v0.1.0...HEAD
[0.1.0]: https://github.com/dbareagimeno/lodestar/releases/tag/v0.1.0
