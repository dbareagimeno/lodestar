# Changelog

Todos los cambios notables de este proyecto se documentan en este archivo.

El formato se basa en [Keep a Changelog](https://keepachangelog.com/es-ES/1.1.0/)
y el proyecto sigue [Versionado Semántico](https://semver.org/lang/es/).

## [No publicado]

## [0.2.0] - 2026-07-23

**Giro a motor headless de integridad semántica** (`ARCHITECTURE.md §19`, ratificado el
2026-07-22; épicas E9–E14). lodestar deja de ser un «editor local-first con git de
primera clase» y pasa a ser un **motor headless** consumido por agentes vía MCP/CLI:
sin GUI y sin git en la superficie. El giro fue **aditivo, no destructivo** — retira
exposición, no capacidad.

### Añadido

- **Superficie MCP objetivo: 10 tools** (`§19.6`) — `workspace_status`,
  `knowledge_search`, `knowledge_get`, `schema_inspect`, `graph_query`,
  `impact_analyze`, `knowledge_check`, `change_plan`, `change_apply`, `change_revert`,
  todas con `outputSchema` (schemars). Perfiles `--profile readonly|standard`:
  `readonly` oculta **y** rechaza las tres tools de cambio. `instructions` de servidor
  para orientar al agente.
- **Modelo transaccional recuperable** (E12–E13): `change_plan` (normaliza, simula y
  valida sin escribir, con `planHash`, `SemanticDiff`, `RiskAssessment` y
  `ValidationReport`) → `change_apply` (staging → lock → backup → write-ahead journal →
  renames atómicos → `ChangeReceipt`) → `change_revert`. **Crash-recovery determinista**
  desde el journal, retención/GC de recibos y auditoría en
  `.lodestar/runtime/audit.jsonl`.
- **Crate `lodestar-app`**: capa de servicios de caso de uso compartida por CLI y MCP
  (envelope de respuesta, 16 `ErrorCode`, cero lógica de dominio).
- **Esquema del bundle** (`core::schema` + loader `.lodestar/schema.yaml`): validación
  schema-driven (`SCHEMA-REQFIELD`, `SCHEMA-STATUS`) y relaciones tipadas
  (`REL-TARGET`, `REL-CARD`, `REL-TYPE`), aditivas sobre los checks existentes.
- **Identidad determinista**: `ConceptRevision`/`WorkspaceRevision` y `ConceptRef`
  (identidad por path), con `resolve_ref`.
- **Grafo e impacto**: `graph_query` consolida las cuatro tools de grafo previas y suma
  `path_between`, `cycles` y `components`; `impact_analyze` cierra E11.
- **Configuración y separación canónico/runtime**: `.lodestar/config.yaml`
  (`WorkspaceConfig`) y `.lodestar/runtime/` (planes, recibos, journal, auditoría)
  fuera de lo canónico y gitignorado. Validación de paths externos (`referenceRoots`).
- **Verificación end-to-end**: benchmark funcional de los 15 escenarios de `§17`,
  cobertura e2e de convivencia con otro software escribiendo el bundle, y arnés de
  escala (~10k conceptos) con presupuesto de métricas.
- **Estructura de agentes y skills** en `.claude/` (SDD · TDD · BDD · jueces ciegos ·
  guardián de contrato) con el planificador de épicas.

### Cambiado

- **`lodestar check` es la puerta de CI sobre el working tree** con conformidad
  completa schema-driven (OKF + schema + refs). Exit codes congelados (0/1/2/3/4) sin
  cambios.
- **`change_apply` auto-regenera `index` y `tags`** dentro de la transacción, de modo
  que el bundle publicado nunca queda en drift de generadores.
- **`contracts/mcp.yml` reescrito** contra la superficie de 10 tools; la superficie
  heredada queda documentada en su `§15`.

### Eliminado

- **UI de escritorio fuera de `main`**: `frontend/` (Svelte 5) y `src-tauri/` se
  movieron íntegros a la rama `experimental/ui-desktop`. El pipeline de release ya no
  publica bundles de escritorio (dmg/deb/appimage/nsis), solo los binarios de CLI y
  MCP. Con ellos desaparecen el espejo de tipos TS y el circuito UX (`/ux`,
  `disenador-ux`).
- **git fuera de la superficie**: retirados los subcomandos `log`, `last-conforming`,
  `branch`, `switch`, `merge`, `pull`, `push` y `hooks` de la CLI, los flags
  `--staged`/`--rev`/`--range` de `check`, y las tools git del MCP. El crate
  `lodestar-vcs` **se conserva dormido** (compila, tests verdes, ninguna fachada lo
  invoca) por si git vuelve a la superficie.
- **Tools MCP heredadas**: `query`, `conformance_check`, `find_*`, `neighborhood`,
  `create_concept`, `update_frontmatter` y `generate_*`, sustituidas por las 10 tools
  objetivo.

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

[No publicado]: https://github.com/dbareagimeno/lodestar/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/dbareagimeno/lodestar/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/dbareagimeno/lodestar/releases/tag/v0.1.0
